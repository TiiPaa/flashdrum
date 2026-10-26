//! Instrument registry — single source of truth for all instrument metadata.
//!
//! Adding a new instrument should only require:
//! 1. Adding a variant to `DrumVoice`
//! 2. Adding fields to `DrumFlashParams` (nih-plug constraint)
//! 3. Adding an entry to `INSTRUMENTS` below
//!
//! Everything else (multi-out, plock, UI, MIDI, parameters) is derived automatically.

use crate::synthesis::DrumVoice;

pub const SOUND_SETTINGS_FIELD_COUNT: usize = 13;

/// Functional family of a parameter, used by the Sound Panel to group sliders.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamFamily {
    /// Oscillator / source parameters: pitch, algo, timbre, special tone controls.
    Osc,
    /// Amplitude envelope parameters: attack, decay, release, curves, hold.
    Env,
    /// Pitch: tuning, fine tuning, and the pitch envelope with its graph
    /// ([227]). Synth voices keep their Frequency in `Osc`; the relative-pitch
    /// voices (Rift) put everything pitch-related here.
    Pitch,
    /// Analog / character parameters: the global analog/digital switch.
    Analog,
    /// Filter parameters: cutoff, filter envelope amount/decay.
    Filter,
    /// Modulation parameters: target, LFO, flanger and modulation depth/mix.
    Modulation,
    /// Distortion: the saturation pack (type, amount, mix, output gain,
    /// pre/post filter) and the lo-fi stages (Crush, Decimate). Shown as
    /// "Distortion" - the name that covers all of them.
    Saturation,
    /// Output / routing parameters: volume, mix, stereo, analog drift.
    Output,
}

impl ParamFamily {
    pub fn label(&self) -> &'static str {
        match self {
            ParamFamily::Osc => "OSC",
            ParamFamily::Env => "AMP",
            ParamFamily::Pitch => "PITCH",
            ParamFamily::Analog => "ANALOG",
            ParamFamily::Filter => "FILTER",
            ParamFamily::Modulation => "MOD",
            ParamFamily::Saturation => "DIST",
            ParamFamily::Output => "OUTPUT",
        }
    }
}

/// [240] Canonical row order of the Amp section, applied by the Sound Panel
/// at RENDER time (the registry tables keep their own declaration order -
/// plock layout and persistence index the FIELDS, not the row order, so this
/// is display-only). Every instrument reads its amplitude envelope the same
/// way: a time, then its curve. Attack, Attack Curve, Hold, Decay, Decay
/// Curve. Unranked fields (none today) keep their declaration order, after.
pub fn env_row_rank(field: StandardField) -> u8 {
    match field {
        StandardField::Attack => 0,
        StandardField::ReleaseCurve => 1, // displayed as "Attack Curve"
        StandardField::Hold => 2,
        StandardField::Decay => 3,
        StandardField::DecayCurve => 4,
        _ => 5,
    }
}

/// Standard sound-setting field index (matches the persistent f32 array order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StandardField {
    Freq = 0,
    Decay = 1,
    Volume = 2,
    FilterFreq = 3,
    Attack = 4,
    Release = 5,
    DecayCurve = 6,
    ReleaseCurve = 7,
    Hold = 8,
    FilterEnvAmount = 9,
    FilterEnvDecay = 10,
    Analog = 11,
    Stereo = 12,
}

impl StandardField {
    /// Every standard field, in discriminant order (= the order of
    /// `InstrumentSettingsState::load()` and of `sound_settings_default`).
    /// Lets code iterate the fields without re-listing them ([184]).
    pub const ALL: [StandardField; 13] = [
        StandardField::Freq,
        StandardField::Decay,
        StandardField::Volume,
        StandardField::FilterFreq,
        StandardField::Attack,
        StandardField::Release,
        StandardField::DecayCurve,
        StandardField::ReleaseCurve,
        StandardField::Hold,
        StandardField::FilterEnvAmount,
        StandardField::FilterEnvDecay,
        StandardField::Analog,
        StandardField::Stereo,
    ];

    /// Returns the plock field index used in `PlockState`.
    /// This mapping aligns with how `PlockState::get_settings` and `set_settings`
    /// store values internally (see `plock.rs`).
    pub const fn plock_field_index(self) -> usize {
        match self {
            StandardField::Freq => 0,
            StandardField::Decay => 1,
            StandardField::Volume => 2,
            StandardField::FilterFreq => 3,
            StandardField::Release => 4,
            StandardField::DecayCurve => 5,
            StandardField::ReleaseCurve => 6,
            StandardField::Hold => 7,
            StandardField::FilterEnvAmount => 8,
            StandardField::FilterEnvDecay => 9,
            StandardField::Analog => 10,
            StandardField::Stereo => 11,
            StandardField::Attack => 18,
        }
    }
}

/// Widget kind for a standard parameter.
#[derive(Clone, Copy, Debug)]
pub enum ParamWidget {
    Slider {
        min: f32,
        max: f32,
        logarithmic: bool,
        suffix: Option<&'static str>,
    },
    Checkbox,
}

/// Metadata for a standard (per-instrument) parameter exposed in the Sound Panel.
pub struct StandardParamDef {
    pub field: StandardField,
    pub label: &'static str,
    pub family: ParamFamily,
    pub widget: ParamWidget,
}

/// Metadata for a special (per-instrument) parameter exposed in the Sound Panel.
pub struct SpecialParamDef {
    pub name: &'static str,
    pub label: &'static str,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    pub special_index: usize,
    pub family: ParamFamily,
    pub continuous: bool,
    /// Display unit, e.g. `Some(" Hz")` ([182]). `None` for dimensionless
    /// amounts (depth, wet, mix…), exactly like a standard param's `suffix`.
    pub unit: Option<&'static str>,
    /// Slider response curve ([189]). 1.0 = linear travel; above 1 the low end
    /// of the range gets more of the travel. Purely ergonomic: it changes where
    /// a value sits on the track, never the value itself nor the sound.
    pub curve: f32,
    /// Named choices for a discrete parameter, rendered as a dropdown ([221]).
    /// The value IS the index into this list. `None` falls back to the older
    /// per-label branches in the Sound Panel, which recognise a handful of
    /// parameters by the wording of their label - declare the list here
    /// instead, it works for any instrument without touching the UI.
    pub options: Option<&'static [&'static str]>,
}

/// [287] Specials added after sessions existed: an older lane stores 0 for
/// them, below their minimum. `DrumFlashVst::seed_late_specials` reseeds those.
pub const LATE_SPECIALS: [&str; 12] = [
    "tom_resonance",
    // [94] The Toms' Pitch LFO Rate: an older tom lane holds 0, below the minimum.
    "tom_pitch_lfo_rate",
    "sdrex_resonance",
    "buzz_resonance",
    "reverb_highcut",
    "delay_highcut",
    "reverb_gate",
    "reverb_gate_release",
    // The Rumble's, added after its first lanes existed: an older lane holds 0 below the minimum.
    "rumble_highpass",
    "rumble_reverb_size",
    "rumble_reverb_motion",
    "rumble_reverb_blur",
];

/// [286] The Rumble's Echo note values: the same menu as the delay's Sync.
pub const RUMBLE_ECHO_SYNC: &[&str] = &[
    "Free", "1/32", "1/16T", "1/16", "1/8T", "1/16.", "1/8", "1/4T", "1/8.", "1/4", "1/2T", "1/4.", "1/2",
    "1 bar",
];

/// [304] The LFO rates that can follow the host tempo, and which of the lane's two LFO-sync
/// settings (`TrackSlot::lfo_sync`) each one reads. The sync is a LANE setting kept in the
/// track layout (the voices' `special[]` has no room left on Rift), never a p-lock.
pub const LFO_SYNC_RATES: [(&str, usize); 5] = [
    ("rift_pitch_lfo_rate", 0),
    ("tom_pitch_lfo_rate", 0),
    ("rift_filter_lfo_rate", 1),
    ("sdrex_flanger_rate", 0),
    ("buzz_gate_rate", 0),
];

/// [304] The lane LFO-sync slot a rate parameter reads, if it can be synced.
pub fn lfo_sync_slot(name: &str) -> Option<usize> {
    LFO_SYNC_RATES.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

/// SDrex: the retired Noise level (special 10) and the Noise <-> Synth balance that replaced
/// it (special 19). A lane saved before holds its Noise level and 0 for the balance.
pub const SDREX_OLD_NOISE_LEVEL: usize = 10;
pub const SDREX_BALANCE: usize = 19;

/// The balance that sounds like an old SDrex Noise level: the noise layer at `level / 0.8`
/// of the recipe, the synth layers at full level (a level above the recipe's 0.8 cannot be
/// reached by the balance, which tops both layers at full: it becomes the centre).
pub fn sdrex_balance_from_noise_level(level: f32) -> f32 {
    (1.0 - level / 1.6).clamp(0.5, 1.0)
}

/// [292] TM6(AC) Model: 0 was "Auto" (the model followed the pitch), now retired;
/// 1 = Low tom, 2 = High tom. The stored index stays so saved lanes keep their model.
pub const TM6_MODEL: &str = "ac_tm_model";

/// [292] The pitch the recorded toms were played at: Frequency is the tom's own
/// pitch, so each model has its own sensible range and starting point.
pub const TM6_LOW_HZ: f32 = 124.435;
pub const TM6_HIGH_HZ: f32 = 208.0;

/// Frequency range of the TM6(AC) slider for a Model value (2 = High, else Low).
pub fn tm6_freq_range(model: f32) -> (f32, f32) {
    if model.round() as i32 == 2 {
        (140.0, 420.0)
    } else {
        (70.0, 200.0)
    }
}

/// The recorded pitch of a Model value.
pub fn tm6_recorded_hz(model: f32) -> f32 {
    if model.round() as i32 == 2 {
        TM6_HIGH_HZ
    } else {
        TM6_LOW_HZ
    }
}

/// Helper for continuous special parameters (morphable).
const fn sp(
    name: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    special_index: usize,
    family: ParamFamily,
) -> SpecialParamDef {
    SpecialParamDef {
        name,
        label,
        default,
        min,
        max,
        special_index,
        family,
        continuous: true,
        unit: None,
        curve: 1.0,
        options: None,
    }
}

/// Helper for a continuous special parameter that carries a display unit ([182]).
const fn sp_unit(
    name: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    special_index: usize,
    family: ParamFamily,
    unit: &'static str,
) -> SpecialParamDef {
    SpecialParamDef {
        name,
        label,
        default,
        min,
        max,
        special_index,
        family,
        continuous: true,
        unit: Some(unit),
        curve: 1.0,
        options: None,
    }
}

/// Slider response curve of every **Saturation Amount** ([189]).
///
/// The DSP maps the amount to drive as `1 + amount^2 * 19`, and the audible
/// distortion then flattens out: measured on a 0.8 sine through SoftClip, a
/// linear slider reached 15 % THD at a quarter of its travel, 32 % at half, and
/// only 43 % at the end - almost everything happened in the first half. An
/// exponent of 1.5 spreads it out (7 % / 23 % / 38 %), which is the closest fit
/// to an even ramp among the exponents tried.
pub const SAT_AMOUNT_CURVE: f32 = 1.5;

/// Helper for a continuous special parameter with a slider response curve.
const fn sp_curved(
    name: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    special_index: usize,
    family: ParamFamily,
    curve: f32,
) -> SpecialParamDef {
    SpecialParamDef {
        name,
        label,
        default,
        min,
        max,
        special_index,
        family,
        continuous: true,
        unit: None,
        curve,
        options: None,
    }
}

/// Helper for a discrete special parameter with NAMED choices ([221]): the
/// value is the index into `options`, and the Sound Panel renders a dropdown.
const fn sp_options(
    name: &'static str,
    label: &'static str,
    default: f32,
    special_index: usize,
    family: ParamFamily,
    options: &'static [&'static str],
) -> SpecialParamDef {
    SpecialParamDef {
        name,
        label,
        default,
        min: 0.0,
        max: (options.len() - 1) as f32,
        special_index,
        family,
        continuous: false,
        unit: None,
        curve: 1.0,
        options: Some(options),
    }
}

/// Helper for discrete special parameters (not morphable).
const fn sp_discrete(
    name: &'static str,
    label: &'static str,
    default: f32,
    min: f32,
    max: f32,
    special_index: usize,
    family: ParamFamily,
) -> SpecialParamDef {
    SpecialParamDef {
        name,
        label,
        default,
        min,
        max,
        special_index,
        family,
        continuous: false,
        unit: None,
        curve: 1.0,
        options: None,
    }
}

/// [286] The Rumble's sections, titled in the order of the audio flow (the families only
/// give the rows their module): `None` for any other voice or family.
pub fn rumble_section_title(voice_idx: usize, family: ParamFamily) -> Option<&'static str> {
    let is_rumble = INSTRUMENTS
        .get(voice_idx)
        .is_some_and(|d| d.special_params.iter().any(|p| p.name == "rumble_duck"));
    if !is_rumble {
        return None;
    }
    Some(match family {
        ParamFamily::Osc => "1 · Echo",
        ParamFamily::Pitch => "2 · Reverb",
        ParamFamily::Filter => "3 · Filter",
        ParamFamily::Modulation => "4 · Width & Drive",
        ParamFamily::Saturation => "5 · Duck",
        _ => "Output",
    })
}

