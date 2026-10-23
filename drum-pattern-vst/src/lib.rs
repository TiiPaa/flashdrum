use nih_plug::prelude::*;
use nih_plug::{
    params::persist::{deserialize_field, serialize_field, PersistentField},
    wrapper::state::{ParamValue, PluginState},
};
use nih_plug_egui::EguiState;
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU16, AtomicU32, Ordering},
    Arc, TryLockError,
};

mod atomic_song;
mod config;
mod factory_presets;
mod fx_locks;
mod generator;
mod groove;
mod instrument_registry;
mod macros;
mod midi_export;
mod param_id;
mod paths;
mod pattern_bank;
mod perf_log;
mod plock;
mod preset_dumps;
mod presets;
mod sequencer;
mod sound_settings;
mod synthesis;
mod track;
mod ui;
mod user_textures;

use generator::{GeneratorType, Style};
use pattern_bank::SONG_BLOCKS;
use plock::{PersistentPlockState, PersistentSequencerPlockState};
use sequencer::{pattern::PersistentPattern, Pattern, Sequencer, SharedPattern};
use sound_settings::{PersistentSoundSettings, SoundSettingsState};
use synthesis::DrumSynthesizer;

// [254] Allocation watchdog, active only inside `assert_no_alloc(...)`
// closures in tests (forbid/permit counters are per-thread; inactive
// otherwise). Also registered in `test_standalone.rs`, which recompiles the
// same modules through `#[path]`.
#[cfg(test)]
#[global_allocator]
static ALLOC_DISABLER: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

/// [257] Crash journal. With `panic = "abort"` a panic kills the host DAW
/// without any trace; this chained hook appends one bounded line per panic
/// (`crash.log` in the temp dir, restarted past 256 Ko) carrying the build
/// ID, thread, location and message. Installed once, before any processing.
fn install_crash_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let path = std::env::temp_dir().join("flash-drum-crash.log");
            if let Ok(meta) = std::fs::metadata(&path) {
                if meta.len() > 256 * 1024 {
                    let _ = std::fs::remove_file(&path);
                }
            }
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                use std::io::Write;
                let location = info
                    .location()
                    .map(|l| format!("{}:{}", l.file(), l.line()))
                    .unwrap_or_default();
                let message = info
                    .payload()
                    .downcast_ref::<&str>()
                    .copied()
                    .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
                    .unwrap_or("(no message)");
                let _ = writeln!(
                    file,
                    "{:?} build={} thread={:?} {} {}",
                    std::time::SystemTime::now(),
                    BUILD_ID,
                    std::thread::current().id(),
                    location,
                    message
                );
            }
            previous(info);
        }));
    });
}

const VST3_CLASS_ID: [u8; 16] = *b"DrumFlashPlugin1";
pub(crate) const BUILD_ID: &str = match option_env!("DRUM_PATTERN_BUILD_ID") {
    Some(build_id) => build_id,
    None => "dev",
};
/// Number of dedicated stereo aux outputs. Changed to 14 for the modular grid.
const AUX_OUT_COUNT: usize = 14;
const OUTPUT_PORT_NAMES: [&str; AUX_OUT_COUNT] = [
    "Out 1", "Out 2", "Out 3", "Out 4", "Out 5", "Out 6", "Out 7", "Out 8", "Out 9", "Out 10",
    "Out 11", "Out 12", "Out 13", "Out 14",
];
const STEP_COUNT: usize = 64;

const PATTERN_STATE_FIELD: &str = "pattern-v5";
/// [282] Key of `DrumFlashParams::instance_seeded` (same literal as its
/// `#[persist]` attribute; a test checks they agree).
const INSTANCE_SEEDED_FIELD: &str = "instance-seeded-v1";

#[inline]
fn add_stereo_aux_sample(
    channels: &mut [&mut [f32]],
    sample_idx: usize,
    left: f32,
    right: f32,
) -> bool {
    if channels.len() < 2 {
        return false;
    }

    let (left_channels, right_channels) = channels.split_at_mut(1);
    let left_channel = &mut left_channels[0];
    let right_channel = &mut right_channels[0];
    if sample_idx >= left_channel.len() || sample_idx >= right_channel.len() {
        return false;
    }

    left_channel[sample_idx] += left;
    right_channel[sample_idx] += right;
    true
}

/// [269] Does a step's condition (its sequencer p-lock) let it fire on this
/// loop? No sequencer p-lock = always.
fn step_condition_passes(
    seq_params: Option<crate::plock::SequencerStepParams>,
    loop_count: usize,
) -> bool {
    if let Some(sp) = seq_params {
        use crate::plock::StepCondition::*;
        // [218] "Not" inverts the condition. `Always` is
        // immune: inverted it would mean "never", which is
        // what turning the step off already does.
        // [219] Both terms must hold when a second one is
        // set: "1/2 and 1/3" fires every sixth loop.
        let evaluate = |cond: crate::plock::StepCondition| match cond {
            Always => true,
            First => loop_count == 0,
            NotFirst => loop_count > 0,
            Half1 => loop_count % 2 == 0,
            Half2 => loop_count % 2 == 1,
            Third1 => loop_count % 3 == 0,
            Third2 => loop_count % 3 == 1,
            Third3 => loop_count % 3 == 2,
            Fourth1 => loop_count % 4 == 0,
            Fourth2 => loop_count % 4 == 1,
            Fourth3 => loop_count % 4 == 2,
            Fourth4 => loop_count % 4 == 3,
        };
        let passes = evaluate(sp.condition) && sp.condition_and.map(evaluate).unwrap_or(true);
        // `Not` inverts the whole expression, second term
        // included. Immune on a bare `Always`, where it
        // would mean "never".
        if sp.condition_negate && !(sp.condition == Always && sp.condition_and.is_none()) {
            !passes
        } else {
            passes
        }
    } else {
        true // No condition = always pass
    }
}

/// [269] Main mix of one sample: the slots' stereo outputs weighted by their
/// mix gains, times the master volume.
fn main_mix(
    voice_outputs: &[[f32; 2]; crate::track::MAX_TRACKS],
    mix_gains: &[f32; crate::track::MAX_TRACKS],
    master_vol: f32,
) -> (f32, f32) {
    let mixed_left = voice_outputs
        .iter()
        .enumerate()
        .map(|(i, o)| o[0] * mix_gains[i])
        .sum::<f32>()
        * master_vol;
    let mixed_right = voice_outputs
        .iter()
        .enumerate()
        .map(|(i, o)| o[1] * mix_gains[i])
        .sum::<f32>()
        * master_vol;
    (mixed_left, mixed_right)
}

/// [269] Silence every aux output before the block is rendered.
fn clear_aux_outputs(aux: &mut AuxiliaryBuffers) {
    for aux_buffer in aux.outputs.iter_mut() {
        for channel in aux_buffer.as_slice().iter_mut() {
            channel.fill(0.0);
        }
    }
}

/// [269] One sample of each slot routed to an aux out, added there.
fn write_aux_outputs(
    aux: &mut AuxiliaryBuffers,
    sample_idx: usize,
    track_routings: &[crate::track::TrackRouting; crate::track::MAX_TRACKS],
    voice_outputs: &[[f32; 2]; crate::track::MAX_TRACKS],
    master_vol: f32,
) {
    for (slot_idx, routing) in track_routings.iter().enumerate() {
        let crate::track::TrackAudioOut::Out(out_number) = routing.out_select else {
            continue;
        };
        let aux_idx = out_number.saturating_sub(1) as usize;
        let Some(aux_buffer) = aux.outputs.get_mut(aux_idx) else {
            continue;
        };
        let channels = aux_buffer.as_slice();
        add_stereo_aux_sample(
            channels,
            sample_idx,
            voice_outputs[slot_idx][0] * master_vol,
            voice_outputs[slot_idx][1] * master_vol,
        );
    }
}

