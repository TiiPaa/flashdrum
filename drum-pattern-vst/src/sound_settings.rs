use nih_plug::params::persist::PersistentField;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

/// Sound settings for a single instrument (used for clipboard).
#[derive(Debug, Clone, Default)]
pub struct SoundSettings {
    pub frequency: f32,
    pub decay: f32,
    pub volume: f32,
    pub filter_freq: f32,
    pub attack: f32,
    pub release: f32,
    pub decay_curve: f32,
    pub release_curve: f32,
    pub hold: f32,
    pub filter_env_amount: f32,
    pub filter_env_decay: f32,
    pub analog: f32,
    pub stereo: f32,
    pub special: [f32; SPECIAL_SLOT_COUNT],
    pub freq_mode: bool,
}

/// Number of per-slot special parameter values (matches `VoiceSettings::special`).
pub const SPECIAL_SLOT_COUNT: usize = 32;

/// Per-SLOT persistent sound settings. The amplitude envelope is bi-stage
/// (decay + release), each stage having an independent time and curve, plus
/// a hold phase between attack and decay (snare/HH use it for sustain).
/// Special params (click, saturation, ...) and the bass-drum Hz/Notes display
/// mode also live here so two slots of the same kind are fully independent.
pub struct InstrumentSettingsState {
    pub frequency: AtomicU32,
    pub decay: AtomicU32,
    pub volume: AtomicU32,
    pub filter_freq: AtomicU32,
    pub attack: AtomicU32,
    pub release: AtomicU32,
    pub decay_curve: AtomicU32,
    pub release_curve: AtomicU32,
    pub hold: AtomicU32,
    pub filter_env_amount: AtomicU32,
    pub filter_env_decay: AtomicU32,
    pub analog: AtomicU32,
    pub stereo: AtomicU32,
    /// Special parameter values indexed by the registry's `special_index`.
    pub special: [AtomicU32; SPECIAL_SLOT_COUNT],
    /// Bass-drum frequency display mode (0.0 = Hz, 1.0 = Notes).
    pub freq_mode: AtomicU32,
}

/// Number of f32 values serialized per instrument in the persisted state
/// (standard fields only — the v3 format appends specials + freq_mode).
pub const FIELDS_PER_INSTRUMENT: usize = crate::instrument_registry::SOUND_SETTINGS_FIELD_COUNT;
const LEGACY_FIELDS_PER_INSTRUMENT: usize = 12;
/// Frozen at 13: legacy persisted blobs predate the BD606 voice and their
/// length-keyed versioning must keep matching the old 13-voice layouts.
/// Do NOT replace with `DrumVoice::COUNT` (which grows with new voices).
const LEGACY_VOICE_COUNT: usize = 13;
/// v3 stride: standards + specials + freq_mode.
pub const FIELDS_PER_INSTRUMENT_V3: usize = FIELDS_PER_INSTRUMENT + SPECIAL_SLOT_COUNT + 1;

impl InstrumentSettingsState {
    pub fn new(
        frequency: f32,
        decay: f32,
        volume: f32,
        filter_freq: f32,
        attack: f32,
        release: f32,
        decay_curve: f32,
        release_curve: f32,
        hold: f32,
        filter_env_amount: f32,
        filter_env_decay: f32,
        analog: f32,
        stereo: f32,
    ) -> Self {
        Self {
            frequency: AtomicU32::new(frequency.to_bits()),
            decay: AtomicU32::new(decay.to_bits()),
            volume: AtomicU32::new(volume.to_bits()),
            filter_freq: AtomicU32::new(filter_freq.to_bits()),
            attack: AtomicU32::new(attack.to_bits()),
            release: AtomicU32::new(release.to_bits()),
            decay_curve: AtomicU32::new(decay_curve.to_bits()),
            release_curve: AtomicU32::new(release_curve.to_bits()),
            hold: AtomicU32::new(hold.to_bits()),
            filter_env_amount: AtomicU32::new(filter_env_amount.to_bits()),
            filter_env_decay: AtomicU32::new(filter_env_decay.to_bits()),
            analog: AtomicU32::new(analog.to_bits()),
            stereo: AtomicU32::new(stereo.to_bits()),
            special: std::array::from_fn(|_| AtomicU32::new(0.0f32.to_bits())),
            freq_mode: AtomicU32::new(0.0f32.to_bits()),
        }
    }