/// Title of the source section (`ParamFamily::Osc`) for a voice: a sampler
/// shows a **Sample**, Rift a **Texture**, everything else an **Oscillator**.
/// Decided from the parameters the voice declares, not from its index.
pub fn source_section_title(voice_idx: usize) -> &'static str {
    if is_sampler(voice_idx) {
        return "Sample";
    }
    let def = INSTRUMENTS.get(voice_idx);
    // [286] The reverb's knobs are its whole source section.
    if def.is_some_and(|d| d.special_params.iter().any(|p| p.name.starts_with("reverb_"))) {
        return "Reverb";
    }
    if def.is_some_and(|d| d.special_params.iter().any(|p| p.name.starts_with("delay_"))) {
        return "Delay";
    }
    if def.is_some_and(|d| d.special_params.iter().any(|p| p.name.starts_with("rumble_"))) {
        return "Rumble";
    }
    let has = |suffix: &str| {
        def.map(|d| d.special_params.iter().any(|p| p.name.ends_with(suffix)))
            .unwrap_or(false)
    };
    if has("_texture") {
        // Rift slices a long texture ("Texture"); One-Shot plays one file,
        // start to finish — that is a Sample ([243]).
        if has("_offset") {
            "Texture"
        } else {
            "Sample"
        }
    } else {
        "Oscillator"
    }
}

/// Is this voice one of the embedded multisample samplers? Declared by each
/// voice's `InstrumentDef::is_sampler` ([269]; the list used to be spelled out
/// by index).
pub fn is_sampler(voice_idx: usize) -> bool {
    INSTRUMENTS.get(voice_idx).is_some_and(|def| def.is_sampler)
}

// [269] Sound Editor rules keyed on parameter names rather than on a voice
// index (same idea as the `*_loop` / `*_grain*` rule), so they follow the
// params a voice declares.
/// Gate controls (Buzz today) get their own sub-row of the Env section,
/// beside the gate shape graph.
pub const GATE_ROW_PREFIX: &str = "buzz_gate";
/// A modulation switch which, turned on (filter mode), greys the row named
/// by [`FLANGER_ONLY_SUFFIX`] (Sdrex today).
pub const FILTER_MOD_SUFFIX: &str = "_filter_mod";
/// The flanger-only row ([181]: the Fade-in applies to both modes).
pub const FLANGER_ONLY_SUFFIX: &str = "_flanger_feedback";

/// The factory default of one parameter on one voice ([184]).
///
/// This is the single definition of "the default", replacing the three the Sound
/// panel used to carry side by side: the per-voice `sound_settings_default`
/// table for the voices that have one, `VoiceSettings::default()` for the
/// others, and `SpecialParamDef::default` for specials. It is the target of
/// double-click-to-reset.
pub fn param_default(voice_idx: usize, id: crate::param_id::ParamId) -> f32 {
    use crate::param_id::ParamId;
    let Some(instrument) = INSTRUMENTS.get(voice_idx) else {
        return 0.0;
    };
    match id {
        // EVERY voice reads its own default table.
        //
        // Until [224] only the samplers, SDrex and Rift did; the other twenty
        // fell back to one `VoiceSettings::default()` shared by all, a legacy
        // of the early voices. Measured before the change: **20 of the 26
        // instruments** reset at least one slider to a value that was not
        // theirs, and **17 sliders** landed outside their own range - the
        // Hi-Hat's Tone reset to 60 Hz instead of 8000 (bottoming out its
        // slider), Tom 1's Filter Env to 0 instead of 1 (losing the sweep that
        // makes a tom), the Clap's Decay to 0.5 s instead of 0.03 s.
        //
        // This is the value behind "reset to factory": the double-click gesture
        // and the Default button both come here.
        ParamId::Std(field) => instrument.sound_settings_default[field as usize],
        ParamId::Special(index) => instrument
            .special_params
            .iter()
            .find(|def| def.special_index == index)
            .map(|def| def.default)
            .unwrap_or(0.0),
        // The algo dropdown and the Hz/Note switch have no reset gesture; a send
        // starts at 0 ([286]).
        ParamId::Algo | ParamId::FreqMode | ParamId::FxSend(_) => 0.0,
    }
}

/// [286] A parameter that belongs to the whole lane and is never locked per step: the
/// effect lane's Glide switch is how its OTHER locks behave, so a lock on it would
/// mean nothing (the audio reads the lane's own value). Every scope edits the lane.
pub fn param_is_lane_global(voice_idx: usize, id: crate::param_id::ParamId) -> bool {
    use crate::param_id::ParamId;
    let ParamId::Special(index) = id else {
        return false;
    };
    INSTRUMENTS.get(voice_idx).is_some_and(|inst| {
        inst.special_params
            .iter()
            .any(|def| def.special_index == index && matches!(
                def.name,
                "reverb_glide" | "reverb_type" | "reverb_gate_on" | "reverb_warp" | "delay_glide" | "reverb_norestart" | "delay_norestart" | "delay_sync"
                    | "rumble_glide" | "rumble_norestart" | "rumble_echo_sync" | "rumble_duck_source"
            ))
    })
}

/// Can this parameter be morphed across a fused group ([184] phase 3)?
///
/// Morphing interpolates linearly between two endpoints, which only means
/// something for a continuous quantity: a `Saturation Type` halfway between
/// Valve and Tape is not a sound, it is a rounding accident. The registry
/// already declares that distinction as `SpecialParamDef::continuous`.
pub fn param_is_morphable(voice_idx: usize, id: crate::param_id::ParamId) -> bool {
    use crate::param_id::ParamId;
    match id {
        // The algo is a discrete selector, a display mode is not a sound, and a send
        // level is not part of the voice at all ([286]).
        ParamId::Algo | ParamId::FreqMode | ParamId::FxSend(_) => false,
        // Every standard field interpolates, checkboxes included (a switch
        // crossing 0.5 mid-roll is a legitimate effect).
        ParamId::Std(_) => true,
        ParamId::Special(index) => INSTRUMENTS
            .get(voice_idx)
            .and_then(|inst| {
                inst.special_params
                    .iter()
                    .find(|def| def.special_index == index)
            })
            .map(|def| def.continuous)
            .unwrap_or(false),
    }
}

/// Metadata for an instrument in the registry.
pub struct InstrumentDef {
    pub index: usize,
    pub name: &'static str,
    pub label: &'static str,
    pub full_name: &'static str,
    pub midi_note: u8,
    pub algo_count: usize,
    pub standard_params: &'static [StandardParamDef],
    pub special_params: &'static [SpecialParamDef],
    pub sound_settings_default: [f32; SOUND_SETTINGS_FIELD_COUNT],
    pub filter_type_label: &'static str,
    /// Ratio applied to the frequency value before displaying as note.
    /// e.g. 0.3 for Kick because the sustain freq is 0.3x the setting.
    pub freq_display_ratio: f32,
    /// One of the embedded multisample samplers (the *606 sample voices).
    /// They share a parameter shape the synthesised voices do not have: pitch
    /// in relative semitones, a sample index, One Shot, Start/End offsets, and
    /// a waveform graph instead of an envelope one. Read through
    /// [`is_sampler`].
    pub is_sampler: bool,
    /// The Frequency row offers the Hz / Notes display switch (kick-like
    /// voices; the per-lane `freq_mode`).
    pub freq_as_notes: bool,
    /// "Analog fixé" (`docs/analog-mode.md`): the Analog slider only governs
    /// phase continuity, the voice has no random drift. The dev preset dump
    /// writes 0.0 in place of its analog value.
    pub analog_fixed: bool,
}

const fn s(
    field: StandardField,
    label: &'static str,
    family: ParamFamily,
    min: f32,
    max: f32,
    logarithmic: bool,
    suffix: Option<&'static str>,
) -> StandardParamDef {
    StandardParamDef {
        field,
        label,
        family,
        widget: ParamWidget::Slider {
            min,
            max,
            logarithmic,
            suffix,
        },
    }
}

const fn cb(field: StandardField, label: &'static str, family: ParamFamily) -> StandardParamDef {
    StandardParamDef {
        field,
        label,
        family,
        widget: ParamWidget::Checkbox,
    }
}

/// Generic standard params for instruments that support everything.
const FULL_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// Buzz-specific: amp is A-H-D (no release), plus an A-H-D filter envelope
/// (Filter Env amount + Filter Decay standard, Filter Attack/Hold as specials).
const BUZZ_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    // [146] The bipolar attack curve: to the left, the fast-rising (concave) attack.
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::FilterEnvAmount,
        "Filter Env",
        ParamFamily::Filter,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterEnvDecay,
        "Filter Decay",
        ParamFamily::Filter,
        0.01,
        1.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// HiHat-specific: like FULL_STD but the Freq knob controls the peaking filter
/// center (the "metallic tone" of the noise) rather than an oscillator pitch.
/// HiHat: the decay caps at 1.5 s ([188]), like the clap in [181] — past that
/// the tail is inaudible and the slider travel did nothing. The OpenHiHat keeps
/// its 5 s on purpose: a long tail is what makes it "open".
const HIHAT_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Tone",
        ParamFamily::Osc,
        100.0,
        20000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        1.5,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Cutoff",
        ParamFamily::Filter,
        100.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// Kick-like: no hold, no filter env, no stereo.
const KICK_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

/// Open-hat / ride / cymbal / clap: no hold, no filter env.
const NO_HOLD_NO_FILTENV_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// Open-hat specific: like NO_HOLD_NO_FILTENV_STD but the Freq knob controls the
/// peaking filter center (the "metallic tone" of the noise).
const OPENHIHAT_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Tone",
        ParamFamily::Osc,
        100.0,
        20000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Cutoff",
        ParamFamily::Filter,
        100.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// Cymbal-specific: no frequency (noise-based), no hold, no filter env.
const NO_FREQ_STD: &[StandardParamDef] = &[
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// Clap: same as `NO_FREQ_STD` but the decay caps at 1.5 s ([181]) — a clap
/// never needs more, and the Cymbal keeps the long 5 s range.
const CLAP_STD: &[StandardParamDef] = &[
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        1.5,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// Tom-like: no hold, filter env, no stereo.
const TOM_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::FilterEnvAmount,
        "Filter Env",
        ParamFamily::Filter,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterEnvDecay,
        "Filter Decay",
        ParamFamily::Filter,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

/// 606 multisample voices (BD6smp / SD6smp): pitch in semitones relative to
/// the native sample rate, amp/filter envelope times as FRACTIONS of the
/// played sample length (no Release � the sample tail is the release).
const SMP606_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Pitch",
        ParamFamily::Osc,
        -24.0,
        24.0,
        false,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.01,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    // [146] The bipolar attack curve: to the left, the fast-rising (concave) attack.
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    // [168] Stereo = two distinct samples L&R in pairs (1+2, 3+4, 5+6, 7+8);
    // with Analog Mode on, a random pair plays per hit. Rendered in the UI
    // directly under the Sample select.
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::FilterEnvAmount,
        "Filter Env",
        ParamFamily::Filter,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterEnvDecay,
        "Filter Decay",
        ParamFamily::Filter,
        0.01,
        1.0,
        false,
        None,
    ),
];

/// Rift [221]: pitch relatif, enveloppe A-H-D complete, filtre multi-mode.
/// Le filtre suit la convention du plugin (voir Buzz) : Filter Env va de 0 a 1
/// et OUVRE la coupure vers 20 kHz, qui se referme ensuite dessus.
const RIFT_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Pitch",
        ParamFamily::Pitch,
        -24.0,
        24.0,
        false,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        1.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.005,
        1.5,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::FilterEnvAmount,
        "Filter Env",
        ParamFamily::Filter,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterEnvDecay,
        "Filter Decay",
        ParamFamily::Filter,
        0.005,
        1.5,
        false,
        Some(" s"),
    ),
    // [228] Un fichier utilisateur stereo joue ses deux canaux tels quels
    // quand ce switch est allume ; eteint, ils sont mixes en mono. Rendu sous
    // la ligne File du menu Texture, pas dans la boucle des standards.
    cb(StandardField::Stereo, "Stereo", ParamFamily::Osc),
];

/// One-Shot [243]: sampler semantics — every envelope TIME is a FRACTION of
/// the played region's heard duration (1.0 = the whole sample), so the
/// envelope always fits the file, short or long, pitched or not (user report
/// 2026-09-23: absolute seconds were meaningless on short samples).
/// [286] The reverb exposes only its return level (the lane Volume); its
/// knobs are special params.
const REVERB_STD: &[StandardParamDef] = &[s(
    StandardField::Volume,
    "Volume",
    ParamFamily::Output,
    0.0,
    2.0,
    false,
    None,
)];

const ONE_SHOT_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Pitch",
        ParamFamily::Pitch,
        -24.0,
        24.0,
        false,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.005,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::FilterEnvAmount,
        "Filter Env",
        ParamFamily::Filter,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterEnvDecay,
        "Filter Decay",
        ParamFamily::Filter,
        0.005,
        1.0,
        false,
        None,
    ),
    // [228] Meme regle que Rift : un fichier stereo joue ses deux canaux tels
    // quels quand le switch est allume ; rendu sous la ligne File.
    cb(StandardField::Stereo, "Stereo", ParamFamily::Osc),
];

/// SDrex: body/noise/metal recipe + volume A-D and LP filter A-D envelopes.
const SDREX_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        60.0,
        600.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        1.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.03,
        1.5,
        false,
        Some(" s"),
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::FilterEnvAmount,
        "Filter Env",
        ParamFamily::Filter,
        0.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::FilterEnvDecay,
        "Filter Decay",
        ParamFamily::Filter,
        0.001,
        1.5,
        false,
        Some(" s"),
    ),
];

/// Snare606: hold, no filter env, stereo-capable.
const SNARE606_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        5.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Hold,
        "Hold",
        ParamFamily::Env,
        0.0,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
    cb(StandardField::Stereo, "Stereo", ParamFamily::Output),
];