/// Computes per-slot effective mutes and Main Mix gain factors.
///
/// - If at least one solo is active, every non-soloed slot is muted and the
///   soloed slots are unmuted, regardless of the mute buttons.
/// - If no solo is active, each slot follows its own mute button.
/// - Main Mix gains are `1.0` for slots whose routing includes the Main bus,
///   and `0.0` otherwise.
fn compute_mix_gating(
    mute_states: &[bool; crate::track::MAX_TRACKS],
    solo_states: &[bool; crate::track::MAX_TRACKS],
    main_mix_enabled: &[bool; crate::track::MAX_TRACKS],
) -> (
    [bool; crate::track::MAX_TRACKS],
    [f32; crate::track::MAX_TRACKS],
) {
    let any_solo_active = solo_states.iter().copied().any(|solo| solo);
    let effective_mutes = std::array::from_fn(|index| {
        if any_solo_active {
            !solo_states[index]
        } else {
            mute_states[index]
        }
    });
    let mix_gains = std::array::from_fn(|index| if main_mix_enabled[index] { 1.0 } else { 0.0 });
    (effective_mutes, mix_gains)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PatternBankActionResult {
    Applied,
    Loaded(u8),
    Busy,
    Skipped,
}

#[derive(Clone)]
pub struct LaneLengthLocks {
    mask: Arc<AtomicU16>,
}

impl LaneLengthLocks {
    pub fn new() -> Self {
        Self {
            mask: Arc::new(AtomicU16::new(0)),
        }
    }

    pub fn is_locked(&self, instrument: usize) -> bool {
        instrument < crate::track::MAX_TRACKS
            && (self.mask.load(Ordering::Relaxed) & (1u16 << instrument)) != 0
    }

    pub fn set_locked(&self, instrument: usize, locked: bool) {
        if instrument >= crate::track::MAX_TRACKS {
            return;
        }
        let bit = 1u16 << instrument;
        if locked {
            self.mask.fetch_or(bit, Ordering::Relaxed);
        } else {
            self.mask.fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

impl Default for LaneLengthLocks {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> PersistentField<'a, u16> for LaneLengthLocks {
    fn set(&self, new_value: u16) {
        let valid_bits = (1u16 << crate::track::MAX_TRACKS).wrapping_sub(1);
        self.mask.store(new_value & valid_bits, Ordering::Relaxed);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&u16) -> R,
    {
        let mask = self.mask.load(Ordering::Relaxed);
        f(&mask)
    }
}

fn resolve_track_length(raw_length: usize, master_length: usize, locked: bool) -> usize {
    let master_length = master_length.clamp(1, 64);
    if locked {
        raw_length.clamp(1, 64)
    } else {
        master_length
    }
}

pub struct DrumFlashVst {
    params: Arc<DrumFlashParams>,
    sequencer: Sequencer,
    synthesizer: DrumSynthesizer,
    sample_rate: f32,
    current_step: Arc<AtomicU32>,
    current_steps: Arc<[AtomicU32; crate::track::MAX_TRACKS]>,
    pattern: Arc<SharedPattern>,
    last_step_masks: [u16; STEP_COUNT],
    /// [216] Audition hook, currently without a trigger: the lane row's "T"
    /// button was its only entry point and has been removed. The audio side is
    /// kept deliberately, so re-exposing auditioning elsewhere - the Sound
    /// panel, a shortcut - is a UI change alone.
    voice_test_triggers: Arc<[AtomicBool; crate::track::MAX_TRACKS]>,
    external_midi_triggers: Arc<[AtomicBool; crate::track::MAX_TRACKS]>,
    sound_settings_state: Arc<SoundSettingsState>,
    last_sound_settings_version: u64,
    /// [304] The host tempo the voices' synced LFO rates are computed from, and the one
    /// they were last pushed with (a tempo change re-pushes the settings).
    lfo_bpm: f32,
    last_push_bpm: f32,
    /// [242] Last macro knob values applied to the lanes; a moved knob is
    /// re-applied, a still one is skipped. -1 forces a first apply.
    last_macro_values: [f32; macros::MACRO_COUNT],
    /// Last host beat position, used to detect seeks.
    last_host_pos: Option<f64>,
    /// Simple LCG RNG state for probability checks (audio-thread safe).
    rng_state: AtomicU32,
    /// Pending delayed triggers: (samples_until, slot_idx, velocity, step, hard, settings).
    /// Fixed-size queue for audio-thread safety (no allocation).
    /// Max: 14 slots x 7 extra triggers = 98, rounded up to 128.
    pending_triggers: [(u32, usize, f32, u32, bool, synthesis::VoiceSettings); 128],
    pending_trigger_count: usize,
    /// Pattern save request from UI: slot index + 1 (0 = none).
    save_pattern_request: Arc<AtomicU32>,
    /// Pattern load request from UI: slot index + 1 (0 = none).
    load_pattern_request: Arc<AtomicU32>,
    /// Clear all plocks request from UI (true = clear on next process()).
    clear_plocks_request: Arc<AtomicBool>,
    /// Song mode state (UI-driven, atomic for lock-free access).
    song_mode: Arc<AtomicBool>,
    /// Previous song-mode value used to detect transitions (off -> on).
    song_mode_was_active: bool,
    /// When true, the song needs to restart from step 0 on the next audio block.
    /// Set when song mode is enabled or when transport stops/starts with song mode on.
    song_needs_init: bool,
    /// Current song sequence used by the audio thread. Updated from the lock-free
    /// controller at the start of each process block.
    song_state: pattern_bank::SongSequence,
    /// Current position in the song sequence (0-63).
    song_position: Arc<AtomicU32>,
    /// Last observed loop count to detect pattern wrap.
    last_loop_count: usize,
    /// How many times the current song step has repeated since it was loaded.
    song_repeat_counter: u32,
    /// Last song position seen by the audio thread, used to detect UI resets.
    last_song_position: usize,
    /// Last observed master length so we can resync the sequencer when it changes.
    last_master_length: usize,
    /// Last active track layout kinds, so the audio thread can reinitialize
    /// synthesizer voices when a slot is added, removed or reassigned.
    last_slot_kinds: [Option<crate::track::TrackInstrumentKind>; crate::track::MAX_TRACKS],
    /// [286] The transport beat (in the Rumble's Duck Trigger division) each effect bus last
    /// started its duck on; `i64::MIN` = none yet (the next beat starts it).
    rumble_beat_tick: [i64; crate::track::FX_SLOT_COUNT],
    /// Last algo pushed to each slot's voice ([184]). The global algo must only
    /// be re-pushed when it actually changes: pushing it every buffer overwrote
    /// the per-step p-locked algo a few milliseconds into the hit, so a plocked
    /// algo was audible only at the very attack and the tail reverted to the
    /// lane's algo. `u8::MAX` forces a push.
    last_algos: [u8; crate::track::MAX_TRACKS],
    /// [227] Advance bookkeeping per slot: hits since the last reset (the
    /// per-hit count), plus the loop and page the previous sample was in, to
    /// catch the wraps that reset it.
    advance_hits: [u32; crate::track::MAX_TRACKS],
    advance_last_loop: usize,
    advance_last_page: usize,
    /// Pending pattern length update after a slot load (1-64, 0 = none).
    /// The UI thread applies this to the IntParam on the next frame.
    pending_pattern_length: Arc<AtomicI32>,
    /// Pre-allocated scratch buffers so restore() never allocates in the audio thread.
    temp_plock_bytes: [u8; pattern_bank::MAX_PLOCK_BYTES],
    temp_seq_plock_bytes: [u8; pattern_bank::MAX_SEQ_PLOCK_BYTES],
    temp_fusion_bytes: [u8; pattern_bank::MAX_FUSION_BYTES],
    /// Which slot the audio thread last successfully loaded (0-15), or u32::MAX if none.
    /// The UI thread reads this to keep last_loaded_slot in sync with reality.
    audio_last_loaded_slot: Arc<AtomicU32>,
    /// Pattern length used by the audio thread. Slot loads update this immediately;
    /// the UI parameter is synchronized asynchronously via pending_pattern_length.
    audio_master_length: usize,
    /// Last pattern length observed from the nih-plug param, used to distinguish
    /// real param edits from a stale UI value after an audio-side pattern load.
    last_param_master_length: usize,
    /// Restart the sequencer at step 0 after a song-mode slot transition.
    pending_song_pattern_restart: bool,
    /// Slot requested by an incoming MIDI pattern-switch note (0-15 for P1-P16).
    /// Applied at the next musical loop boundary to avoid glitches.
    pending_midi_pattern_slot: Option<usize>,
    /// Save/load requests retried when the pattern bank is temporarily locked by the UI.
    deferred_save_slot: Option<usize>,
    deferred_load_slot: Option<usize>,
}

/// [227] Bits of a slot's `advance_mode` parameter - how Rift's Advance
/// counts. Lane settings (nih-plug params), not sound parameters: they say how
/// the SEQUENCER counts hits, so they live beside the lane's algo, persisted by
/// the host, and never in `special[]`.
pub mod advance_mode {
    /// The switch. Off, the offset never advances whatever `Advance Step` says.
    pub const ON: i32 = 1;
    /// Restart the count when the pattern wraps.
    pub const RESET_PATTERN: i32 = 2;
    /// Restart the count when the page loops: the playhead enters another
    /// page of 16 steps, or the pattern wraps back to its first page. Mutually
    /// exclusive with `RESET_PATTERN` in the UI.
    pub const RESET_PAGE: i32 = 4;
    /// Count every cell the playhead passes, active or not, instead of every
    /// hit of the lane.
    pub const EVERY_CELL: i32 = 8;
}

#[derive(Params)]
pub struct DrumFlashParams {
    #[persist = "editor-state-v2"]
    pub editor_state: Arc<EguiState>,

    #[persist = "pattern-v5"]
    pub pattern_state: PersistentPattern,

    #[persist = "sound-settings-v2"]
    pub sound_settings: PersistentSoundSettings,

    #[persist = "track-layout-v1"]
    pub track_layout: track::PersistentTrackLayout,

    #[persist = "plock-v1"]
    pub plock_state: PersistentPlockState,

    #[persist = "seq-plock-v1"]
    pub seq_plock_state: PersistentSequencerPlockState,

    #[persist = "pattern-bank-v1"]
    pub pattern_bank: pattern_bank::PersistentPatternBank,

    /// Runtime lock-free controller for the song sequence. The UI publishes
    /// snapshots here; the audio thread consumes them without locking the bank.
    /// Not persisted; the source of truth remains `pattern_bank.song`.
    pub song_controller: atomic_song::SharedSongStateController,

    #[persist = "lane-locks-v1"]
    pub lane_length_locks: LaneLengthLocks,

    /// [228] One custom texture per lane for Rift: fourteen optional file
    /// paths, decoded on restore (main thread) into the pool the voices read.
    #[persist = "lane-textures-v1"]
    pub user_textures: user_textures::UserTextures,

    /// [242] Macro assignments: host-driven "Macro N" knob -> (lane, sound
    /// parameter). The blob holds 32 slots so extending past 16 knobs later
    /// is not a format bump; 16 are exposed below.
    #[persist = "macro-map-v1"]
    pub macro_map_state: macros::PersistentMacroMap,

    /// [282] True once this instance has its content: the starting pattern
    /// (Tommy01) applied at the first editor open of a NEW instance, or any
    /// restored state ([`Plugin::filter_state`] adds it to sessions saved
    /// before it existed, so an old project never receives Tommy01).
    #[persist = "instance-seeded-v1"]
    pub instance_seeded: Arc<AtomicBool>,

    #[id = "macro_1"]
    pub macro_1: FloatParam,
    #[id = "macro_2"]
    pub macro_2: FloatParam,
    #[id = "macro_3"]
    pub macro_3: FloatParam,
    #[id = "macro_4"]
    pub macro_4: FloatParam,
    #[id = "macro_5"]
    pub macro_5: FloatParam,
    #[id = "macro_6"]
    pub macro_6: FloatParam,
    #[id = "macro_7"]
    pub macro_7: FloatParam,
    #[id = "macro_8"]
    pub macro_8: FloatParam,
    #[id = "macro_9"]
    pub macro_9: FloatParam,
    #[id = "macro_10"]
    pub macro_10: FloatParam,
    #[id = "macro_11"]
    pub macro_11: FloatParam,
    #[id = "macro_12"]
    pub macro_12: FloatParam,
    #[id = "macro_13"]
    pub macro_13: FloatParam,
    #[id = "macro_14"]
    pub macro_14: FloatParam,
    #[id = "macro_15"]
    pub macro_15: FloatParam,
    #[id = "macro_16"]
    pub macro_16: FloatParam,

    #[id = "master_vol"]
    pub master_volume: FloatParam,

    #[id = "bpm"]
    pub bpm: FloatParam,

    #[id = "swing"]
    pub swing: FloatParam,

    #[id = "groove_type"]
    pub groove_type: EnumParam<groove::GrooveType>,

    /// When true (default), the internal step sequencer generates triggers.
    /// When false, the plugin responds to incoming MIDI notes on the configured channel.
    #[id = "int_seq"]
    pub use_internal_sequencer: BoolParam,

    /// When true, the sequencer plays through the song sequence (P1-P16 chain).
    /// When false, the current pattern loops normally.
    #[id = "song_mode"]
    pub song_mode: BoolParam,

    /// When true, incoming MIDI notes C3..G3 (60-67) switch the pattern bank
    /// slot (P1..P8) in real-time while the internal sequencer is active and
    /// Song mode is off.
    #[id = "midi_pat_sw"]
    pub midi_pattern_switch: BoolParam,

    // Per-track groove parameters (3 × 7 instruments)
    #[id = "hu_kick"]
    pub humanize_kick: FloatParam,
    #[id = "hu_snare"]
    pub humanize_snare: FloatParam,
    #[id = "hu_hihat"]
    pub humanize_hihat: FloatParam,
    #[id = "hu_ohh"]
    pub humanize_open_hh: FloatParam,
    #[id = "hu_t1"]
    pub humanize_tom1: FloatParam,
    #[id = "hu_t2"]
    pub humanize_tom2: FloatParam,
    #[id = "hu_t3"]
    pub humanize_tom3: FloatParam,
    #[id = "hu_clap"]
    pub humanize_clap: FloatParam,
    #[id = "hu_ride"]
    pub humanize_ride: FloatParam,
    #[id = "hu_cymbal"]
    pub humanize_cymbal: FloatParam,
    #[id = "hu_snare606"]
    pub humanize_snare606: FloatParam,
    #[id = "hu_b8"]
    pub humanize_bassdrum808: FloatParam,
    #[id = "hu_perc1"]
    pub humanize_perc1: FloatParam,
    #[id = "hum_s13"]
    pub hum_s13: FloatParam,

    #[id = "pp_kick"]
    pub push_kick: FloatParam,
    #[id = "pp_snare"]
    pub push_snare: FloatParam,
    #[id = "pp_hihat"]
    pub push_hihat: FloatParam,
    #[id = "pp_ohh"]
    pub push_open_hh: FloatParam,
    #[id = "pp_t1"]
    pub push_tom1: FloatParam,
    #[id = "pp_t2"]
    pub push_tom2: FloatParam,
    #[id = "pp_t3"]
    pub push_tom3: FloatParam,
    #[id = "pp_clap"]
    pub push_clap: FloatParam,
    #[id = "pp_ride"]
    pub push_ride: FloatParam,
    #[id = "pp_cymbal"]
    pub push_cymbal: FloatParam,
    #[id = "pp_snare606"]
    pub push_snare606: FloatParam,
    #[id = "pp_b8"]
    pub push_bassdrum808: FloatParam,
    #[id = "pp_perc1"]
    pub push_perc1: FloatParam,
    #[id = "push_s13"]
    pub push_s13: FloatParam,

    #[id = "pl_kick"]
    pub length_kick: IntParam,
    #[id = "pl_snare"]
    pub length_snare: IntParam,
    #[id = "pl_hihat"]
    pub length_hihat: IntParam,
    #[id = "pl_ohh"]
    pub length_open_hh: IntParam,
    #[id = "pl_t1"]
    pub length_tom1: IntParam,
    #[id = "pl_t2"]
    pub length_tom2: IntParam,
    #[id = "pl_t3"]
    pub length_tom3: IntParam,
    #[id = "pl_clap"]
    pub length_clap: IntParam,
    #[id = "pl_ride"]
    pub length_ride: IntParam,
    #[id = "pl_cymbal"]
    pub length_cymbal: IntParam,
    #[id = "pl_snare606"]
    pub length_snare606: IntParam,
    #[id = "pl_b8"]
    pub length_bassdrum808: IntParam,
    #[id = "pl_perc1"]
    pub length_perc1: IntParam,
    #[id = "len_s13"]
    pub len_s13: IntParam,

    // Global pattern length (master length)
    #[id = "pat_len"]
    pub pattern_length: IntParam,

    /// [212] Page loop: 0 = off, 1-4 = loop that 16-step page alone. Ignored
    /// while Song mode plays. Not hidden: a hidden parameter is read-only for
    /// the host and could not be toggled from the editor while playing.
    #[id = "page_loop"]
    pub page_loop: IntParam,

    #[id = "kick_click"]
    pub kick_click: FloatParam,
    #[id = "kick_click_type"]
    pub kick_click_type: FloatParam,
    // Kick saturation parameters
    #[id = "kick_sat_type"]
    pub kick_saturation_type: FloatParam,
    #[id = "kick_sat_amt"]
    pub kick_saturation_amount: FloatParam,
    #[id = "kick_sat_mix"]
    pub kick_saturation_mix: FloatParam,
    #[id = "kick_sat_out"]
    pub kick_saturation_output_gain: FloatParam,
    #[id = "kick_sat_pre"]
    pub kick_saturation_pre_filter: FloatParam,

    #[id = "tom_stick"]
    pub tom_stick: FloatParam,
    // Tom saturation parameters (shared by all 3 toms)
    #[id = "tom_sat_type"]
    pub tom_saturation_type: FloatParam,
    #[id = "tom_sat_amt"]
    pub tom_saturation_amount: FloatParam,
    #[id = "tom_sat_mix"]
    pub tom_saturation_mix: FloatParam,
    #[id = "tom_sat_out"]
    pub tom_saturation_output_gain: FloatParam,
    #[id = "tom_sat_pre"]
    pub tom_saturation_pre_filter: FloatParam,

    // 808 Bass Drum special parameters
    #[id = "b8_accent"]
    pub bassdrum808_accent: FloatParam,

    #[id = "b8_snap"]
    pub bassdrum808_snap: FloatParam,

    #[id = "b8_drop"]
    pub bassdrum808_pitch_drop: FloatParam,

    #[id = "b8_click_tone"]
    pub bassdrum808_click_tone: FloatParam,
    // 808 Bass Drum saturation parameters
    #[id = "b8_sat_type"]
    pub bassdrum808_saturation_type: FloatParam,
    #[id = "b8_sat_amt"]
    pub bassdrum808_saturation_amount: FloatParam,
    #[id = "b8_sat_mix"]
    pub bassdrum808_saturation_mix: FloatParam,
    #[id = "b8_sat_out"]
    pub bassdrum808_saturation_output_gain: FloatParam,
    #[id = "b8_sat_pre"]
    pub bassdrum808_saturation_pre_filter: FloatParam,

    #[id = "mute_kick"]
    pub mute_kick: BoolParam,

    #[id = "mute_snare"]
    pub mute_snare: BoolParam,

    #[id = "mute_hihat"]
    pub mute_hihat: BoolParam,

    #[id = "mute_open_hh"]
    pub mute_open_hh: BoolParam,

    #[id = "mute_tom1"]
    pub mute_tom1: BoolParam,

    #[id = "mute_tom2"]
    pub mute_tom2: BoolParam,

    #[id = "mute_tom3"]
    pub mute_tom3: BoolParam,
    #[id = "mute_clap"]
    pub mute_clap: BoolParam,
    #[id = "mute_ride"]
    pub mute_ride: BoolParam,
    #[id = "mute_cymbal"]
    pub mute_cymbal: BoolParam,
    #[id = "mute_snare606"]
    pub mute_snare606: BoolParam,
    #[id = "mute_b8"]
    pub mute_bassdrum808: BoolParam,
    #[id = "mute_perc1"]
    pub mute_perc1: BoolParam,
    #[id = "mute_s13"]
    pub mute_s13: BoolParam,

    // Per-instrument Main Mix inclusion (true = routed to Main Mix)
    #[id = "mix_kick"]
    pub mix_kick: BoolParam,
    #[id = "mix_snare"]
    pub mix_snare: BoolParam,
    #[id = "mix_hihat"]
    pub mix_hihat: BoolParam,
    #[id = "mix_open_hh"]
    pub mix_open_hh: BoolParam,
    #[id = "mix_tom1"]
    pub mix_tom1: BoolParam,
    #[id = "mix_tom2"]
    pub mix_tom2: BoolParam,
    #[id = "mix_tom3"]
    pub mix_tom3: BoolParam,
    #[id = "mix_clap"]
    pub mix_clap: BoolParam,
    #[id = "mix_ride"]
    pub mix_ride: BoolParam,
    #[id = "mix_cymbal"]
    pub mix_cymbal: BoolParam,
    #[id = "mix_snare606"]
    pub mix_snare606: BoolParam,
    #[id = "mix_b8"]
    pub mix_bassdrum808: BoolParam,
    #[id = "mix_perc1"]
    pub mix_perc1: BoolParam,
    #[id = "mix_s13"]
    pub mix_s13: BoolParam,

    #[id = "solo_kick"]
    pub solo_kick: BoolParam,

    #[id = "solo_snare"]
    pub solo_snare: BoolParam,

    #[id = "solo_hihat"]
    pub solo_hihat: BoolParam,

    #[id = "solo_open_hh"]
    pub solo_open_hh: BoolParam,

    #[id = "solo_tom1"]
    pub solo_tom1: BoolParam,

    #[id = "solo_tom2"]
    pub solo_tom2: BoolParam,

    #[id = "solo_tom3"]
    pub solo_tom3: BoolParam,
    #[id = "solo_clap"]
    pub solo_clap: BoolParam,
    #[id = "solo_ride"]
    pub solo_ride: BoolParam,
    #[id = "solo_cymbal"]
    pub solo_cymbal: BoolParam,
    #[id = "solo_snare606"]
    pub solo_snare606: BoolParam,
    #[id = "solo_b8"]
    pub solo_bassdrum808: BoolParam,
    #[id = "solo_perc1"]
    pub solo_perc1: BoolParam,
    #[id = "solo_s13"]
    pub solo_s13: BoolParam,

    #[id = "gen_type"]
    pub generator_type: EnumParam<GeneratorType>,

    #[id = "style_pri"]
    pub style_primary: EnumParam<Style>,

    #[id = "style_sec"]
    pub style_secondary: EnumParam<Style>,

    #[id = "style_mix"]
    pub style_mix: FloatParam,

    #[id = "gen_density"]
    pub gen_density: FloatParam,

    #[id = "gen_var"]
    pub gen_variation: FloatParam,

    // Synthesis algorithms per instrument (0 = default)
    #[id = "algo_kick"]
    pub algo_kick: IntParam,
    #[id = "algo_snare"]
    pub algo_snare: IntParam,
    #[id = "algo_hihat"]
    pub algo_hihat: IntParam,
    #[id = "algo_open_hh"]
    pub algo_open_hh: IntParam,
    #[id = "algo_tom1"]
    pub algo_tom1: IntParam,
    #[id = "algo_tom2"]
    pub algo_tom2: IntParam,
    #[id = "algo_tom3"]
    pub algo_tom3: IntParam,
    #[id = "algo_clap"]
    pub algo_clap: IntParam,
    #[id = "algo_ride"]
    pub algo_ride: IntParam,
    #[id = "algo_cymbal"]
    pub algo_cymbal: IntParam,
    #[id = "algo_snare606"]
    pub algo_snare606: IntParam,
    #[id = "algo_b8"]
    pub algo_bassdrum808: IntParam,
    #[id = "algo_perc1"]
    pub algo_perc1: IntParam,
    #[id = "algo_s13"]
    pub algo_s13: IntParam,

    // [227] Advance mode per slot (bitfield, see `advance_mode`).
    #[id = "advmode_1"]
    pub advance_mode_1: IntParam,
    #[id = "advmode_2"]
    pub advance_mode_2: IntParam,
    #[id = "advmode_3"]
    pub advance_mode_3: IntParam,
    #[id = "advmode_4"]
    pub advance_mode_4: IntParam,
    #[id = "advmode_5"]
    pub advance_mode_5: IntParam,
    #[id = "advmode_6"]
    pub advance_mode_6: IntParam,
    #[id = "advmode_7"]
    pub advance_mode_7: IntParam,
    #[id = "advmode_8"]
    pub advance_mode_8: IntParam,
    #[id = "advmode_9"]
    pub advance_mode_9: IntParam,
    #[id = "advmode_10"]
    pub advance_mode_10: IntParam,
    #[id = "advmode_11"]
    pub advance_mode_11: IntParam,
    #[id = "advmode_12"]
    pub advance_mode_12: IntParam,
    #[id = "advmode_13"]
    pub advance_mode_13: IntParam,
    #[id = "advmode_14"]
    pub advance_mode_14: IntParam,

    // Frequency display mode per bass drum (false = Hz, true = Notes)
    #[id = "freq_mode_kick"]
    pub freq_mode_kick: BoolParam,
    #[id = "freq_mode_b8"]
    pub freq_mode_bassdrum808: BoolParam,

    // Special parameters per instrument
    #[id = "snare_snap"]
    pub snare_snap: FloatParam,
    // Snare saturation parameters
    #[id = "snare_sat_type"]
    pub snare_saturation_type: FloatParam,
    #[id = "snare_sat_amt"]
    pub snare_saturation_amount: FloatParam,
    #[id = "snare_sat_mix"]
    pub snare_saturation_mix: FloatParam,
    #[id = "snare_sat_out"]
    pub snare_saturation_output_gain: FloatParam,
    #[id = "snare_sat_pre"]
    pub snare_saturation_pre_filter: FloatParam,

    // Hi-Hat saturation parameters
    #[id = "hihat_sat_type"]
    pub hihat_saturation_type: FloatParam,
    #[id = "hihat_sat_amt"]
    pub hihat_saturation_amount: FloatParam,
    #[id = "hihat_sat_mix"]
    pub hihat_saturation_mix: FloatParam,
    #[id = "hihat_sat_out"]
    pub hihat_saturation_output_gain: FloatParam,
    #[id = "hihat_sat_pre"]
    pub hihat_saturation_pre_filter: FloatParam,

    // Open Hi-Hat saturation parameters
    #[id = "oh_sat_type"]
    pub open_hihat_saturation_type: FloatParam,
    #[id = "oh_sat_amt"]
    pub open_hihat_saturation_amount: FloatParam,
    #[id = "oh_sat_mix"]
    pub open_hihat_saturation_mix: FloatParam,
    #[id = "oh_sat_out"]
    pub open_hihat_saturation_output_gain: FloatParam,
    #[id = "oh_sat_pre"]
    pub open_hihat_saturation_pre_filter: FloatParam,

    // Hi-hat chokes open hi-hat when triggered
    #[id = "hihat_chokes_oh"]
    pub hihat_chokes_oh: BoolParam,

    // Auto-edit: clicking a step in the grid automatically selects its instrument
    #[id = "auto_edit"]
    pub auto_edit: BoolParam,

    // Clap echo: scales burst spacing and tone diffusion between the 4 bursts.
    // 0 = collapse to a single burst, 1 = default 12 ms spread, 2 = wider.
    #[id = "clap_echo"]
    pub clap_echo: FloatParam,

    // Clap saturation parameters
    #[id = "clap_sat_type"]
    pub clap_saturation_type: FloatParam,
    #[id = "clap_sat_amt"]
    pub clap_saturation_amount: FloatParam,
    #[id = "clap_sat_mix"]
    pub clap_saturation_mix: FloatParam,
    #[id = "clap_sat_out"]
    pub clap_saturation_output_gain: FloatParam,
    #[id = "clap_sat_pre"]
    pub clap_saturation_pre_filter: FloatParam,

    // Snare 606 specials: bridged-T resonator fine-tuning.
    #[id = "sn606_res"]
    pub snare606_resonance: FloatParam,
    #[id = "sn606_tone"]
    pub snare606_tone: FloatParam,
    #[id = "sn606_snap"]
    pub snare606_snap: FloatParam,
    // Snare 606 saturation parameters
    #[id = "sn606_sat_type"]
    pub snare606_saturation_type: FloatParam,
    #[id = "sn606_sat_amt"]
    pub snare606_saturation_amount: FloatParam,
    #[id = "sn606_sat_mix"]
    pub snare606_saturation_mix: FloatParam,
    #[id = "sn606_sat_out"]
    pub snare606_saturation_output_gain: FloatParam,
    #[id = "sn606_sat_pre"]
    pub snare606_saturation_pre_filter: FloatParam,

    // Ride saturation parameters
    #[id = "ride_sat_type"]
    pub ride_saturation_type: FloatParam,
    #[id = "ride_sat_amt"]
    pub ride_saturation_amount: FloatParam,
    #[id = "ride_sat_mix"]
    pub ride_saturation_mix: FloatParam,
    #[id = "ride_sat_out"]
    pub ride_saturation_output_gain: FloatParam,
    #[id = "ride_sat_pre"]
    pub ride_saturation_pre_filter: FloatParam,

    // Perc1 special parameters
    #[id = "perc1_sweep"]
    pub perc1_sweep: FloatParam,
    #[id = "perc1_speed"]
    pub perc1_speed: FloatParam,
    #[id = "perc1_bite"]
    pub perc1_bite: FloatParam,
    #[id = "perc1_width"]
    pub perc1_width: FloatParam,
    // Perc1 saturation parameters
    #[id = "perc1_sat_type"]
    pub perc1_saturation_type: FloatParam,
    #[id = "perc1_sat_amt"]
    pub perc1_saturation_amount: FloatParam,
    #[id = "perc1_sat_mix"]
    pub perc1_saturation_mix: FloatParam,
    #[id = "perc1_sat_out"]
    pub perc1_saturation_output_gain: FloatParam,
    #[id = "perc1_sat_pre"]
    pub perc1_saturation_pre_filter: FloatParam,

    // Cymbal special parameters
    #[id = "cy_shimmer"]
    pub cymbal_shimmer_freq: FloatParam,
    #[id = "cy_noise"]
    pub cymbal_noise_type: FloatParam,
    #[id = "cy_shimmer_amt"]
    pub cymbal_shimmer_amount: FloatParam,
    // Cymbal saturation parameters
    #[id = "cy_sat_type"]
    pub cymbal_saturation_type: FloatParam,
    #[id = "cy_sat_amt"]
    pub cymbal_saturation_amount: FloatParam,
    #[id = "cy_sat_mix"]
    pub cymbal_saturation_mix: FloatParam,
    #[id = "cy_sat_out"]
    pub cymbal_saturation_output_gain: FloatParam,
    #[id = "cy_sat_pre"]
    pub cymbal_saturation_pre_filter: FloatParam,
}

impl Default for DrumFlashParams {
    fn default() -> Self {
        install_crash_hook();
        let default_layout = crate::track::TrackLayoutState::default_layout();
        let default_pattern = Pattern::rock_pattern();
        let _default_masks = default_pattern.step_masks();
        let pattern_state = PersistentPattern::new(&default_pattern);
        let algo_range = IntRange::Linear {
            min: 0,
            max: crate::instrument_registry::max_algo_index(),
        };

        let global_config = crate::config::GlobalConfig::load();
        let track_layout = track::PersistentTrackLayout::new();
        track_layout.state.global_channel.store(
            global_config.global_midi_channel.clamp(1, 16),
            Ordering::Relaxed,
        );

        Self {
            editor_state: EguiState::from_size(ui::EDITOR_SIZE.0, ui::EDITOR_SIZE.1),
            pattern_state,
            sound_settings: PersistentSoundSettings::new(&default_layout),
            track_layout,
            plock_state: PersistentPlockState::new(),
            seq_plock_state: PersistentSequencerPlockState::new(),
            pattern_bank: pattern_bank::PersistentPatternBank::new(),
            song_controller: atomic_song::SharedSongStateController::default(),
            lane_length_locks: LaneLengthLocks::new(),
            user_textures: user_textures::UserTextures::new(),

            macro_map_state: macros::PersistentMacroMap::new(),
            instance_seeded: Arc::new(AtomicBool::new(false)),

            macro_1: FloatParam::new("Macro 1", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_2: FloatParam::new("Macro 2", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_3: FloatParam::new("Macro 3", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_4: FloatParam::new("Macro 4", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_5: FloatParam::new("Macro 5", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_6: FloatParam::new("Macro 6", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_7: FloatParam::new("Macro 7", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_8: FloatParam::new("Macro 8", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_9: FloatParam::new("Macro 9", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_10: FloatParam::new("Macro 10", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_11: FloatParam::new("Macro 11", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_12: FloatParam::new("Macro 12", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_13: FloatParam::new("Macro 13", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_14: FloatParam::new("Macro 14", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_15: FloatParam::new("Macro 15", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            macro_16: FloatParam::new("Macro 16", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),

            master_volume: FloatParam::new(
                "Master Volume",
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            // The slider can reach 0.0 (-inf dB), so logarithmic smoothing is unsafe here.
            .with_smoother(SmoothingStyle::Exponential(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2)),

            bpm: FloatParam::new(
                "BPM",
                120.0,
                FloatRange::Linear {
                    min: 60.0,
                    max: 180.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            swing: FloatParam::new(
                "Swing",
                0.0,
                FloatRange::Linear {
                    min: -0.5,
                    max: 0.5,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(0)),

            groove_type: EnumParam::new("Groove", groove::GrooveType::Swing16),

            use_internal_sequencer: BoolParam::new("Internal Sequencer", true),
            song_mode: BoolParam::new("Song Mode", false),
            midi_pattern_switch: BoolParam::new("MIDI Pattern Switch", false),

            // Humanize per track (0 = none, 1 = max)
            humanize_kick: FloatParam::new(
                "Humanize Kick",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_snare: FloatParam::new(
                "Humanize Snare",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_hihat: FloatParam::new(
                "Humanize Hi-Hat",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_open_hh: FloatParam::new(
                "Humanize Open HH",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_tom1: FloatParam::new(
                "Humanize Tom 1",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_tom2: FloatParam::new(
                "Humanize Tom 2",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_tom3: FloatParam::new(
                "Humanize Tom 3",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_clap: FloatParam::new(
                "Humanize Clap",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_ride: FloatParam::new(
                "Humanize Ride",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_cymbal: FloatParam::new(
                "Humanize Cymbal",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_snare606: FloatParam::new(
                "Humanize Snare 606",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_bassdrum808: FloatParam::new(
                "Humanize 808 Kick",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            humanize_perc1: FloatParam::new(
                "Humanize Perc1",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            hum_s13: FloatParam::new(
                "Humanize Slot 13",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            // Push/pull per track (-50 ms = early, +50 ms = late)
            push_kick: FloatParam::new(
                "Push/Pull Kick",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_snare: FloatParam::new(
                "Push/Pull Snare",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_hihat: FloatParam::new(
                "Push/Pull Hi-Hat",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_open_hh: FloatParam::new(
                "Push/Pull Open HH",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_tom1: FloatParam::new(
                "Push/Pull Tom 1",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_tom2: FloatParam::new(
                "Push/Pull Tom 2",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_tom3: FloatParam::new(
                "Push/Pull Tom 3",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_clap: FloatParam::new(
                "Push/Pull Clap",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_ride: FloatParam::new(
                "Push/Pull Ride",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_cymbal: FloatParam::new(
                "Push/Pull Cymbal",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_snare606: FloatParam::new(
                "Push/Pull Snare 606",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_bassdrum808: FloatParam::new(
                "Push/Pull 808 Kick",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_perc1: FloatParam::new(
                "Push/Pull Perc1",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            push_s13: FloatParam::new(
                "Push/Pull Slot 13",
                0.0,
                FloatRange::Linear {
                    min: -50.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),

            // Pattern length per track (1-64 steps)
            length_kick: IntParam::new("Length Kick", 16, IntRange::Linear { min: 1, max: 64 }),
            length_snare: IntParam::new("Length Snare", 16, IntRange::Linear { min: 1, max: 64 }),
            length_hihat: IntParam::new("Length Hi-Hat", 16, IntRange::Linear { min: 1, max: 64 }),
            length_open_hh: IntParam::new(
                "Length Open HH",
                16,
                IntRange::Linear { min: 1, max: 64 },
            ),
            length_tom1: IntParam::new("Length Tom 1", 16, IntRange::Linear { min: 1, max: 64 }),
            length_tom2: IntParam::new("Length Tom 2", 16, IntRange::Linear { min: 1, max: 64 }),
            length_tom3: IntParam::new("Length Tom 3", 16, IntRange::Linear { min: 1, max: 64 }),
            length_clap: IntParam::new("Length Clap", 16, IntRange::Linear { min: 1, max: 64 }),
            length_ride: IntParam::new("Length Ride", 16, IntRange::Linear { min: 1, max: 64 }),
            length_cymbal: IntParam::new("Length Cymbal", 16, IntRange::Linear { min: 1, max: 64 }),
            length_snare606: IntParam::new(
                "Length Snare 606",
                16,
                IntRange::Linear { min: 1, max: 64 },
            ),
            length_bassdrum808: IntParam::new(
                "Length 808 Kick",
                16,
                IntRange::Linear { min: 1, max: 64 },
            ),
            length_perc1: IntParam::new("Length Perc1", 16, IntRange::Linear { min: 1, max: 64 }),
            len_s13: IntParam::new("Length Slot 13", 16, IntRange::Linear { min: 1, max: 64 }),
            pattern_length: IntParam::new(
                "Pattern Length",
                16,
                IntRange::Linear { min: 1, max: 64 },
            ),
            page_loop: IntParam::new("Page Loop", 0, IntRange::Linear { min: 0, max: 4 }),

            kick_click: FloatParam::new(
                "Kick Click",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            kick_click_type: FloatParam::new(
                "Kick Click Type",
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            kick_saturation_type: FloatParam::new(
                "Kick Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            kick_saturation_amount: FloatParam::new(
                "Kick Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            kick_saturation_mix: FloatParam::new(
                "Kick Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            kick_saturation_output_gain: FloatParam::new(
                "Kick Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            kick_saturation_pre_filter: FloatParam::new(
                "Kick Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            tom_stick: FloatParam::new(
                "Tom Stick Attack",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            tom_saturation_type: FloatParam::new(
                "Tom Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            tom_saturation_amount: FloatParam::new(
                "Tom Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            tom_saturation_mix: FloatParam::new(
                "Tom Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            tom_saturation_output_gain: FloatParam::new(
                "Tom Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            tom_saturation_pre_filter: FloatParam::new(
                "Tom Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            bassdrum808_accent: FloatParam::new(
                "808 Accent",
                0.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            bassdrum808_snap: FloatParam::new(
                "808 Snap",
                0.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            bassdrum808_pitch_drop: FloatParam::new(
                "808 Pitch Drop",
                0.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            bassdrum808_click_tone: FloatParam::new(
                "808 Click Tone",
                4000.0,
                FloatRange::Linear {
                    min: 100.0,
                    max: 8000.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            bassdrum808_saturation_type: FloatParam::new(
                "808 Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            bassdrum808_saturation_amount: FloatParam::new(
                "808 Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            bassdrum808_saturation_mix: FloatParam::new(
                "808 Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            bassdrum808_saturation_output_gain: FloatParam::new(
                "808 Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            bassdrum808_saturation_pre_filter: FloatParam::new(
                "808 Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),

            mute_kick: BoolParam::new("Mute Kick", false),
            mute_snare: BoolParam::new("Mute Snare", false),
            mute_hihat: BoolParam::new("Mute Hi-Hat", false),
            mute_open_hh: BoolParam::new("Mute Open HH", false),
            mute_tom1: BoolParam::new("Mute Tom 1", false),
            mute_tom2: BoolParam::new("Mute Tom 2", false),
            mute_tom3: BoolParam::new("Mute Tom 3", false),
            mute_clap: BoolParam::new("Mute Clap", false),
            mute_ride: BoolParam::new("Mute Ride", false),
            mute_cymbal: BoolParam::new("Mute Cymbal", false),
            mute_snare606: BoolParam::new("Mute Snare 606", false),
            mute_bassdrum808: BoolParam::new("Mute 808 Kick", false),
            mute_perc1: BoolParam::new("Mute Perc1", false),
            mute_s13: BoolParam::new("Mute Slot 13", false),
            mix_kick: BoolParam::new("Mix Kick", true),
            mix_snare: BoolParam::new("Mix Snare", true),
            mix_hihat: BoolParam::new("Mix Hi-Hat", true),
            mix_open_hh: BoolParam::new("Mix Open HH", true),
            mix_tom1: BoolParam::new("Mix Tom 1", true),
            mix_tom2: BoolParam::new("Mix Tom 2", true),
            mix_tom3: BoolParam::new("Mix Tom 3", true),
            mix_clap: BoolParam::new("Mix Clap", true),
            mix_ride: BoolParam::new("Mix Ride", true),
            mix_cymbal: BoolParam::new("Mix Cymbal", true),
            mix_snare606: BoolParam::new("Mix Snare 606", true),
            mix_bassdrum808: BoolParam::new("Mix 808 Kick", true),
            mix_perc1: BoolParam::new("Mix Perc1", true),
            mix_s13: BoolParam::new("Mix Slot 13", true),
            solo_kick: BoolParam::new("Solo Kick", false),
            solo_snare: BoolParam::new("Solo Snare", false),
            solo_hihat: BoolParam::new("Solo Hi-Hat", false),
            solo_open_hh: BoolParam::new("Solo Open HH", false),
            solo_tom1: BoolParam::new("Solo Tom 1", false),
            solo_tom2: BoolParam::new("Solo Tom 2", false),
            solo_tom3: BoolParam::new("Solo Tom 3", false),
            solo_clap: BoolParam::new("Solo Clap", false),
            solo_ride: BoolParam::new("Solo Ride", false),
            solo_cymbal: BoolParam::new("Solo Cymbal", false),
            solo_snare606: BoolParam::new("Solo Snare 606", false),
            solo_bassdrum808: BoolParam::new("Solo 808 Kick", false),
            solo_perc1: BoolParam::new("Solo Perc1", false),
            solo_s13: BoolParam::new("Solo Slot 13", false),

            generator_type: EnumParam::new("Generator", GeneratorType::Probabilistic),
            style_primary: EnumParam::new("Style A", Style::Rock),
            style_secondary: EnumParam::new("Style B", Style::Rock),
            style_mix: FloatParam::new("Style Mix", 0.0, FloatRange::Linear { min: 0.0, max: 1.0 }),
            gen_density: FloatParam::new(
                "Density",
                generator::styles::DEFAULT_DENSITY,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            gen_variation: FloatParam::new(
                "Variation",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),

            // Algo params are positional PER SLOT (any kind can live on any slot),
            // so every param shares the widest range of the registry; the engine
            // and UI clamp to the current kind's algo count. A shared max also
            // avoids the IntRange { min: 0, max: 0 } division-by-zero crash in
            // nih-plug normalization (see bug [42]).
            // Params for slots that hold a single-algo kind in the default layout
            // are hidden from host automation so they don't clutter the DAW lanes.
            algo_kick: IntParam::new("Slot 1 Algo", 0, algo_range),
            algo_snare: IntParam::new("Slot 2 Algo", 0, algo_range),
            algo_hihat: IntParam::new("Slot 3 Algo", 0, algo_range).hide(),
            algo_open_hh: IntParam::new("Slot 4 Algo", 0, algo_range).hide(),
            algo_tom1: IntParam::new("Slot 5 Algo", 0, algo_range),
            algo_tom2: IntParam::new("Slot 6 Algo", 0, algo_range),
            algo_tom3: IntParam::new("Slot 7 Algo", 0, algo_range),
            algo_clap: IntParam::new("Slot 8 Algo", 0, algo_range),
            algo_ride: IntParam::new("Slot 9 Algo", 0, algo_range),
            algo_cymbal: IntParam::new("Slot 10 Algo", 0, algo_range).hide(),
            algo_snare606: IntParam::new("Slot 11 Algo", 0, algo_range).hide(),
            algo_bassdrum808: IntParam::new("Slot 12 Algo", 0, algo_range).hide(),
            algo_perc1: IntParam::new("Slot 13 Algo", 0, algo_range),
            algo_s13: IntParam::new("Slot 14 Algo", 0, algo_range).hide(),
            // NOT hidden: nih-plug reports a hidden parameter to the host as
            // read-only, and while the host is processing audio every change
            // from the editor goes through the host, which then refuses it -
            // the switch could not be toggled at all in Studio One.
            advance_mode_1: IntParam::new(
                "Slot 1 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_2: IntParam::new(
                "Slot 2 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_3: IntParam::new(
                "Slot 3 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_4: IntParam::new(
                "Slot 4 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_5: IntParam::new(
                "Slot 5 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_6: IntParam::new(
                "Slot 6 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_7: IntParam::new(
                "Slot 7 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_8: IntParam::new(
                "Slot 8 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_9: IntParam::new(
                "Slot 9 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_10: IntParam::new(
                "Slot 10 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_11: IntParam::new(
                "Slot 11 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_12: IntParam::new(
                "Slot 12 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_13: IntParam::new(
                "Slot 13 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),
            advance_mode_14: IntParam::new(
                "Slot 14 Advance Mode",
                0,
                IntRange::Linear { min: 0, max: 15 },
            ),

            freq_mode_kick: BoolParam::new("Kick Freq in Notes", false),
            freq_mode_bassdrum808: BoolParam::new("808 Kick Freq in Notes", false),

            snare_snap: FloatParam::new(
                "Snare Snap",
                0.5,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare_saturation_type: FloatParam::new(
                "Snare Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            snare_saturation_amount: FloatParam::new(
                "Snare Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare_saturation_mix: FloatParam::new(
                "Snare Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare_saturation_output_gain: FloatParam::new(
                "Snare Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare_saturation_pre_filter: FloatParam::new(
                "Snare Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            hihat_saturation_type: FloatParam::new(
                "HiHat Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            hihat_saturation_amount: FloatParam::new(
                "HiHat Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            hihat_saturation_mix: FloatParam::new(
                "HiHat Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            hihat_saturation_output_gain: FloatParam::new(
                "HiHat Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            hihat_saturation_pre_filter: FloatParam::new(
                "HiHat Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            open_hihat_saturation_type: FloatParam::new(
                "OpenHiHat Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            open_hihat_saturation_amount: FloatParam::new(
                "OpenHiHat Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            open_hihat_saturation_mix: FloatParam::new(
                "OpenHiHat Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            open_hihat_saturation_output_gain: FloatParam::new(
                "OpenHiHat Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            open_hihat_saturation_pre_filter: FloatParam::new(
                "OpenHiHat Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            // Legacy global HH→OH choke, replaced by per-slot choke groups
            // (Track tab). Kept declared (hidden) so old sessions still load.
            hihat_chokes_oh: BoolParam::new("HiHat Chokes OpenHH", true).hide(),
            auto_edit: BoolParam::new("Auto Edit", true),

            clap_echo: FloatParam::new("Clap Echo", 1.0, FloatRange::Linear { min: 0.0, max: 3.0 })
                .with_smoother(SmoothingStyle::Linear(10.0)),

            clap_saturation_type: FloatParam::new(
                "Clap Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            clap_saturation_amount: FloatParam::new(
                "Clap Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            clap_saturation_mix: FloatParam::new(
                "Clap Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            clap_saturation_output_gain: FloatParam::new(
                "Clap Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            clap_saturation_pre_filter: FloatParam::new(
                "Clap Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            snare606_resonance: FloatParam::new(
                "Snare 606 Resonance",
                4.5,
                FloatRange::Linear {
                    min: 0.5,
                    max: 12.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare606_tone: FloatParam::new(
                "Snare 606 Tone",
                0.55,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare606_snap: FloatParam::new(
                "Snare 606 Snap",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare606_saturation_type: FloatParam::new(
                "Snare 606 Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            snare606_saturation_amount: FloatParam::new(
                "Snare 606 Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare606_saturation_mix: FloatParam::new(
                "Snare 606 Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare606_saturation_output_gain: FloatParam::new(
                "Snare 606 Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            snare606_saturation_pre_filter: FloatParam::new(
                "Snare 606 Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            ride_saturation_type: FloatParam::new(
                "Ride Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            ride_saturation_amount: FloatParam::new(
                "Ride Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            ride_saturation_mix: FloatParam::new(
                "Ride Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            ride_saturation_output_gain: FloatParam::new(
                "Ride Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            ride_saturation_pre_filter: FloatParam::new(
                "Ride Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            perc1_sweep: FloatParam::new(
                "Perc1 Sweep",
                0.5,
                FloatRange::Linear {
                    min: -1.0,
                    max: 1.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            perc1_speed: FloatParam::new(
                "Perc1 Speed",
                80.0,
                FloatRange::Linear {
                    min: 5.0,
                    max: 300.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" ms"),
            perc1_bite: FloatParam::new(
                "Perc1 Bite",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            perc1_width: FloatParam::new(
                "Perc1 Width",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            perc1_saturation_type: FloatParam::new(
                "Perc1 Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            perc1_saturation_amount: FloatParam::new(
                "Perc1 Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            perc1_saturation_mix: FloatParam::new(
                "Perc1 Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            perc1_saturation_output_gain: FloatParam::new(
                "Perc1 Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            perc1_saturation_pre_filter: FloatParam::new(
                "Perc1 Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),

            cymbal_shimmer_freq: FloatParam::new(
                "Cymbal Shimmer Freq",
                15.0,
                FloatRange::Linear {
                    min: 1.0,
                    max: 50.0,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" Hz"),

            cymbal_noise_type: FloatParam::new(
                "Cymbal Noise Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 3.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),

            cymbal_shimmer_amount: FloatParam::new(
                "Cymbal Shimmer Amount",
                0.15,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_unit(" %")
            .with_value_to_string(formatters::v2s_f32_percentage(2)),

            cymbal_saturation_type: FloatParam::new(
                "Cymbal Saturation Type",
                0.0,
                FloatRange::Linear { min: 0.0, max: 5.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_step_size(1.0),
            cymbal_saturation_amount: FloatParam::new(
                "Cymbal Saturation Amount",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            cymbal_saturation_mix: FloatParam::new(
                "Cymbal Saturation Mix",
                1.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            cymbal_saturation_output_gain: FloatParam::new(
                "Cymbal Saturation Output Gain",
                1.0,
                FloatRange::Linear { min: 0.5, max: 2.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
            cymbal_saturation_pre_filter: FloatParam::new(
                "Cymbal Saturation Pre-Filter",
                0.0,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(10.0)),
        }
    }
}

impl DrumFlashParams {
    /// Indexed access to mute parameters.
    pub fn mutes(&self) -> [&BoolParam; crate::track::MAX_TRACKS] {
        [
            &self.mute_kick,
            &self.mute_snare,
            &self.mute_hihat,
            &self.mute_open_hh,
            &self.mute_tom1,
            &self.mute_tom2,
            &self.mute_tom3,
            &self.mute_clap,
            &self.mute_ride,
            &self.mute_cymbal,
            &self.mute_snare606,
            &self.mute_bassdrum808,
            &self.mute_perc1,
            &self.mute_s13,
        ]
    }

    /// Indexed access to solo parameters.
    pub fn solos(&self) -> [&BoolParam; crate::track::MAX_TRACKS] {
        [
            &self.solo_kick,
            &self.solo_snare,
            &self.solo_hihat,
            &self.solo_open_hh,
            &self.solo_tom1,
            &self.solo_tom2,
            &self.solo_tom3,
            &self.solo_clap,
            &self.solo_ride,
            &self.solo_cymbal,
            &self.solo_snare606,
            &self.solo_bassdrum808,
            &self.solo_perc1,
            &self.solo_s13,
        ]
    }

    /// Indexed access to mix parameters.
    pub fn mixes(&self) -> [&BoolParam; crate::track::MAX_TRACKS] {
        [
            &self.mix_kick,
            &self.mix_snare,
            &self.mix_hihat,
            &self.mix_open_hh,
            &self.mix_tom1,
            &self.mix_tom2,
            &self.mix_tom3,
            &self.mix_clap,
            &self.mix_ride,
            &self.mix_cymbal,
            &self.mix_snare606,
            &self.mix_bassdrum808,
            &self.mix_perc1,
            &self.mix_s13,
        ]
    }

    /// Indexed access to algo parameters.
    pub fn algos(&self) -> [&IntParam; crate::track::MAX_TRACKS] {
        [
            &self.algo_kick,
            &self.algo_snare,
            &self.algo_hihat,
            &self.algo_open_hh,
            &self.algo_tom1,
            &self.algo_tom2,
            &self.algo_tom3,
            &self.algo_clap,
            &self.algo_ride,
            &self.algo_cymbal,
            &self.algo_snare606,
            &self.algo_bassdrum808,
            &self.algo_perc1,
            &self.algo_s13,
        ]
    }

    /// [227] Indexed access to the per-slot Advance mode bitfields.
    pub fn macro_params(&self) -> [&FloatParam; macros::MACRO_COUNT] {
        [
            &self.macro_1,
            &self.macro_2,
            &self.macro_3,
            &self.macro_4,
            &self.macro_5,
            &self.macro_6,
            &self.macro_7,
            &self.macro_8,
            &self.macro_9,
            &self.macro_10,
            &self.macro_11,
            &self.macro_12,
            &self.macro_13,
            &self.macro_14,
            &self.macro_15,
            &self.macro_16,
        ]
    }

    pub fn advance_modes(&self) -> [&IntParam; crate::track::MAX_TRACKS] {
        [
            &self.advance_mode_1,
            &self.advance_mode_2,
            &self.advance_mode_3,
            &self.advance_mode_4,
            &self.advance_mode_5,
            &self.advance_mode_6,
            &self.advance_mode_7,
            &self.advance_mode_8,
            &self.advance_mode_9,
            &self.advance_mode_10,
            &self.advance_mode_11,
            &self.advance_mode_12,
            &self.advance_mode_13,
            &self.advance_mode_14,
        ]
    }

    pub fn humanizes(&self) -> [&FloatParam; crate::track::MAX_TRACKS] {
        [
            &self.humanize_kick,
            &self.humanize_snare,
            &self.humanize_hihat,
            &self.humanize_open_hh,
            &self.humanize_tom1,
            &self.humanize_tom2,
            &self.humanize_tom3,
            &self.humanize_clap,
            &self.humanize_ride,
            &self.humanize_cymbal,
            &self.humanize_snare606,
            &self.humanize_bassdrum808,
            &self.humanize_perc1,
            &self.hum_s13,
        ]
    }

    pub fn pushes(&self) -> [&FloatParam; crate::track::MAX_TRACKS] {
        [
            &self.push_kick,
            &self.push_snare,
            &self.push_hihat,
            &self.push_open_hh,
            &self.push_tom1,
            &self.push_tom2,
            &self.push_tom3,
            &self.push_clap,
            &self.push_ride,
            &self.push_cymbal,
            &self.push_snare606,
            &self.push_bassdrum808,
            &self.push_perc1,
            &self.push_s13,
        ]
    }

    pub fn lengths(&self) -> [&IntParam; crate::track::MAX_TRACKS] {
        [
            &self.length_kick,
            &self.length_snare,
            &self.length_hihat,
            &self.length_open_hh,
            &self.length_tom1,
            &self.length_tom2,
            &self.length_tom3,
            &self.length_clap,
            &self.length_ride,
            &self.length_cymbal,
            &self.length_snare606,
            &self.length_bassdrum808,
            &self.length_perc1,
            &self.len_s13,
        ]
    }

    /// Lookup a special parameter by instrument index and special slot.
    pub fn special_param(&self, instrument: usize, special_idx: usize) -> Option<&FloatParam> {
        match (instrument, special_idx) {
            (0, 0) => Some(&self.kick_click),
            (0, 6) => Some(&self.kick_click_type),
            (0, 1) => Some(&self.kick_saturation_type),
            (0, 2) => Some(&self.kick_saturation_amount),
            (0, 3) => Some(&self.kick_saturation_mix),
            (0, 4) => Some(&self.kick_saturation_output_gain),
            (0, 5) => Some(&self.kick_saturation_pre_filter),
            (1, 0) => Some(&self.snare_snap),
            (1, 1) => Some(&self.snare_saturation_type),
            (1, 2) => Some(&self.snare_saturation_amount),
            (1, 3) => Some(&self.snare_saturation_mix),
            (1, 4) => Some(&self.snare_saturation_output_gain),
            (1, 5) => Some(&self.snare_saturation_pre_filter),
            (2, 0) => Some(&self.hihat_saturation_type),
            (2, 1) => Some(&self.hihat_saturation_amount),
            (2, 2) => Some(&self.hihat_saturation_mix),
            (2, 3) => Some(&self.hihat_saturation_output_gain),
            (2, 4) => Some(&self.hihat_saturation_pre_filter),
            (3, 0) => Some(&self.open_hihat_saturation_type),
            (3, 1) => Some(&self.open_hihat_saturation_amount),
            (3, 2) => Some(&self.open_hihat_saturation_mix),
            (3, 3) => Some(&self.open_hihat_saturation_output_gain),
            (3, 4) => Some(&self.open_hihat_saturation_pre_filter),
            (4 | 5 | 6, 0) => Some(&self.tom_stick),
            (4 | 5 | 6, 1) => Some(&self.tom_saturation_type),
            (4 | 5 | 6, 2) => Some(&self.tom_saturation_amount),
            (4 | 5 | 6, 3) => Some(&self.tom_saturation_mix),
            (4 | 5 | 6, 4) => Some(&self.tom_saturation_output_gain),
            (4 | 5 | 6, 5) => Some(&self.tom_saturation_pre_filter),
            (7, 0) => Some(&self.clap_echo),
            (7, 1) => Some(&self.clap_saturation_type),
            (7, 2) => Some(&self.clap_saturation_amount),
            (7, 3) => Some(&self.clap_saturation_mix),
            (7, 4) => Some(&self.clap_saturation_output_gain),
            (7, 5) => Some(&self.clap_saturation_pre_filter),
            (8, 0) => Some(&self.ride_saturation_type),
            (8, 1) => Some(&self.ride_saturation_amount),
            (8, 2) => Some(&self.ride_saturation_mix),
            (8, 3) => Some(&self.ride_saturation_output_gain),
            (8, 4) => Some(&self.ride_saturation_pre_filter),
            (9, 0) => Some(&self.cymbal_shimmer_freq),
            (9, 1) => Some(&self.cymbal_noise_type),
            (9, 2) => Some(&self.cymbal_shimmer_amount),
            (9, 3) => Some(&self.cymbal_saturation_type),
            (9, 4) => Some(&self.cymbal_saturation_amount),
            (9, 5) => Some(&self.cymbal_saturation_mix),
            (9, 6) => Some(&self.cymbal_saturation_output_gain),
            (9, 7) => Some(&self.cymbal_saturation_pre_filter),
            (10, 0) => Some(&self.snare606_resonance),
            (10, 1) => Some(&self.snare606_tone),
            (10, 2) => Some(&self.snare606_snap),
            (10, 3) => Some(&self.snare606_saturation_type),
            (10, 4) => Some(&self.snare606_saturation_amount),
            (10, 5) => Some(&self.snare606_saturation_mix),
            (10, 6) => Some(&self.snare606_saturation_output_gain),
            (10, 7) => Some(&self.snare606_saturation_pre_filter),
            (11, 0) => Some(&self.bassdrum808_accent),
            (11, 1) => Some(&self.bassdrum808_snap),
            (11, 2) => Some(&self.bassdrum808_pitch_drop),
            (11, 3) => Some(&self.bassdrum808_click_tone),
            (11, 4) => Some(&self.bassdrum808_saturation_type),
            (11, 5) => Some(&self.bassdrum808_saturation_amount),
            (11, 6) => Some(&self.bassdrum808_saturation_mix),
            (11, 7) => Some(&self.bassdrum808_saturation_output_gain),
            (11, 8) => Some(&self.bassdrum808_saturation_pre_filter),
            (12, 0) => Some(&self.perc1_sweep),
            (12, 1) => Some(&self.perc1_speed),
            (12, 2) => Some(&self.perc1_bite),
            (12, 3) => Some(&self.perc1_width),
            (12, 4) => Some(&self.perc1_saturation_type),
            (12, 5) => Some(&self.perc1_saturation_amount),
            (12, 6) => Some(&self.perc1_saturation_mix),
            (12, 7) => Some(&self.perc1_saturation_output_gain),
            (12, 8) => Some(&self.perc1_saturation_pre_filter),
            _ => None,
        }
    }
}

impl Default for DrumFlashVst {
    fn default() -> Self {
        let params = Arc::new(DrumFlashParams::default());
        let pattern = params.pattern_state.shared();
        let default_masks = pattern.step_masks();
        let voice_test_triggers = Arc::new(std::array::from_fn(|_| AtomicBool::new(false)));
        let external_midi_triggers = Arc::new(std::array::from_fn(|_| AtomicBool::new(false)));
        let sound_settings_state = params.sound_settings.state.clone();
        let song_state = {
            if let Ok(bank) = params.pattern_bank.bank.try_lock() {
                bank.song.clone()
            } else {
                pattern_bank::SongSequence::default()
            }
        };
        // [228] The voices read the instance's user textures through the pool.
        let mut synthesizer = DrumSynthesizer::new();
        synthesizer.set_texture_pool(params.user_textures.pool.clone());
        let mut plugin = Self {
            params,
            pattern: pattern.clone(),
            sequencer: Sequencer::new(pattern.clone()),
            synthesizer,
            sample_rate: 44100.0,
            current_step: Arc::new(AtomicU32::new(0)),
            current_steps: Arc::new(std::array::from_fn(|_| AtomicU32::new(0))),
            last_step_masks: default_masks,
            voice_test_triggers: voice_test_triggers.clone(),
            external_midi_triggers: external_midi_triggers.clone(),
            sound_settings_state: sound_settings_state.clone(),
            last_sound_settings_version: 0,
            lfo_bpm: 120.0,
            last_push_bpm: 120.0,
            last_macro_values: [-1.0; macros::MACRO_COUNT],
            last_host_pos: None,
            rng_state: AtomicU32::new(0xACE1_0000),
            pending_triggers: [(0, 0, 0.0, 0, false, synthesis::VoiceSettings::default()); 128],
            pending_trigger_count: 0,
            save_pattern_request: Arc::new(AtomicU32::new(0)),
            load_pattern_request: Arc::new(AtomicU32::new(0)),
            clear_plocks_request: Arc::new(AtomicBool::new(false)),
            song_mode: Arc::new(AtomicBool::new(false)),
            song_mode_was_active: false,
            song_needs_init: false,
            song_state,
            song_position: Arc::new(AtomicU32::new(0)),
            last_loop_count: 0,
            song_repeat_counter: 0,
            last_song_position: 0,
            last_master_length: 16,
            last_slot_kinds: [None; crate::track::MAX_TRACKS],
            rumble_beat_tick: [i64::MIN; crate::track::FX_SLOT_COUNT],
            last_algos: [u8::MAX; crate::track::MAX_TRACKS],
            advance_hits: [0; crate::track::MAX_TRACKS],
            advance_last_loop: 0,
            advance_last_page: 0,
            pending_pattern_length: Arc::new(AtomicI32::new(0)),
            temp_plock_bytes: [0; pattern_bank::MAX_PLOCK_BYTES],
            temp_seq_plock_bytes: [0; pattern_bank::MAX_SEQ_PLOCK_BYTES],
            temp_fusion_bytes: [0; pattern_bank::MAX_FUSION_BYTES],
            audio_last_loaded_slot: Arc::new(AtomicU32::new(u32::MAX)),
            audio_master_length: 16,
            last_param_master_length: 16,
            pending_song_pattern_restart: false,
            pending_midi_pattern_slot: None,
            deferred_save_slot: None,
            deferred_load_slot: None,
        };
        plugin.sequencer.play();
        plugin
    }
}

/// Silence every other active slot sharing `slot_idx`'s choke group.
/// Generalizes the old HH→OH choke to any instrument, driven by the per-slot
/// `choke_group` (0 = none) set in the Track tab. Lock-free, audio-thread safe.
fn apply_choke_groups(
    synthesizer: &mut synthesis::DrumSynthesizer,
    layout: &track::AtomicTrackLayout,
    slot_idx: usize,
) {
    let group = layout.choke_group_for_slot(slot_idx);
    if group == 0 {
        return;
    }
    for s in 0..track::MAX_TRACKS {
        if s != slot_idx && layout.is_active(s) && layout.choke_group_for_slot(s) == group {
            synthesizer.reset_voice(s);
        }
    }
}

impl DrumFlashVst {
    fn remember_current_pattern(&mut self) {
        self.last_step_masks = self.pattern.step_masks();
    }

    /// Simple LCG random number generator (audio-thread safe, no allocation).
    fn next_rand(&self) -> f32 {
        let prev = self.rng_state.load(Ordering::Relaxed);
        let next = prev.wrapping_mul(1103515245).wrapping_add(12345);
        self.rng_state.store(next, Ordering::Relaxed);
        ((next >> 16) & 0x7FFF) as f32 / 32767.0
    }

    /// Build VoiceSettings including the correct special params for each instrument.
    fn voice_settings_for(
        &self,
        slot_idx: usize,
        voice_idx: usize,
        freq: f32,
        decay: f32,
        vol: f32,
        filt: f32,
        attack: f32,
        release: f32,
        decay_curve: f32,
        release_curve: f32,
        hold: f32,
        filter_env_amount: f32,
        filter_env_decay: f32,
        analog: f32,
        stereo: f32,
    ) -> synthesis::VoiceSettings {
        // Specials live per SLOT (two slots of the same kind are independent).
        let mut special = self.sound_settings_state.instruments[slot_idx].load_specials();
        // [304] An LFO rate synced to the tempo: one cycle per note value at the host tempo,
        // inside the knob's own range.
        for def in crate::instrument_registry::INSTRUMENTS[voice_idx].special_params {
            let Some(k) = crate::instrument_registry::lfo_sync_slot(def.name) else {
                continue;
            };
            let entry = self.params.track_layout.state.lfo_sync(slot_idx, k) as usize;
            if let Some(beats) = synthesis::NOTE_SYNC_BEATS.get(entry).copied().filter(|b| *b > 0.0) {
                let hz = self.lfo_bpm / (60.0 * beats);
                if let Some(cell) = special.get_mut(def.special_index) {
                    *cell = hz.clamp(def.min, def.max);
                }
            }
        }
        // Algo params are positional per slot; clamp to the kind's algo count
        // because every param now shares the widest range.
        let algo_count = crate::instrument_registry::INSTRUMENTS[voice_idx]
            .algo_count
            .max(1) as u8;
        let algo = (self.params.algos()[slot_idx].value().max(0) as u8).min(algo_count - 1);
        synthesis::VoiceSettings {
            frequency: freq,
            decay,
            volume: vol,
            filter_freq: filt,
            attack,
            release,
            decay_curve,
            release_curve,
            hold,
            filter_env_amount,
            filter_env_decay,
            analog,
            stereo,
            algo,
            special,
        }
    }

    /// Build the final VoiceSettings for a slot at the current sequencer step,
    /// merging global settings with any per-step plock override.
    /// Standard settings, specials and plocks are all stored PER SLOT.
    fn voice_settings_at_step(
        &self,
        slot_idx: usize,
        voice_idx: usize,
        step: usize,
    ) -> synthesis::VoiceSettings {
        let inst = &self.sound_settings_state.instruments[slot_idx];
        let (freq, decay, vol, filt, attack, release, dc, rc, hold, fea, fed, analog, stereo) =
            inst.load();
        let global = self.voice_settings_for(
            slot_idx, voice_idx, freq, decay, vol, filt, attack, release, dc, rc, hold, fea, fed,
            analog, stereo,
        );
        self.params
            .plock_state
            .state
            .get_settings(slot_idx, step, &global)
            .unwrap_or(global)
    }

    /// Read the current value of a plock field from VoiceSettings.
    /// Read the morph source value for a p-lock field index.
    ///
    /// [184] The field-to-member mapping lives in `VoiceSettings::get`; this used
    /// to be a hand-written index list that had to stay in sync with four other
    /// copies. Still audio-thread safe: a plain match, no allocation.
    fn read_morph_value(
        &self,
        settings: &synthesis::VoiceSettings,
        field_index: usize,
        _voice_idx: usize,
    ) -> f32 {
        crate::param_id::ParamId::from_plock_field(field_index)
            .map(|id| settings.get(id))
            .unwrap_or(0.0)
    }

    /// Apply a morphed value to the given VoiceSettings field ([184]).
    fn apply_morph_value(
        &self,
        settings: &mut synthesis::VoiceSettings,
        field_index: usize,
        _voice_idx: usize,
        value: f32,
    ) {
        if let Some(id) = crate::param_id::ParamId::from_plock_field(field_index) {
            settings.set(id, value);
        }
    }

    /// Save the current pattern into the bank slot.
    fn save_pattern_to_slot(&mut self, slot: usize) -> PatternBankActionResult {
        if slot >= pattern_bank::SLOT_COUNT {
            return PatternBankActionResult::Skipped;
        }
        let plock = &self.params.plock_state.state;
        let seq_plock = &self.params.seq_plock_state.state;
        let mut bank = match self.params.pattern_bank.bank.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return PatternBankActionResult::Busy,
            Err(TryLockError::Poisoned(_)) => return PatternBankActionResult::Skipped,
        };

        bank.slots[slot].capture(
            &self.pattern,
            plock,
            seq_plock,
            self.params.pattern_length.value() as u8,
        );
        drop(bank);
        // [244] RT-safe: the persisted snapshot is rebuilt lazily by
        // `PersistentField::map()` when the host asks for the state, off the
        // audio callback. `refresh_snapshot()` would lock, deep-clone the
        // bank and serialize JSON right here in process().
        self.params.pattern_bank.mark_snapshot_dirty();
        #[cfg(debug_assertions)]
        {
            nih_log!("Pattern saved to slot P{}", slot + 1);
        }
        // Mark this slot as the current one so the UI highlights it in green.
        self.audio_last_loaded_slot
            .store(slot as u32, Ordering::Relaxed);
        PatternBankActionResult::Applied
    }

    /// Load a pattern from the bank slot.
    fn load_pattern_from_slot(&mut self, slot: usize) -> PatternBankActionResult {
        if slot >= pattern_bank::SLOT_COUNT {
            return PatternBankActionResult::Skipped;
        }

        // 1. Copy slot data under the lock (fast), then release before restore.
        let mut step_masks = [0u16; crate::sequencer::pattern::STEP_COUNT];
        let plock_len: usize;
        let seq_plock_len: usize;
        let fusion_len: usize;
        let pattern_length: u8;
        {
            let bank = match self.params.pattern_bank.bank.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::WouldBlock) => return PatternBankActionResult::Busy,
                Err(TryLockError::Poisoned(_)) => return PatternBankActionResult::Skipped,
            };
            let slot_ref = &bank.slots[slot];
            if !slot_ref.occupied {
                #[cfg(debug_assertions)]
                {
                    nih_log!("Slot P{} is empty", slot + 1);
                }
                return PatternBankActionResult::Skipped;
            }
            let maybe_len = slot_ref.copy_data_for_restore(
                &mut step_masks,
                &mut self.temp_plock_bytes,
                &mut self.temp_seq_plock_bytes,
                &mut self.temp_fusion_bytes,
            );
            pattern_length = match maybe_len {
                Some(len) => len,
                None => return PatternBankActionResult::Skipped,
            };
            plock_len = slot_ref.plock_bytes.len();
            seq_plock_len = slot_ref.seq_plock_bytes.len();
            fusion_len = slot_ref.fusion_bytes.len();
        } // lock released here

        // 2. Wipe old plocks so nothing leaks from the previous pattern, then restore.
        let plock = &self.params.plock_state.state;
        let seq_plock = &self.params.seq_plock_state.state;
        #[cfg(debug_assertions)]
        {
            nih_log!(
                "[LOAD] Slot P{}: plock_len={}, seq_plock_len={}, fusion_len={}, pattern_length={}",
                slot + 1,
                plock_len,
                seq_plock_len,
                fusion_len,
                pattern_length
            );
        }
        plock.clear_all();
        seq_plock.clear_all();
        // Clear fusions (Step Fusion feature)
        for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
            self.pattern.store_fusions(inst, &[]);
        }
        #[cfg(debug_assertions)]
        {
            nih_log!(
                "[LOAD] After clear_all: plock mask[0]={:016x}, seq_plock mask[0]={:016x}",
                plock.masks.masks[0].load(Ordering::Relaxed),
                seq_plock.masks[0].load(Ordering::Relaxed)
            );
        }
        pattern_bank::restore_from_buffers(
            &step_masks,
            &self.temp_plock_bytes[..plock_len],
            &self.temp_seq_plock_bytes[..seq_plock_len],
            &self.temp_fusion_bytes[..fusion_len],
            &self.pattern,
            plock,
            seq_plock,
        );
        #[cfg(debug_assertions)]
        {
            nih_log!(
                "[LOAD] After restore: plock mask[0]={:016x}, seq_plock mask[0]={:016x}",
                plock.masks.masks[0].load(Ordering::Relaxed),
                seq_plock.masks[0].load(Ordering::Relaxed)
            );
        }

        // 3. Notify UI and record which slot the audio thread actually loaded.
        self.audio_master_length = pattern_length.clamp(1, 64) as usize;
        self.pending_pattern_length
            .store(pattern_length as i32, Ordering::Relaxed);
        self.audio_last_loaded_slot
            .store(slot as u32, Ordering::Relaxed);
        #[cfg(debug_assertions)]
        {
            nih_log!("Pattern loaded from slot P{}", slot + 1);
        }
        PatternBankActionResult::Loaded(pattern_length)
    }

    /// Returns the MIDI note and output channel for a given slot.
    ///
    /// The note is taken from the track layout (per-slot, editable in the Track
    /// tab), not the instrument registry default. The channel is the global MIDI
    /// channel (1-16) converted to 0-based indexing for `NoteEvent`.
    fn resolve_midi_output_for_slot(&self, slot_idx: usize) -> (u8, u8) {
        let note = self.params.track_layout.state.midi_note_for_slot(slot_idx);
        let channel = self
            .params
            .track_layout
            .state
            .global_midi_channel()
            .saturating_sub(1)
            .min(15);
        (note, channel)
    }

    /// Fire a single voice trigger (audio + MIDI) at the given sample offset.
    /// `slot_idx` is the active track slot; `voice_idx` is the legacy DrumVoice
    /// algorithm used for MIDI note / instrument metadata.
    /// `hard` uses `trigger_hard()` for machine-gun stutter repeats instead of
    /// the smooth anti-click `trigger()`.
    /// [227] Which advance step a hit on `slot` gets. Switch off: none.
    /// Counting every cell: derived from the cell's position, no state (the
    /// cell within the page, within the pattern, or since the start). Counting
    /// hits: the slot's counter, reset by `reset_advance_counts_on_wrap`.
    fn advance_hit_index(&mut self, slot: usize, step: usize) -> u32 {
        use advance_mode::*;
        let mode = self.params.advance_modes()[slot].value();
        if mode & ON == 0 {
            return 0;
        }
        if mode & EVERY_CELL != 0 {
            return if mode & RESET_PAGE != 0 {
                (step % 16) as u32
            } else if mode & RESET_PATTERN != 0 {
                step as u32
            } else {
                (self.sequencer.loop_count() * (self.audio_master_length as usize).max(1) + step)
                    as u32
            };
        }
        let n = self.advance_hits[slot];
        self.advance_hits[slot] = n.wrapping_add(1);
        n
    }

    /// [227] Once per sample, after the sequencer moved: a pattern wrap or a
    /// page change resets the hit count of every slot that asked for it, so
    /// the first hit of the new pattern or page starts from the slider's offset.
    fn reset_advance_counts_on_wrap(&mut self) {
        use advance_mode::*;
        let loop_now = self.sequencer.loop_count();
        let page_now = self.sequencer.current_step() / 16;
        let wrapped = loop_now != self.advance_last_loop;
        // A pattern wrap IS a page loop too: on a 16-step pattern the page
        // never changes, yet it does come round.
        let paged = page_now != self.advance_last_page || wrapped;
        if !wrapped && !paged {
            return;
        }
        for slot in 0..crate::track::MAX_TRACKS {
            let mode = self.params.advance_modes()[slot].value();
            if (wrapped && mode & RESET_PATTERN != 0) || (paged && mode & RESET_PAGE != 0) {
                self.advance_hits[slot] = 0;
            }
        }
        self.advance_last_loop = loop_now;
        self.advance_last_page = page_now;
    }

    fn fire_voice_trigger(
        &mut self,
        slot_idx: usize,
        voice_idx: usize,
        velocity: f32,
        step: u32,
        sample_idx: usize,
        context: &mut impl ProcessContext<Self>,
        hard: bool,
        settings: synthesis::VoiceSettings,
    ) {
        let Some(_voice) = synthesis::DrumVoice::from_index(voice_idx) else {
            return;
        };
        // [227] Every path that fires a voice (first hit, stutter, fusion
        // pulse) comes through here, so the slot's Advance count has one home.
        let hit = self.advance_hit_index(slot_idx, step as usize);
        self.synthesizer.set_hit_index(slot_idx, hit);
        self.synthesizer.set_voice_settings(slot_idx, settings);
        if hard {
            self.synthesizer.trigger_hard(slot_idx, velocity);
        } else {
            self.synthesizer.trigger(slot_idx, velocity);
        }
        apply_choke_groups(
            &mut self.synthesizer,
            &self.params.track_layout.state,
            slot_idx,
        );
        let (note, channel) = self.resolve_midi_output_for_slot(slot_idx);
        context.send_event(NoteEvent::NoteOn {
            timing: sample_idx as u32,
            voice_id: None,
            channel,
            note,
            velocity,
        });
        context.send_event(NoteEvent::NoteOff {
            timing: sample_idx as u32,
            voice_id: None,
            channel,
            note,
            velocity: 0.0,
        });
    }
}

// [269] The stages of `process()`, in the order it runs them: each one is the
// code that sat inline there, moved as is. The real-time rules hold in every
// one of them (no allocation, no blocking lock, nothing that can panic on host
// data).
impl DrumFlashVst {
    /// UI requests published since the last block: song snapshot, pattern
    /// bank save/load (retried while the bank is locked), clear p-locks.
    fn consume_ui_requests(&mut self) {
        // Consume any UI-published song sequence snapshot without blocking.
        if let Some(new_song) = self.params.song_controller.consume_latest() {
            self.song_state = new_song;
        }

        // Process pattern save/load requests from UI without blocking the audio thread.
        if self.deferred_save_slot.is_none() {
            self.deferred_save_slot = self
                .save_pattern_request
                .swap(0, Ordering::Relaxed)
                .checked_sub(1)
                .map(|slot| slot as usize)
                .filter(|&slot| slot < pattern_bank::SLOT_COUNT);
        }
        if let Some(slot) = self.deferred_save_slot.take() {
            if self.save_pattern_to_slot(slot) == PatternBankActionResult::Busy {
                self.deferred_save_slot = Some(slot);
            }
        }

        if self.deferred_load_slot.is_none() {
            self.deferred_load_slot = self
                .load_pattern_request
                .swap(0, Ordering::Relaxed)
                .checked_sub(1)
                .map(|slot| slot as usize)
                .filter(|&slot| slot < pattern_bank::SLOT_COUNT);
        }
        if let Some(slot) = self.deferred_load_slot.take() {
            if self.load_pattern_from_slot(slot) == PatternBankActionResult::Busy {
                self.deferred_load_slot = Some(slot);
            }
        }
        if self.clear_plocks_request.swap(false, Ordering::Relaxed) {
            self.params.plock_state.state.clear_all();
            self.params.seq_plock_state.state.clear_all();
            #[cfg(debug_assertions)]
            {
                nih_log!("All plocks cleared");
            }
        }
    }

    /// Follow the host transport: start/stop with it, resync on a seek.
    fn follow_host_transport(
        &mut self,
        playing: bool,
        position_beats_opt: Option<f64>,
        host_reports_timeline: bool,
        bpm: f32,
        sample_rate: f32,
    ) {
        if host_reports_timeline {
            if playing != self.sequencer.is_playing() {
                if playing {
                    self.sequencer.play();
                    // Sync on play start
                    if let Some(position_beats) = position_beats_opt {
                        self.sequencer
                            .sync_to_host(position_beats, bpm, sample_rate);
                        // If starting near beat 0, force step 0 trigger.
                        // sync_to_host overwrites previous_step, which would swallow the first step.
                        if position_beats.rem_euclid(4.0) < 0.1 {
                            self.sequencer.force_step0_trigger();
                        }
                        self.last_host_pos = Some(position_beats);
                    }
                } else {
                    self.sequencer.stop();
                    self.last_host_pos = None;
                }
            } else if playing && !self.params.song_mode.value() {
                // Detect significant host seeks and resync. Threshold raised
                // from 0.2 to 1.0 beats: at 0.2, Reaper and Bitwig's
                // sub-buffer position drift accumulated until a spurious
                // resync fired, skipping steps and producing audible drops in
                // the running mix. Studio One sends sample-accurate
                // pos_beats so the bug never surfaced there.
                if let Some(position_beats) = position_beats_opt {
                    // [212] Compare on the loop's own circle, through the
                    // sequencer's mapping: with a page loop the local position
                    // is the host's folded into the page, and a page wrap must
                    // not read as a seek (a partial last page has a span
                    // shorter than a bar).
                    let circle = if self.sequencer.page_loop().is_some() {
                        self.sequencer.loop_span_beats()
                    } else {
                        4.0
                    };
                    let host_pos_mod = self
                        .sequencer
                        .host_to_local(position_beats)
                        .rem_euclid(circle);
                    let seq_pos_mod = self.sequencer.beat_position().rem_euclid(circle);
                    let diff = (host_pos_mod - seq_pos_mod).abs();
                    // Use shortest distance on the circle
                    let diff = diff.min(circle - diff);
                    if diff > (circle * 0.5).min(1.0) {
                        self.sequencer
                            .sync_to_host(position_beats, bpm, sample_rate);
                    }
                    self.last_host_pos = Some(position_beats);
                }
            }
        } else if !self.sequencer.is_playing() {
            self.sequencer.play();
        }
    }

    /// Mute / solo / main-mix gating for this block (the sequencer gets the
    /// effective mutes). Returns each slot's routing and main-mix gain.
    fn update_mix_gating(
        &mut self,
    ) -> (
        [crate::track::TrackRouting; crate::track::MAX_TRACKS],
        [f32; crate::track::MAX_TRACKS],
    ) {
        let mute_states: [bool; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| self.params.mutes()[i].value());
        let solo_states: [bool; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| self.params.solos()[i].value());
        let track_routings: [crate::track::TrackRouting; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| self.params.track_layout.state.routing_for_slot(i));
        let main_mix_enabled: [bool; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| track_routings[i].main_on);

        let (effective_mutes, mix_gains) =
            compute_mix_gating(&mute_states, &solo_states, &main_mix_enabled);

        self.sequencer.set_mutes(effective_mutes);
        (track_routings, mix_gains)
    }

    /// [286] A hit's sends: the p-locked levels of its step, else the lane's own.
    /// The levels hold until the lane's next hit, so the tail of a hit keeps the
    /// send it was thrown with.
    fn apply_step_sends(&mut self, slot: usize, step: usize) {
        if crate::track::is_fx_slot(slot) {
            return;
        }
        let plock = &self.params.plock_state.state;
        for bus in 0..crate::track::FX_SLOT_COUNT {
            let field = crate::plock::SEND_FIELD_START + bus;
            let level = (plock.masks.is_active(slot, step)
                && plock.field_masks.is_set(slot, step, field))
            .then(|| plock.values.get(slot, step, field));
            self.synthesizer.set_send_override(slot, bus, level);
        }
    }

    /// [286] The parameters of effect bus `bus` for this moment: the sliders, with
    /// each parameter that carries p-locks on the lane replaced by its value at
    /// the playhead (`fx_locks::locked_value`). Stopped, or a lane without any
    /// lock, it is just the sliders. Atomic reads only; called at control rate.
    fn fx_params_now(
        &self,
        bus: usize,
        playing: bool,
        clock: fx_locks::StepClock,
        bpm: f32,
    ) -> Option<synthesis::FxParams> {
        use crate::param_id::ParamId;
        use synthesis::{DelayParams, FxParams, ReverbParams, RumbleParams};
        let slot = crate::track::FX_FIRST_SLOT + bus;
        let kind = self.params.track_layout.state.kind_for_slot(slot)?;
        if !kind.is_effect() {
            return None;
        }
        let inst = &self.sound_settings_state.instruments[slot];
        let (freq, decay, vol, filt, attack, release, dc, rc, hold, fea, fed, analog, stereo) =
            inst.load();
        let global = self.voice_settings_for(
            slot,
            kind.drum_voice_index(),
            freq, decay, vol, filt, attack, release, dc, rc, hold, fea, fed, analog, stereo,
        );
        let restart = global.special[synthesis::reverb_sp::NO_RESTART] < 0.5;
        let (mut params, glide) = match kind {
            crate::track::TrackInstrumentKind::Delay => (
                FxParams::Delay(DelayParams::from(&global)),
                global.special[synthesis::delay_sp::GLIDE] >= 0.5,
            ),
            crate::track::TrackInstrumentKind::Rumble => (
                FxParams::Rumble(RumbleParams::from(&global)),
                global.special[synthesis::rumble_sp::GLIDE] >= 0.5,
            ),
            _ => (
                FxParams::Reverb(ReverbParams::from(&global)),
                global.special[synthesis::reverb_sp::GLIDE] >= 0.5,
            ),
        };
        // The delay's Sync menu locks its time to a note value at the host tempo.
        let sync_entry = global.special[synthesis::delay_sp::SYNC].round().max(0.0) as usize;
        let echo_sync_entry = global.special[synthesis::rumble_sp::ECHO_SYNC].round().max(0.0) as usize;
        let with_sync = |mut p: FxParams| {
            if let FxParams::Delay(d) = &mut p {
                if let Some(ms) = synthesis::delay_synced_time_ms(sync_entry, bpm) {
                    d.time_ms = ms;
                }
            }
            if let FxParams::Rumble(r) = &mut p {
                if let Some(ms) = synthesis::delay_synced_time_ms(echo_sync_entry, bpm) {
                    r.echo_ms = ms;
                }
            }
            p
        };
        let plock = &self.params.plock_state.state;
        if !playing || plock.masks.masks[slot].load(Ordering::Relaxed) == 0 {
            return Some(with_sync(params));
        }
        let beat = self.sequencer.beat_position();
        let locked = |id: ParamId, global_value: f32| -> f32 {
            let Some(field) = id.plock_field() else {
                return global_value;
            };
            fx_locks::locked_value_restart(global_value, beat, clock, glide, restart, |step| {
                (plock.masks.is_active(slot, step)
                    && plock.field_masks.get(slot, step) & (1u64 << field) != 0)
                    .then(|| plock.values.get(slot, step, field))
            })
        };
        let volume = ParamId::Std(crate::instrument_registry::StandardField::Volume);
        match &mut params {
            FxParams::Reverb(p) => {
                use synthesis::reverb_sp as sp;
                p.size = locked(ParamId::Special(sp::SIZE), p.size);
                p.decay_s = locked(ParamId::Special(sp::DECAY), p.decay_s);
                p.damping = locked(ParamId::Special(sp::DAMPING), p.damping);
                p.predelay_ms = locked(ParamId::Special(sp::PREDELAY), p.predelay_ms);
                p.low_cut_hz = locked(ParamId::Special(sp::LOW_CUT), p.low_cut_hz);
                p.high_cut_hz = locked(ParamId::Special(sp::HIGH_CUT), p.high_cut_hz);
                p.width = locked(ParamId::Special(sp::WIDTH), p.width);
                p.level = locked(volume, p.level);
                p.gate_ms = locked(ParamId::Special(sp::GATE), p.gate_ms);
                p.gate_release_ms = locked(ParamId::Special(sp::GATE_RELEASE), p.gate_release_ms);
            }
            FxParams::Rumble(p) => {
                use synthesis::rumble_sp as sp;
                p.decay_s = locked(ParamId::Special(sp::DECAY), p.decay_s);
                p.reverb_size = locked(ParamId::Special(sp::REVERB_SIZE), p.reverb_size);
                p.reverb_predelay_ms = locked(ParamId::Special(sp::REVERB_PREDELAY), p.reverb_predelay_ms);
                p.reverb_motion_ms = locked(ParamId::Special(sp::REVERB_MOTION), p.reverb_motion_ms);
                p.reverb_blur = locked(ParamId::Special(sp::REVERB_BLUR), p.reverb_blur);
                p.high_pass_hz = locked(ParamId::Special(sp::HIGH_PASS), p.high_pass_hz);
                p.low_pass_hz = locked(ParamId::Special(sp::LOW_PASS), p.low_pass_hz);
                p.duck = locked(ParamId::Special(sp::DUCK), p.duck);
                p.duck_curve = locked(ParamId::Special(sp::DUCK_CURVE), p.duck_curve);
                p.release_ms = locked(ParamId::Special(sp::RELEASE), p.release_ms);
                p.drive = locked(ParamId::Special(sp::DRIVE), p.drive);
                p.width = locked(ParamId::Special(sp::WIDTH), p.width);
                p.echo_mix = locked(ParamId::Special(sp::ECHO_MIX), p.echo_mix);
                p.echo_ms = locked(ParamId::Special(sp::ECHO_TIME), p.echo_ms);
                p.echo_feedback = locked(ParamId::Special(sp::ECHO_FEEDBACK), p.echo_feedback);
                p.level = locked(volume, p.level);
            }
            FxParams::Delay(p) => {
                use synthesis::delay_sp as sp;
                p.time_ms = locked(ParamId::Special(sp::TIME), p.time_ms);
                p.feedback = locked(ParamId::Special(sp::FEEDBACK), p.feedback);
                p.damping = locked(ParamId::Special(sp::DAMPING), p.damping);
                p.low_cut_hz = locked(ParamId::Special(sp::LOW_CUT), p.low_cut_hz);
                p.high_cut_hz = locked(ParamId::Special(sp::HIGH_CUT), p.high_cut_hz);
                p.pingpong = locked(ParamId::Special(sp::PINGPONG), p.pingpong);
                p.width = locked(ParamId::Special(sp::WIDTH), p.width);
                p.level = locked(volume, p.level);
            }
        }
        Some(with_sync(params))
    }

    /// [286] Send levels of every instrument lane to Fx1 / Fx2, and the mute of
    /// each bus (a muted bus bypasses its reverb; Solo never touches the buses),
    /// read once per block.
    fn fx_block_state(
        &self,
    ) -> (
        [[f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
        [bool; crate::track::FX_SLOT_COUNT],
    ) {
        let layout = &self.params.track_layout.state;
        let sends = std::array::from_fn(|slot| {
            std::array::from_fn(|bus| {
                if crate::track::is_fx_slot(slot) {
                    0.0
                } else {
                    layout.fx_send(slot, bus)
                }
            })
        });
        let bypass = std::array::from_fn(|bus| {
            self.params.mutes()[crate::track::FX_FIRST_SLOT + bus].value()
        });
        (sends, bypass)
    }

    /// Per-track groove parameters and the page loop, once per block.
    /// Returns the audio-side master length.
    fn update_track_params(&mut self) -> usize {
        // Update per-track groove parameters once per buffer.
        // Lane lengths are clamped to the audio-side pattern length. Pattern loads
        // update this immediately, then the UI param catches up asynchronously.
        let param_master_length = self.params.pattern_length.value() as usize;
        if param_master_length != self.last_param_master_length {
            self.audio_master_length = param_master_length.clamp(1, 64);
            self.last_param_master_length = self.audio_master_length;
        }
        let master_length = self.audio_master_length;
        let raw_lengths: [usize; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| self.params.lengths()[i].value() as usize);
        let effective_lengths = std::array::from_fn(|i| {
            resolve_track_length(
                raw_lengths[i],
                master_length,
                self.params.lane_length_locks.is_locked(i),
            )
        });
        self.sequencer.set_track_params(
            effective_lengths,
            std::array::from_fn(|i| self.params.pushes()[i].value()),
            std::array::from_fn(|i| self.params.humanizes()[i].value()),
            master_length,
        );
        // [212] Page loop, after the master length so a page beyond the
        // pattern reads as off. Ignored in Song mode: a chain of patterns and a
        // page held in a loop contradict each other.
        let page_loop = match self.params.page_loop.value() {
            p if p >= 1 && !self.params.song_mode.value() => Some(p as usize - 1),
            _ => None,
        };
        self.sequencer.set_page_loop(page_loop);
        master_length
    }

    /// A pending Song-mode pattern restart, then the host resync when the
    /// master length changed.
    fn restart_or_resync_pattern(
        &mut self,
        master_length: usize,
        playing: bool,
        position_beats_opt: Option<f64>,
        bpm: f32,
        sample_rate: f32,
    ) {
        if self.pending_song_pattern_restart {
            self.sequencer.restart_pattern_from_step0();
            self.last_loop_count = self.sequencer.loop_count();
            self.last_master_length = master_length;
            self.pending_song_pattern_restart = false;
        }

        // If the pattern length changed, resync the sequencer to the host timeline
        // so loop_count and beat_position stay consistent with the new master length.
        if master_length != self.last_master_length {
            self.last_master_length = master_length;
            if playing && !self.params.song_mode.value() {
                if let Some(position_beats) = position_beats_opt {
                    self.sequencer
                        .sync_to_host(position_beats, bpm, sample_rate);
                }
            }
        }
    }

    /// Fusions, slot kinds, grid links and microtiming into the sequencer.
    /// Returns each slot's voice index for the rest of the block.
    fn sync_sequencer_layout(&mut self) -> [Option<usize>; crate::track::MAX_TRACKS] {
        // Sync fusions from pattern to sequencer without allocating in the audio thread.
        self.sequencer.sync_fusions_from_pattern();

        // Snapshot the active track layout and propagate it to the sequencer.
        let mut slot_voices = [None; crate::track::MAX_TRACKS];
        for slot in 0..crate::track::MAX_TRACKS {
            if let Some(kind) = self.params.track_layout.state.kind_for_slot(slot) {
                slot_voices[slot] = Some(kind.drum_voice_index());
            }
        }
        self.sequencer.set_slot_voices(slot_voices);

        // Grid linking: resolve each slot's grid source (a linked lane plays the
        // steps + fusions of the lane above it) once per buffer.
        let grid_slots: [usize; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| self.params.track_layout.state.grid_slot(i));
        self.sequencer.set_grid_slots(grid_slots);

        // Per-cell microtiming (nudge): copied from the seq-plock atomics once
        // per buffer; the sequencer fires nudged cells early/late.
        let seq_plock_state = &self.params.seq_plock_state.state;
        self.sequencer.set_microtimings(std::array::from_fn(|slot| {
            std::array::from_fn(|step| {
                f32::from_bits(seq_plock_state.microtimings[slot][step].load(Ordering::Relaxed))
            })
        }));
        slot_voices
    }

    /// Recreate the voices whose kind changed, push changed algorithms.
    fn sync_synthesizer_slots(&mut self, slot_voices: &[Option<usize>; crate::track::MAX_TRACKS]) {
        // Reinitialize synthesizer slots whose kind changed (added/removed/reassigned).
        for slot in 0..crate::track::MAX_TRACKS {
            let current_kind = self.params.track_layout.state.kind_for_slot(slot);
            if current_kind != self.last_slot_kinds[slot] {
                if let Some(kind) = current_kind {
                    self.synthesizer.reinitialize_slot(slot, kind);
                } else {
                    self.synthesizer.set_slot_active(slot, false);
                }
                self.last_slot_kinds[slot] = current_kind;
                // A recreated voice has its own default algo: push ours again.
                self.last_algos[slot] = u8::MAX;
            }
        }

        // Propagate synthesis algorithms (synthesizer is indexed by slot).
        //
        // [184] ONLY on change. This used to run unconditionally every buffer,
        // which overwrote the algo that `fire_voice_trigger` had just applied
        // from a step's p-lock: the plocked algo lasted a few milliseconds and
        // the rest of the tail reverted to the lane's algo. Every other global
        // setting is already re-pushed only when its version changes; the algo
        // was the outlier.
        for slot_idx in 0..crate::track::MAX_TRACKS {
            if let Some(voice_idx) = slot_voices[slot_idx] {
                let algo_count = crate::instrument_registry::INSTRUMENTS[voice_idx]
                    .algo_count
                    .max(1) as u8;
                let algo = (self.params.algos()[slot_idx].value().max(0) as u8).min(algo_count - 1);
                if self.last_algos[slot_idx] != algo {
                    self.synthesizer.set_algo(slot_idx, algo);
                    self.last_algos[slot_idx] = algo;
                }
            }
        }
    }

    /// One-shot migration of the legacy per-voice special params.
    fn seed_legacy_specials(&mut self, slot_voices: &[Option<usize>; crate::track::MAX_TRACKS]) {
        // One-shot migration: sessions saved before per-slot specials carried
        // them as per-voice nih-plug params — seed the per-slot storage from
        // those params (atomic stores only, RT-safe).
        if self
            .sound_settings_state
            .needs_param_seed
            .swap(false, Ordering::AcqRel)
        {
            for slot_idx in 0..crate::track::MAX_TRACKS {
                let Some(voice_idx) = slot_voices[slot_idx] else {
                    continue;
                };
                let inst = &self.sound_settings_state.instruments[slot_idx];
                for sp_def in crate::instrument_registry::INSTRUMENTS[voice_idx].special_params {
                    if let Some(param) = self.params.special_param(voice_idx, sp_def.special_index)
                    {
                        inst.set_special(sp_def.special_index, param.value());
                    }
                }
                let in_notes = match voice_idx {
                    0 => self.params.freq_mode_kick.value(),
                    11 => self.params.freq_mode_bassdrum808.value(),
                    _ => false,
                };
                inst.set_freq_mode(in_notes);
            }
            self.sound_settings_state.bump_version();
        }
    }

    /// [287] A lane saved (or a preset written) before a special existed holds 0
    /// for it, below its declared minimum: reseed it to the registry default so
    /// the slider reads its real value. Atomic loads/stores only (RT-safe); a
    /// no-op once seeded.
    fn seed_late_specials(&mut self, slot_voices: &[Option<usize>; crate::track::MAX_TRACKS]) {
        let mut changed = false;
        for slot_idx in 0..crate::track::MAX_TRACKS {
            let Some(voice_idx) = slot_voices[slot_idx] else {
                continue;
            };
            // SDrex: a lane saved with the old Noise level (and no balance) gets the balance
            // that sounds like it; the old level is cleared so this runs once.
            if voice_idx == synthesis::DrumVoice::Sdrex as usize {
                let inst = &self.sound_settings_state.instruments[slot_idx];
                let old = inst.special_value(crate::instrument_registry::SDREX_OLD_NOISE_LEVEL);
                if old > 0.0
                    && inst.special_value(crate::instrument_registry::SDREX_BALANCE) == 0.0
                {
                    inst.set_special(
                        crate::instrument_registry::SDREX_BALANCE,
                        crate::instrument_registry::sdrex_balance_from_noise_level(old),
                    );
                    inst.set_special(crate::instrument_registry::SDREX_OLD_NOISE_LEVEL, 0.0);
                    changed = true;
                }
            }
            let inst = &self.sound_settings_state.instruments[slot_idx];
            for sp_def in crate::instrument_registry::INSTRUMENTS[voice_idx].special_params {
                if crate::instrument_registry::LATE_SPECIALS.contains(&sp_def.name)
                    && inst.special_value(sp_def.special_index) < sp_def.min
                {
                    inst.set_special(sp_def.special_index, sp_def.default);
                    changed = true;
                }
                // [292] TM6 Model: the retired Auto (0) becomes the model it
                // would have picked at the lane's pitch, so the sound is kept.
                if sp_def.name == crate::instrument_registry::TM6_MODEL
                    && inst.special_value(sp_def.special_index) < 0.5
                {
                    let hz = inst.get(crate::param_id::ParamId::Std(
                        crate::instrument_registry::StandardField::Freq,
                    ));
                    inst.set_special(sp_def.special_index, if hz < 166.0 { 1.0 } else { 2.0 });
                    changed = true;
                }
            }
        }
        if changed {
            self.sound_settings_state.bump_version();
        }
    }

    fn apply_macro_knobs(&mut self, slot_voices: &[Option<usize>; crate::track::MAX_TRACKS]) {
        // [242] Macro knobs (host-driven) -> lane sound atomics, BEFORE the
        // settings poll so a macro move is heard in this very buffer. Only a
        // moved knob re-applies; the writes bump the settings version, which
        // the poll below turns into voice settings (p-locks still win per step).
        let macro_values: [f32; macros::MACRO_COUNT] =
            std::array::from_fn(|i| self.params.macro_params()[i].value());
        if macro_values != self.last_macro_values {
            self.last_macro_values = macro_values;
            macros::apply_macros(
                macro_values,
                &self.params.macro_map_state.state,
                &self.sound_settings_state,
                slot_voices,
            );
        }
    }

    fn push_sound_settings(&mut self, slot_voices: &[Option<usize>; crate::track::MAX_TRACKS]) {
        // Update global sound settings once per buffer, BEFORE triggers.
        // Previously this was inside iter_samples, which caused a click:
        // a trigger with plock settings would be overwritten by global settings
        // in the same buffer, creating a one-sample discontinuity.
        let current_version = self.sound_settings_state.version.load(Ordering::Acquire);
        // [304] A tempo change re-pushes too: synced LFO rates follow it.
        let tempo_moved = (self.lfo_bpm - self.last_push_bpm).abs() > 1.0e-3;
        if current_version != self.last_sound_settings_version || tempo_moved {
            self.last_sound_settings_version = current_version;
            self.last_push_bpm = self.lfo_bpm;
            for (slot_idx, inst) in self.sound_settings_state.instruments.iter().enumerate() {
                let Some(voice_idx) = slot_voices[slot_idx] else {
                    continue;
                };
                let Some(_voice) = synthesis::DrumVoice::from_index(voice_idx) else {
                    continue;
                };
                let (
                    freq,
                    decay,
                    vol,
                    filt,
                    attack,
                    release,
                    decay_curve,
                    release_curve,
                    hold,
                    filter_env_amount,
                    filter_env_decay,
                    analog,
                    stereo,
                ) = inst.load();
                self.synthesizer.set_voice_settings(
                    slot_idx,
                    self.voice_settings_for(
                        slot_idx,
                        voice_idx,
                        freq,
                        decay,
                        vol,
                        filt,
                        attack,
                        release,
                        decay_curve,
                        release_curve,
                        hold,
                        filter_env_amount,
                        filter_env_decay,
                        analog,
                        stereo,
                    ),
                );
            }
        }
    }

    fn step_solo_window(&self) -> u64 {
        // Step-solo window for this block: union of every soloed seq-plock
        // cell's span (a normal cell covers its own step; a fused cell covers
        // its whole span). While the sequencer plays a step inside this window,
        // only soloed lanes trigger there — every other lane is muted for those
        // steps only. Independent of the lane-level `S` tag. RT-safe: bit ops +
        // stack-buffered fusion reads, no allocation, computed once per block.
        let solo_window: u64 = {
            let seq = &self.params.seq_plock_state.state;
            let pattern = &self.pattern;
            let mut fusion_buf = [crate::sequencer::pattern::FusedGroup::default();
                crate::sequencer::pattern::MAX_FUSIONS];
            seq.solo_window(|inst, start| {
                let n = pattern.load_fusions_into(inst, &mut fusion_buf);
                fusion_buf[..n]
                    .iter()
                    .find(|g| g.start_cell as usize == start)
                    .map(|g| (g.end_cell - g.start_cell + 1) as usize)
                    .unwrap_or(1)
            })
        };
        solo_window
    }

    /// Per sample: fire the delayed triggers that are due.
    fn fire_pending_triggers(
        &mut self,
        sample_idx: usize,
        slot_voices: &[Option<usize>; crate::track::MAX_TRACKS],
        context: &mut impl ProcessContext<Self>,
    ) {
        // Process pending delayed triggers (stutter or fusion pulses).
        let mut i = 0;
        while i < self.pending_trigger_count {
            if self.pending_triggers[i].0 == 0 {
                let (_, slot_idx, velocity, step, hard, settings) = self.pending_triggers[i];
                let Some(voice_idx) = slot_voices[slot_idx] else {
                    self.pending_triggers[i] =
                        self.pending_triggers[self.pending_trigger_count - 1];
                    self.pending_trigger_count -= 1;
                    continue;
                };
                self.fire_voice_trigger(
                    slot_idx, voice_idx, velocity, step, sample_idx, context, hard, settings,
                );
                self.pending_triggers[i] = self.pending_triggers[self.pending_trigger_count - 1];
                self.pending_trigger_count -= 1;
            } else {
                self.pending_triggers[i].0 -= 1;
                i += 1;
            }
        }
    }

    /// Per sample (internal sequencer): queue a MIDI pattern-switch note.
    fn collect_pattern_switch_notes(
        &mut self,
        mut next_event: Option<PluginNoteEvent<Self>>,
        sample_idx: usize,
        context: &mut impl ProcessContext<Self>,
    ) -> Option<PluginNoteEvent<Self>> {
        // Process incoming MIDI pattern-switch notes in real-time.
        // Notes from C3 (60) upward select pattern bank slots P1..P16
        // (60-75). Only active when not in Song mode, so the two
        // pattern-change mechanisms do not fight each other.
        while let Some(event) = next_event {
            if event.timing() != sample_idx as u32 {
                break;
            }
            if let NoteEvent::NoteOn { note, .. } = event {
                const PATTERN_SWITCH_BASE: u8 = 60;
                if self.params.midi_pattern_switch.value()
                    && !self.params.song_mode.value()
                    && note >= PATTERN_SWITCH_BASE
                    && note < PATTERN_SWITCH_BASE + pattern_bank::SLOT_COUNT as u8
                {
                    self.pending_midi_pattern_slot = Some((note - PATTERN_SWITCH_BASE) as usize);
                }
            }
            next_event = context.next_event();
        }
        next_event
    }

    /// Per sample (internal sequencer): advance the sequencer and fire what
    /// it triggers (step solo, probability, conditions, fusions, stutter).
    #[allow(clippy::too_many_arguments)]
    fn run_sequencer_sample(
        &mut self,
        sample_idx: usize,
        bpm: f32,
        sample_rate: f32,
        step_duration_samples: f32,
        solo_window: u64,
        slot_voices: &[Option<usize>; crate::track::MAX_TRACKS],
        context: &mut impl ProcessContext<Self>,
    ) {
        let swing = self.params.swing.value();
        let groove_type = self.params.groove_type.value();
        let triggers = self
            .sequencer
            .process_sample(bpm, sample_rate, swing, groove_type);
        self.reset_advance_counts_on_wrap();

        for (slot_idx, trigger) in triggers.iter().enumerate() {
            let Some(voice_idx) = slot_voices[slot_idx] else {
                continue;
            };
            // [286] An effect bus has no steps to play (and sends no MIDI note).
            if synthesis::DrumVoice::is_effect_index(voice_idx) {
                continue;
            }
            if trigger.should_trigger {
                let step = trigger.step;

                // Step-solo gating: if any soloed cell covers this step,
                // only soloed lanes may trigger here (this lane is muted
                // for this step unless it is itself soloed).
                if solo_window & (1u64 << step) != 0
                    && !self.params.seq_plock_state.state.is_solo(slot_idx, step)
                {
                    continue;
                }

                let seq_params = self.params.seq_plock_state.state.get(slot_idx, step);

                // Apply sequencer probability
                let prob = seq_params.map(|sp| sp.probability).unwrap_or(1.0);
                if prob < 1.0 && self.next_rand() > prob {
                    continue; // Skip this trigger based on probability
                }

                // Apply step conditions. A trigger that fired early
                // across the pattern wrap (negative microtiming on the
                // first step) belongs to the NEXT loop.
                let loop_count = self.sequencer.loop_count() + usize::from(trigger.early_next_loop);
                let condition_passes = step_condition_passes(seq_params, loop_count);
                if !condition_passes {
                    continue;
                }

                let stutter = if trigger.is_fusion() {
                    1
                } else {
                    seq_params
                        .as_ref()
                        .map(|sp| sp.stutter_count.max(1))
                        .unwrap_or(1)
                };
                let step_for_trigger = step as u32;

                let base_settings = self.voice_settings_at_step(slot_idx, voice_idx, step);
                self.apply_step_sends(slot_idx, step);

                // Fusion pulses are not stutter plocks: they are evenly
                // distributed over the whole fused-cell duration and use
                // the start cell's sound plock for every pulse.
                if trigger.is_fusion() {
                    self.schedule_fusion_pulses(
                        slot_idx,
                        voice_idx,
                        trigger,
                        step_for_trigger,
                        base_settings,
                        step_duration_samples,
                        sample_idx,
                        context,
                    );
                    continue;
                }

                // Non-fusion: fire the first trigger immediately.
                self.fire_voice_trigger(
                    slot_idx,
                    voice_idx,
                    trigger.velocity,
                    step_for_trigger,
                    sample_idx,
                    context,
                    false,
                    base_settings,
                );

                // Schedule additional stutter triggers with temporal spacing.
                // Spacing is derived from the current step duration (which itself
                // depends on the DAW BPM) so the roll always fits musically inside
                // the step.  stutter=N => N evenly-spaced hits across one step.
                if stutter > 1 {
                    self.schedule_stutter(
                        slot_idx,
                        trigger.velocity,
                        step_for_trigger,
                        base_settings,
                        stutter,
                        step_duration_samples,
                    );
                }
            }
        }
    }

    /// [269] A fused cell's pulses, spread evenly over its whole span: the
    /// first fires now, the others are queued; with morph targets each pulse
    /// interpolates them.
    #[allow(clippy::too_many_arguments)]
    fn schedule_fusion_pulses(
        &mut self,
        slot_idx: usize,
        voice_idx: usize,
        trigger: &crate::sequencer::TriggerResult,
        step_for_trigger: u32,
        base_settings: synthesis::VoiceSettings,
        step_duration_samples: f32,
        sample_idx: usize,
        context: &mut impl ProcessContext<Self>,
    ) {
        let pulse_count = trigger.fusion_pulse_count.max(1);
        let spacing = if pulse_count > 1 {
            (step_duration_samples * trigger.fusion_span_cells.max(1) as f32 / pulse_count as f32)
                .max(1.0) as u32
        } else {
            0
        };
        let morph_active = trigger.morph_count > 0;
        for k in 0..pulse_count {
            let mut settings = base_settings;
            if morph_active && pulse_count > 1 {
                let t = k as f32 / (pulse_count - 1) as f32;
                for m in 0..trigger.morph_count as usize {
                    let target = trigger.morph_targets[m];
                    let lane_value =
                        self.read_morph_value(&settings, target.field as usize, voice_idx);
                    let (start_value, end_value) = match target.direction {
                        crate::sequencer::pattern::MorphDirection::Target => {
                            (lane_value, target.end_value)
                        }
                        crate::sequencer::pattern::MorphDirection::Source => {
                            (target.end_value, lane_value)
                        }
                    };
                    let morphed = start_value + (end_value - start_value) * t;
                    self.apply_morph_value(
                        &mut settings,
                        target.field as usize,
                        voice_idx,
                        morphed,
                    );
                }
            }
            if k == 0 {
                self.fire_voice_trigger(
                    slot_idx,
                    voice_idx,
                    trigger.velocity,
                    step_for_trigger,
                    sample_idx,
                    context,
                    false,
                    settings,
                );
            } else if self.pending_trigger_count < self.pending_triggers.len() {
                self.pending_triggers[self.pending_trigger_count] = (
                    spacing * k as u32,
                    slot_idx,
                    trigger.velocity,
                    step_for_trigger,
                    false,
                    settings,
                );
                self.pending_trigger_count += 1;
            }
        }
    }

    /// [269] Stutter = N evenly spaced hits across one step: the first one
    /// already fired, the N - 1 others are queued as hard retriggers.
    fn schedule_stutter(
        &mut self,
        slot_idx: usize,
        velocity: f32,
        step_for_trigger: u32,
        base_settings: synthesis::VoiceSettings,
        stutter: u8,
        step_duration_samples: f32,
    ) {
        let spacing = (step_duration_samples / stutter as f32).max(1.0) as u32;
        for k in 1..stutter {
            if self.pending_trigger_count < self.pending_triggers.len() {
                self.pending_triggers[self.pending_trigger_count] = (
                    spacing * k as u32,
                    slot_idx,
                    velocity,
                    step_for_trigger,
                    true,
                    base_settings,
                );
                self.pending_trigger_count += 1;
            }
        }
    }

    /// Per sample (external MIDI mode): trigger the lanes listening to each
    /// incoming note, and forward it on the global MIDI channel.
    fn trigger_from_midi_notes(
        &mut self,
        mut next_event: Option<PluginNoteEvent<Self>>,
        sample_idx: usize,
        slot_voices: &[Option<usize>; crate::track::MAX_TRACKS],
        context: &mut impl ProcessContext<Self>,
    ) -> Option<PluginNoteEvent<Self>> {
        // MIDI mode: process incoming NoteOn events
        while let Some(event) = next_event {
            if event.timing() != sample_idx as u32 {
                break;
            }
            if let NoteEvent::NoteOn { note, velocity, .. } = event {
                // [221] Match the LANE's own MIDI note, the one the
                // plugin also sends on — not the registry's factory
                // note. Several lanes may share a note on purpose, and
                // they all trigger.
                let listening = self.params.track_layout.state.slots_listening_to(note);
                if listening != 0 {
                    for slot_idx in 0..crate::track::MAX_TRACKS {
                        if listening & (1 << slot_idx) == 0 {
                            continue;
                        }
                        let Some(voice_idx) = slot_voices[slot_idx] else {
                            continue;
                        };
                        self.external_midi_triggers[slot_idx].store(true, Ordering::Release);
                        let settings = self.voice_settings_at_step(slot_idx, voice_idx, 0);
                        self.apply_step_sends(slot_idx, 0);
                        let hit = self.advance_hit_index(slot_idx, 0);
                        self.synthesizer.set_hit_index(slot_idx, hit);
                        self.synthesizer.set_voice_settings(slot_idx, settings);
                        self.synthesizer.trigger(slot_idx, velocity);
                        apply_choke_groups(
                            &mut self.synthesizer,
                            &self.params.track_layout.state,
                            slot_idx,
                        );
                    }
                    // Forward the MIDI event to the output on the global
                    // MIDI channel. The note itself is preserved from the
                    // incoming event.
                    let output_channel = self
                        .params
                        .track_layout
                        .state
                        .global_midi_channel()
                        .saturating_sub(1)
                        .min(15);
                    context.send_event(NoteEvent::NoteOn {
                        timing: sample_idx as u32,
                        voice_id: None,
                        channel: output_channel,
                        note,
                        velocity,
                    });
                    context.send_event(NoteEvent::NoteOff {
                        timing: sample_idx as u32,
                        voice_id: None,
                        channel: output_channel,
                        note,
                        velocity: 0.0,
                    });
                }
            }
            next_event = context.next_event();
        }
        next_event
    }

    /// Per sample: the Sound panel's test triggers.
    fn fire_voice_test_triggers(
        &mut self,
        slot_voices: &[Option<usize>; crate::track::MAX_TRACKS],
    ) {
        for (slot_idx, trigger) in self.voice_test_triggers.iter().enumerate() {
            if trigger.swap(false, Ordering::Acquire) {
                let Some(voice_idx) = slot_voices[slot_idx] else {
                    continue;
                };
                let Some(_voice) = synthesis::DrumVoice::from_index(voice_idx) else {
                    continue;
                };
                let settings = self.voice_settings_at_step(slot_idx, voice_idx, 0);
                self.synthesizer.set_voice_settings(slot_idx, settings);
                self.synthesizer.trigger(slot_idx, 0.8);
            }
        }
    }

    /// [283] MIDI Pat switches at once, without the UI's unsaved-changes
    /// warning: the grid's edits are first saved into the slot it came from,
    /// as Song mode does, so nothing is lost. Nothing is saved when the grid
    /// already matches that slot, when it comes from no slot, or when the
    /// target slot is empty (the switch will not happen). RT-safe: `try_lock`
    /// and `capture` into the slot's preallocated buffers.
    fn keep_edits_before_midi_switch(&mut self, target: usize) -> PatternBankActionResult {
        let current = self.audio_last_loaded_slot.load(Ordering::Relaxed) as usize;
        if current >= pattern_bank::SLOT_COUNT || target >= pattern_bank::SLOT_COUNT {
            return PatternBankActionResult::Skipped;
        }
        let plock = &self.params.plock_state.state;
        let seq_plock = &self.params.seq_plock_state.state;
        let length = self.params.pattern_length.value() as u8;
        let mut bank = match self.params.pattern_bank.bank.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return PatternBankActionResult::Busy,
            Err(TryLockError::Poisoned(_)) => return PatternBankActionResult::Skipped,
        };
        if !bank.slots[target].occupied {
            return PatternBankActionResult::Skipped;
        }
        let slot = &mut bank.slots[current];
        let grid_empty = self.pattern.step_masks().iter().all(|&m| m == 0);
        if (!slot.occupied && grid_empty)
            || slot.matches_live(&self.pattern, plock, seq_plock, length)
        {
            return PatternBankActionResult::Skipped;
        }
        slot.capture(&self.pattern, plock, seq_plock, length);
        drop(bank);
        // RT-safe: the persisted snapshot is rebuilt lazily off the audio
        // callback (see `save_pattern_to_slot`).
        self.params.pattern_bank.mark_snapshot_dirty();
        PatternBankActionResult::Applied
    }

    fn apply_midi_pattern_switch(&mut self) {
        // Apply a MIDI pattern-switch request received during the audio block.
        // Only active when not in Song mode, so the two live-switch mechanisms
        // do not compete. Pattern loads are retried if the bank is temporarily
        // locked by the UI thread.
        //
        // [171] NO restart here: the new pattern picks up at the current
        // position ("à la volée"). If its length differs, the length-change
        // handler above resyncs to the host timeline modulo the new length —
        // so a playhead on a page that no longer exists lands back inside the
        // pattern (page 1 for a 1-page pattern).
        if self.params.midi_pattern_switch.value() && !self.params.song_mode.value() {
            if let Some(slot) = self.pending_midi_pattern_slot.take() {
                if let PatternBankActionResult::Busy = self.keep_edits_before_midi_switch(slot) {
                    self.pending_midi_pattern_slot = Some(slot);
                    return;
                }
                match self.load_pattern_from_slot(slot) {
                    PatternBankActionResult::Loaded(_) | PatternBankActionResult::Applied => {
                        self.audio_last_loaded_slot
                            .store(slot as u32, Ordering::Relaxed);
                    }
                    PatternBankActionResult::Busy => {
                        self.pending_midi_pattern_slot = Some(slot);
                    }
                    PatternBankActionResult::Skipped => {}
                }
            }
        }
    }

    fn advance_song_mode(&mut self) {
        // Song mode: advance to next pattern when current pattern wraps, respecting per-step repeats.
        if self.params.song_mode.value() && self.sequencer.is_playing() {
            // Detect song mode being enabled while the sequencer is already running:
            // we must restart from the top of the song sequence.
            if !self.song_mode_was_active {
                self.song_needs_init = true;
            }
            self.song_mode_was_active = true;

            let current_loop_count = self.sequencer.loop_count();

            // If the song just started (transport started with song mode on, or song mode was
            // enabled while playing), load the first song block immediately.
            if self.song_needs_init {
                self.song_needs_init = false;
                self.song_repeat_counter = 0;
                self.song_position.store(0, Ordering::Relaxed);
                self.last_song_position = 0;
                self.last_loop_count = current_loop_count;
                let first_slot = self.song_state.slot_at(0);
                match first_slot {
                    Some(slot) => {
                        match self.load_pattern_from_slot(slot) {
                            PatternBankActionResult::Loaded(_)
                            | PatternBankActionResult::Applied => {
                                self.pending_song_pattern_restart = true;
                            }
                            PatternBankActionResult::Busy => {
                                // Bank was locked; try again next block.
                                self.song_needs_init = true;
                            }
                            PatternBankActionResult::Skipped => {}
                        }
                    }
                    None => {
                        // First song block is empty: the current pattern keeps playing
                        // until the next loop advances or the user fills the block.
                    }
                }
            }

            if current_loop_count != self.last_loop_count {
                let song = &self.song_state;
                let song_length = (song.length as usize).min(SONG_BLOCKS).max(1);
                let current_pos = self.song_position.load(Ordering::Relaxed) as usize;
                // Detect a UI reset or out-of-bounds song position and restart the repeat counter.
                if current_pos != self.last_song_position || current_pos >= song_length {
                    self.last_song_position = current_pos.min(song_length - 1);
                    self.song_repeat_counter = 0;
                }
                let current_pos = self.last_song_position;
                let repeats = song.repeat_at(current_pos) as u32;

                let song_advance = if self.song_repeat_counter + 1 < repeats {
                    // Stay on the current song step for another pattern loop.
                    self.song_repeat_counter += 1;
                    self.last_loop_count = current_loop_count;
                    None
                } else {
                    // Advance to the next song step.
                    let next_pos = current_pos + 1;
                    if next_pos < song_length {
                        let slot = song.slot_at(next_pos);
                        if slot.is_some() {
                            Some((slot, Some(next_pos)))
                        } else {
                            // Empty block: loop back to the start.
                            let slot = song.slot_at(0);
                            Some((slot, Some(0)))
                        }
                    } else {
                        // End of the song: always loop back to the start.
                        let slot = song.slot_at(0);
                        Some((slot, Some(0)))
                    }
                };

                if let Some((next_slot, next_pos)) = song_advance {
                    let mut advance_song = true;
                    if let Some(slot) = next_slot {
                        match self.load_pattern_from_slot(slot) {
                            PatternBankActionResult::Loaded(_)
                            | PatternBankActionResult::Applied => {
                                self.pending_song_pattern_restart = true;
                            }
                            PatternBankActionResult::Busy => advance_song = false,
                            PatternBankActionResult::Skipped => {}
                        }
                    }

                    if advance_song {
                        self.last_loop_count = current_loop_count;
                        if let Some(pos) = next_pos {
                            self.last_song_position = pos;
                            self.song_position.store(pos as u32, Ordering::Relaxed);
                            self.song_repeat_counter = 0;
                        }
                    }
                }
            }
        } else {
            // Reset song position when not in song mode or when transport is stopped
            // so the song always starts from the top when re-enabled.
            self.song_mode_was_active = false;
            if self.params.song_mode.value() {
                self.song_needs_init = true;
            }
            self.last_loop_count = self.sequencer.loop_count();
            self.song_position.store(0, Ordering::Relaxed);
            self.song_repeat_counter = 0;
        }
    }

    /// The playheads the UI draws.
    fn publish_playheads(&self, use_internal_sequencer: bool) {
        if use_internal_sequencer {
            self.current_step
                .store(self.sequencer.current_step() as u32, Ordering::Relaxed);
            for (i, step) in self.sequencer.current_steps().iter().enumerate() {
                self.current_steps[i].store(*step as u32, Ordering::Relaxed);
            }
        } else {
            // In external MIDI mode the internal playhead is not meaningful;
            // hide it from the UI by storing an out-of-range sentinel.
            self.current_step.store(u32::MAX, Ordering::Relaxed);
            for s in self.current_steps.iter() {
                s.store(u32::MAX, Ordering::Relaxed);
            }
        }
    }
}

impl Plugin for DrumFlashVst {
    const NAME: &'static str = "Flash Drum";
    const VENDOR: &'static str = "DrumFlash";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        aux_input_ports: &[],
        aux_output_ports: &[new_nonzero_u32(2); AUX_OUT_COUNT],
        names: PortNames {
            layout: Some("Stereo mix + 14 stereo drum outs"),
            main_input: None,
            main_output: Some("Main Mix"),
            aux_inputs: &[],
            aux_outputs: &OUTPUT_PORT_NAMES,
        },
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        self.remember_current_pattern();
        let global_config = crate::config::GlobalConfig::load();
        ui::create_editor(
            self.params.clone(),
            self.current_step.clone(),
            self.current_steps.clone(),
            self.pattern.clone(),
            self.external_midi_triggers.clone(),
            self.sound_settings_state.clone(),
            self.params.plock_state.state.clone(),
            self.save_pattern_request.clone(),
            self.load_pattern_request.clone(),
            self.song_mode.clone(),
            self.song_position.clone(),
            self.pending_pattern_length.clone(),
            self.audio_last_loaded_slot.clone(),
            global_config,
        )
    }

    fn initialize(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl InitContext<Self>,
    ) -> bool {
        self.sample_rate = buffer_config.sample_rate;
        let track_layout = PersistentField::<crate::track::TrackLayoutState>::map(
            &self.params.track_layout,
            |state| state.clone(),
        );
        self.synthesizer
            .initialize_with_layout(buffer_config.sample_rate, &track_layout);
        self.last_slot_kinds = std::array::from_fn(|slot| {
            track_layout
                .slots
                .get(slot)
                .filter(|track_slot| track_slot.active)
                .map(|track_slot| track_slot.kind)
        });
        self.last_sound_settings_version = u64::MAX; // force re-sync on next process()
        self.last_algos = [u8::MAX; crate::track::MAX_TRACKS]; // ...and re-push the algos
        self.song_state = {
            // Use try_lock to avoid blocking the audio thread if the UI happens
            // to hold the bank lock during init. If the bank is unavailable, the
            // default empty song will be replaced by the UI-published snapshot
            // consumed in process().
            if let Ok(bank) = self.params.pattern_bank.bank.try_lock() {
                bank.song.clone()
            } else {
                pattern_bank::SongSequence::default()
            }
        };
        self.params.song_controller.publish(self.song_state.clone());
        self.remember_current_pattern();
        self.sequencer.play();
        self.current_step.store(0, Ordering::Relaxed);
        for s in self.current_steps.iter() {
            s.store(0, Ordering::Relaxed);
        }

        nih_log!("Flash Drum initialized at {} Hz", buffer_config.sample_rate);
        true
    }

    fn reset(&mut self) {
        self.sequencer.stop();
        self.synthesizer.reset();
        self.current_step.store(0, Ordering::Relaxed);
        for s in self.current_steps.iter() {
            s.store(0, Ordering::Relaxed);
        }
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // [269] The order of the stages matters (each one's comments say why);
        // they are the methods of the `impl DrumFlashVst` block above.
        self.consume_ui_requests();

        let transport = context.transport();
        let sample_rate = self.sample_rate;
        let bpm = transport
            .tempo
            .map(|tempo| tempo as f32)
            .unwrap_or_else(|| self.params.bpm.smoothed.next());
        let position_beats_opt = transport.pos_beats();
        let host_reports_timeline = position_beats_opt.is_some() || transport.tempo.is_some();
        let playing = transport.playing;
        self.follow_host_transport(
            playing,
            position_beats_opt,
            host_reports_timeline,
            bpm,
            sample_rate,
        );

        let (track_routings, mix_gains) = self.update_mix_gating();
        let (fx_sends, fx_bypass) = self.fx_block_state();
        // [300] Send mutes, once per block.
        self.synthesizer.set_send_mutes(std::array::from_fn(|slot| {
            std::array::from_fn(|bus| self.params.track_layout.state.fx_send_muted(slot, bus))
        }));
        // [303] The lanes' inserts, read once per block from the layout's atomics.
        for slot in 0..crate::track::FX_FIRST_SLOT {
            self.synthesizer.set_inserts(slot, self.params.track_layout.state.inserts(slot));
        }
        if !self.sequencer.is_playing() {
            self.synthesizer.clear_send_overrides();
        }
        clear_aux_outputs(aux);

        let master_length = self.update_track_params();
        self.restart_or_resync_pattern(
            master_length,
            playing,
            position_beats_opt,
            bpm,
            sample_rate,
        );
        let slot_voices = self.sync_sequencer_layout();
        self.sync_synthesizer_slots(&slot_voices);
        self.seed_legacy_specials(&slot_voices);
        self.seed_late_specials(&slot_voices);
        self.apply_macro_knobs(&slot_voices);
        self.lfo_bpm = if bpm.is_finite() && bpm > 0.0 { bpm as f32 } else { 120.0 };
        self.push_sound_settings(&slot_voices);

        // Pre-calculate step duration in samples for stutter spacing.
        let step_duration_samples = (sample_rate * 60.0 / (bpm * 4.0)).max(1.0);

        let use_internal_sequencer = self.params.use_internal_sequencer.value();
        let mut next_event = context.next_event();
        let solo_window = self.step_solo_window();

        // [302] Each lane's output peak over the block, for the lane meters.
        let mut lane_peaks = [0.0f32; crate::track::MAX_TRACKS];
        for (sample_idx, mut channel_samples) in buffer.iter_samples().enumerate() {
            self.fire_pending_triggers(sample_idx, &slot_voices, context);

            if use_internal_sequencer {
                next_event = self.collect_pattern_switch_notes(next_event, sample_idx, context);
                self.run_sequencer_sample(
                    sample_idx,
                    bpm,
                    sample_rate,
                    step_duration_samples,
                    solo_window,
                    &slot_voices,
                    context,
                );
            } else {
                next_event =
                    self.trigger_from_midi_notes(next_event, sample_idx, &slot_voices, context);
            }

            self.fire_voice_test_triggers(&slot_voices);

            // [286] Effect-lane p-locks, evaluated at control rate (every 32 samples).
            if sample_idx % 32 == 0 {
                let clock = fx_locks::StepClock {
                    steps: master_length,
                    swing: self.params.swing.value(),
                    groove: self.params.groove_type.value(),
                };
                for bus in 0..crate::track::FX_SLOT_COUNT {
                    let playing = self.sequencer.is_playing();
                    if let Some(params) = self.fx_params_now(bus, playing, clock, bpm) {
                        // [286] The Rumble's duck on the transport: a new beat (or eighth, or
                        // sixteenth) starts it, the first beat after Play included.
                        if let synthesis::FxParams::Rumble(r) = &params {
                            if playing && r.duck_source < synthesis::rumble_duck_source_kick() {
                                let per_beat = [1.0f64, 2.0, 4.0][r.duck_source as usize];
                                let tick = (self.sequencer.beat_position() * per_beat).floor() as i64;
                                if tick != self.rumble_beat_tick[bus] {
                                    self.rumble_beat_tick[bus] = tick;
                                    self.synthesizer.rumble_trigger_duck(bus);
                                }
                            } else if !playing {
                                self.rumble_beat_tick[bus] = i64::MIN;
                            }
                        }
                        let mut live = params.live_values();
                        // The Rumble's duck, as it is RIGHT NOW (the last value is not a knob).
                        if let Some(gain) = self.synthesizer.rumble_duck_gain(bus) {
                            live[synthesis::FX_LIVE_FIELDS - 1] = gain;
                        }
                        self.sound_settings_state.publish_fx_live(bus, live, playing);
                        self.synthesizer.set_fx_params(bus, params);
                    }
                }
            }

            let master_vol = self.params.master_volume.smoothed.next();
            let mut voice_outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            self.synthesizer
                .process_voice_samples_stereo(&mut voice_outputs);
            // [303] Each lane's inserts, before the sends hear it.
            self.synthesizer.process_inserts(&mut voice_outputs);
            // [286] The effect buses: sends in, wet return into the bus slots.
            self.synthesizer
                .process_fx(&mut voice_outputs, &fx_sends, fx_bypass);
            let (mixed_left, mixed_right) = main_mix(&voice_outputs, &mix_gains, master_vol);
            for (peak, out) in lane_peaks.iter_mut().zip(voice_outputs.iter()) {
                *peak = peak.max(out[0].abs()).max(out[1].abs());
            }

            for (ch, sample) in channel_samples.iter_mut().enumerate() {
                *sample = if ch == 0 { mixed_left } else { mixed_right };
            }

            write_aux_outputs(aux, sample_idx, &track_routings, &voice_outputs, master_vol);
        }

        for (slot, peak) in lane_peaks.iter().enumerate() {
            self.sound_settings_state.publish_lane_peak(slot, *peak);
        }
        // [303] The compressor inserts' gain reduction, for the editor's GR meters.
        for slot in 0..crate::track::FX_FIRST_SLOT {
            let gr = self.synthesizer.take_insert_gr(slot);
            self.sound_settings_state.publish_insert_gr(slot, gr);
        }
        self.apply_midi_pattern_switch();
        self.advance_song_mode();
        self.publish_playheads(use_internal_sequencer);

        ProcessStatus::Normal
    }

    fn filter_state(state: &mut PluginState) {
        // [282] A restored state is never a new instance: it must not get the
        // starting pattern. Sessions saved before the marker existed get it
        // here (this runs before the fields are deserialized).
        if !state.fields.contains_key(INSTANCE_SEEDED_FIELD) {
            if let Ok(serialized) = serialize_field(&true) {
                state
                    .fields
                    .insert(INSTANCE_SEEDED_FIELD.to_string(), serialized);
            }
        }

        if state.fields.contains_key(PATTERN_STATE_FIELD) {
            return;
        }

        // Migration pattern-v4 (13 instruments) → pattern-v5 (14 instruments)
        if let Some(data) = state.fields.get("pattern-v4") {
            if let Ok(old_state) = deserialize_field::<sequencer::pattern::PatternStateV4>(data) {
                let new_state = old_state.expand();
                if let Ok(serialized) = serialize_field(&new_state) {
                    state
                        .fields
                        .insert(PATTERN_STATE_FIELD.to_string(), serialized);
                    return;
                }
            }
        }

        // Migration pattern-v3 (old broken fusion layout, 13 instruments) → pattern-v5 (fixed layout)
        if let Some(data) = state.fields.get("pattern-v3") {
            if let Ok(old_state) = deserialize_field::<sequencer::pattern::PatternStateV3>(data) {
                let mut new_fusions = [0u64;
                    sequencer::pattern::INSTRUMENT_COUNT
                        * sequencer::pattern::MAX_FUSIONS
                        * sequencer::pattern::FUSION_SLOT_COUNT];
                for inst in 0..13 {
                    let base = inst
                        * sequencer::pattern::MAX_FUSIONS
                        * sequencer::pattern::FUSION_SLOT_COUNT;
                    let count_packed = old_state.fusions[base];
                    let count = (count_packed & 0xFF) as usize;
                    let mut new_count = 0usize;
                    for i in 0..count.min(sequencer::pattern::MAX_FUSIONS - 1) {
                        let slot_base = base + (1 + i) * sequencer::pattern::FUSION_SLOT_COUNT;
                        let old_slots = [
                            old_state.fusions[slot_base],
                            old_state.fusions[slot_base + 1],
                            old_state.fusions[slot_base + 2],
                        ];
                        if let Some(group) = sequencer::pattern::unpack_fusion_v3_old(old_slots) {
                            let packed = sequencer::pattern::pack_fusion(&group);
                            let new_slot_base =
                                base + (1 + new_count) * sequencer::pattern::FUSION_SLOT_COUNT;
                            new_fusions[new_slot_base] = packed[0];
                            new_fusions[new_slot_base + 1] = packed[1];
                            new_fusions[new_slot_base + 2] = packed[2];
                            new_count += 1;
                        }
                    }
                    for i in new_count..(sequencer::pattern::MAX_FUSIONS - 1) {
                        let new_slot_base = base + (1 + i) * sequencer::pattern::FUSION_SLOT_COUNT;
                        new_fusions[new_slot_base] = 0;
                        new_fusions[new_slot_base + 1] = 0;
                        new_fusions[new_slot_base + 2] = 0;
                    }
                    new_fusions[base] = (new_count as u64) | (1u64 << 24);
                }
                // The 14th instrument row starts empty.
                let empty_base =
                    13 * sequencer::pattern::MAX_FUSIONS * sequencer::pattern::FUSION_SLOT_COUNT;
                new_fusions[empty_base] = 0 | (1u64 << 24);
                let new_state = sequencer::pattern::PatternState {
                    masks: old_state.masks,
                    fusions: new_fusions,
                };
                if let Ok(serialized) = serialize_field(&new_state) {
                    state
                        .fields
                        .insert(PATTERN_STATE_FIELD.to_string(), serialized);
                    return;
                }
            }
        }

        // Migration pattern-v2 (masks only) → pattern-v4 (masks + fusions)
        if let Some(data) = state.fields.get("pattern-v2") {
            if let Ok(old_masks) = deserialize_field::<sequencer::pattern::PatternMasks>(data) {
                let new_state = sequencer::pattern::PatternState {
                    masks: old_masks.0,
                    fusions: [0u64;
                        sequencer::pattern::INSTRUMENT_COUNT
                            * sequencer::pattern::MAX_FUSIONS
                            * sequencer::pattern::FUSION_SLOT_COUNT],
                };
                if let Ok(serialized) = serialize_field(&new_state) {
                    state
                        .fields
                        .insert(PATTERN_STATE_FIELD.to_string(), serialized);
                    return;
                }
            }
        }

        // Migration pattern-v1 (16 steps) → pattern-v4 (64 steps)
        if let Some(data) = state.fields.get("pattern-v1") {
            if let Ok(old_masks) = deserialize_field::<[u16; 16]>(data) {
                let mut new_masks = [0u16; STEP_COUNT];
                new_masks[..16].copy_from_slice(&old_masks);
                let new_state = sequencer::pattern::PatternState {
                    masks: new_masks,
                    fusions: [0u64;
                        sequencer::pattern::INSTRUMENT_COUNT
                            * sequencer::pattern::MAX_FUSIONS
                            * sequencer::pattern::FUSION_SLOT_COUNT],
                };
                if let Ok(serialized) = serialize_field(&new_state) {
                    state
                        .fields
                        .insert(PATTERN_STATE_FIELD.to_string(), serialized);
                    return;
                }
            }
        }

        // Migration legacy st01..st16 (8-bit masks) → pattern-v4 (64 steps)
        let masks: [u16; STEP_COUNT] = std::array::from_fn(|step| {
            if step < 16 {
                let key = format!("st{:02}", step + 1);
                match state.params.get(&key) {
                    Some(ParamValue::I32(value)) => (*value).clamp(0, 127) as u16,
                    _ => 0,
                }
            } else {
                0
            }
        });
        let new_state = sequencer::pattern::PatternState {
            masks,
            fusions: [0u64;
                sequencer::pattern::INSTRUMENT_COUNT
                    * sequencer::pattern::MAX_FUSIONS
                    * sequencer::pattern::FUSION_SLOT_COUNT],
        };
        if let Ok(serialized_pattern) = serialize_field(&new_state) {
            state
                .fields
                .insert(PATTERN_STATE_FIELD.to_string(), serialized_pattern);
        }
    }
}

impl Vst3Plugin for DrumFlashVst {
    const VST3_CLASS_ID: [u8; 16] = VST3_CLASS_ID;
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Drum, Vst3SubCategory::Instrument];
}

nih_export_vst3!(DrumFlashVst);

#[cfg(test)]
mod tests {
    /// [283] MIDI Pat keeps the grid's edits: they are saved into the slot
    /// the grid came from before the switch, and only when there is a change
    /// and the switch will happen.
    #[test]
    fn midi_pattern_switch_saves_edits_into_their_slot_first() {
        let mut plugin = DrumFlashVst::default();
        let params = plugin.params.clone();
        let plock = &params.plock_state.state;
        let seq = &params.seq_plock_state.state;
        let length = params.pattern_length.value() as u8;
        plugin.pattern.set_step_mask(0, 0b1);
        {
            let mut bank = params.pattern_bank.bank.lock().unwrap();
            bank.slots[0].capture(&plugin.pattern, plock, seq, length);
            bank.slots[1].capture(&plugin.pattern, plock, seq, length);
        }
        plugin.audio_last_loaded_slot.store(0, Ordering::Relaxed);

        // No change: nothing to save.
        assert!(matches!(
            plugin.keep_edits_before_midi_switch(1),
            PatternBankActionResult::Skipped
        ));

        // A p-lock edit only (no step change): saved into P1.
        plock.masks.set_active(0, 0, true);
        plock.field_masks.set_raw(0, 0, 1);
        plock.values.set(0, 0, 0, 0.42);
        // Target empty: the switch will not happen, so nothing is saved.
        assert!(matches!(
            plugin.keep_edits_before_midi_switch(5),
            PatternBankActionResult::Skipped
        ));
        assert!(matches!(
            plugin.keep_edits_before_midi_switch(1),
            PatternBankActionResult::Applied
        ));
        let plock = &params.plock_state.state;
        let seq = &params.seq_plock_state.state;
        let bank = params.pattern_bank.bank.lock().unwrap();
        assert!(bank.slots[0].matches_live(&plugin.pattern, plock, seq, length));
        assert!(!bank.slots[1].matches_live(&plugin.pattern, plock, seq, length));
    }

    use super::*;
    use nih_plug::params::persist::{deserialize_field, PersistentField};
    use std::collections::BTreeMap;

    // ------------------------------------------------------------------
    // [263] Golden state fixtures: a state blob written by a PAST build
    // must keep restoring through filter_state + deserialize_fields, in
    // any field order. Round-trip tests with the current code cannot catch
    // a format regression shipped together with its own updated tests;
    // a frozen fixture can. Regenerate with `regenerate_golden_fixture`
    // (--ignored) only when the persisted format intentionally changes —
    // then keep the old file, add a fixture per format, and update the
    // frozen summary literal.
    // ------------------------------------------------------------------

    /// A state populated through every persisted field, via public APIs.
    fn populated_params() -> DrumFlashParams {
        let params = DrumFlashParams::default();

        // Track layout: three lanes with distinct kinds.
        let mut layout =
            PersistentField::<track::TrackLayoutState>::map(&params.track_layout, |s| s.clone());
        layout.slots[0] = track::TrackSlot::active_with_kind(track::TrackInstrumentKind::Kick);
        layout.slots[1] = track::TrackSlot::active_with_kind(track::TrackInstrumentKind::OneShot);
        layout.slots[2] = track::TrackSlot::active_with_kind(track::TrackInstrumentKind::Rift);
        PersistentField::set(&params.track_layout, layout);

        // Pattern steps on slots 0 and 2.
        let shared = params.pattern_state.shared();
        shared.set_step_mask(0, 0b0101);
        shared.set_step_mask(8, 0b0001);

        // Sound settings: slot 0 volume.
        params.sound_settings.state.instruments[0].set(
            param_id::ParamId::Std(instrument_registry::StandardField::Volume),
            0.42,
        );

        // Sound plock (slot 0, step 5, volume) + seq plock (slot 1, step 10).
        params.plock_state.state.set_field(0, 5, 2, 0.75);
        params.seq_plock_state.state.set_probability(1, 10, 0.5);

        // Bank: capture into slot 0, song step 0 → P1.
        {
            let mut guard = params.pattern_bank.bank.lock().unwrap();
            guard.slots[0].capture(
                &shared,
                &params.plock_state.state,
                &params.seq_plock_state.state,
                16,
            );
            guard.song.set_step(0, 0);
        }

        // Macro 0 → slot 0 volume.
        params.macro_map_state.state.set(
            0,
            Some(macros::MacroTarget::Std(
                0,
                instrument_registry::StandardField::Volume,
            )),
        );

        // Lane texture on slot 1 (missing on disk — only the path matters).
        let mut paths = vec![None; synthesis::sample_bank::LANE_TEXTURE_SLOTS];
        paths[1] = Some("C:\\flash-drum-fixture\\golden-texture.wav".to_string());
        PersistentField::set(&params.user_textures, paths);

        params
    }

    /// The stable fingerprint compared against the frozen literal.
    fn state_summary(params: &DrumFlashParams) -> String {
        let mut s = String::new();
        let layout =
            PersistentField::<track::TrackLayoutState>::map(&params.track_layout, |s| s.clone());
        for (i, slot) in layout.slots.iter().enumerate() {
            let kind = if slot.active { slot.kind.index() } else { 255 };
            s.push_str(&format!("{i}:{kind};"));
        }
        let masks = params.pattern_state.shared().step_masks();
        s.push_str(&format!("|m:{:04x},{:04x}", masks[0], masks[8]));
        let vol = params.sound_settings.state.instruments[0].get(param_id::ParamId::Std(
            instrument_registry::StandardField::Volume,
        ));
        s.push_str(&format!("|v:{vol:.3}"));
        let pv = params.plock_state.state.values.get(0, 5, 2);
        s.push_str(&format!("|p:{pv:.3}"));
        let prob = f32::from_bits(
            params.seq_plock_state.state.probabilities[1][10].load(Ordering::Relaxed),
        );
        s.push_str(&format!("|q:{prob:.3}"));
        {
            let bank = params.pattern_bank.bank.lock().unwrap();
            s.push_str(&format!(
                "|b:{}|s:{:?}",
                bank.slots[0].occupied,
                bank.song.slot_at(0)
            ));
        }
        s.push_str(&format!("|k:{:?}", params.macro_map_state.state.get(0)));
        s.push_str(&format!("|t:{:?}", params.user_textures.path(1)));
        s
    }

    #[test]
    #[ignore = "fixture generator — run `cargo test regenerate_golden -- --ignored` after an INTENDED format change, then update the frozen literal in golden_state_restores_identically"]
    fn regenerate_golden_fixture() {
        let params = populated_params();
        let state = PluginState {
            version: env!("CARGO_PKG_VERSION").to_string(),
            params: BTreeMap::new(),
            fields: params.serialize_fields(),
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        std::fs::create_dir_all(&dir).unwrap();
        let out = dir.join("state-2026-09-25.json");
        std::fs::write(&out, serde_json::to_string_pretty(&state).unwrap()).unwrap();
        println!("wrote {}", out.display());
    }

    #[test]
    fn golden_state_restores_identically() {
        let json = include_str!("../tests/fixtures/state-2026-09-25.json");
        let mut state: PluginState = serde_json::from_str(json).expect("fixture parses");
        <DrumFlashVst as Plugin>::filter_state(&mut state);
        let params = DrumFlashParams::default();
        params.deserialize_fields(&state.fields);

        // Frozen literal, captured from the fixture on 2026-09-25 ([263]).
        const EXPECTED: &str = "0:0;1:24;2:23;3:4;4:255;5:255;6:255;7:255;8:255;9:255;10:255;11:255;12:255;13:255;|m:0005,0001|v:0.420|p:0.750|q:0.500|b:true|s:Some(0)|k:Some(Std(0, Volume))|t:Some(\"C:\\\\flash-drum-fixture\\\\golden-texture.wav\")";
        assert_eq!(state_summary(&params), EXPECTED);
    }

    #[test]
    fn full_state_roundtrip_is_order_independent() {
        // The host restores fields in BTreeMap (alphabetical) order, which
        // puts track-layout-v1 last; restoring in the reverse order must
        // yield the very same state.
        let fields = populated_params().serialize_fields();
        let expected = {
            let params = DrumFlashParams::default();
            params.deserialize_fields(&fields);
            state_summary(&params)
        };

        let mut singles: Vec<(String, String)> = fields.into_iter().collect();
        singles.sort_by(|a, b| b.0.cmp(&a.0));
        let params = DrumFlashParams::default();
        for (key, value) in singles {
            let mut one = BTreeMap::new();
            one.insert(key, value);
            params.deserialize_fields(&one);
        }
        assert_eq!(state_summary(&params), expected);
    }

    #[test]
    fn fresh_plugin_and_params_default_construct_without_panic() {
        // Reproduces the fresh-instance path: build the default params + plugin,
        // then round-trip the persisted pattern bank + seq-plock snapshots the
        // way a host does on add.
        let params = DrumFlashParams::default();

        // Pattern bank default snapshot round-trip (16 slots).
        params.pattern_bank.refresh_snapshot();
        let bank_bytes = params.pattern_bank.map(|b| b.clone());
        params.pattern_bank.set(bank_bytes);

        // Seq-plock default snapshot round-trip.
        let seq_bytes = PersistentField::<Vec<u8>>::map(&params.seq_plock_state, |b| b.clone());
        PersistentField::<Vec<u8>>::set(&params.seq_plock_state, seq_bytes);

        let _plugin = DrumFlashVst::default();
    }

    #[test]
    fn morphable_fields_are_non_empty_for_all_voices() {
        for inst in 0..crate::instrument_registry::INSTRUMENTS.len() {
            let fields = crate::instrument_registry::morphable_fields(inst);
            assert!(
                !fields.is_empty(),
                "instrument {} should have morphable fields",
                inst
            );
        }
    }

    #[test]
    fn morphable_fields_have_unique_indices_and_no_standard_conflicts() {
        for inst in 0..crate::instrument_registry::INSTRUMENTS.len() {
            let fields = crate::instrument_registry::morphable_fields(inst);
            let mut seen = std::collections::HashSet::new();
            for f in &fields {
                assert!(
                    seen.insert(f.field_index),
                    "instrument {} has duplicate morphable field index {}",
                    inst,
                    f.field_index
                );
                assert!(
                    f.field_index < crate::plock::FIELD_COUNT,
                    "instrument {} has morphable field index {} out of range",
                    inst,
                    f.field_index
                );
            }
        }
    }

    #[test]
    fn morphable_fields_only_include_continuous_params() {
        use crate::instrument_registry::{special_params, INSTRUMENTS};
        for inst in 0..INSTRUMENTS.len() {
            let inst_def = &INSTRUMENTS[inst];
            let standard_indices: std::collections::HashSet<usize> = inst_def
                .standard_params
                .iter()
                .map(|def| def.field.plock_field_index())
                .collect();
            let fields = crate::instrument_registry::morphable_fields(inst);
            let special_defs = special_params(inst);
            for f in &fields {
                // Skip standard fields that happen to live in the special range (Attack at 18).
                if standard_indices.contains(&f.field_index) {
                    continue;
                }
                if f.field_index >= crate::plock::SPECIAL_FIELD_START {
                    let special_index = f.field_index - crate::plock::SPECIAL_FIELD_START;
                    let def = special_defs
                        .iter()
                        .find(|d| d.special_index == special_index)
                        .expect("morphable special field should match a defined special param");
                    assert!(
                        def.continuous,
                        "instrument {} special param {} ({}) is discrete but listed as morphable",
                        inst, special_index, def.label
                    );
                }
            }
        }
    }

    #[test]
    fn morphable_fields_do_not_overlap_standard_fields() {
        use crate::instrument_registry::INSTRUMENTS;
        for inst in 0..INSTRUMENTS.len() {
            let inst_def = &INSTRUMENTS[inst];
            let standard_indices: std::collections::HashSet<usize> = inst_def
                .standard_params
                .iter()
                .map(|def| def.field.plock_field_index())
                .collect();
            let fields = crate::instrument_registry::morphable_fields(inst);
            for f in &fields {
                // Only check special-origin fields; Attack (field 18) is a standard field.
                if standard_indices.contains(&f.field_index) {
                    continue;
                }
                assert!(
                    !standard_indices.contains(&f.field_index),
                    "instrument {} morphable field {} ({}) overlaps a standard field",
                    inst,
                    f.field_index,
                    f.label
                );
            }
        }
    }

    #[test]
    fn morphable_fields_include_checkbox_standard_fields() {
        use crate::instrument_registry::{ParamWidget, INSTRUMENTS};
        for inst in 0..INSTRUMENTS.len() {
            let inst_def = &INSTRUMENTS[inst];
            let fields = crate::instrument_registry::morphable_fields(inst);
            let field_indices: std::collections::HashSet<usize> =
                fields.iter().map(|f| f.field_index).collect();
            for def in inst_def.standard_params {
                let field_index = def.field.plock_field_index();
                assert!(
                    field_indices.contains(&field_index),
                    "instrument {} standard field {} ({:?}) is missing from morphable_fields",
                    inst,
                    def.label,
                    def.widget
                );
                if let ParamWidget::Checkbox = def.widget {
                    let f = fields
                        .iter()
                        .find(|f| f.field_index == field_index)
                        .expect("checked above");
                    assert_eq!(
                        f.min, 0.0,
                        "instrument {} checkbox field {} should have min 0.0",
                        inst, def.label
                    );
                    assert_eq!(
                        f.max, 1.0,
                        "instrument {} checkbox field {} should have max 1.0",
                        inst, def.label
                    );
                }
            }
        }
    }

    #[test]
    fn persistent_pattern_serializes_shared_pattern_edits() {
        let pattern_state = PersistentPattern::new(&Pattern::rock_pattern());
        let shared_pattern = pattern_state.shared();

        shared_pattern.set_step_mask(0, 0);
        shared_pattern.set_step_mask(1, 0x3ff);

        pattern_state.map(|state| {
            assert_eq!(state.masks[0], 0);
            assert_eq!(state.masks[1], 0x3ff);
        });

        let restored_state = sequencer::pattern::PatternState {
            masks: [3u16; STEP_COUNT],
            fusions: [0u64;
                sequencer::pattern::INSTRUMENT_COUNT
                    * sequencer::pattern::MAX_FUSIONS
                    * sequencer::pattern::FUSION_SLOT_COUNT],
        };
        pattern_state.set(restored_state);

        assert_eq!(shared_pattern.load_step_mask(0), 3);
        assert_eq!(shared_pattern.load_step_mask(15), 3);
    }

    #[test]
    fn pattern_v3_migrates_to_v5_preserving_fusion_geometry() {
        // Build an old-pattern-v3 PatternState with one fusion in old layout (13 instruments).
        let mut old_fusions =
            [0u64; 13 * sequencer::pattern::MAX_FUSIONS * sequencer::pattern::FUSION_SLOT_COUNT];
        let inst = 4usize;
        let base = inst * sequencer::pattern::MAX_FUSIONS * sequencer::pattern::FUSION_SLOT_COUNT;
        let old_meta: u64 = (2u64 << 0) | (6u64 << 8) | (4u64 << 16) | (1u64 << 24);
        let slot_base = base + sequencer::pattern::FUSION_SLOT_COUNT;
        old_fusions[slot_base] = old_meta;
        old_fusions[slot_base + 1] = 0;
        old_fusions[slot_base + 2] = 0;
        old_fusions[base] = (1u64) | (1u64 << 24);

        let mut masks = [0u16; STEP_COUNT];
        masks[0] = 0x10;

        let old_state = sequencer::pattern::PatternStateV3 {
            masks,
            fusions: old_fusions,
        };
        let mut fields = BTreeMap::new();
        fields.insert(
            "pattern-v3".to_string(),
            serialize_field(&old_state).unwrap(),
        );
        let mut state = PluginState {
            version: "0.1.0".to_string(),
            params: BTreeMap::new(),
            fields,
        };

        <DrumFlashVst as Plugin>::filter_state(&mut state);

        let serialized_pattern = state
            .fields
            .get(PATTERN_STATE_FIELD)
            .expect("pattern-v5 field should be created");
        let pattern_state: sequencer::pattern::PatternState =
            deserialize_field(serialized_pattern).expect("pattern field should deserialize");

        assert_eq!(pattern_state.masks[0], 0x10);

        // Verify the fusion geometry was preserved via PersistentPattern::set.
        let persistent = PersistentPattern::new(&Pattern::empty());
        persistent.set(pattern_state);
        let loaded = persistent.shared().load_fusions(inst);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].start_cell, 2);
        assert_eq!(loaded[0].end_cell, 6);
        assert_eq!(loaded[0].step_count, 4);
        assert_eq!(loaded[0].morph_count, 0);
    }

    /// [282] A restored state never gets the starting pattern: filter_state
    /// marks any state without the marker (sessions saved before it existed)
    /// as seeded, and leaves a saved marker as it is. A fresh instance's
    /// marker starts false, and the key is the one the params persist under.
    #[test]
    fn restored_states_are_marked_seeded_and_new_instances_are_not() {
        let mut old_session = PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        };
        DrumFlashVst::filter_state(&mut old_session);
        let marker = old_session.fields.get(INSTANCE_SEEDED_FIELD).expect("marker added");
        assert!(deserialize_field::<bool>(marker).unwrap());

        let mut saved_unseeded = PluginState {
            version: String::new(),
            params: Default::default(),
            fields: Default::default(),
        };
        saved_unseeded.fields.insert(
            INSTANCE_SEEDED_FIELD.to_string(),
            serialize_field(&false).unwrap(),
        );
        DrumFlashVst::filter_state(&mut saved_unseeded);
        let marker = saved_unseeded.fields.get(INSTANCE_SEEDED_FIELD).unwrap();
        assert!(!deserialize_field::<bool>(marker).unwrap());

        let params = DrumFlashParams::default();
        assert!(!params.instance_seeded.load(Ordering::SeqCst));
        assert!(params.serialize_fields().contains_key(INSTANCE_SEEDED_FIELD));
    }

    #[test]
    fn pattern_v4_migrates_to_v5_adding_empty_14th_row() {
        // Build a pattern-v4 state (13 instruments) with a Kick on step 0.
        let mut masks = [0u16; STEP_COUNT];
        masks[0] = 0x0001; // Kick bit
        let fusions =
            [0u64; 13 * sequencer::pattern::MAX_FUSIONS * sequencer::pattern::FUSION_SLOT_COUNT];
        let old_state = sequencer::pattern::PatternStateV4 { masks, fusions };

        let mut fields = BTreeMap::new();
        fields.insert(
            "pattern-v4".to_string(),
            serialize_field(&old_state).unwrap(),
        );
        let mut state = PluginState {
            version: "0.1.0".to_string(),
            params: BTreeMap::new(),
            fields,
        };

        <DrumFlashVst as Plugin>::filter_state(&mut state);

        let serialized_pattern = state
            .fields
            .get(PATTERN_STATE_FIELD)
            .expect("pattern-v5 field should be created");
        let pattern_state: sequencer::pattern::PatternState =
            deserialize_field(serialized_pattern).expect("pattern field should deserialize");

        // Kick bit preserved.
        assert_eq!(pattern_state.masks[0], 0x0001);
        // 14th instrument row is empty.
        assert_eq!(pattern_state.masks[13], 0);
    }

    #[test]
    fn legacy_step_params_migrate_to_persistent_pattern_field() {
        let mut params = BTreeMap::new();
        params.insert("st01".to_string(), ParamValue::I32(0));
        params.insert("st02".to_string(), ParamValue::I32(0x7f));

        let mut state = PluginState {
            version: "0.1.0".to_string(),
            params,
            fields: BTreeMap::new(),
        };

        <DrumFlashVst as Plugin>::filter_state(&mut state);

        let serialized_pattern = state
            .fields
            .get(PATTERN_STATE_FIELD)
            .expect("legacy pattern field should be created");
        let pattern_state: sequencer::pattern::PatternState =
            deserialize_field(serialized_pattern).expect("pattern field should deserialize");

        assert_eq!(pattern_state.masks[0], 0);
        assert_eq!(pattern_state.masks[1], 0x7f);
        assert_eq!(pattern_state.masks[2], 0);
    }

    #[test]
    fn master_volume_smoothing_stays_finite_from_silence() {
        let params = DrumFlashParams::default();

        params.master_volume.smoothed.reset(0.0);
        params.master_volume.smoothed.set_target(44_100.0, 0.8);

        for _ in 0..2_205 {
            assert!(params.master_volume.smoothed.next().is_finite());
        }
    }

    /// [184] A p-locked algo must survive the whole hit.
    ///
    /// The lane's algo used to be pushed to every voice **unconditionally, once
    /// per buffer** (`lib.rs` prologue), while `fire_voice_trigger` applies the
    /// step's merged settings — including its p-locked algo — at the trigger
    /// sample. So a plocked algo was overwritten a few milliseconds into the hit
    /// and the tail reverted to the lane's algo, which reads as "the p-locked
    /// algo does nothing".
    ///
    /// This test pins the mechanism the fix relies on: re-pushing the lane algo
    /// in the middle of a ringing tail **is** audible. That is why the push has
    /// to happen only when the lane's algo actually changes.
    #[test]
    fn pushing_the_lane_algo_mid_tail_is_audible_so_the_push_must_be_change_only() {
        use crate::track::{TrackInstrumentKind, TrackLayoutState, TrackSlot};

        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);

        // A kick with a long tail, so the second half of the render is pure tail.
        let mut plocked = synthesis::VoiceSettings::kick();
        plocked.decay = 1.0;
        plocked.algo = 2; // FM — clearly not the default sine

        let render = |push_lane_algo_mid_tail: bool| -> Vec<f32> {
            let mut synth = synthesis::DrumSynthesizer::new();
            synth.initialize_with_layout(48_000.0, &layout);
            // Buffer prologue: the lane's algo is 0 (sine).
            synth.set_algo(0, 0);
            // Trigger applies the STEP's settings, algo included.
            synth.set_voice_settings(0, plocked);
            synth.trigger(0, 1.0);

            let mut out = Vec::with_capacity(4800);
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            for _ in 0..2400 {
                synth.process_voice_samples_stereo(&mut outputs);
                out.push(outputs[0][0]);
            }
            if push_lane_algo_mid_tail {
                // What the old unconditional per-buffer propagation did.
                synth.set_algo(0, 0);
            }
            for _ in 0..2400 {
                synth.process_voice_samples_stereo(&mut outputs);
                out.push(outputs[0][0]);
            }
            out
        };

        let kept = render(false);
        let overwritten = render(true);

        // The attack is identical either way — the divergence is in the tail.
        let head_diff = kept
            .iter()
            .zip(&overwritten)
            .take(2400)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(
            head_diff < 1e-6,
            "the first half must be identical: {head_diff}"
        );

        let tail_diff = kept
            .iter()
            .zip(&overwritten)
            .skip(2400)
            .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(
            tail_diff > 1e-3,
            "re-pushing the lane algo mid-tail must change the sound, otherwise this regression would be invisible: {tail_diff}"
        );
    }

    #[test]
    fn choke_group_silences_same_group_slots() {
        use crate::track::{AtomicTrackLayout, TrackInstrumentKind, TrackLayoutState, TrackSlot};

        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::HiHat);
        layout.slots[1] = TrackSlot::active_with_kind(TrackInstrumentKind::OpenHiHat);
        layout.slots[2] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        layout.slots[0].routing.choke_group = 1;
        layout.slots[1].routing.choke_group = 1;
        layout.slots[2].routing.choke_group = 2; // different group: must survive
        let atomic = AtomicTrackLayout::from_state(&layout);

        let mut synth = synthesis::DrumSynthesizer::new();
        synth.initialize_with_layout(44100.0, &layout);

        let render = |synth: &mut synthesis::DrumSynthesizer, slot: usize, n: usize| -> f32 {
            let mut sum = 0.0f32;
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            for _ in 0..n {
                synth.process_voice_samples_stereo(&mut outputs);
                sum += outputs[slot][0].abs() + outputs[slot][1].abs();
            }
            sum
        };

        // OH (long decay) rings.
        synth.trigger(1, 1.0);
        let before = render(&mut synth, 1, 64);
        assert!(before > 0.0, "OH should ring before the choke: {before}");

        // HH (same group 1) triggers → OH choked; Kick (group 2) untouched.
        synth.trigger(1, 1.0); // retrigger to make sure it is ringing
        synth.trigger(0, 1.0);
        apply_choke_groups(&mut synth, &atomic, 0);
        let oh_after = render(&mut synth, 1, 2000);
        assert_eq!(
            oh_after, 0.0,
            "OH must be choked by group 1, got {oh_after}"
        );
        // Kick was never triggered here — trigger it now to prove it still works.
        synth.trigger(2, 1.0);
        let kick_out = render(&mut synth, 2, 64);
        assert!(kick_out > 0.0, "Kick in another group must still sound");
        let _ = kick_out;

        // A slot with no group chokes nothing.
        synth.trigger(1, 1.0);
        apply_choke_groups(&mut synth, &atomic, 2); // kick's group = 2
        let oh_still = render(&mut synth, 1, 256);
        assert!(oh_still > 0.0, "slot with another group must not choke OH");
    }

    #[test]
    fn add_stereo_aux_sample_skips_inactive_or_incomplete_outputs() {
        let mut empty_left: [f32; 0] = [];
        let mut empty_right: [f32; 0] = [];
        let mut inactive_channels: [&mut [f32]; 2] = [&mut empty_left, &mut empty_right];
        assert!(!add_stereo_aux_sample(&mut inactive_channels, 0, 1.0, 1.0));

        let mut mono = [0.0f32; 4];
        let mut mono_channels: [&mut [f32]; 1] = [&mut mono];
        assert!(!add_stereo_aux_sample(&mut mono_channels, 0, 1.0, 1.0));

        let mut short_left = [0.0f32; 1];
        let mut short_right = [0.0f32; 1];
        let mut short_channels: [&mut [f32]; 2] = [&mut short_left, &mut short_right];
        assert!(!add_stereo_aux_sample(&mut short_channels, 1, 1.0, 1.0));
    }

    /// [269] The step-condition logic, now one pure function out of `process()`.
    #[test]
    fn step_condition_passes_follows_loop_count_negation_and_second_term() {
        use crate::plock::{SequencerStepParams, StepCondition::*};
        let cond = |condition, condition_and, condition_negate| {
            Some(SequencerStepParams {
                condition,
                condition_and,
                condition_negate,
                ..SequencerStepParams::default()
            })
        };
        assert!(step_condition_passes(None, 7), "no seq p-lock = always");
        assert!(step_condition_passes(cond(Half1, None, false), 0));
        assert!(!step_condition_passes(cond(Half1, None, false), 1));
        assert!(step_condition_passes(cond(Half1, None, true), 1), "Not 1/2");
        assert!(!step_condition_passes(cond(First, None, false), 1));
        // [219] "1/2 and 1/3" fires every sixth loop.
        let both = cond(Half1, Some(Third1), false);
        let fired: Vec<usize> = (0..12)
            .filter(|&l| step_condition_passes(both, l))
            .collect();
        assert_eq!(fired, vec![0, 6]);
        // [218] `Not` on a bare `Always` stays "always".
        assert!(step_condition_passes(cond(Always, None, true), 3));
    }

    /// [269] The main mix, now one pure function out of `process()`.
    #[test]
    fn main_mix_weights_each_slot_and_applies_master_volume() {
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        outputs[0] = [1.0, -1.0];
        outputs[3] = [0.5, 0.25];
        let mut gains = [0.0f32; crate::track::MAX_TRACKS];
        gains[0] = 1.0;
        gains[3] = 0.5;
        let (left, right) = main_mix(&outputs, &gains, 0.5);
        assert!((left - (1.0 + 0.25) * 0.5).abs() < 1e-6);
        assert!((right - (-1.0 + 0.125) * 0.5).abs() < 1e-6);
    }

    #[test]
    fn add_stereo_aux_sample_accumulates_valid_stereo_output() {
        let mut left = [0.0f32, 0.0, 1.0, 0.0];
        let mut right = [0.0f32, 0.0, -1.0, 0.0];
        let mut channels: [&mut [f32]; 2] = [&mut left, &mut right];

        assert!(add_stereo_aux_sample(&mut channels, 2, 0.25, -0.5));

        assert_eq!(channels[0][2], 1.25);
        assert_eq!(channels[1][2], -1.5);
    }

    /// [286] The effect-lane p-lock chain end to end: a lock placed on a late step
    /// holds until the end of the loop, and the lane's own value is back at the loop's
    /// start.
    #[test]
    fn an_effect_lock_ends_where_the_pattern_loops() {
        use nih_plug::params::persist::PersistentField;
        let mut vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[12] = crate::track::TrackSlot::active_with_kind(
            crate::track::TrackInstrumentKind::Reverb,
        );
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout);
        let inst = &vst.sound_settings_state.instruments[12];
        inst.set_special(synthesis::reverb_sp::DECAY, 0.5); // the lane's own: short
        inst.set_special(synthesis::reverb_sp::GLIDE, 0.0);
        // A long decay locked on the 13th step.
        let field = crate::param_id::ParamId::Special(synthesis::reverb_sp::DECAY)
            .plock_field()
            .unwrap();
        vst.params.plock_state.state.set_field(12, 12, field, 8.0);
        vst.sequencer.play();

        let clock = fx_locks::StepClock {
            steps: 16,
            swing: 0.0,
            groove: groove::GrooveType::Straight,
        };
        let decay_at = |vst: &mut DrumFlashVst, step: f64| {
            vst.sequencer.sync_to_host(step * 0.25, 120.0, 44100.0);
            vst.fx_params_now(0, true, clock, 120.0).and_then(|p| p.reverb()).expect("a reverb lane").decay_s
        };
        println!("decay by step: {:?}", (0..16).map(|s| decay_at(&mut vst, s as f64 + 0.5)).collect::<Vec<_>>());
        assert_eq!(decay_at(&mut vst, 0.5), 0.5, "loop start: the lane's own value");
        assert_eq!(decay_at(&mut vst, 11.5), 0.5, "before the lock");
        assert_eq!(decay_at(&mut vst, 12.5), 8.0, "on the lock");
        assert_eq!(decay_at(&mut vst, 15.5), 8.0, "held to the end of the loop");
        assert_eq!(decay_at(&mut vst, 16.5), 0.5, "back at the lane's value after the wrap");
    }

    /// [286] The whole audio path of an effect lane, simulated: a kick feeds a reverb
    /// whose Decay is locked long on the 13th step. The return heard at the start of
    /// the next loop must be the lane's short decay again.
    /// [286] Glide on: the decay moves between two locks, never past the last one.
    #[test]
    fn an_effect_lane_glides_between_its_locks_when_glide_is_on() {
        use nih_plug::params::persist::PersistentField;
        let mut vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[12] = crate::track::TrackSlot::active_with_kind(
            crate::track::TrackInstrumentKind::Reverb,
        );
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout);
        vst.sound_settings_state.reset_slot_to_defaults(12, crate::track::TrackInstrumentKind::Reverb, 0.5);
        let inst = &vst.sound_settings_state.instruments[12];
        inst.set_special(synthesis::reverb_sp::DECAY, 0.5);
        inst.set_special(synthesis::reverb_sp::GLIDE, 1.0);
        let field = crate::param_id::ParamId::Special(synthesis::reverb_sp::DECAY).plock_field().unwrap();
        vst.params.plock_state.state.set_field(12, 4, field, 2.0);
        vst.params.plock_state.state.set_field(12, 12, field, 8.0);
        vst.sequencer.play();
        let clock = fx_locks::StepClock { steps: 16, swing: 0.0, groove: groove::GrooveType::Straight };
        let decay_at = |vst: &mut DrumFlashVst, step: f64| {
            vst.sequencer.sync_to_host(step * 0.25, 120.0, 44100.0);
            vst.fx_params_now(0, true, clock, 120.0).and_then(|p| p.reverb()).expect("a reverb lane").decay_s
        };
        assert_eq!(decay_at(&mut vst, 0.0), 0.5, "starts on the slider");
        let early = decay_at(&mut vst, 2.0);
        assert!((early - 1.25).abs() < 0.01, "rises towards the first lock, got {early}");
        assert_eq!(decay_at(&mut vst, 4.0), 2.0, "on the first lock");
        let mid = decay_at(&mut vst, 8.0);
        assert!((mid - 5.0).abs() < 0.01, "halfway between 2 and 8, got {mid}");
        assert_eq!(decay_at(&mut vst, 12.0), 8.0, "on the second lock");
        assert_eq!(decay_at(&mut vst, 15.5), 8.0, "held after the last lock");
        assert_eq!(decay_at(&mut vst, 16.0), 0.5, "the lane again at the next loop");
    }

    /// [286] Loop Restart off: nothing resets at the loop point, the last lock carries on.
    #[test]
    fn without_loop_restart_the_last_lock_carries_into_the_next_loop() {
        use nih_plug::params::persist::PersistentField;
        let mut vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[12] = crate::track::TrackSlot::active_with_kind(
            crate::track::TrackInstrumentKind::Reverb,
        );
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout);
        vst.sound_settings_state.reset_slot_to_defaults(12, crate::track::TrackInstrumentKind::Reverb, 0.5);
        let inst = &vst.sound_settings_state.instruments[12];
        inst.set_special(synthesis::reverb_sp::DECAY, 0.5);
        let field = crate::param_id::ParamId::Special(synthesis::reverb_sp::DECAY).plock_field().unwrap();
        vst.params.plock_state.state.set_field(12, 12, field, 8.0);
        vst.sequencer.play();
        let clock = fx_locks::StepClock { steps: 16, swing: 0.0, groove: groove::GrooveType::Straight };
        let decay_at = |vst: &mut DrumFlashVst, step: f64| {
            vst.sequencer.sync_to_host(step * 0.25, 120.0, 44100.0);
            vst.fx_params_now(0, true, clock, 120.0).and_then(|p| p.reverb()).expect("a reverb lane").decay_s
        };
        assert_eq!(decay_at(&mut vst, 0.5), 0.5, "restart on (the default): back to the lane");
        vst.sound_settings_state.instruments[12].set_special(synthesis::reverb_sp::NO_RESTART, 1.0);
        assert_eq!(decay_at(&mut vst, 0.5), 8.0, "restart off: the lock carries on");
        assert_eq!(decay_at(&mut vst, 11.5), 8.0);
    }

    /// [286] The delay's Sync menu locks its time to a note value at the host tempo.
    #[test]
    fn the_delay_time_follows_the_tempo_when_synced() {
        use nih_plug::params::persist::PersistentField;
        let vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[13] = crate::track::TrackSlot::active_with_kind(
            crate::track::TrackInstrumentKind::Delay,
        );
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout);
        vst.sound_settings_state.reset_slot_to_defaults(13, crate::track::TrackInstrumentKind::Delay, 0.5);
        let inst = &vst.sound_settings_state.instruments[13];
        inst.set_special(synthesis::delay_sp::TIME, 400.0);
        let clock = fx_locks::StepClock { steps: 16, swing: 0.0, groove: groove::GrooveType::Straight };
        let time = |bpm: f32| match vst.fx_params_now(1, false, clock, bpm) {
            Some(synthesis::FxParams::Delay(d)) => d.time_ms,
            other => panic!("a delay lane, got {other:?}"),
        };
        assert_eq!(time(90.0), 400.0, "Free: the Time knob");
        inst.set_special(synthesis::delay_sp::SYNC, 9.0); // 1/4
        assert!((time(90.0) - 666.67).abs() < 0.1);
        assert!((time(120.0) - 500.0).abs() < 0.01);
        inst.set_special(synthesis::delay_sp::SYNC, 8.0); // dotted 1/8
        assert!((time(120.0) - 375.0).abs() < 0.01);
        // Width comes through too.
        inst.set_special(synthesis::delay_sp::WIDTH, 0.6);
        match vst.fx_params_now(1, false, clock, 120.0) {
            Some(synthesis::FxParams::Delay(d)) => assert!((d.width - 0.6).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
    }

    /// [286] The whole audio path of a Rumble lane, as the plugin runs it: the Duck and Duck
    /// Curve sliders of the LANE change what is heard, and the duck gain dips at each kick.
    #[test]
    fn the_rumble_lane_ducks_and_its_sliders_change_the_sound() {
        use nih_plug::params::persist::PersistentField;
        let run = |duck: f32, curve: f32| -> (f32, f32) {
            let mut vst = DrumFlashVst::default();
            let mut layout = crate::track::TrackLayoutState::modular_default_layout();
            layout.slots[0].fx_send = [0.0, 0.3];
            layout.slots[13] = crate::track::TrackSlot::active_with_kind(
                crate::track::TrackInstrumentKind::Rumble,
            );
            PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout.clone());
            vst.synthesizer.initialize_with_layout(44100.0, &layout);
            vst.sound_settings_state.reset_slot_to_defaults(13, crate::track::TrackInstrumentKind::Rumble, 0.5);
            let inst = &vst.sound_settings_state.instruments[13];
            inst.set_special(synthesis::rumble_sp::DUCK, duck);
            inst.set_special(synthesis::rumble_sp::DUCK_CURVE, curve);
            inst.set_special(synthesis::rumble_sp::RELEASE, 400.0);
            // Duck Trigger = Beat (the default): the transport starts the duck, as `process` does.
            assert_eq!(inst.special_value(synthesis::rumble_sp::DUCK_SOURCE), 0.0);
            vst.synthesizer.set_voice_settings(0, synthesis::VoiceSettings::kick());
            let now = vst.voice_settings_at_step(13, synthesis::DrumVoice::Rumble as usize, 0);
            vst.synthesizer.set_voice_settings(13, now);
            vst.sequencer.set_track_params([16; 14], [0.0; 14], [0.0; 14], 16);
            vst.sequencer.play();
            let clock = fx_locks::StepClock { steps: 16, swing: 0.0, groove: groove::GrooveType::Straight };
            let (sends, bypass) = vst.fx_block_state();
            let (mut lowest, mut energy) = (1.0f32, 0.0f32);
            let mut last_step = usize::MAX;
            for n in 0..(44_100 * 8) {
                vst.sequencer.process_sample(120.0, 44_100.0, 0.0, groove::GrooveType::Straight);
                let step = vst.sequencer.current_step();
                if step != last_step {
                    last_step = step;
                    if step % 4 == 0 {
                        vst.synthesizer.trigger(0, 1.0);
                    }
                }
                if n % 32 == 0 {
                    if let Some(p) = vst.fx_params_now(1, true, clock, 120.0) {
                        if let synthesis::FxParams::Rumble(r) = &p {
                            let tick = vst.sequencer.beat_position().floor() as i64;
                            if r.duck_source == 0 && tick != vst.rumble_beat_tick[1] {
                                vst.rumble_beat_tick[1] = tick;
                                vst.synthesizer.rumble_trigger_duck(1);
                            }
                        }
                        vst.synthesizer.set_fx_params(1, p);
                    }
                }
                let mut outs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
                vst.synthesizer.process_voice_samples_stereo(&mut outs);
                vst.synthesizer.process_fx(&mut outs, &sends, bypass);
                if n > 44_100 * 4 {
                    lowest = lowest.min(vst.synthesizer.rumble_duck_gain(1).unwrap_or(1.0));
                    energy += outs[13][0].powi(2) + outs[13][1].powi(2);
                }
            }
            (lowest, energy)
        };
        let (g_on, e_on) = run(1.0, 0.5);
        let (g_off, e_off) = run(0.0, 0.5);
        let (_, e_late) = run(1.0, 1.0);
        let (_, e_early) = run(1.0, 0.0);
        println!("duck 1: lowest gain {g_on:.3}, energy {e_on:.4}; duck 0: lowest {g_off:.3}, energy {e_off:.4}; curve 1: {e_late:.4}, curve 0: {e_early:.4}");
        assert!(g_on < 0.3, "the duck never dips in the plugin path ({g_on})");
        assert!(g_off > 0.99, "no duck, no dip ({g_off})");
        assert!((e_on - e_off).abs() > 0.05 * e_off, "Duck must change the sound ({e_on} vs {e_off})");
        assert!((e_late - e_early).abs() > 0.03 * e_early, "Duck Curve must change the sound ({e_late} vs {e_early})");
    }

    /// The send slider of the Effects tab, end to end: the editor's source writes the
    /// lane's send, the audio thread reads it and the bus returns a tail.
    #[test]
    fn the_send_slider_reaches_the_effect_bus() {
        use nih_plug::params::persist::PersistentField;
        use crate::ui::param_source::ParamSource;
        let mut vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[12] = crate::track::TrackSlot::active_with_kind(crate::track::TrackInstrumentKind::Reverb);
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout.clone());
        vst.synthesizer.initialize_with_layout(44100.0, &layout);
        vst.sound_settings_state.reset_slot_to_defaults(12, crate::track::TrackInstrumentKind::Reverb, 0.5);
        let now = vst.voice_settings_at_step(12, synthesis::DrumVoice::Reverb as usize, 0);
        vst.synthesizer.set_voice_settings(12, now);
        vst.synthesizer.set_voice_settings(0, synthesis::VoiceSettings::kick());
        // The slider, through the same source the Effects tab uses.
        struct Algo;
        impl crate::ui::param_source::AlgoSink for Algo {
            fn get(&self) -> u8 {
                0
            }
            fn set(&self, _: u8) {}
        }
        let algo = Algo;
        let mut src = crate::ui::param_source::GlobalSource::new(&vst.sound_settings_state, 0, 0, &algo)
            .with_layout(&vst.params.track_layout);
        src.set(crate::param_id::ParamId::FxSend(0), 0.8);
        src.commit();
        assert!((vst.params.track_layout.state.fx_send(0, 0) - 0.8).abs() < 1e-6, "the slider wrote the lane");
        let (sends, bypass) = vst.fx_block_state();
        assert!((sends[0][0] - 0.8).abs() < 1e-6, "the audio thread reads it");
        vst.synthesizer.trigger(0, 1.0);
        let mut energy = 0.0f32;
        for _ in 0..44_100 {
            let mut outs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            vst.synthesizer.process_voice_samples_stereo(&mut outs);
            vst.synthesizer.process_fx(&mut outs, &sends, bypass);
            energy += outs[12][0].powi(2) + outs[12][1].powi(2);
        }
        assert!(energy > 0.01, "the bus must return the kick ({energy})");
    }

    /// [304] A synced LFO rate follows the tempo: one cycle per note value; Free keeps the
    /// knob; the value stays inside the knob's range.
    #[test]
    fn synced_lfo_rates_follow_the_tempo() {
        use nih_plug::params::persist::PersistentField;
        let mut vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[0] = crate::track::TrackSlot::active_with_kind(crate::track::TrackInstrumentKind::Rift);
        layout.slots[0].lfo_sync = [9, 0]; // 1/4 on the pitch LFO, Free on the filter LFO
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout);
        vst.sound_settings_state.reset_slot_to_defaults(0, crate::track::TrackInstrumentKind::Rift, 0.5);
        let rift = crate::track::TrackInstrumentKind::Rift.drum_voice_index();
        let rate_of = |vst: &DrumFlashVst, name: &str| {
            let def = crate::instrument_registry::INSTRUMENTS[rift]
                .special_params
                .iter()
                .find(|d| d.name == name)
                .unwrap();
            vst.voice_settings_at_step(0, rift, 0).special[def.special_index]
        };
        vst.lfo_bpm = 120.0;
        assert!((rate_of(&vst, "rift_pitch_lfo_rate") - 2.0).abs() < 1e-4, "1/4 at 120 bpm = 2 Hz");
        vst.lfo_bpm = 90.0;
        assert!((rate_of(&vst, "rift_pitch_lfo_rate") - 1.5).abs() < 1e-4, "1/4 at 90 bpm = 1.5 Hz");
        let free = rate_of(&vst, "rift_filter_lfo_rate");
        let knob = vst.sound_settings_state.instruments[0].special_value(
            crate::instrument_registry::INSTRUMENTS[rift]
                .special_params
                .iter()
                .find(|d| d.name == "rift_filter_lfo_rate")
                .unwrap()
                .special_index,
        );
        assert_eq!(free, knob, "Free keeps the knob");
    }

    #[test]
    fn the_locked_decay_is_not_heard_after_the_loop_wraps() {
        use nih_plug::params::persist::PersistentField;
        let mut vst = DrumFlashVst::default();
        let mut layout = crate::track::TrackLayoutState::modular_default_layout();
        layout.slots[0].fx_send = [1.0, 0.0];
        layout.slots[12] = crate::track::TrackSlot::active_with_kind(
            crate::track::TrackInstrumentKind::Reverb,
        );
        PersistentField::<crate::track::TrackLayoutState>::set(&vst.params.track_layout, layout.clone());
        vst.synthesizer.initialize_with_layout(44100.0, &layout);
        vst.sound_settings_state.reset_slot_to_defaults(12, crate::track::TrackInstrumentKind::Reverb, 0.5);
        let inst = &vst.sound_settings_state.instruments[12];
        inst.set_special(synthesis::reverb_sp::DECAY, 0.5);
        let field = crate::param_id::ParamId::Special(synthesis::reverb_sp::DECAY).plock_field().unwrap();
        vst.params.plock_state.state.set_field(12, 12, field, 8.0);
        vst.synthesizer.set_voice_settings(0, synthesis::VoiceSettings::kick());
        let reverb_now = vst.voice_settings_at_step(12, synthesis::DrumVoice::Reverb as usize, 0);
        vst.synthesizer.set_voice_settings(12, reverb_now);
        vst.sequencer.set_track_params([16; 14], [0.0; 14], [0.0; 14], 16);
        vst.sequencer.play();
        let clock = fx_locks::StepClock { steps: 16, swing: 0.0, groove: groove::GrooveType::Straight };
        let (sends, bypass) = vst.fx_block_state();
        let mut per_step = vec![0.0f32; 16];
        let mut last_step = usize::MAX;
        let total = 44_100 * 7;
        for n in 0..total {
            vst.sequencer.process_sample(120.0, 44_100.0, 0.0, groove::GrooveType::Straight);
            let step = vst.sequencer.current_step();
            if step != last_step {
                last_step = step;
                if step == 8 {
                    vst.synthesizer.trigger(0, 1.0);
                }
            }
            if n % 32 == 0 {
                if let Some(p) = vst.fx_params_now(0, true, clock, 120.0) {
                    vst.synthesizer.set_fx_params(0, p);
                }
            }
            let mut outs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            vst.synthesizer.process_voice_samples_stereo(&mut outs);
            vst.synthesizer.process_fx(&mut outs, &sends, bypass);
            if n > 44_100 * 4 {
                per_step[step.min(15)] += outs[12][0].powi(2) + outs[12][1].powi(2);
            }
        }
        let rms: Vec<String> = per_step.iter().map(|e| format!("{:.4}", e.sqrt())).collect();
        println!("return level by step (3rd loop on): {rms:?}");
    }

    #[test]
    fn pattern_bank_actions_return_busy_instead_of_blocking_when_locked() {
        let mut vst = DrumFlashVst::default();
        let bank = vst.params.pattern_bank.bank.clone();
        let _guard = bank.lock().unwrap();

        assert_eq!(vst.save_pattern_to_slot(0), PatternBankActionResult::Busy);
        assert_eq!(vst.load_pattern_from_slot(0), PatternBankActionResult::Busy);
    }

    #[test]
    fn pattern_load_updates_audio_length_before_ui_param_catches_up() {
        let mut vst = DrumFlashVst::default();
        {
            let mut bank = vst.params.pattern_bank.bank.lock().unwrap();
            bank.slots[0].capture(
                &vst.pattern,
                &vst.params.plock_state.state,
                &vst.params.seq_plock_state.state,
                24,
            );
        }

        assert_eq!(vst.params.pattern_length.value(), 16);
        assert_eq!(
            vst.load_pattern_from_slot(0),
            PatternBankActionResult::Loaded(24)
        );
        assert_eq!(vst.audio_master_length, 24);
        assert_eq!(vst.pending_pattern_length.load(Ordering::Relaxed), 24);
    }

    #[test]
    fn track_length_follows_master_when_unlocked() {
        // Unlocked : toujours master
        assert_eq!(resolve_track_length(16, 64, false), 64);
        assert_eq!(resolve_track_length(32, 16, false), 16);
        assert_eq!(resolve_track_length(12, 32, false), 32);
    }

    #[test]
    fn track_length_locked_uses_raw_length() {
        // Locked : vraie longueur individuelle 1..64, indépendamment du master.
        assert_eq!(resolve_track_length(12, 64, true), 12);
        assert_eq!(resolve_track_length(16, 32, true), 16);
        assert_eq!(resolve_track_length(32, 16, true), 32);
        assert_eq!(resolve_track_length(16, 16, true), 16);
        assert_eq!(resolve_track_length(64, 8, true), 64);
    }

    #[test]
    fn lane_length_locks_persistence_roundtrips() {
        let locks = LaneLengthLocks::new();
        locks.set_locked(0, true);
        locks.set_locked(5, true);
        locks.set_locked(12, true);

        locks.map(|mask| {
            assert_eq!(*mask, (1u16 << 0) | (1u16 << 5) | (1u16 << 12));
        });

        let restored = LaneLengthLocks::new();
        restored.set((1u16 << 2) | (1u16 << 12));
        assert!(!restored.is_locked(0));
        assert!(restored.is_locked(2));
        assert!(restored.is_locked(12));
        assert!(!restored.is_locked(5));
    }

    #[test]
    fn cymbal_shimmer_freq_propagates_through_voice_settings_for() {
        let vst = DrumFlashVst::default();
        let cy_idx = synthesis::DrumVoice::Cymbal as usize;

        // Slot 9 is inactive in the default 4-lane layout, so its per-slot
        // specials are seeded from the legacy voice of the same index (Cymbal).
        let settings = vst.voice_settings_for(
            cy_idx, cy_idx, 6000.0, // slot, voice, freq
            2.0,    // decay
            0.4,    // volume
            8000.0, // filter_freq
            0.002,  // attack
            2.5,    // release
            2.8,    // decay_curve
            3.0,    // release_curve
            0.0,    // hold
            0.0,    // filter_env_amount
            0.05,   // filter_env_decay
            1.0,    // analog
            1.0,    // stereo
        );

        assert!(
            (settings.special[0] - 15.0).abs() < 0.001,
            "cymbal_shimmer_freq should propagate to special[0], expected ~15.0 got {}",
            settings.special[0]
        );
    }

    fn bool_array(pattern: &[bool]) -> [bool; crate::track::MAX_TRACKS] {
        assert_eq!(pattern.len(), crate::track::MAX_TRACKS);
        std::array::from_fn(|i| pattern[i])
    }

    #[test]
    fn mix_gating_no_mute_no_solo_routes_all_to_main() {
        let mutes = bool_array(&[false; crate::track::MAX_TRACKS]);
        let solos = bool_array(&[false; crate::track::MAX_TRACKS]);
        let main = bool_array(&[true; crate::track::MAX_TRACKS]);

        let (effective_mutes, gains) = compute_mix_gating(&mutes, &solos, &main);

        assert!(effective_mutes.iter().all(|m| !m));
        assert!(gains.iter().all(|g| *g == 1.0));
    }

    #[test]
    fn mix_gating_one_mute_mutes_only_that_slot() {
        let mut mutes = [false; crate::track::MAX_TRACKS];
        mutes[2] = true;
        let mutes = bool_array(&mutes);
        let solos = bool_array(&[false; crate::track::MAX_TRACKS]);
        let main = bool_array(&[true; crate::track::MAX_TRACKS]);

        let (effective_mutes, _) = compute_mix_gating(&mutes, &solos, &main);

        assert!(effective_mutes[2]);
        assert!(
            effective_mutes
                .iter()
                .enumerate()
                .all(|(i, m)| i == 2 || !m),
            "only slot 2 should be muted"
        );
    }

    #[test]
    fn mix_gating_one_solo_ignores_mutes_and_mutes_others() {
        let mut mutes = [false; crate::track::MAX_TRACKS];
        mutes[2] = true;
        mutes[5] = true;
        let mutes = bool_array(&mutes);
        let mut solos = [false; crate::track::MAX_TRACKS];
        solos[7] = true;
        let solos = bool_array(&solos);
        let main = bool_array(&[true; crate::track::MAX_TRACKS]);

        let (effective_mutes, _) = compute_mix_gating(&mutes, &solos, &main);

        assert!(
            !effective_mutes[7],
            "soloed slot 7 must remain audible regardless of mutes"
        );
        assert!(
            effective_mutes
                .iter()
                .enumerate()
                .all(|(i, m)| i == 7 || *m),
            "all non-soloed slots should be muted when any solo is active"
        );
    }

    #[test]
    fn mix_gating_multiple_solos_unmute_only_those_slots() {
        let mut solos = [false; crate::track::MAX_TRACKS];
        solos[0] = true;
        solos[3] = true;
        solos[13] = true;
        let solos = bool_array(&solos);
        let mutes = bool_array(&[false; crate::track::MAX_TRACKS]);
        let main = bool_array(&[true; crate::track::MAX_TRACKS]);

        let (effective_mutes, _) = compute_mix_gating(&mutes, &solos, &main);

        for i in 0..crate::track::MAX_TRACKS {
            if solos[i] {
                assert!(!effective_mutes[i], "soloed slot {} should be unmuted", i);
            } else {
                assert!(
                    effective_mutes[i],
                    "non-soloed slot {} should be muted when any solo is active",
                    i
                );
            }
        }
    }

    #[test]
    fn mix_gating_main_mix_disabled_gives_zero_gain() {
        let mutes = bool_array(&[false; crate::track::MAX_TRACKS]);
        let solos = bool_array(&[false; crate::track::MAX_TRACKS]);
        let mut main = [true; crate::track::MAX_TRACKS];
        main[1] = false;
        main[10] = false;
        let main = bool_array(&main);

        let (_, gains) = compute_mix_gating(&mutes, &solos, &main);

        assert_eq!(gains[1], 0.0);
        assert_eq!(gains[10], 0.0);
        assert!(gains
            .iter()
            .enumerate()
            .all(|(i, g)| i == 1 || i == 10 || *g == 1.0));
    }

    #[test]
    fn mix_gating_solo_and_main_mix_interact_independently() {
        // Solo and Main routing affect different things: solo mutes non-soloed
        // slots, Main routing determines whether the *allowed* slots go to Main.
        let mut solos = [false; crate::track::MAX_TRACKS];
        solos[2] = true;
        solos[4] = true;
        let solos = bool_array(&solos);
        let mut main = [false; crate::track::MAX_TRACKS];
        main[2] = true;
        main[5] = true;
        let main = bool_array(&main);
        let mutes = bool_array(&[false; crate::track::MAX_TRACKS]);

        let (effective_mutes, gains) = compute_mix_gating(&mutes, &solos, &main);

        assert!(!effective_mutes[2]);
        assert!(!effective_mutes[4]);
        assert!(effective_mutes[5]);
        assert_eq!(gains[2], 1.0);
        assert_eq!(gains[4], 0.0); // soloed but not routed to Main
        assert_eq!(gains[5], 1.0); // routed to Main; gains are independent of mute/solo
    }

    #[test]
    fn resolve_midi_output_uses_slot_note_and_global_channel() {
        use std::sync::atomic::Ordering;

        let vst = DrumFlashVst::default();
        let default_kick_note = crate::instrument_registry::INSTRUMENTS[0].midi_note;

        // Default layout: slot 0 is Kick; force global channel to 10 for a
        // deterministic assertion (otherwise the test depends on the user's
        // saved config).
        vst.params
            .track_layout
            .state
            .global_channel
            .store(10, Ordering::Relaxed);

        // global channel is 10 -> output channel 9.
        let (note, channel) = vst.resolve_midi_output_for_slot(0);
        assert_eq!(note, default_kick_note);
        assert_eq!(channel, 9);

        // Override the slot MIDI note and global channel.
        vst.params.track_layout.state.slot_midi_notes[0].store(99, Ordering::Relaxed);
        vst.params
            .track_layout
            .state
            .global_channel
            .store(16, Ordering::Relaxed);

        let (note, channel) = vst.resolve_midi_output_for_slot(0);
        assert_eq!(
            note, 99,
            "MIDI output must use the per-slot note, not the instrument registry default"
        );
        assert_eq!(
            channel, 15,
            "MIDI output channel must be global MIDI channel minus one"
        );
    }

    /// An SDrex lane saved with the old Noise level gets the balance that sounds like it.
    #[test]
    fn an_old_sdrex_noise_level_becomes_the_balance() {
        let mut vst = DrumFlashVst::default();
        let sdrex = synthesis::DrumVoice::Sdrex as usize;
        let inst = &vst.sound_settings_state.instruments[3];
        inst.set_special(crate::instrument_registry::SDREX_OLD_NOISE_LEVEL, 0.8);
        inst.set_special(crate::instrument_registry::SDREX_BALANCE, 0.0);
        let mut voices = [None; crate::track::MAX_TRACKS];
        voices[3] = Some(sdrex);
        vst.seed_late_specials(&voices);
        let inst = &vst.sound_settings_state.instruments[3];
        assert_eq!(inst.special_value(crate::instrument_registry::SDREX_BALANCE), 0.5, "the recipe level is the centre");
        assert_eq!(inst.special_value(crate::instrument_registry::SDREX_OLD_NOISE_LEVEL), 0.0);
        // Quieter noise -> towards Synth; a second pass changes nothing.
        inst.set_special(crate::instrument_registry::SDREX_OLD_NOISE_LEVEL, 0.4);
        inst.set_special(crate::instrument_registry::SDREX_BALANCE, 0.0);
        vst.seed_late_specials(&voices);
        let b = vst.sound_settings_state.instruments[3].special_value(crate::instrument_registry::SDREX_BALANCE);
        assert!((b - 0.75).abs() < 1e-6, "{b}");
        vst.seed_late_specials(&voices);
        assert!((vst.sound_settings_state.instruments[3].special_value(crate::instrument_registry::SDREX_BALANCE) - 0.75).abs() < 1e-6);
    }
}