    pub fn special_value(&self, index: usize) -> f32 {
        if index >= SPECIAL_SLOT_COUNT {
            return 0.0;
        }
        f32::from_bits(self.special[index].load(Ordering::Relaxed))
    }

    pub fn set_special(&self, index: usize, value: f32) {
        if index < SPECIAL_SLOT_COUNT {
            self.special[index].store(value.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn load_specials(&self) -> [f32; SPECIAL_SLOT_COUNT] {
        std::array::from_fn(|i| f32::from_bits(self.special[i].load(Ordering::Relaxed)))
    }

    /// Read one parameter of this slot by its canonical id ([184]).
    ///
    /// `ParamId::Algo` is NOT owned here — the algo is a nih-plug `IntParam`, so
    /// the UI layer routes it. Reading it here returns 0.0 rather than lying.
    pub fn get(&self, id: crate::param_id::ParamId) -> f32 {
        use crate::param_id::ParamId;
        match id {
            ParamId::Std(field) => self.standard(field),
            ParamId::Special(index) => self.special_value(index),
            ParamId::FreqMode => {
                if self.freq_mode() {
                    1.0
                } else {
                    0.0
                }
            }
            ParamId::Algo => {
                debug_assert!(
                    false,
                    "the algo lives in a nih-plug param, not in the atomics"
                );
                0.0
            }
            // [286] The send levels live in the track layout (`GlobalSource` routes them).
            ParamId::FxSend(_) => 0.0,
        }
    }

    /// Write one parameter of this slot by its canonical id ([184]).
    /// Does **not** bump the version — the caller batches that once per frame.
    pub fn set(&self, id: crate::param_id::ParamId, value: f32) {
        use crate::param_id::ParamId;
        match id {
            ParamId::Std(field) => self.set_standard(field, value),
            ParamId::Special(index) => self.set_special(index, value),
            ParamId::FreqMode => self.set_freq_mode(value >= 0.5),
            ParamId::FxSend(_) => {}
            ParamId::Algo => {
                debug_assert!(
                    false,
                    "the algo lives in a nih-plug param, not in the atomics"
                )
            }
        }
    }

    /// One standard field, by name. The single mapping of `StandardField` onto
    /// the per-slot atomics — the counterpart of `set_standard`, and the reason
    /// the `load()` tuple order (Attack at 4) can no longer be confused with the
    /// p-lock index (Attack at 18).
    pub fn standard(&self, field: crate::instrument_registry::StandardField) -> f32 {
        use crate::instrument_registry::StandardField as F;
        let raw = match field {
            F::Freq => &self.frequency,
            F::Decay => &self.decay,
            F::Volume => &self.volume,
            F::FilterFreq => &self.filter_freq,
            F::Attack => &self.attack,
            F::Release => &self.release,
            F::DecayCurve => &self.decay_curve,
            F::ReleaseCurve => &self.release_curve,
            F::Hold => &self.hold,
            F::FilterEnvAmount => &self.filter_env_amount,
            F::FilterEnvDecay => &self.filter_env_decay,
            F::Analog => &self.analog,
            F::Stereo => &self.stereo,
        };
        f32::from_bits(raw.load(Ordering::Relaxed))
    }

    pub fn set_standard(&self, field: crate::instrument_registry::StandardField, value: f32) {
        use crate::instrument_registry::StandardField as F;
        let raw = match field {
            F::Freq => &self.frequency,
            F::Decay => &self.decay,
            F::Volume => &self.volume,
            F::FilterFreq => &self.filter_freq,
            F::Attack => &self.attack,
            F::Release => &self.release,
            F::DecayCurve => &self.decay_curve,
            F::ReleaseCurve => &self.release_curve,
            F::Hold => &self.hold,
            F::FilterEnvAmount => &self.filter_env_amount,
            F::FilterEnvDecay => &self.filter_env_decay,
            F::Analog => &self.analog,
            F::Stereo => &self.stereo,
        };
        raw.store(value.to_bits(), Ordering::Relaxed);
    }

    pub fn freq_mode(&self) -> bool {
        f32::from_bits(self.freq_mode.load(Ordering::Relaxed)) >= 0.5
    }

    pub fn set_freq_mode(&self, in_notes: bool) {
        self.freq_mode.store(
            if in_notes { 1.0f32 } else { 0.0f32 }.to_bits(),
            Ordering::Relaxed,
        );
    }

    /// Reset the special values to the registry defaults of the given voice.
    fn reset_specials_for_voice(&self, voice_idx: usize) {
        for slot in self.special.iter() {
            slot.store(0.0f32.to_bits(), Ordering::Relaxed);
        }
        for def in crate::instrument_registry::special_params(voice_idx) {
            self.set_special(def.special_index, def.default);
        }
        if crate::instrument_registry::is_sampler(voice_idx) {
            // Multisample pitch is stored as relative semitones. Earlier
            // sampler builds used Hz and persist with this marker at zero.
            self.set_special(10, 1.0);
        }
        self.set_freq_mode(false);
    }

    pub fn load(
        &self,
    ) -> (
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
        f32,
    ) {
        (
            f32::from_bits(self.frequency.load(Ordering::Relaxed)),
            f32::from_bits(self.decay.load(Ordering::Relaxed)),
            f32::from_bits(self.volume.load(Ordering::Relaxed)),
            f32::from_bits(self.filter_freq.load(Ordering::Relaxed)),
            f32::from_bits(self.attack.load(Ordering::Relaxed)),
            f32::from_bits(self.release.load(Ordering::Relaxed)),
            f32::from_bits(self.decay_curve.load(Ordering::Relaxed)),
            f32::from_bits(self.release_curve.load(Ordering::Relaxed)),
            f32::from_bits(self.hold.load(Ordering::Relaxed)),
            f32::from_bits(self.filter_env_amount.load(Ordering::Relaxed)),
            f32::from_bits(self.filter_env_decay.load(Ordering::Relaxed)),
            f32::from_bits(self.analog.load(Ordering::Relaxed)),
            f32::from_bits(self.stereo.load(Ordering::Relaxed)),
        )
    }

    pub fn store(
        &self,
        frequency: f32,
        decay: f32,
        volume: f32,
        filter_freq: f32,
        attack: f32,
        release: f32,
        decay_curve: f32,
        release_curve: f32,
        hold: f32,
        filter_env_amount: f32,
        filter_env_decay: f32,
        analog: f32,
        stereo: f32,
    ) {
        self.frequency.store(frequency.to_bits(), Ordering::Relaxed);
        self.decay.store(decay.to_bits(), Ordering::Relaxed);
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
        self.filter_freq
            .store(filter_freq.to_bits(), Ordering::Relaxed);
        self.attack.store(attack.to_bits(), Ordering::Relaxed);
        self.release.store(release.to_bits(), Ordering::Relaxed);
        self.decay_curve
            .store(decay_curve.to_bits(), Ordering::Relaxed);
        self.release_curve
            .store(release_curve.to_bits(), Ordering::Relaxed);
        self.hold.store(hold.to_bits(), Ordering::Relaxed);
        self.filter_env_amount
            .store(filter_env_amount.to_bits(), Ordering::Relaxed);
        self.filter_env_decay
            .store(filter_env_decay.to_bits(), Ordering::Relaxed);
        self.analog.store(analog.to_bits(), Ordering::Relaxed);
        self.stereo.store(stereo.to_bits(), Ordering::Relaxed);
    }
}

const MAX_TRACKS: usize = crate::track::MAX_TRACKS;

pub struct SoundSettingsState {
    pub instruments: [InstrumentSettingsState; MAX_TRACKS],
    pub version: AtomicU64,
    /// True when the persisted state predates per-slot specials: the plugin
    /// must seed `special`/`freq_mode` once from the legacy per-voice params.
    pub needs_param_seed: AtomicBool,
    /// [286] What each effect bus is really using right now (the sliders with the
    /// p-locks applied), written by the audio thread, read by the Lane Editor to
    /// show it. Not persisted. Order: size, decay, damping, pre-delay, low cut, width,
    /// level; `fx_live_on` says whether the values are fresh (the transport plays).
    pub fx_live: [[AtomicU32; FX_LIVE_FIELDS]; crate::track::FX_SLOT_COUNT],
    pub fx_live_on: [AtomicBool; crate::track::FX_SLOT_COUNT],
    /// [303] Each lane's compressor-insert gain reduction (dB, f32 bits): the largest since
    /// the editor last read it (the audio thread keeps the max, the editor takes it).
    pub insert_gr: [AtomicU32; crate::track::MAX_TRACKS],
}

pub use crate::synthesis::FX_LIVE_FIELDS;

impl SoundSettingsState {
    /// [303] Audio thread: add a block's gain reduction (positive floats order like their
    /// bits, so `fetch_max` on the bits keeps the largest).
    pub fn publish_insert_gr(&self, slot: usize, gr_db: f32) {
        if let Some(cell) = self.insert_gr.get(slot) {
            cell.fetch_max(gr_db.max(0.0).to_bits(), Ordering::Relaxed);
        }
    }

    /// [303] Editor: the largest gain reduction since the last read, and reset it.
    pub fn take_insert_gr(&self, slot: usize) -> f32 {
        self.insert_gr.get(slot).map_or(0.0, |cell| f32::from_bits(cell.swap(0, Ordering::Relaxed)))
    }

    /// [286] Publish what effect bus `bus` is using (audio thread; atomic stores).
    pub fn publish_fx_live(&self, bus: usize, values: [f32; FX_LIVE_FIELDS], playing: bool) {
        if bus >= crate::track::FX_SLOT_COUNT {
            return;
        }
        for (cell, value) in self.fx_live[bus].iter().zip(values) {
            cell.store(value.to_bits(), Ordering::Relaxed);
        }
        self.fx_live_on[bus].store(playing, Ordering::Relaxed);
    }

    /// [286] The live values of effect bus `bus`, `None` while the transport is stopped.
    pub fn fx_live(&self, bus: usize) -> Option<[f32; FX_LIVE_FIELDS]> {
        if bus >= crate::track::FX_SLOT_COUNT || !self.fx_live_on[bus].load(Ordering::Relaxed) {
            return None;
        }
        Some(std::array::from_fn(|i| {
            f32::from_bits(self.fx_live[bus][i].load(Ordering::Relaxed))
        }))
    }


    /// Create settings using the registry defaults for every slot.
    pub fn new(layout: &crate::track::TrackLayoutState) -> Arc<Self> {
        Self::new_with_default_analog(layout, 0.5)
    }

    /// Create settings, overriding the registry's analog default with the user's
    /// global preference.
    pub fn new_with_default_analog(
        layout: &crate::track::TrackLayoutState,
        default_analog: f32,
    ) -> Arc<Self> {
        let voice_for_slot = |i: usize| -> usize {
            if i < MAX_TRACKS && layout.slots[i].active {
                layout.slots[i].kind.drum_voice_index()
            } else if i < LEGACY_VOICE_COUNT {
                i
            } else {
                0
            }
        };

        let instruments: [InstrumentSettingsState; MAX_TRACKS] = std::array::from_fn(|i| {
            let [f, d, v, fl, at, r, dc, rc, h, fea, fed, _a, s] =
                crate::instrument_registry::INSTRUMENTS[voice_for_slot(i)].sound_settings_default;
            InstrumentSettingsState::new(f, d, v, fl, at, r, dc, rc, h, fea, fed, default_analog, s)
        });
        for (i, inst) in instruments.iter().enumerate() {
            inst.reset_specials_for_voice(voice_for_slot(i));
        }

        Arc::new(Self {
            instruments,
            version: AtomicU64::new(0),
            needs_param_seed: AtomicBool::new(false),
            fx_live: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            fx_live_on: std::array::from_fn(|_| AtomicBool::new(false)),
            insert_gr: std::array::from_fn(|_| AtomicU32::new(0)),
        })
    }

    pub fn reset_slot_to_defaults(
        &self,
        slot: usize,
        kind: crate::track::TrackInstrumentKind,
        default_analog: f32,
    ) {
        if slot >= MAX_TRACKS {
            return;
        }
        let [f, d, v, fl, at, r, dc, rc, h, fea, fed, _a, s] =
            kind.instrument_def().sound_settings_default;
        self.instruments[slot].store(f, d, v, fl, at, r, dc, rc, h, fea, fed, default_analog, s);
        self.instruments[slot].reset_specials_for_voice(kind.drum_voice_index());
        self.bump_version();
    }

    pub fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    /// Get the sound settings for a specific slot.
    pub fn get_settings_for_slot(&self, slot: usize) -> SoundSettings {
        if slot >= MAX_TRACKS {
            return SoundSettings::default();
        }

        let inst = &self.instruments[slot];
        let (f, d, v, fl, at, r, dc, rc, h, fea, fed, a, s) = inst.load();
        let specials = inst.load_specials();
        let freq_mode = inst.freq_mode();

        SoundSettings {
            frequency: f,
            decay: d,
            volume: v,
            filter_freq: fl,
            attack: at,
            release: r,
            decay_curve: dc,
            release_curve: rc,
            hold: h,
            filter_env_amount: fea,
            filter_env_decay: fed,
            analog: a,
            stereo: s,
            special: specials,
            freq_mode,
        }
    }

    /// Set the sound settings for a specific slot.
    pub fn set_settings_for_slot(&self, slot: usize, settings: &SoundSettings) {
        if slot >= MAX_TRACKS {
            return;
        }

        let inst = &self.instruments[slot];
        inst.store(
            settings.frequency,
            settings.decay,
            settings.volume,
            settings.filter_freq,
            settings.attack,
            settings.release,
            settings.decay_curve,
            settings.release_curve,
            settings.hold,
            settings.filter_env_amount,
            settings.filter_env_decay,
            settings.analog,
            settings.stereo,
        );
        for (i, &value) in settings.special.iter().enumerate() {
            inst.set_special(i, value);
        }
        inst.set_freq_mode(settings.freq_mode);
        self.bump_version();
    }

    pub fn read_all(&self) -> Vec<f32> {
        let mut result = Vec::with_capacity(self.instruments.len() * FIELDS_PER_INSTRUMENT_V3);
        for inst in self.instruments.iter() {
            let (f, d, v, fl, at, r, dc, rc, h, fea, fed, a, s) = inst.load();
            result.extend_from_slice(&[f, d, v, fl, at, r, dc, rc, h, fea, fed, a, s]);
            result.extend_from_slice(&inst.load_specials());
            result.push(if inst.freq_mode() { 1.0 } else { 0.0 });
        }
        result
    }

    pub fn write_all(&self, values: &[f32]) {
        // Current v3 format: standards + specials + freq_mode, per slot.
        let v3_len = MAX_TRACKS * FIELDS_PER_INSTRUMENT_V3;
        if values.len() == v3_len {
            for (i, inst) in self.instruments.iter().enumerate() {
                let base = i * FIELDS_PER_INSTRUMENT_V3;
                let v = &values[base..base + FIELDS_PER_INSTRUMENT];
                inst.store(
                    v[0], v[1], v[2], v[3], v[4], v[5], v[6], v[7], v[8], v[9], v[10], v[11], v[12],
                );
                for k in 0..SPECIAL_SLOT_COUNT {
                    inst.set_special(k, values[base + FIELDS_PER_INSTRUMENT + k]);
                }
                inst.set_freq_mode(
                    values[base + FIELDS_PER_INSTRUMENT + SPECIAL_SLOT_COUNT] >= 0.5,
                );
            }
            self.needs_param_seed.store(false, Ordering::Release);
            self.bump_version();
            return;
        }

        // Legacy formats carry standards only: the specials must be seeded once
        // from the legacy per-voice nih-plug params after the state restore.
        self.needs_param_seed.store(true, Ordering::Release);

        let legacy_12_len = LEGACY_VOICE_COUNT * LEGACY_FIELDS_PER_INSTRUMENT;
        let legacy_13_len = LEGACY_VOICE_COUNT * FIELDS_PER_INSTRUMENT;
        let current_len = MAX_TRACKS * FIELDS_PER_INSTRUMENT;

        let (stride, source_count) = if values.len() == legacy_12_len {
            (LEGACY_FIELDS_PER_INSTRUMENT, LEGACY_VOICE_COUNT)
        } else if values.len() == legacy_13_len {
            (FIELDS_PER_INSTRUMENT, LEGACY_VOICE_COUNT)
        } else if values.len() == current_len {
            (FIELDS_PER_INSTRUMENT, MAX_TRACKS)
        } else {
            // Unknown size: load as many full slots as possible, leave the rest as defaults.
            (
                FIELDS_PER_INSTRUMENT,
                (values.len() / FIELDS_PER_INSTRUMENT).min(MAX_TRACKS),
            )
        };

        for (i, inst) in self.instruments.iter().enumerate() {
            let legacy_defaults = if i < LEGACY_VOICE_COUNT {
                crate::instrument_registry::INSTRUMENTS[i].sound_settings_default
            } else {
                crate::instrument_registry::INSTRUMENTS[0].sound_settings_default
            };

            if i >= source_count {
                inst.store(
                    legacy_defaults[0],
                    legacy_defaults[1],
                    legacy_defaults[2],
                    legacy_defaults[3],
                    legacy_defaults[4],
                    legacy_defaults[5],
                    legacy_defaults[6],
                    legacy_defaults[7],
                    legacy_defaults[8],
                    legacy_defaults[9],
                    legacy_defaults[10],
                    legacy_defaults[11],
                    legacy_defaults[12],
                );
                continue;
            }

            let base = i * stride;
            let value_or_default = |offset: usize| {
                values
                    .get(base + offset)
                    .copied()
                    .unwrap_or(legacy_defaults[offset])
            };
            inst.store(
                value_or_default(0),
                value_or_default(1),
                value_or_default(2),
                value_or_default(3),
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    legacy_defaults[4]
                } else {
                    value_or_default(4)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(4)
                } else {
                    value_or_default(5)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(5)
                } else {
                    value_or_default(6)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(6)
                } else {
                    value_or_default(7)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(7)
                } else {
                    value_or_default(8)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(8)
                } else {
                    value_or_default(9)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(9)
                } else {
                    value_or_default(10)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(10)
                } else {
                    value_or_default(11)
                },
                if stride == LEGACY_FIELDS_PER_INSTRUMENT {
                    value_or_default(11)
                } else {
                    value_or_default(12)
                },
            );
        }
        self.bump_version();
    }
}

#[derive(Clone)]
pub struct PersistentSoundSettings {
    pub state: Arc<SoundSettingsState>,
}

impl PersistentSoundSettings {
    pub fn new(layout: &crate::track::TrackLayoutState) -> Self {
        let config = crate::config::GlobalConfig::load();
        Self {
            state: SoundSettingsState::new_with_default_analog(layout, config.default_analog),
        }
    }
}

impl<'a> PersistentField<'a, Vec<f32>> for PersistentSoundSettings {
    fn set(&self, new_value: Vec<f32>) {
        self.state.write_all(&new_value);
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Vec<f32>) -> R,
    {
        let values = self.state.read_all();
        f(&values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{TrackInstrumentKind, TrackLayoutState, TrackSlot};

    #[test]
    fn v3_roundtrip_preserves_specials_and_freq_mode() {
        let layout = TrackLayoutState::default_layout();
        let state = SoundSettingsState::new(&layout);
        state.instruments[0].set_special(6, 2.0);
        state.instruments[0].set_freq_mode(true);
        let blob = state.read_all();
        assert_eq!(blob.len(), MAX_TRACKS * FIELDS_PER_INSTRUMENT_V3);

        let restored = SoundSettingsState::new(&layout);
        restored.write_all(&blob);
        assert_eq!(restored.instruments[0].special_value(6), 2.0);
        assert!(restored.instruments[0].freq_mode());
        assert!(!restored.needs_param_seed.load(Ordering::Relaxed));
    }

    #[test]
    fn legacy_blob_restores_standards_and_requests_param_seed() {
        let layout = TrackLayoutState::from_legacy_13();
        let state = SoundSettingsState::new(&layout);
        // Legacy 14x13 format: standards only, no specials.
        let legacy = vec![0.5f32; MAX_TRACKS * FIELDS_PER_INSTRUMENT];
        state.write_all(&legacy);
        let (freq, ..) = state.instruments[3].load();
        assert_eq!(freq, 0.5);
        assert!(state.needs_param_seed.load(Ordering::Relaxed));
    }

    /// [201b] The Lane Editor's Store/Restore relies on this roundtrip:
    /// get_settings_for_slot → tweak → set_settings_for_slot must restore
    /// standards AND specials exactly.
    #[test]
    fn slot_settings_roundtrip_restores_standards_and_specials() {
        let layout = TrackLayoutState::default_layout();
        let state = SoundSettingsState::new(&layout);
        let snap = state.get_settings_for_slot(0);
        // Tweak a standard and a special.
        state.instruments[0]
            .frequency
            .store(4321.5f32.to_bits(), Ordering::Relaxed);
        state.instruments[0].set_special(3, 7.25);
        state.set_settings_for_slot(0, &snap);
        let back = state.get_settings_for_slot(0);
        assert_eq!(back.frequency, snap.frequency);
        assert_eq!(back.special[3], snap.special[3]);
        assert_eq!(back.decay, snap.decay);
    }

    #[test]
    fn reset_slot_to_defaults_applies_kind_specials() {
        let layout = TrackLayoutState::default_layout();
        let state = SoundSettingsState::new(&layout);
        state.instruments[5].set_special(0, 99.0);
        state.reset_slot_to_defaults(5, TrackInstrumentKind::Kick, 0.5);
        for def in crate::instrument_registry::special_params(0) {
            assert_eq!(
                state.instruments[5].special_value(def.special_index),
                def.default
            );
        }
        assert!(!state.instruments[5].freq_mode());
    }

    /// Regression for [105]: every instrument kind must initialise its global
    /// Frequency to a strictly positive value so the plock sound menu never
    /// defaults to 0 Hz. Voices whose Freq slider is a RELATIVE pitch
    /// (semitones, range spanning 0) are exempt: 0 is their neutral value.
    #[test]
    fn default_frequency_is_nonzero_for_every_instrument_kind() {
        use crate::instrument_registry::{ParamWidget, StandardField, INSTRUMENTS};

        for kind_idx in 0..TrackInstrumentKind::COUNT {
            let kind = TrackInstrumentKind::from_index(kind_idx).unwrap();
            let voice_idx = kind.drum_voice_index();
            let expected = INSTRUMENTS[voice_idx].sound_settings_default[0];
            let relative_pitch = INSTRUMENTS[voice_idx].standard_params.iter().any(|d| {
                d.field == StandardField::Freq
                    && matches!(d.widget, ParamWidget::Slider { min, .. } if min < 0.0)
            });
            if !relative_pitch {
                assert!(
                    expected > 0.0,
                    "{:?} default frequency must be > 0, got {}",
                    kind,
                    expected
                );
            }

            let mut layout = TrackLayoutState::default_layout();
            layout.slots[0] = TrackSlot::active_with_kind(kind);
            let state = SoundSettingsState::new(&layout);
            let (freq, ..) = state.instruments[0].load();
            if !relative_pitch {
                assert!(
                    freq > 0.0,
                    "{:?} slot frequency must be > 0, got {}",
                    kind,
                    freq
                );
            }
            assert!(
                (freq - expected).abs() < 1e-6,
                "{:?} slot frequency {} does not match registry default {}",
                kind,
                freq,
                expected
            );
        }
    }

    #[test]
    fn default_analog_is_50_percent_for_every_instrument_kind() {
        for kind_idx in 0..TrackInstrumentKind::COUNT {
            let kind = TrackInstrumentKind::from_index(kind_idx).unwrap();
            let mut layout = TrackLayoutState::default_layout();
            layout.slots[0] = TrackSlot::active_with_kind(kind);
            let state = SoundSettingsState::new(&layout);
            let (_, _, _, _, _, _, _, _, _, _, _, analog, _) = state.instruments[0].load();
            assert!(
                (analog - 0.5).abs() < 1e-6,
                "{:?} default analog should be 0.5, got {}",
                kind,
                analog
            );
        }
    }

    /// Two slots of the same kind must both keep the kind's default Frequency,
    /// not collapse to 0 or share a single value.
    #[test]
    fn duplicate_slots_keep_nonzero_default_frequency() {
        let mut layout = TrackLayoutState::modular_default_layout();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::BassDrum808);
        layout.slots[1] = TrackSlot::active_with_kind(TrackInstrumentKind::BassDrum808);

        let state = SoundSettingsState::new(&layout);
        let (freq0, ..) = state.instruments[0].load();
        let (freq1, ..) = state.instruments[1].load();

        assert!(
            freq0 > 0.0 && freq1 > 0.0,
            "duplicate B8 slots must both have positive frequency (got {} and {})",
            freq0,
            freq1
        );
        assert!(
            (freq0 - freq1).abs() < 1e-6,
            "duplicate B8 slots must share the same default frequency ({} vs {})",
            freq0,
            freq1
        );
    }

    #[test]
    fn multisample_defaults_mark_relative_pitch_format() {
        let mut layout = TrackLayoutState::modular_default_layout();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::Bd6smp);
        layout.slots[1] = TrackSlot::active_with_kind(TrackInstrumentKind::Sd6smp);
        let state = SoundSettingsState::new(&layout);
        assert_eq!(state.instruments[0].special_value(10), 1.0);
        assert_eq!(state.instruments[1].special_value(10), 1.0);
    }
}