/// B8: no hold, no filter env, no stereo.
const MINIMAL_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        12000.0,
        true,
        None,
    ),
    s(
        StandardField::Attack,
        "Attack",
        ParamFamily::Env,
        0.0,
        0.2,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -2.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::ReleaseCurve,
        "Attack Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::FilterFreq,
        "Filter",
        ParamFamily::Filter,
        20.0,
        20000.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

/// AC606 voices (ported analogcode engines): only the params the engines
/// actually consume — no filter, no envelope curves (their shapes are fitted).
const AC_BD_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        20.0,
        500.0,
        true,
        None,
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        1.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::DecayCurve,
        "Decay Curve",
        ParamFamily::Env,
        -1.0,
        1.0,
        false,
        None,
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

const AC_SD_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        100.0,
        600.0,
        true,
        None,
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

/// HH6(AC) / OH6(AC): "Tone" = pitch of the metallic partials.
const AC_TONE_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Tone",
        ParamFamily::Osc,
        2000.0,
        16000.0,
        true,
        None,
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

const AC_CLAP_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Tone",
        ParamFamily::Osc,
        500.0,
        2000.0,
        true,
        None,
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

const AC_TOM_STD: &[StandardParamDef] = &[
    s(
        StandardField::Freq,
        "Frequency",
        ParamFamily::Osc,
        70.0,
        420.0,
        true,
        Some(" Hz"),
    ),
    s(
        StandardField::Decay,
        "Decay",
        ParamFamily::Env,
        0.001,
        2.0,
        false,
        Some(" s"),
    ),
    s(
        StandardField::Volume,
        "Volume",
        ParamFamily::Output,
        0.0,
        2.0,
        false,
        None,
    ),
    s(
        StandardField::Analog,
        "Analog",
        ParamFamily::Analog,
        0.0,
        1.0,
        false,
        None,
    ),
];

pub const INSTRUMENTS: [InstrumentDef; DrumVoice::COUNT] = [
    InstrumentDef {
        index: 0,
        name: "Kick",
        label: "BD",
        full_name: "Kick Drum",
        midi_note: 36,
        algo_count: 3,
        standard_params: KICK_STD,
        special_params: &[
            sp(
                "kick_click",
                "Click Level",
                0.5,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "kick_click_type",
                "Click Type",
                1.0,
                0.0,
                2.0,
                6,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "kick_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                1,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "kick_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "kick_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                3,
                ParamFamily::Saturation,
            ),
            sp(
                "kick_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "kick_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
        ],
        // Kick-like: no hold, no filter env, no stereo
        sound_settings_default: [
            60.0, 0.5, 1.0, 30.0, 0.0015, 0.5, 5.0, 3.0, 0.0, 1.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 0.3,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: true,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 1,
        name: "Snare",
        label: "SD",
        full_name: "Snare Drum",
        midi_note: 38,
        algo_count: 3,
        standard_params: FULL_STD,
        special_params: &[
            sp("snare_snap", "Snap", 0.5, 0.0, 1.0, 0, ParamFamily::Osc),
            sp_discrete(
                "snare_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                1,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "snare_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "snare_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                3,
                ParamFamily::Saturation,
            ),
            sp(
                "snare_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "snare_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
        ],
        // Full: hold, filter env, stereo
        sound_settings_default: [
            200.0, 0.47, 0.6, 200.0, 0.0003, 0.2, 5.0, 3.0, 0.0, 1.0, 0.03, 0.5, 1.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "HP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 2,
        name: "HiHat",
        label: "HH",
        full_name: "Hi-Hat",
        midi_note: 42,
        algo_count: 1,
        standard_params: HIHAT_STD,
        special_params: &[
            sp_discrete(
                "hihat_noise_type",
                "Noise Type",
                0.0,
                0.0,
                3.0,
                5,
                ParamFamily::Osc,
            ),
            sp(
                "hihat_resonance",
                "Resonance",
                2.0,
                0.1,
                10.0,
                6,
                ParamFamily::Osc,
            ),
            sp(
                "hihat_shimmer",
                "Shimmer",
                0.0,
                0.0,
                1.0,
                7,
                ParamFamily::Filter,
            ),
            sp_discrete(
                "hihat_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                0,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "hihat_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                1,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "hihat_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
            ),
            sp(
                "hihat_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                3,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "hihat_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                4,
                ParamFamily::Saturation,
            ),
        ],
        // Full: hold, filter env, stereo
        sound_settings_default: [
            8000.0, 0.36, 0.2, 5000.0, 0.0003, 0.0, 8.0, 3.0, 0.0, 1.0, 0.04, 0.5, 1.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "HP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    InstrumentDef {
        index: 3,
        name: "OpenHiHat",
        label: "OH",
        full_name: "Open Hi-Hat",
        midi_note: 46,
        algo_count: 1,
        standard_params: OPENHIHAT_STD,
        special_params: &[
            sp_discrete(
                "openhihat_noise_type",
                "Noise Type",
                0.0,
                0.0,
                3.0,
                5,
                ParamFamily::Osc,
            ),
            sp(
                "openhihat_resonance",
                "Resonance",
                2.0,
                0.1,
                10.0,
                6,
                ParamFamily::Osc,
            ),
            sp(
                "openhihat_shimmer",
                "Shimmer",
                0.0,
                0.0,
                1.0,
                7,
                ParamFamily::Filter,
            ),
            sp_discrete(
                "openhihat_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                0,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "openhihat_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                1,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "openhihat_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
            ),
            sp(
                "openhihat_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                3,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "openhihat_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                4,
                ParamFamily::Saturation,
            ),
        ],
        // No hold, no filter env, stereo-capable
        sound_settings_default: [
            6000.0, 0.66, 0.3, 8000.0, 0.0003, 0.4, 5.5, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "HP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    InstrumentDef {
        index: 4,
        name: "Tom1",
        label: "T1",
        full_name: "Tom 1",
        midi_note: 50,
        algo_count: 2,
        standard_params: TOM_STD,
        special_params: &[
            sp(
                "tom_stick",
                "Stick Attack",
                0.3,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "tom_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                1,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "tom_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "tom_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                3,
                ParamFamily::Saturation,
            ),
            sp(
                "tom_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "tom_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
            sp(
                "tom_resonance",
                "Resonance",
                0.7071,
                0.5,
                10.0,
                6,
                ParamFamily::Filter,
            ),
            // [94] Pitch LFO: a vibrato on the body, restarted on each hit, that can come in
            // progressively (Fade-in) and follow the tempo (Sync, [304]).
            sp_unit(
                "tom_pitch_lfo_rate",
                "Pitch LFO Rate",
                5.0,
                0.05,
                40.0,
                8,
                ParamFamily::Modulation,
                " Hz",
            ),
            sp(
                "tom_pitch_lfo_depth",
                "Pitch LFO Depth",
                0.0,
                0.0,
                12.0,
                9,
                ParamFamily::Modulation,
            ),
            sp_options(
                "tom_lfo_shape",
                "LFO Shape",
                0.0,
                7,
                ParamFamily::Modulation,
                &["Sine", "Triangle", "Square", "Saw"],
            ),
            sp_unit(
                "tom_pitch_lfo_fade",
                "Fade-in",
                0.0,
                0.0,
                1000.0,
                10,
                ParamFamily::Modulation,
                " ms",
            ),
        ],
        // No hold, filter env, no stereo
        sound_settings_default: [
            196.0, 0.35, 0.7, 600.0, 0.0015, 0.25, 4.0, 3.0, 0.0, 1.0, 0.06, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 5,
        name: "Tom2",
        label: "T2",
        full_name: "Tom 2",
        midi_note: 47,
        algo_count: 2,
        standard_params: TOM_STD,
        special_params: &[
            sp(
                "tom_stick",
                "Stick Attack",
                0.3,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "tom_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                1,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "tom_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "tom_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                3,
                ParamFamily::Saturation,
            ),
            sp(
                "tom_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "tom_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
            sp(
                "tom_resonance",
                "Resonance",
                0.7071,
                0.5,
                10.0,
                6,
                ParamFamily::Filter,
            ),
            // [94] Pitch LFO: a vibrato on the body, restarted on each hit, that can come in
            // progressively (Fade-in) and follow the tempo (Sync, [304]).
            sp_unit(
                "tom_pitch_lfo_rate",
                "Pitch LFO Rate",
                5.0,
                0.05,
                40.0,
                8,
                ParamFamily::Modulation,
                " Hz",
            ),
            sp(
                "tom_pitch_lfo_depth",
                "Pitch LFO Depth",
                0.0,
                0.0,
                12.0,
                9,
                ParamFamily::Modulation,
            ),
            sp_options(
                "tom_lfo_shape",
                "LFO Shape",
                0.0,
                7,
                ParamFamily::Modulation,
                &["Sine", "Triangle", "Square", "Saw"],
            ),
            sp_unit(
                "tom_pitch_lfo_fade",
                "Fade-in",
                0.0,
                0.0,
                1000.0,
                10,
                ParamFamily::Modulation,
                " ms",
            ),
        ],
        // No hold, filter env, no stereo
        sound_settings_default: [
            150.0, 0.3, 0.7, 650.0, 0.0015, 0.2, 4.0, 3.0, 0.0, 1.0, 0.06, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 6,
        name: "Tom3",
        label: "T3",
        full_name: "Tom 3",
        midi_note: 43,
        algo_count: 2,
        standard_params: TOM_STD,
        special_params: &[
            sp(
                "tom_stick",
                "Stick Attack",
                0.3,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "tom_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                1,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "tom_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "tom_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                3,
                ParamFamily::Saturation,
            ),
            sp(
                "tom_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "tom_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
            sp(
                "tom_resonance",
                "Resonance",
                0.7071,
                0.5,
                10.0,
                6,
                ParamFamily::Filter,
            ),
            // [94] Pitch LFO: a vibrato on the body, restarted on each hit, that can come in
            // progressively (Fade-in) and follow the tempo (Sync, [304]).
            sp_unit(
                "tom_pitch_lfo_rate",
                "Pitch LFO Rate",
                5.0,
                0.05,
                40.0,
                8,
                ParamFamily::Modulation,
                " Hz",
            ),
            sp(
                "tom_pitch_lfo_depth",
                "Pitch LFO Depth",
                0.0,
                0.0,
                12.0,
                9,
                ParamFamily::Modulation,
            ),
            sp_options(
                "tom_lfo_shape",
                "LFO Shape",
                0.0,
                7,
                ParamFamily::Modulation,
                &["Sine", "Triangle", "Square", "Saw"],
            ),
            sp_unit(
                "tom_pitch_lfo_fade",
                "Fade-in",
                0.0,
                0.0,
                1000.0,
                10,
                ParamFamily::Modulation,
                " ms",
            ),
        ],
        // No hold, filter env, no stereo
        sound_settings_default: [
            100.0, 0.45, 0.7, 500.0, 0.0015, 0.35, 4.0, 3.0, 0.0, 1.0, 0.06, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 7,
        name: "Clap",
        label: "CL",
        full_name: "Clap",
        midi_note: 39,
        algo_count: 2,
        standard_params: CLAP_STD,
        special_params: &[
            sp("clap_echo", "Echo", 0.5, 0.0, 3.0, 0, ParamFamily::Env),
            sp_discrete(
                "clap_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                1,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "clap_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "clap_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                3,
                ParamFamily::Saturation,
            ),
            sp(
                "clap_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "clap_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
        ],
        // No hold, no filter env, stereo
        sound_settings_default: [
            1200.0, 0.03, 1.0, 1000.0, 0.0015, 0.12, 6.0, 3.0, 0.0, 0.0, 0.05, 0.5, 1.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "HP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    InstrumentDef {
        index: 8,
        name: "Ride",
        label: "RD",
        full_name: "Ride",
        midi_note: 51,
        algo_count: 2,
        standard_params: NO_HOLD_NO_FILTENV_STD,
        special_params: &[
            sp_discrete(
                "ride_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                0,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ride_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                1,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ride_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Saturation,
            ),
            sp(
                "ride_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                3,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "ride_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                4,
                ParamFamily::Saturation,
            ),
        ],
        // No hold, no filter env, stereo (default mono for stability)
        sound_settings_default: [
            8000.0, 1.2, 0.35, 10000.0, 0.002, 1.5, 3.5, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "HP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    InstrumentDef {
        index: 9,
        name: "Cymbal",
        label: "CY",
        full_name: "Cymbal",
        midi_note: 49,
        algo_count: 1,
        standard_params: NO_FREQ_STD,
        special_params: &[
            sp_discrete(
                "cymbal_noise_type",
                "Noise Type",
                0.0,
                0.0,
                3.0,
                1,
                ParamFamily::Osc,
            ),
            sp_unit(
                "cymbal_shimmer_freq",
                "Shimmer Freq",
                15.0,
                1.0,
                50.0,
                0,
                ParamFamily::Osc,
                " Hz",
            ),
            sp(
                "cymbal_shimmer_amount",
                "Shimmer Amount",
                0.15,
                0.0,
                1.0,
                2,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "cymbal_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                3,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "cymbal_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                4,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "cymbal_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
            sp(
                "cymbal_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                6,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "cymbal_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                7,
                ParamFamily::Saturation,
            ),
        ],
        // No hold, no filter env, stereo (default mono for stability)
        sound_settings_default: [
            6000.0, 2.0, 0.4, 8000.0, 0.002, 2.5, 2.8, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "HP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 10,
        name: "Snare606",
        label: "S6",
        full_name: "Snare 606",
        midi_note: 40,
        algo_count: 1,
        standard_params: SNARE606_STD,
        special_params: &[
            sp(
                "snare606_resonance",
                "Resonance",
                4.5,
                0.5,
                12.0,
                0,
                ParamFamily::Filter,
            ),
            sp("snare606_tone", "Tone", 0.55, 0.0, 1.0, 1, ParamFamily::Osc),
            sp("snare606_snap", "Snap", 0.3, 0.0, 1.0, 2, ParamFamily::Osc),
            sp_discrete(
                "snare606_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                3,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "snare606_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                4,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "snare606_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
            ),
            sp(
                "snare606_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                6,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "snare606_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                7,
                ParamFamily::Saturation,
            ),
        ],
        // Snare606: hold, no filter env, stereo-capable
        sound_settings_default: [
            220.0, 0.08, 0.7, 3000.0, 0.0003, 0.15, 5.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    InstrumentDef {
        index: 11,
        name: "BassDrum808",
        label: "B8",
        full_name: "808 Kick",
        midi_note: 35,
        algo_count: 1,
        standard_params: MINIMAL_STD,
        special_params: &[
            sp(
                "bassdrum808_accent",
                "Accent",
                0.0,
                0.0,
                2.0,
                0,
                ParamFamily::Osc,
            ),
            sp(
                "bassdrum808_snap",
                "Snap",
                0.0,
                0.0,
                2.0,
                1,
                ParamFamily::Osc,
            ),
            sp(
                "bassdrum808_pitch_drop",
                "Pitch Drop",
                0.0,
                0.0,
                2.0,
                2,
                ParamFamily::Osc,
            ),
            sp_unit(
                "bassdrum808_click_tone",
                "Click Tone",
                4000.0,
                100.0,
                8000.0,
                3,
                ParamFamily::Filter,
                " Hz",
            ),
            sp_discrete(
                "bassdrum808_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "bassdrum808_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "bassdrum808_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
            ),
            sp(
                "bassdrum808_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "bassdrum808_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
        ],
        // Minimal: no hold, no filter env, no stereo
        sound_settings_default: [
            50.0, 0.4, 1.0, 3000.0, 0.0015, 0.0, 3.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: true,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 12,
        name: "Perc1",
        label: "P1",
        full_name: "Perc1",
        midi_note: 37,
        algo_count: 2,
        standard_params: FULL_STD,
        special_params: &[
            sp("perc1_sweep", "Sweep", 0.5, -1.0, 1.0, 0, ParamFamily::Osc),
            sp(
                "perc1_speed",
                "Speed",
                80.0,
                5.0,
                300.0,
                1,
                ParamFamily::Osc,
            ),
            sp("perc1_bite", "Bite", 0.0, 0.0, 1.0, 2, ParamFamily::Osc),
            sp(
                "perc1_width",
                "Width",
                0.0,
                0.0,
                1.0,
                3,
                ParamFamily::Output,
            ),
            sp_discrete(
                "perc1_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "perc1_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "perc1_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
            ),
            sp(
                "perc1_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "perc1_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
        ],
        // Full: filter env, stereo
        sound_settings_default: [
            2000.0, 0.15, 0.6, 6000.0, 0.0005, 0.0, 5.0, 3.0, 0.0, 0.7, 0.03, 0.5, 1.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    InstrumentDef {
        index: 13,
        name: "BD606",
        label: "B6",
        full_name: "BD6smp",
        midi_note: 41,
        algo_count: 1,
        standard_params: SMP606_STD,
        special_params: &[
            sp_discrete(
                "bd606_analog_mode",
                "Analog Mode",
                1.0,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete("bd606_sample", "Sample", 1.0, 1.0, 8.0, 1, ParamFamily::Osc),
            sp_discrete(
                "bd606_one_shot",
                "One Shot",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Env,
            ),
            sp(
                "bd606_start_offset",
                "Start",
                0.0,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp_unit(
                "bd606_fine_tune",
                "Pitch Fine",
                0.0,
                -100.0,
                100.0,
                9,
                ParamFamily::Osc,
                " ct",
            ),
            sp("bd606_end", "End", 1.0, 0.0, 1.0, 11, ParamFamily::Osc),
            sp_discrete(
                "bd606_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "bd606_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "bd606_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
            ),
            sp(
                "bd606_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "bd606_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        // (filter_env_decay = fraction of the sample length)
        sound_settings_default: [
            0.0, 0.4, 1.0, 20000.0, 0.002, 0.0, 4.0, 3.0, 0.0, 0.0, 0.15, 1.0, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: true,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 14,
        name: "SD606",
        label: "s6",
        full_name: "SD6smp",
        midi_note: 40,
        algo_count: 1,
        standard_params: SMP606_STD,
        special_params: &[
            sp_discrete(
                "sd606_analog_mode",
                "Analog Mode",
                1.0,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete("sd606_sample", "Sample", 1.0, 1.0, 8.0, 1, ParamFamily::Osc),
            sp_discrete(
                "sd606_one_shot",
                "One Shot",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Env,
            ),
            sp(
                "sd606_start_offset",
                "Start",
                0.0,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp_unit(
                "sd606_fine_tune",
                "Pitch Fine",
                0.0,
                -100.0,
                100.0,
                9,
                ParamFamily::Osc,
                " ct",
            ),
            sp("sd606_end", "End", 1.0, 0.0, 1.0, 11, ParamFamily::Osc),
            sp_discrete(
                "sd606_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "sd606_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "sd606_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
            ),
            sp(
                "sd606_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "sd606_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        // (filter_env_decay = fraction of the sample length)
        sound_settings_default: [
            0.0, 0.3, 0.8, 20000.0, 0.002, 0.0, 4.0, 3.0, 0.0, 0.0, 0.15, 1.0, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: true,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 15,
        name: "CH606",
        label: "c6",
        full_name: "CH6smp",
        midi_note: 42,
        algo_count: 1,
        standard_params: SMP606_STD,
        special_params: &[
            sp_discrete(
                "ch606_analog_mode",
                "Analog Mode",
                1.0,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete("ch606_sample", "Sample", 1.0, 1.0, 8.0, 1, ParamFamily::Osc),
            sp_discrete(
                "ch606_one_shot",
                "One Shot",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Env,
            ),
            sp(
                "ch606_start_offset",
                "Start",
                0.0,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp_unit(
                "ch606_fine_tune",
                "Pitch Fine",
                0.0,
                -100.0,
                100.0,
                9,
                ParamFamily::Osc,
                " ct",
            ),
            sp("ch606_end", "End", 1.0, 0.0, 1.0, 11, ParamFamily::Osc),
            sp_discrete(
                "ch606_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ch606_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ch606_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
            ),
            sp(
                "ch606_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "ch606_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        sound_settings_default: [
            0.0, 0.2, 0.6, 20000.0, 0.001, 0.0, 4.0, 3.0, 0.0, 0.0, 0.15, 1.0, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: true,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 16,
        name: "Buzz",
        label: "Bz",
        full_name: "Buzz",
        midi_note: 44,
        algo_count: 2,
        standard_params: BUZZ_STD,
        special_params: &[
            sp_unit(
                "buzz_gate_rate",
                "Gate Rate",
                55.0,
                1.0,
                500.0,
                0,
                ParamFamily::Env,
                " Hz",
            ),
            sp(
                "buzz_gate_depth",
                "Gate Depth",
                0.85,
                0.0,
                1.0,
                1,
                ParamFamily::Env,
            ),
            sp(
                "buzz_gate_shape",
                "Gate Shape",
                0.55,
                0.0,
                1.0,
                2,
                ParamFamily::Env,
            ),
            sp(
                "buzz_noise_amount",
                "Noise",
                0.3,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "buzz_noise_type",
                "Noise Type",
                0.0,
                0.0,
                3.0,
                4,
                ParamFamily::Osc,
            ),
            sp(
                "buzz_pitch_sweep",
                "Pitch Sweep",
                0.3,
                0.0,
                1.0,
                5,
                ParamFamily::Osc,
            ),
            sp_discrete("buzz_wave", "Wave", 0.0, 0.0, 2.0, 11, ParamFamily::Osc),
            sp_unit(
                "buzz_filter_attack",
                "Filter Attack",
                0.0,
                0.0,
                0.5,
                12,
                ParamFamily::Filter,
                " s",
            ),
            sp_unit(
                "buzz_filter_hold",
                "Filter Hold",
                0.0,
                0.0,
                0.5,
                13,
                ParamFamily::Filter,
                " s",
            ),
            sp(
                "buzz_filter_atk_curve",
                "Filter Atk Curve",
                0.0,
                -1.0,
                1.0,
                16,
                ParamFamily::Filter,
            ),
            sp(
                "buzz_filter_curve",
                "Filter Dec Curve",
                0.6,
                -1.0,
                1.0,
                15,
                ParamFamily::Filter,
            ),
            sp_discrete(
                "buzz_filter_type",
                "Filter Type",
                0.0,
                0.0,
                2.0,
                14,
                ParamFamily::Filter,
            ),
            sp(
                "buzz_resonance",
                "Resonance",
                0.9,
                0.5,
                10.0,
                17,
                ParamFamily::Filter,
            ),
            sp_discrete(
                "buzz_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                6,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "buzz_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                7,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "buzz_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
            sp(
                "buzz_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                9,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "buzz_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                10,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        sound_settings_default: [
            200.0, 0.5, 1.3, 1200.0, 0.0005, 0.2, 4.0, 0.0, 0.0, 0.6, 0.12, 0.5, 1.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 17,
        name: "Sdrex",
        label: "Sx",
        full_name: "SDrex",
        midi_note: 48,
        algo_count: 1,
        standard_params: SDREX_STD,
        special_params: &[
            sp_discrete(
                "sdrex_filter_mod",
                "Modulation",
                0.0,
                0.0,
                1.0,
                17,
                ParamFamily::Modulation,
            ),
            sp_unit(
                "sdrex_flanger_rate",
                "Rate",
                5.7,
                0.1,
                20.0,
                0,
                ParamFamily::Modulation,
                " Hz",
            ),
            sp_unit(
                "sdrex_modulation_fade",
                "Fade-in",
                0.0,
                0.0,
                300.0,
                1,
                ParamFamily::Modulation,
                " ms",
            ),
            sp(
                "sdrex_flanger_depth",
                "Depth",
                1.8,
                -3.0,
                3.0,
                2,
                ParamFamily::Modulation,
            ),
            sp(
                "sdrex_flanger_feedback",
                "Feedback",
                0.38,
                0.0,
                0.9,
                3,
                ParamFamily::Modulation,
            ),
            sp(
                "sdrex_flanger_wet",
                "Wet",
                0.32,
                0.0,
                1.0,
                4,
                ParamFamily::Modulation,
            ),
            sp_discrete(
                "sdrex_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                5,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "sdrex_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "sdrex_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                7,
                ParamFamily::Saturation,
            ),
            sp(
                "sdrex_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                8,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "sdrex_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                9,
                ParamFamily::Saturation,
            ),
            // The balance between the noise and the synth layers (body + metal); 0.5 = the
            // recipe, both at full level. Replaced the Noise level (special 10, retired:
            // older lanes are converted by `seed_late_specials`).
            sp(
                "sdrex_balance",
                "Noise <-> Synth",
                0.5,
                0.0,
                1.0,
                19,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "sdrex_noise_type",
                "Noise Type",
                0.0,
                0.0,
                3.0,
                11,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "sdrex_free_phase",
                "Free Phase",
                0.0,
                0.0,
                1.0,
                12,
                ParamFamily::Modulation,
            ),
            sp_unit(
                "sdrex_filter_attack",
                "Filter Attack",
                0.0,
                0.0,
                0.5,
                13,
                ParamFamily::Filter,
                " s",
            ),
            sp_unit(
                "sdrex_filter_hold",
                "Filter Hold",
                0.0,
                0.0,
                1.0,
                16,
                ParamFamily::Filter,
                " s",
            ),
            sp(
                "sdrex_filter_atk_curve",
                "Filter Atk Curve",
                0.0,
                -1.0,
                1.0,
                14,
                ParamFamily::Filter,
            ),
            sp(
                "sdrex_filter_dec_curve",
                "Filter Dec Curve",
                0.6,
                -1.0,
                1.0,
                15,
                ParamFamily::Filter,
            ),
            sp(
                "sdrex_resonance",
                "Resonance",
                0.7071,
                0.5,
                10.0,
                18,
                ParamFamily::Filter,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        sound_settings_default: [
            185.0, 0.15, 0.9, 20000.0, 0.0005, 0.0, 0.0, 0.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    // ── AC606 voices (ported analogcode engines, MIT (c) 2026 Matthew Fecher)
    InstrumentDef {
        index: 18,
        name: "BD6(AC)",
        label: "BA",
        full_name: "BD6 (analogcode)",
        midi_note: 52,
        algo_count: 1,
        standard_params: AC_BD_STD,
        special_params: &[
            sp("ac_bd_sweep", "Sweep", 0.5, 0.0, 2.0, 1, ParamFamily::Env),
            sp("ac_bd_bend", "Bend", 0.5, 0.0, 1.0, 2, ParamFamily::Env),
            sp("ac_bd_click", "Click", 0.2, 0.0, 1.0, 3, ParamFamily::Osc),
            sp_unit(
                "ac_bd_click_tone",
                "Click Tone",
                1400.0,
                400.0,
                4000.0,
                4,
                ParamFamily::Osc,
                " Hz",
            ),
            sp("ac_bd_punch", "Punch", 0.2, 0.0, 2.0, 5, ParamFamily::Osc),
            sp("ac_bd_tone", "Tone", 0.34, 0.0, 2.0, 7, ParamFamily::Osc),
            sp("ac_bd_drive", "Drive", 0.18, 0.0, 2.0, 8, ParamFamily::Osc),
            sp_discrete(
                "ac_bd_sat_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                10,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ac_bd_sat_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ac_bd_sat_mix",
                "Saturation Mix",
                0.5,
                0.0,
                1.0,
                12,
                ParamFamily::Saturation,
            ),
            sp(
                "ac_bd_sat_gain",
                "Saturation Output Gain",
                1.25,
                0.5,
                2.0,
                13,
                ParamFamily::Saturation,
            ),
        ],
        sound_settings_default: [
            60.0, 0.5, 1.0, 20000.0, 0.002, 0.0, 0.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 0.3,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: true,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 19,
        name: "SD6(AC)",
        label: "SA",
        full_name: "SD6 (analogcode)",
        midi_note: 53,
        algo_count: 1,
        standard_params: AC_SD_STD,
        special_params: &[
            sp("ac_sd_snap", "Snap", 0.75, 0.0, 1.0, 0, ParamFamily::Osc),
            sp(
                "ac_sd_wire_color",
                "Wire Color",
                1.0,
                0.25,
                4.0,
                1,
                ParamFamily::Osc,
            ),
            sp(
                "ac_sd_shell_bend",
                "Shell Bend",
                0.5,
                0.0,
                1.0,
                2,
                ParamFamily::Osc,
            ),
            sp("ac_sd_impact", "Impact", 0.5, 0.0, 1.0, 3, ParamFamily::Osc),
            sp("ac_sd_ring", "Ring", 0.5, 0.0, 1.0, 4, ParamFamily::Osc),
            sp_discrete(
                "ac_sd_sat_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                10,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ac_sd_sat_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ac_sd_sat_mix",
                "Saturation Mix",
                0.5,
                0.0,
                1.0,
                12,
                ParamFamily::Saturation,
            ),
            sp(
                "ac_sd_sat_gain",
                "Saturation Output Gain",
                1.25,
                0.5,
                2.0,
                13,
                ParamFamily::Saturation,
            ),
        ],
        sound_settings_default: [
            201.09, 0.25, 0.9, 20000.0, 0.0005, 0.0, 5.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 20,
        name: "HH6(AC)",
        label: "HA",
        full_name: "HH6 (analogcode)",
        midi_note: 54,
        algo_count: 1,
        standard_params: AC_TONE_STD,
        special_params: &[
            sp("ac_hh_metal", "Metal", 0.11, 0.0, 1.0, 0, ParamFamily::Osc),
            sp("ac_hh_click", "Click", 0.35, 0.0, 1.0, 1, ParamFamily::Osc),
            sp("ac_hh_bell", "Bell", 0.7, 0.0, 2.0, 2, ParamFamily::Osc),
            sp(
                "ac_hh_wobble",
                "Wobble",
                0.25,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp("ac_hh_spread", "Spread", 0.0, 0.0, 1.0, 4, ParamFamily::Osc),
            sp(
                "ac_hh_brightness",
                "Brightness",
                0.0,
                -1.0,
                1.0,
                5,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "ac_hh_sat_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                10,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ac_hh_sat_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ac_hh_sat_mix",
                "Saturation Mix",
                0.5,
                0.0,
                1.0,
                12,
                ParamFamily::Saturation,
            ),
            sp(
                "ac_hh_sat_gain",
                "Saturation Output Gain",
                1.25,
                0.5,
                2.0,
                13,
                ParamFamily::Saturation,
            ),
        ],
        sound_settings_default: [
            8000.0, 0.15, 0.6, 20000.0, 0.0003, 0.0, 5.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 21,
        name: "OH6(AC)",
        label: "OA",
        full_name: "OH6 (analogcode)",
        midi_note: 55,
        algo_count: 1,
        standard_params: AC_TONE_STD,
        special_params: &[
            sp("ac_oh_metal", "Metal", 0.11, 0.0, 1.0, 0, ParamFamily::Osc),
            sp("ac_oh_click", "Click", 0.35, 0.0, 1.0, 1, ParamFamily::Osc),
            sp("ac_oh_bell", "Bell", 0.7, 0.0, 2.0, 2, ParamFamily::Osc),
            sp(
                "ac_oh_wobble",
                "Wobble",
                0.25,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp("ac_oh_spread", "Spread", 0.0, 0.0, 1.0, 4, ParamFamily::Osc),
            sp(
                "ac_oh_brightness",
                "Brightness",
                0.0,
                -1.0,
                1.0,
                5,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "ac_oh_sat_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                10,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ac_oh_sat_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ac_oh_sat_mix",
                "Saturation Mix",
                0.5,
                0.0,
                1.0,
                12,
                ParamFamily::Saturation,
            ),
            sp(
                "ac_oh_sat_gain",
                "Saturation Output Gain",
                1.25,
                0.5,
                2.0,
                13,
                ParamFamily::Saturation,
            ),
        ],
        sound_settings_default: [
            8000.0, 0.8, 0.7, 20000.0, 0.0003, 0.0, 5.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 22,
        name: "CL6(AC)",
        label: "CA",
        full_name: "CL6 (analogcode)",
        midi_note: 56,
        algo_count: 1,
        standard_params: AC_CLAP_STD,
        special_params: &[
            sp("ac_cl_noise", "Noise", 0.5, 0.0, 1.0, 0, ParamFamily::Osc),
            sp(
                "ac_cl_spread",
                "Spread",
                1.0,
                0.25,
                3.0,
                1,
                ParamFamily::Osc,
            ),
            sp("ac_cl_tail", "Tail", 0.5, 0.0, 1.0, 2, ParamFamily::Env),
            sp("ac_cl_air", "Air", 0.0, 0.0, 1.0, 3, ParamFamily::Osc),
            sp_discrete(
                "ac_cl_sat_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                10,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ac_cl_sat_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ac_cl_sat_mix",
                "Saturation Mix",
                0.5,
                0.0,
                1.0,
                12,
                ParamFamily::Saturation,
            ),
            sp(
                "ac_cl_sat_gain",
                "Saturation Output Gain",
                1.25,
                0.5,
                2.0,
                13,
                ParamFamily::Saturation,
            ),
        ],
        sound_settings_default: [
            1000.0, 0.25, 1.0, 20000.0, 0.0005, 0.0, 5.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    InstrumentDef {
        index: 23,
        name: "TM6(AC)",
        label: "TA",
        full_name: "TM6 (analogcode)",
        midi_note: 57,
        algo_count: 1,
        standard_params: AC_TOM_STD,
        special_params: &[
            sp_options(
                "ac_tm_model",
                "Model",
                1.0,
                0,
                ParamFamily::Osc,
                &["Auto", "Low tom", "High tom"],
            ),
            sp("ac_tm_strike", "Strike", 0.5, 0.0, 1.0, 1, ParamFamily::Osc),
            sp("ac_tm_snap", "Snap", 0.5, 0.0, 1.0, 2, ParamFamily::Osc),
            sp("ac_tm_glide", "Glide", 0.5, 0.0, 1.0, 3, ParamFamily::Osc),
            sp("ac_tm_modes", "Overtones", 0.5, 0.0, 1.0, 4, ParamFamily::Osc),
            sp(
                "ac_tm_tail_noise",
                "Tail Noise",
                0.5,
                0.0,
                1.0,
                5,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "ac_tm_sat_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                10,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "ac_tm_sat_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "ac_tm_sat_mix",
                "Saturation Mix",
                0.5,
                0.0,
                1.0,
                12,
                ParamFamily::Saturation,
            ),
            sp(
                "ac_tm_sat_gain",
                "Saturation Output Gain",
                1.25,
                0.5,
                2.0,
                13,
                ParamFamily::Saturation,
            ),
        ],
        sound_settings_default: [
            124.435, 0.35, 0.8, 20000.0, 0.0005, 0.0, 5.0, 3.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    // [208] OH6smp: the CH6smp definition on the open-hat bank. Only the
    // decay default differs - the point of an open hat is that it rings.
    InstrumentDef {
        index: 24,
        name: "OH606",
        label: "o6",
        full_name: "OH6smp",
        midi_note: 46,
        algo_count: 1,
        standard_params: SMP606_STD,
        special_params: &[
            sp_discrete(
                "oh606_analog_mode",
                "Analog Mode",
                1.0,
                0.0,
                1.0,
                0,
                ParamFamily::Osc,
            ),
            sp_discrete("oh606_sample", "Sample", 1.0, 1.0, 8.0, 1, ParamFamily::Osc),
            sp_discrete(
                "oh606_one_shot",
                "One Shot",
                1.0,
                0.0,
                1.0,
                2,
                ParamFamily::Env,
            ),
            sp(
                "oh606_start_offset",
                "Start",
                0.0,
                0.0,
                1.0,
                3,
                ParamFamily::Osc,
            ),
            sp_unit(
                "oh606_fine_tune",
                "Pitch Fine",
                0.0,
                -100.0,
                100.0,
                9,
                ParamFamily::Osc,
                " ct",
            ),
            sp("oh606_end", "End", 1.0, 0.0, 1.0, 11, ParamFamily::Osc),
            sp_discrete(
                "oh606_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                4,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "oh606_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "oh606_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                6,
                ParamFamily::Saturation,
            ),
            sp(
                "oh606_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_discrete(
                "oh606_saturation_pre_filter",
                "Saturation Pre-Filter",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        sound_settings_default: [
            0.0, 0.9, 0.6, 20000.0, 0.001, 0.0, 4.0, 3.0, 0.0, 0.0, 0.15, 1.0, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "LP",
        is_sampler: true,
        freq_as_notes: false,
        analog_fixed: false,
    },
    // [221] Rift - a slice lifted out of a long texture. Offset is the whole
    // point: declared `sp`, so it is p-lockable per step and morphable across
    // a fusion without a line of code of ours.
    InstrumentDef {
        index: 25,
        name: "Rift",
        label: "Rf",
        full_name: "Rift",
        midi_note: 58,
        algo_count: 1,
        standard_params: RIFT_STD,
        special_params: &[
            // [228] Les quatre textures embarquees, puis le fichier de la
            // LANE ("Custom", charge par la ligne File ; le panneau affiche
            // le nom du fichier a la place). [291] "Pad" vient APRES Custom :
            // les lanes sauvegardees stockent l'indice du menu (Custom = 4).
            sp_options(
                "rift_texture",
                "Texture",
                0.0,
                0,
                ParamFamily::Osc,
                &["Noise", "Metal", "Crackle", "Sweep", "Custom", "Pad"],
            ),
            // 0,05 : mesure sur la texture Noise v2, la zone la plus stable
            // (variation d'amplitude 5 % entre fenetres de 10 ms) et brillante ;
            // 0,25 tombait dans une zone rugueuse (34 %) qui, sur un coup de
            // 120 ms, s'entendait comme une enveloppe de pitch.
            sp("rift_offset", "Offset", 0.05, 0.0, 1.0, 1, ParamFamily::Osc),
            sp("rift_wander", "Wander", 0.0, 0.0, 1.0, 2, ParamFamily::Osc),
            // Avance sequentielle : chaque declenchement decale l'offset d'un
            // cran, ce qui fait progresser le son meme sur un pas repete.
            // [227] Le bouton Advance et ses options sont des reglages de
            // lane (`advance_mode`, lib.rs) ; ce slider est le pas.
            sp(
                "rift_advance",
                "Advance Step",
                0.0,
                0.0,
                0.25,
                20,
                ParamFamily::Osc,
            ),
            sp_unit(
                "rift_pitch_fine",
                "Pitch Fine",
                0.0,
                -100.0,
                100.0,
                17,
                ParamFamily::Pitch,
                " ct",
            ),
            // [227] Section a part, avec son graphe.
            sp(
                "rift_pitch_env",
                "Pitch Env Depth",
                0.0,
                -24.0,
                24.0,
                18,
                ParamFamily::Pitch,
            ),
            sp_unit(
                "rift_pitch_env_time",
                "Pitch Env Time",
                0.08,
                0.005,
                1.5,
                19,
                ParamFamily::Pitch,
                " s",
            ),
            sp_discrete(
                "rift_reverse",
                "Reverse",
                0.0,
                0.0,
                1.0,
                27,
                ParamFamily::Osc,
            ),
            sp_discrete("rift_loop", "Loop", 0.0, 0.0, 1.0, 12, ParamFamily::Osc),
            sp("rift_grain", "Grain", 0.35, 0.0, 1.0, 3, ParamFamily::Osc),
            sp(
                "rift_grain_shape",
                "Grain Shape",
                0.15,
                0.0,
                1.0,
                4,
                ParamFamily::Osc,
            ),
            // Build 3 : la voie droite lit la texture un peu PLUS LOIN que la
            // gauche - meme enveloppe, matiere differente. 0 = mono.
            sp(
                "rift_stereo_spread",
                "Stereo Spread",
                0.0,
                0.0,
                1.0,
                30,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "rift_filter_type",
                "Filter Type",
                0.0,
                0.0,
                2.0,
                5,
                ParamFamily::Filter,
            ),
            // Enveloppe de filtre A-H-D complete, comme Buzz et SDrex : le
            // decay seul ne donnait ni attaque ni courbe, et le graphe du
            // panneau n'avait rien a montrer.
            sp_unit(
                "rift_filter_attack",
                "Filter Attack",
                0.0,
                0.0,
                0.5,
                13,
                ParamFamily::Filter,
                " s",
            ),
            sp_unit(
                "rift_filter_hold",
                "Filter Hold",
                0.0,
                0.0,
                0.5,
                14,
                ParamFamily::Filter,
                " s",
            ),
            sp(
                "rift_filter_atk_curve",
                "Filter Atk Curve",
                0.0,
                -1.0,
                1.0,
                15,
                ParamFamily::Filter,
            ),
            sp(
                "rift_filter_dec_curve",
                "Filter Dec Curve",
                0.6,
                -1.0,
                1.0,
                16,
                ParamFamily::Filter,
            ),
            // Les deux LFO partagent forme et politique de phase : ce sont deux
            // destinations d'une meme idee, pas deux modulations sans rapport.
            sp_options(
                "rift_lfo_shape",
                "LFO Shape",
                0.0,
                21,
                ParamFamily::Modulation,
                &["Sine", "Triangle", "Square", "Saw", "S&H"],
            ),
            sp_discrete(
                "rift_lfo_free_phase",
                "LFO Free Phase",
                0.0,
                0.0,
                1.0,
                22,
                ParamFamily::Modulation,
            ),
            sp_unit(
                "rift_pitch_lfo_rate",
                "Pitch LFO Rate",
                5.0,
                0.05,
                40.0,
                23,
                ParamFamily::Modulation,
                " Hz",
            ),
            sp(
                "rift_pitch_lfo_depth",
                "Pitch LFO Depth",
                0.0,
                0.0,
                12.0,
                24,
                ParamFamily::Modulation,
            ),
            sp_unit(
                "rift_filter_lfo_rate",
                "Filter LFO Rate",
                5.0,
                0.05,
                40.0,
                25,
                ParamFamily::Modulation,
                " Hz",
            ),
            sp(
                "rift_filter_lfo_depth",
                "Filter LFO Depth",
                0.0,
                0.0,
                // [297] 5 octaves each way: at full depth from the lowest Filter the sweep
                // covers the whole range, 20 Hz to 20 kHz (3 only reached 1.3 kHz).
                5.0,
                26,
                ParamFamily::Modulation,
            ),
            // Defaut 0,9 = le Q fixe de Buzz, donc un filtre qui se comporte
            // exactement comme les autres tant qu'on n'y touche pas.
            sp(
                "rift_resonance",
                "Resonance",
                0.9,
                0.5,
                20.0,
                6,
                ParamFamily::Filter,
            ),
            sp_discrete(
                "rift_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                7,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "rift_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                8,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "rift_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                9,
                ParamFamily::Saturation,
            ),
            sp(
                "rift_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                10,
                ParamFamily::Saturation,
            ),
            // Build 3, la paire lo-fi : reduction de bits et division de la
            // frequence d'echantillonnage.
            sp(
                "rift_crush",
                "Crush",
                0.0,
                0.0,
                1.0,
                28,
                ParamFamily::Saturation,
            ),
            sp(
                "rift_decimate",
                "Decimate",
                0.0,
                0.0,
                1.0,
                29,
                ParamFamily::Saturation,
            ),
            // [232] En DERNIER : le switch place tout le bloc Distortion
            // (Decimate, Crush, saturation) avant ou apres le filtre.
            sp_discrete(
                "rift_saturation_pre_filter",
                "Pre-Filter",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        // Filtre d'usine NEUTRE : ouvert (20 kHz), enveloppe a 0. Le reglage
        // repris de Buzz (1200 Hz + env 0,6) balayait la coupure de 6,5 kHz a
        // 1,2 kHz en 120 ms sur chaque coup, ce que l'oreille lisait comme une
        // enveloppe de pitch impossible a retirer par « Default ». Sur une
        // texture, le defaut doit etre la tranche telle qu'elle est.
        sound_settings_default: [
            0.0, 0.12, 0.8, 20000.0, 0.05, 0.0, 0.0, 0.0, 0.0, 0.0, 0.12, 0.0, 0.0,
        ],
        freq_display_ratio: 1.0,
        // Vide, comme Buzz : le type est un menu, l'ecrire dans le libelle de
        // la coupure donnerait "Filter (Multi)", qui ne veut rien dire.
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    // [243] One-Shot - the lane's own sample file, played start to finish. No
    // embedded content: the `_texture` special is an infra marker that keeps
    // the [228] lane-file rules alive (File row, presets, reconcile); its
    // one-entry "menu" is not rendered (sound_editor skips options.len() <= 1).
    InstrumentDef {
        index: 26,
        name: "OneShot",
        label: "OS",
        full_name: "One-Shot",
        midi_note: 59,
        algo_count: 1,
        standard_params: ONE_SHOT_STD,
        special_params: &[
            sp_options(
                "oneshot_texture",
                "Texture",
                0.0,
                0,
                ParamFamily::Osc,
                &["Custom"],
            ),
            // Offset start : fraction du fichier ou la lecture COMMENCE, dans
            // les deux sens (en Reverse elle compte depuis la fin). Index 20,
            // ajoute apres coup - les indices existants ne bougent pas.
            sp(
                "oneshot_offset",
                "Offset",
                0.0,
                0.0,
                1.0,
                20,
                ParamFamily::Osc,
            ),
            sp_discrete(
                "oneshot_reverse",
                "Reverse",
                0.0,
                0.0,
                1.0,
                1,
                ParamFamily::Osc,
            ),
            sp_unit(
                "oneshot_pitch_fine",
                "Pitch Fine",
                0.0,
                -100.0,
                100.0,
                2,
                ParamFamily::Pitch,
                " ct",
            ),
            // Enveloppe de pitch A-H-D complete (pas le simple depth+time de
            // Rift) : profondeur en demi-tons, temps en FRACTION de la duree
            // jouee du sample (1.0 = tout le sample), chaque rampe avec sa
            // courbe bipolaire. Suffixes _atk/_hld : les suffixes _attack/
            // _hold sont reserves aux quantites physiques AVEC unite.
            // Ordre canonique [240] a l'affichage (un temps, puis sa courbe) :
            // Depth, Attack, Atk Curve, Hold, Decay, Dec Curve. Les indices
            // ne bougent pas — l'ordre de liste ne pilote que le rendu.
            sp(
                "oneshot_pitch_env",
                "Pitch Env Depth",
                0.0,
                -24.0,
                24.0,
                3,
                ParamFamily::Pitch,
            ),
            sp(
                "oneshot_pitch_env_atk",
                "Pitch Env Attack",
                0.0,
                0.0,
                1.0,
                4,
                ParamFamily::Pitch,
            ),
            sp(
                "oneshot_pitch_env_atk_curve",
                "Pitch Env Atk Curve",
                0.0,
                -1.0,
                1.0,
                7,
                ParamFamily::Pitch,
            ),
            sp(
                "oneshot_pitch_env_hld",
                "Pitch Env Hold",
                0.0,
                0.0,
                1.0,
                5,
                ParamFamily::Pitch,
            ),
            sp(
                "oneshot_pitch_env_decay",
                "Pitch Env Decay",
                0.1,
                0.005,
                1.0,
                6,
                ParamFamily::Pitch,
            ),
            sp(
                "oneshot_pitch_env_dec_curve",
                "Pitch Env Dec Curve",
                0.6,
                -1.0,
                1.0,
                8,
                ParamFamily::Pitch,
            ),
            sp_discrete(
                "oneshot_filter_type",
                "Filter Type",
                0.0,
                0.0,
                2.0,
                9,
                ParamFamily::Filter,
            ),
            sp(
                "oneshot_resonance",
                "Resonance",
                0.9,
                0.5,
                20.0,
                10,
                ParamFamily::Filter,
            ),
            sp(
                "oneshot_filter_atk",
                "Filter Attack",
                0.0,
                0.0,
                1.0,
                11,
                ParamFamily::Filter,
            ),
            sp(
                "oneshot_filter_hld",
                "Filter Hold",
                0.0,
                0.0,
                1.0,
                12,
                ParamFamily::Filter,
            ),
            sp(
                "oneshot_filter_atk_curve",
                "Filter Atk Curve",
                0.0,
                -1.0,
                1.0,
                13,
                ParamFamily::Filter,
            ),
            sp(
                "oneshot_filter_dec_curve",
                "Filter Dec Curve",
                0.6,
                -1.0,
                1.0,
                14,
                ParamFamily::Filter,
            ),
            sp_discrete(
                "oneshot_saturation_type",
                "Saturation Type",
                0.0,
                0.0,
                5.0,
                15,
                ParamFamily::Saturation,
            ),
            sp_curved(
                "oneshot_saturation_amount",
                "Saturation Amount",
                0.0,
                0.0,
                1.0,
                16,
                ParamFamily::Saturation,
                SAT_AMOUNT_CURVE,
            ),
            sp(
                "oneshot_saturation_mix",
                "Saturation Mix",
                1.0,
                0.0,
                1.0,
                17,
                ParamFamily::Saturation,
            ),
            sp(
                "oneshot_saturation_output_gain",
                "Saturation Output Gain",
                1.0,
                0.5,
                2.0,
                18,
                ParamFamily::Saturation,
            ),
            // [232] En DERNIER : le switch place tout le bloc Distortion
            // avant ou apres le filtre.
            sp_discrete(
                "oneshot_saturation_pre_filter",
                "Pre-Filter",
                0.0,
                0.0,
                1.0,
                19,
                ParamFamily::Saturation,
            ),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        // Usine neutre : filtre ouvert, enveloppes a 0, decay a 1.0 = toute la
        // duree jouee pour que le fichier sonne entier ; le son vient du
        // fichier, pas des reglages.
        sound_settings_default: [
            0.0, 1.0, 0.8, 20000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.12, 0.0, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: false,
    },
    // [286] Reverb: the send effect of an effect-bus slot. Not a sound source:
    // no steps, no note. Its knobs are specials 0-3, 5, 6 (4 and 31 are reserved
    // by the p-lock layout); special 7 is the lane's p-lock mode (Glide).
    InstrumentDef {
        index: 27,
        name: "Reverb",
        label: "RV",
        full_name: "Reverb",
        midi_note: 0,
        algo_count: 1,
        standard_params: REVERB_STD,
        special_params: &[
            sp("reverb_size", "Size", 1.0, 0.5, 2.0, 0, ParamFamily::Osc),
            sp_unit("reverb_decay", "Decay", 2.5, 0.2, 12.0, 1, ParamFamily::Osc, " s"),
            // [294] Warp: Decay also sets the length of the lines (a lane setting, never locked).
            sp_discrete("reverb_warp", "Warp", 0.0, 0.0, 1.0, 14, ParamFamily::Osc),
            sp("reverb_damping", "Damping", 0.5, 0.0, 1.0, 2, ParamFamily::Osc),
            sp_unit("reverb_predelay", "Pre-delay", 20.0, 0.0, 250.0, 3, ParamFamily::Osc, " ms"),
            sp_unit("reverb_lowcut", "Low Cut", 120.0, 20.0, 1000.0, 5, ParamFamily::Osc, " Hz"),
            sp_unit("reverb_highcut", "High Cut", 20000.0, 20.0, 20000.0, 9, ParamFamily::Osc, " Hz"),
            sp("reverb_width", "Width", 1.0, 0.0, 1.0, 6, ParamFamily::Osc),
            // p-lock mode of this lane: 0 = a lock holds until the next, 1 = the value
            // glides from one lock to the next (see `fx_locks.rs`).
            sp_discrete("reverb_glide", "Glide", 0.0, 0.0, 1.0, 7, ParamFamily::Osc),
            // Loop restart (shown as an ON switch; stored inverted so older lanes, which
            // hold 0 here, keep restarting).
            sp_discrete("reverb_norestart", "Loop Restart", 0.0, 0.0, 1.0, 8, ParamFamily::Osc),
            // The lane's character (a lane setting, never locked): Hall is the original sound.
            sp_options(
                "reverb_type",
                "Type",
                0.0,
                10,
                ParamFamily::Osc,
                &["Hall", "Room", "Plate"],
            ),
            // Gate: with the switch on, the tail is cut this long after each hit.
            sp_discrete("reverb_gate_on", "Gate", 0.0, 0.0, 1.0, 12, ParamFamily::Osc),
            sp_unit("reverb_gate", "Gate Time", 250.0, 10.0, 1500.0, 11, ParamFamily::Osc, " ms"),
            sp_unit("reverb_gate_release", "Gate Release", 30.0, 2.0, 500.0, 13, ParamFamily::Osc, " ms"),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        // Only Volume (the return level) is exposed; the rest is inert.
        sound_settings_default: [
            1000.0, 1.0, 1.0, 20000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    // [286] Delay: the second send effect of an effect-bus slot. Not a sound source.
    // Its knobs are specials 0-3 and 5 (4 and 31 are reserved by the p-lock layout);
    // special 7 is the lane's p-lock mode (Glide). Gliding the time bends the repeats.
    InstrumentDef {
        index: 28,
        name: "Delay",
        label: "DL",
        full_name: "Delay",
        midi_note: 0,
        algo_count: 1,
        standard_params: REVERB_STD,
        special_params: &[
            // (Min 0, not 1: the panel draws any range of more than 20x on an exponential slider, no use
            // for a time set by feel; the engine keeps it at 1 ms or more.)
            sp_unit("delay_time", "Time", 375.0, 0.0, 2000.0, 0, ParamFamily::Osc, " ms"),
            sp("delay_feedback", "Feedback", 0.4, 0.0, 0.95, 1, ParamFamily::Osc),
            sp("delay_damping", "Damping", 0.4, 0.0, 1.0, 2, ParamFamily::Osc),
            sp_unit("delay_lowcut", "Low Cut", 100.0, 20.0, 1000.0, 3, ParamFamily::Osc, " Hz"),
            sp_unit("delay_highcut", "High Cut", 20000.0, 20.0, 20000.0, 9, ParamFamily::Osc, " Hz"),
            sp("delay_pingpong", "Ping-pong", 0.5, 0.0, 1.0, 5, ParamFamily::Osc),
            sp("delay_width", "Width", 0.0, 0.0, 1.0, 10, ParamFamily::Osc),
            // Free = the Time knob; the others lock the time to a note value at the
            // host tempo (the Time knob is then greyed). A lane-wide choice.
            sp_options(
                "delay_sync",
                "Sync",
                0.0,
                6,
                ParamFamily::Osc,
                &[
                    "Free", "1/32", "1/16T", "1/16", "1/8T", "1/16.", "1/8", "1/4T", "1/8.", "1/4", "1/2T",
                    "1/4.", "1/2", "1 bar",
                ],
            ),
            // p-lock mode of this lane: 0 = a lock holds until the next, 1 = the value
            // glides from one lock to the next (see `fx_locks.rs`).
            sp_discrete("delay_glide", "Glide", 0.0, 0.0, 1.0, 7, ParamFamily::Osc),
            sp_discrete("delay_norestart", "Loop Restart", 0.0, 0.0, 1.0, 8, ParamFamily::Osc),
        ],
        // [freq, decay, vol, filter_freq, attack, release, decay_curve,
        //  release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo]
        // Only Volume (the return level) is exposed; the rest is inert.
        sound_settings_default: [
            1000.0, 1.0, 1.0, 20000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
    // [286] Rumble: the third send effect of an effect-bus slot: the kick rumble (a long,
    // dark reverb of what is sent, kept under a low-pass and ducked by the input).
    // Its knobs are specials 0-3, 5, 6 (4 and 31 are reserved by the p-lock layout);
    // special 7 is the lane's p-lock mode (Glide), 8 its loop restart.
    InstrumentDef {
        index: 29,
        name: "Rumble",
        label: "RU",
        full_name: "Rumble",
        midi_note: 0,
        algo_count: 1,
        standard_params: REVERB_STD,
        special_params: &[
            // The Rumble's sections follow the AUDIO FLOW, top to bottom; the Sound tab
            // titles them (see `rumble_section_title`): Echo (Osc) -> Reverb (Pitch) ->
            // Filter -> Width & Drive (Modulation) -> Duck (Saturation) -> Output.
            //
            // 1. Echo: a mono echo in front of the reverb, it makes the rumble gallop.
            sp("rumble_echo", "Echo", 0.0, 0.0, 1.0, 11, ParamFamily::Osc),
            // (Min 0, not 10: a range of more than 20x would make the slider exponential, which is
            // no use for a time set by feel; the engine keeps it at 1 ms or more.)
            sp_unit("rumble_echo_time", "Echo Time", 250.0, 0.0, 1000.0, 12, ParamFamily::Osc, " ms"),
            sp_options("rumble_echo_sync", "Echo Sync", 0.0, 13, ParamFamily::Osc, RUMBLE_ECHO_SYNC),
            sp("rumble_echo_feedback", "Echo Feedback", 0.3, 0.0, 0.9, 14, ParamFamily::Osc),
            // 2. The reverb underneath. The defaults are the sound the Rumble always had (the
            // biggest room, no pre-delay, a gentle 0.7 ms wobble); zeros are what older lanes
            // hold and are reseeded to these.
            sp_unit("rumble_decay", "Decay", 3.0, 0.5, 8.0, 0, ParamFamily::Pitch, " s"),
            sp("rumble_reverb_size", "Reverb Size", 2.0, 0.5, 2.0, 17, ParamFamily::Pitch),
            sp_unit("rumble_reverb_predelay", "Reverb Pre-delay", 0.0, 0.0, 250.0, 18, ParamFamily::Pitch, " ms"),
            sp_unit("rumble_reverb_motion", "Reverb Motion", 0.7, 0.1, 3.0, 19, ParamFamily::Pitch, " ms"),
            // The "blur" of the rumble: 1 = fully smeared (the original sound), low = the kick's
            // repeats stay distinct.
            sp("rumble_reverb_blur", "Reverb Blur", 1.0, 0.05, 1.0, 20, ParamFamily::Pitch),
            // 3. What the rumble keeps.
            sp_unit("rumble_highpass", "High Pass", 30.0, 20.0, 200.0, 9, ParamFamily::Filter, " Hz"),
            sp_unit("rumble_lowpass", "Low Pass", 120.0, 40.0, 300.0, 1, ParamFamily::Filter, " Hz"),
            // 4. Stereo, then the saturation.
            sp("rumble_width", "Width", 0.0, 0.0, 1.0, 10, ParamFamily::Modulation),
            sp("rumble_drive", "Drive", 0.25, 0.0, 1.0, 5, ParamFamily::Modulation),
            // 5. The duck, last: its graph (to the right of the rows) faces these four. What
            // starts it: Beat (0, the default and what a lane saved before this setting reads)
            // follows the transport, so it pumps whatever is sent; Audio listens to the input.
            sp_options(
                "rumble_duck_source",
                "Duck Trigger",
                0.0,
                16,
                ParamFamily::Saturation,
                &["Beat", "1/8", "1/16", "Audio"],
            ),
            sp("rumble_duck", "Duck", 0.8, 0.0, 1.0, 2, ParamFamily::Saturation),
            sp("rumble_duck_curve", "Duck Curve", 0.5, 0.0, 1.0, 15, ParamFamily::Saturation),
            sp_unit("rumble_release", "Duck Length", 250.0, 20.0, 1000.0, 3, ParamFamily::Saturation, " ms"),
            // 6. Output: the lane Volume, and how the lane's p-locks behave.
            sp_discrete("rumble_glide", "Glide", 0.0, 0.0, 1.0, 7, ParamFamily::Output),
            sp_discrete("rumble_norestart", "Loop Restart", 0.0, 0.0, 1.0, 8, ParamFamily::Output),
        ],
        // Only Volume (the return level) is exposed; the rest is inert.
        sound_settings_default: [
            1000.0, 1.0, 1.0, 20000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.05, 0.5, 0.0,
        ],
        freq_display_ratio: 1.0,
        filter_type_label: "",
        is_sampler: false,
        freq_as_notes: false,
        analog_fixed: true,
    },
];

/// Highest algo index across all instruments. Used as the shared range of the
/// per-slot algo params (any kind can live on any slot); kept >= 1 because an
/// IntRange with min == max crashes nih-plug normalization (bug [42]).
pub fn max_algo_index() -> i32 {
    INSTRUMENTS
        .iter()
        .map(|i| i.algo_count)
        .max()
        .unwrap_or(2)
        .saturating_sub(1)
        .max(1) as i32
}

pub fn special_params(voice_idx: usize) -> &'static [SpecialParamDef] {
    INSTRUMENTS[voice_idx].special_params
}

pub fn filter_type_label(voice_idx: usize) -> &'static str {
    INSTRUMENTS[voice_idx].filter_type_label
}

pub struct MorphableField {
    pub field_index: usize,
    pub label: &'static str,
    // [269] The slider range rides along for the morphing tests, which check
    // it; the engine morphs plock values as stored.
    #[cfg_attr(not(test), allow(dead_code))]
    pub min: f32,
    #[cfg_attr(not(test), allow(dead_code))]
    pub max: f32,
}

const SPECIAL_FIELD_START: usize = 14;

/// Returns all plock fields that support continuous morphing for a given instrument.
/// Includes all standard (slider) fields and continuous special parameters.
pub fn morphable_fields(voice_idx: usize) -> Vec<MorphableField> {
    if voice_idx >= INSTRUMENTS.len() {
        return Vec::new();
    }
    let inst = &INSTRUMENTS[voice_idx];
    let mut fields = Vec::new();

    for def in inst.standard_params {
        match def.widget {
            ParamWidget::Slider { min, max, .. } => {
                fields.push(MorphableField {
                    field_index: def.field.plock_field_index(),
                    label: def.label,
                    min,
                    max,
                });
            }
            ParamWidget::Checkbox => {
                fields.push(MorphableField {
                    field_index: def.field.plock_field_index(),
                    label: def.label,
                    min: 0.0,
                    max: 1.0,
                });
            }
        }
    }

    let standard_field_indices: std::collections::HashSet<usize> = inst
        .standard_params
        .iter()
        .map(|def| def.field.plock_field_index())
        .collect();

    for def in inst.special_params {
        if !def.continuous {
            continue;
        }
        let field_index = SPECIAL_FIELD_START + def.special_index;
        if field_index == StandardField::Attack.plock_field_index()
            || standard_field_indices.contains(&field_index)
        {
            continue;
        }
        fields.push(MorphableField {
            field_index,
            label: def.label,
            min: def.min,
            max: def.max,
        });
    }

    fields
}

/// Map an incoming MIDI note number to a voice index.
/// Returns `Some(index)` if the note matches one of the instrument's default
/// MIDI notes, `None` otherwise.
#[cfg(test)]
mod tests {
    use super::*;

    fn has_stereo(def: &InstrumentDef) -> bool {
        def.standard_params
            .iter()
            .any(|p| p.field == StandardField::Stereo)
    }

    /// [269] These traits used to be index lists in the Sound Editor; they now
    /// live on each `InstrumentDef`. Pin the voices each list covered so the
    /// move changed nothing. Only the 27 voices of the time are pinned: a voice
    /// added later declares its traits freely.
    #[test]
    fn voice_traits_match_the_former_index_lists() {
        use crate::synthesis::DrumVoice as V;
        const PINNED: usize = 27;
        let with = |t: fn(&InstrumentDef) -> bool| -> Vec<usize> {
            (0..PINNED).filter(|&i| t(&INSTRUMENTS[i])).collect()
        };
        let idx = |vs: &[V]| -> Vec<usize> {
            let mut v: Vec<usize> = vs.iter().map(|&v| v as usize).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(
            with(|d| d.is_sampler),
            idx(&[V::Bd606, V::Sd606, V::Ch606, V::Oh606])
        );
        assert_eq!(
            with(|d| d.freq_as_notes),
            idx(&[V::Kick, V::BassDrum808, V::Bd6Ac])
        );
        assert_eq!(
            with(|d| d.analog_fixed),
            idx(&[
                V::HiHat,
                V::OpenHiHat,
                V::Clap,
                V::Ride,
                V::Snare606,
                V::Perc1
            ])
        );
        for (i, def) in INSTRUMENTS.iter().enumerate() {
            assert_eq!(is_sampler(i), def.is_sampler, "{}", def.full_name);
        }
        assert!(!is_sampler(INSTRUMENTS.len()), "out of range must be false");
    }

    /// [269] The Sound Editor names the Algorithm selector's entries from
    /// `algos_for` and the engine clamps the choice to `algo_count`: they must
    /// agree on every voice. This replaced the UI's `voice_idx != 3` exception
    /// (OpenHiHat shares HiHat's list), and caught Snare606 / 808 Kick
    /// declaring 2 algorithms for a single one.
    #[test]
    fn algo_count_matches_the_named_algorithms() {
        for (i, def) in INSTRUMENTS.iter().enumerate() {
            let voice = crate::synthesis::DrumVoice::from_index(i).expect("voice index");
            assert_eq!(
                def.algo_count,
                crate::synthesis::algos_for(voice).len(),
                "{}: algo_count disagrees with the algorithms algos_for names",
                def.full_name
            );
        }
    }

    /// [269] Two Sound Editor rules are keyed on parameter names instead of a
    /// voice index: a rename would switch them off without a word.
    #[test]
    fn name_keyed_sound_editor_rules_find_their_params() {
        use crate::synthesis::DrumVoice as V;
        let specials = |v: V| INSTRUMENTS[v as usize].special_params.iter();
        assert!(
            specials(V::Buzz)
                .any(|d| d.family == ParamFamily::Env && d.name.starts_with(GATE_ROW_PREFIX)),
            "Buzz lost its gate sub-row params"
        );
        assert!(specials(V::Sdrex).any(|d| d.name.ends_with(FILTER_MOD_SUFFIX)));
        assert!(specials(V::Sdrex).any(|d| d.name.ends_with(FLANGER_ONLY_SUFFIX)));
        // Nobody else picks the rules up by accident.
        for (i, def) in INSTRUMENTS.iter().enumerate() {
            let has = |f: &dyn Fn(&str) -> bool| def.special_params.iter().any(|d| f(d.name));
            if i != V::Buzz as usize {
                assert!(
                    !has(&|n| n.starts_with(GATE_ROW_PREFIX)),
                    "{}",
                    def.full_name
                );
            }
            if i != V::Sdrex as usize {
                assert!(
                    !has(&|n| n.ends_with(FILTER_MOD_SUFFIX)),
                    "{}",
                    def.full_name
                );
            }
        }
    }

    /// "Reset to factory" must mean the INSTRUMENT's factory value, on every
    /// voice ([224]).
    ///
    /// Regression: `param_default` read a voice's own table only for the
    /// samplers, SDrex and Rift; the other twenty fell back to one shared
    /// default. Measured then: 20 of the 26 instruments reset at least one
    /// slider to a value that was not theirs, 17 of those outside the slider's
    /// own range.
    #[test]
    fn every_voice_resets_to_its_own_factory_value() {
        use crate::param_id::ParamId;
        for idx in 0..crate::synthesis::DrumVoice::COUNT {
            let inst = &INSTRUMENTS[idx];
            for def in inst.standard_params {
                let expected = inst.sound_settings_default[def.field as usize];
                let got = param_default(idx, ParamId::Std(def.field));
                assert!(
                    (got - expected).abs() < 1e-6,
                    "{} {:?} resets to {got}, expected its own {expected}",
                    inst.full_name,
                    def.field
                );
            }
        }
    }

    /// Rift's Pitch is in semitones, so its factory value is 0 and it has to
    /// land inside the slider that shows it — the case that surfaced [224].
    #[test]
    fn rift_factory_defaults_fit_their_sliders() {
        use crate::param_id::ParamId;
        let idx = crate::synthesis::DrumVoice::Rift as usize;
        let rift = &INSTRUMENTS[idx];
        for def in rift.standard_params {
            let got = param_default(idx, ParamId::Std(def.field));
            if let ParamWidget::Slider { min, max, .. } = def.widget {
                assert!(
                    got >= min && got <= max,
                    "{:?} resets to {got}, outside its own {min}..{max} slider",
                    def.field
                );
            }
        }
        assert_eq!(param_default(idx, ParamId::Std(StandardField::Freq)), 0.0);
    }

    #[test]
    fn stereo_capable_voices_expose_the_stereo_checkbox() {
        // Snare, HiHat, OpenHiHat, Clap, Ride, Cymbal, Snare606, Perc1,
        // BD606, SD606, CH606 ([168]: two-sample L&R stereo), Buzz,
        // One-Shot ([243]: a stereo user file plays its two channels).
        for idx in [1usize, 2, 3, 7, 8, 9, 10, 12, 13, 14, 15, 16, 26] {
            assert!(
                has_stereo(&INSTRUMENTS[idx]),
                "{} (idx {idx}) lost its Stereo standard param",
                INSTRUMENTS[idx].name
            );
        }
    }

    #[test]
    fn mono_voices_do_not_expose_the_stereo_checkbox() {
        // Kick, Tom1-3, B8, Sdrex stay mono. (Rift has the switch since
        // [228]: a stereo user file plays its two channels.)
        for idx in [0usize, 4, 5, 6, 11, 17] {
            assert!(
                !has_stereo(&INSTRUMENTS[idx]),
                "{} (idx {idx}) unexpectedly exposes Stereo",
                INSTRUMENTS[idx].name
            );
        }
    }

    #[test]
    fn sdrex_modulation_has_its_own_parameter_family() {
        let sdrex = &INSTRUMENTS[17];
        // The Modulation section is exactly these params ([181] renamed the
        // flanger's Delay into the shared `sdrex_modulation_fade`).
        let modulation: Vec<&str> = sdrex
            .special_params
            .iter()
            .filter(|param| param.family == ParamFamily::Modulation)
            .map(|param| param.name)
            .collect();
        assert_eq!(
            modulation,
            vec![
                "sdrex_filter_mod",
                "sdrex_flanger_rate",
                "sdrex_modulation_fade",
                "sdrex_flanger_depth",
                "sdrex_flanger_feedback",
                "sdrex_flanger_wet",
                "sdrex_free_phase",
            ]
        );
        assert!(sdrex
            .special_params
            .iter()
            .filter(|param| {
                param.name.starts_with("sdrex_filter_") && param.name != "sdrex_filter_mod"
            })
            .all(|param| param.family == ParamFamily::Filter));
        let free_phase = sdrex
            .special_params
            .iter()
            .find(|param| param.name == "sdrex_free_phase")
            .unwrap();
        assert_eq!(free_phase.family, ParamFamily::Modulation);
        let filter_mod = sdrex
            .special_params
            .iter()
            .find(|param| param.name == "sdrex_filter_mod")
            .unwrap();
        assert_eq!(filter_mod.family, ParamFamily::Modulation);
        assert!(!filter_mod.continuous);
    }

    /// [182] Special params measuring a physical quantity must declare their unit
    /// — `SpecialParamDef` had no unit field at all, so every one of them used to
    /// render as a bare number while standard params showed " s" / " Hz".
    ///
    /// Two guards: an exact snapshot of who carries a unit (so a dimensionless
    /// "amount" never gains a bogus one), and a keyword rule so the NEXT
    /// frequency or time parameter added cannot silently forget it.
    #[test]
    fn physical_special_params_declare_their_unit() {
        let with_unit: Vec<(&str, &str)> = INSTRUMENTS
            .iter()
            .flat_map(|inst| inst.special_params.iter())
            .filter_map(|param| param.unit.map(|unit| (param.name, unit)))
            .collect();
        assert_eq!(
            with_unit,
            vec![
                ("tom_pitch_lfo_rate", " Hz"),
                ("tom_pitch_lfo_fade", " ms"),
                ("tom_pitch_lfo_rate", " Hz"),
                ("tom_pitch_lfo_fade", " ms"),
                ("tom_pitch_lfo_rate", " Hz"),
                ("tom_pitch_lfo_fade", " ms"),
                ("cymbal_shimmer_freq", " Hz"),
                ("bassdrum808_click_tone", " Hz"),
                ("bd606_fine_tune", " ct"),
                ("sd606_fine_tune", " ct"),
                ("ch606_fine_tune", " ct"),
                ("buzz_gate_rate", " Hz"),
                ("buzz_filter_attack", " s"),
                ("buzz_filter_hold", " s"),
                ("sdrex_flanger_rate", " Hz"),
                ("sdrex_modulation_fade", " ms"),
                ("sdrex_filter_attack", " s"),
                ("sdrex_filter_hold", " s"),
                ("ac_bd_click_tone", " Hz"),
                ("oh606_fine_tune", " ct"),
                ("rift_pitch_fine", " ct"),
                ("rift_pitch_env_time", " s"),
                ("rift_filter_attack", " s"),
                ("rift_filter_hold", " s"),
                ("rift_pitch_lfo_rate", " Hz"),
                ("rift_filter_lfo_rate", " Hz"),
                ("oneshot_pitch_fine", " ct"),
                ("reverb_decay", " s"),
                ("reverb_predelay", " ms"),
                ("reverb_lowcut", " Hz"),
                ("reverb_highcut", " Hz"),
                ("reverb_gate", " ms"),
                ("reverb_gate_release", " ms"),
                ("delay_time", " ms"),
                ("delay_lowcut", " Hz"),
                ("delay_highcut", " Hz"),
                ("rumble_echo_time", " ms"),
                ("rumble_decay", " s"),
                ("rumble_reverb_predelay", " ms"),
                ("rumble_reverb_motion", " ms"),
                ("rumble_highpass", " Hz"),
                ("rumble_lowpass", " Hz"),
                ("rumble_release", " ms"),
            ]
        );

        // Anything named like a frequency or a duration needs one. `_atk_curve`
        // and `snare606_tone` (a 0..1 blend) deliberately do not match.
        const NEEDS_UNIT: [&str; 6] = ["_rate", "_freq", "_attack", "_hold", "_fade", "_fine_tune"];
        for inst in INSTRUMENTS.iter() {
            for param in inst.special_params {
                if NEEDS_UNIT.iter().any(|kw| param.name.contains(kw)) {
                    assert!(
                        param.unit.is_some(),
                        "{} measures a physical quantity but declares no unit",
                        param.name
                    );
                }
            }
        }
    }

    /// [181] The decay sliders of the two kicks and the clap are capped where the
    /// sound actually stops changing — 5 s of convex decay was mostly inaudible
    /// tail, so most of the slider travel did nothing. The Cymbal, which shares
    /// the clap's parameter shape, must KEEP its long range.
    #[test]
    fn kick_bd808_and_clap_decay_caps_leave_the_cymbal_alone() {
        let decay_max = |name: &str| {
            let inst = INSTRUMENTS
                .iter()
                .find(|inst| inst.name == name)
                .unwrap_or_else(|| panic!("no instrument named {name}"));
            let def = inst
                .standard_params
                .iter()
                .find(|param| param.field == StandardField::Decay)
                .unwrap();
            let ParamWidget::Slider { max, .. } = def.widget else {
                panic!("Decay should be a slider on {name}");
            };
            max
        };
        assert_eq!(decay_max("Kick"), 2.0);
        assert_eq!(decay_max("BassDrum808"), 2.0);
        assert_eq!(decay_max("Clap"), 1.5);
        assert_eq!(decay_max("Cymbal"), 5.0, "the cymbal needs its long tail");
        // [188] Same treatment for the closed hat — and the open one keeps its
        // range, since the long tail is the point of an OPEN hi-hat.
        assert_eq!(decay_max("HiHat"), 1.5);
        assert_eq!(
            decay_max("OpenHiHat"),
            5.0,
            "the open hat needs its long tail"
        );
    }

    /// Decays reach 1.5 s; the holds are deliberately capped at 1 s ([181] — 2 s
    /// of hold was unusable slider travel).
    #[test]
    fn sdrex_envelope_ranges_are_capped_where_the_user_wants_them() {
        let sdrex = &INSTRUMENTS[17];
        for field in [
            StandardField::Decay,
            StandardField::Hold,
            StandardField::FilterEnvDecay,
        ] {
            let def = sdrex
                .standard_params
                .iter()
                .find(|param| param.field == field)
                .unwrap();
            let ParamWidget::Slider { max, .. } = def.widget else {
                panic!("{field:?} should be a slider");
            };
            let expected = if field == StandardField::Hold {
                1.0
            } else {
                1.5
            };
            assert_eq!(max, expected, "unexpected maximum for {field:?}");
        }
        let filter_hold = sdrex
            .special_params
            .iter()
            .find(|param| param.name == "sdrex_filter_hold")
            .unwrap();
        assert_eq!(filter_hold.max, 1.0);
        assert_eq!(filter_hold.family, ParamFamily::Filter);
    }

    /// [240] The Sound Panel sorts the Amp rows by `env_row_rank` at render
    /// time: every instrument must read its envelope in the canonical order
    /// (a time, then its curve), whatever its table's declaration order.
    #[test]
    fn amp_section_sorts_to_the_canonical_order_on_every_voice() {
        const CANONICAL: [StandardField; 5] = [
            StandardField::Attack,
            StandardField::ReleaseCurve, // "Attack Curve"
            StandardField::Hold,
            StandardField::Decay,
            StandardField::DecayCurve,
        ];
        for inst in INSTRUMENTS.iter() {
            let mut env: Vec<StandardField> = inst
                .standard_params
                .iter()
                .filter(|d| d.family == ParamFamily::Env)
                .map(|d| d.field)
                .collect();
            env.sort_by_key(|f| env_row_rank(*f)); // what the panel does
            let ranked: Vec<StandardField> = env
                .iter()
                .copied()
                .filter(|f| CANONICAL.contains(f))
                .collect();
            let expected: Vec<StandardField> = CANONICAL
                .iter()
                .copied()
                .filter(|f| ranked.contains(f))
                .collect();
            assert_eq!(ranked, expected, "{}: Amp rows out of order", inst.label);
        }
    }

    /// [240] The panel hoists the filter-envelope stages under Filter Env /
    /// Filter Decay by NAME SUFFIX: every voice with a filter envelope must
    /// declare them, in the Filter family, under the expected names.
    #[test]
    fn filter_envelope_stages_are_hoistable_on_every_voice_that_has_one() {
        // Each voice lists its stages in CANONICAL display order (a time,
        // then its curve) — One-Shot spells them `_atk`/`_hld` ([243]).
        for (prefix, stages) in [
            (
                "buzz",
                [
                    "buzz_filter_attack",
                    "buzz_filter_atk_curve",
                    "buzz_filter_hold",
                    "buzz_filter_curve",
                ],
            ),
            (
                "sdrex",
                [
                    "sdrex_filter_attack",
                    "sdrex_filter_atk_curve",
                    "sdrex_filter_hold",
                    "sdrex_filter_dec_curve",
                ],
            ),
            (
                "rift",
                [
                    "rift_filter_attack",
                    "rift_filter_atk_curve",
                    "rift_filter_hold",
                    "rift_filter_dec_curve",
                ],
            ),
            (
                "oneshot",
                [
                    "oneshot_filter_atk",
                    "oneshot_filter_atk_curve",
                    "oneshot_filter_hld",
                    "oneshot_filter_dec_curve",
                ],
            ),
        ] {
            let inst = INSTRUMENTS
                .iter()
                .find(|i| i.special_params.iter().any(|d| d.name.starts_with(prefix)))
                .unwrap_or_else(|| panic!("no {prefix} instrument"));
            for name in stages {
                let def = inst
                    .special_params
                    .iter()
                    .find(|d| d.name == name)
                    .unwrap_or_else(|| panic!("{} misses {name}", inst.label));
                assert_eq!(def.family, ParamFamily::Filter, "{name}");
            }
            // The hoist anchors.
            for field in [
                StandardField::FilterEnvAmount,
                StandardField::FilterEnvDecay,
            ] {
                assert!(
                    inst.standard_params.iter().any(|d| d.field == field),
                    "{} misses {field:?}",
                    inst.label
                );
            }
        }
    }

    /// [240]/[243] The One-Shot's pitch envelope renders in declaration order
    /// (it has no standard anchor rows to hoist under), so its table must
    /// DECLARE the stages in the canonical order: a time, then its curve.
    #[test]
    fn oneshot_pitch_envelope_declares_its_stages_in_canonical_order() {
        let inst = INSTRUMENTS
            .iter()
            .find(|i| i.name == "OneShot")
            .expect("One-Shot is registered");
        let pitch: Vec<&str> = inst
            .special_params
            .iter()
            .filter(|d| d.family == ParamFamily::Pitch && d.name.contains("_pitch_env"))
            .map(|d| d.name)
            .collect();
        assert_eq!(
            pitch,
            vec![
                "oneshot_pitch_env",
                "oneshot_pitch_env_atk",
                "oneshot_pitch_env_atk_curve",
                "oneshot_pitch_env_hld",
                "oneshot_pitch_env_decay",
                "oneshot_pitch_env_dec_curve",
            ]
        );
    }
}
