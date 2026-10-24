//! Audio synthesis module for drum sounds

mod ac606;
mod ac_voice;
mod bd606;
mod buzz;
mod ch606;
mod clap;
mod cymbal;
mod delay;
mod dsp;
mod hihat;
pub mod inserts;
mod kick;
mod kick_808;
mod oneshot;
mod open_hihat;
mod perc1;
#[cfg(test)]
mod retrig_tests;
mod reverb;
mod rumble;
mod ride;
mod rift;
pub mod sample_bank;
mod saturation;
mod sd606;
mod sdrex;
mod settings;
mod snare;
mod snare606;
mod special_params;
mod tom;

// `algos_for` is consumed by the editor (`ui.rs`) but not by the standalone
// binary — allow the unused-import warning so both build configurations stay
// clean.
#[allow(unused_imports)]
pub use special_params::{algos_for, AlgoDef};

pub use bd606::Bd606Voice;
pub use buzz::BuzzVoice;
pub use ch606::Ch606Voice;
pub use clap::ClapVoice;
pub use cymbal::CymbalVoice;
pub use hihat::HiHatVoice;
pub use kick::KickVoice;
pub use kick_808::Kick808Voice;
pub use oneshot::OneShotVoice;
#[allow(unused_imports)]
pub use delay::sp as delay_sp;
#[allow(unused_imports)]
pub use delay::synced_time_ms as delay_synced_time_ms;
pub use delay::{DelayCore, DelayParams, DelayVoice};
#[allow(unused_imports)]
pub use dsp::{decay_curve_amount, shape_decay_curve};
#[allow(unused_imports)]
pub use delay::SYNC_BEATS as NOTE_SYNC_BEATS;
#[allow(unused_imports)]
pub use reverb::sp as reverb_sp;
pub use reverb::{ReverbCore, ReverbParams, ReverbVoice};
#[allow(unused_imports)]
pub use rumble::sp as rumble_sp;
#[allow(unused_imports)]
pub use rumble::{duck_depth, duck_exponent, duck_ratio};

/// `Duck Trigger` value that listens to the kick instead of the transport.
#[allow(dead_code)]
pub const fn rumble_duck_source_kick() -> u8 {
    rumble::DUCK_SOURCE_KICK
}
pub use rumble::{RumbleCore, RumbleParams, RumbleVoice};
pub use open_hihat::OpenHiHatVoice;
pub use perc1::Perc1Voice;
pub use ride::RideVoice;
#[allow(unused_imports)] // le binaire de test headless n'a pas d'UI
pub use rift::grain_seconds as rift_grain_seconds;
pub use sdrex::SdrexVoice;
// Read by the Sound Panel's pitch-envelope graph; the standalone harness
// has no UI, hence the allow.
pub use ac_voice::{AcEngineKind, AcVoice};
pub use rift::RiftVoice;
#[allow(unused_imports)]
pub use rift::PITCH_ENV_CURVE as RIFT_PITCH_ENV_CURVE;
pub use sd606::Sd606Voice;
pub use settings::ac_voice::AcVoiceSettings;
pub use settings::bd606::Bd606Settings;
pub use settings::buzz::BuzzSettings;
pub use settings::ch606::Ch606Settings;
pub use settings::clap::ClapSettings;
pub use settings::cymbal::CymbalSettings;
pub use settings::hihat::HiHatSettings;
pub use settings::kick::KickSettings;
pub use settings::kick_808::Kick808Settings;
pub use settings::oneshot::OneShotSettings;
pub use settings::open_hihat::OpenHiHatSettings;
pub use settings::perc1::Perc1Settings;
pub use settings::ride::RideSettings;
pub use settings::rift::RiftSettings;
pub use settings::sd606::Sd606Settings;
pub use settings::sdrex::SdrexSettings;
pub use settings::snare::SnareSettings;
pub use settings::snare606::Snare606Settings;
pub use settings::tom::TomSettings;
pub use snare::SnareVoice;
pub use snare606::Snare606Voice;
pub use tom::TomVoice;

/// Drum voice types matching the original web app
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrumVoice {
    Kick = 0,
    Snare = 1,
    HiHat = 2,
    OpenHiHat = 3,
    Tom1 = 4,
    Tom2 = 5,
    Tom3 = 6,
    Clap = 7,
    Ride = 8,
    Cymbal = 9,
    Snare606 = 10,
    BassDrum808 = 11,
    Perc1 = 12,
    Bd606 = 13,
    Sd606 = 14,
    Ch606 = 15,
    Buzz = 16,
    Sdrex = 17,
    Bd6Ac = 18,
    Sd6Ac = 19,
    Hh6Ac = 20,
    Oh6Ac = 21,
    Cl6Ac = 22,
    Tm6Ac = 23,
    Oh606 = 24,
    /// [221] Rift - a slice lifted out of a long texture.
    Rift = 25,
    /// [243] One-Shot - the lane's own sample file, played start to finish.
    OneShot = 26,
    /// [286] Reverb - the send effect of an effect-bus slot (not a sound source).
    Reverb = 27,
    /// [286] Delay - the second send effect.
    Delay = 28,
    /// [286] Rumble - the third send effect: the kick rumble.
    Rumble = 29,
}

impl DrumVoice {
    pub const COUNT: usize = 30;

    /// [286] An effect (Reverb, Delay), not a sound source: no steps, no note.
    pub const fn is_effect_index(index: usize) -> bool {
        index == Self::Reverb as usize
            || index == Self::Delay as usize
            || index == Self::Rumble as usize
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Kick),
            1 => Some(Self::Snare),
            2 => Some(Self::HiHat),
            3 => Some(Self::OpenHiHat),
            4 => Some(Self::Tom1),
            5 => Some(Self::Tom2),
            6 => Some(Self::Tom3),
            7 => Some(Self::Clap),
            8 => Some(Self::Ride),
            9 => Some(Self::Cymbal),
            10 => Some(Self::Snare606),
            11 => Some(Self::BassDrum808),
            12 => Some(Self::Perc1),
            13 => Some(Self::Bd606),
            14 => Some(Self::Sd606),
            15 => Some(Self::Ch606),
            16 => Some(Self::Buzz),
            17 => Some(Self::Sdrex),
            18 => Some(Self::Bd6Ac),
            19 => Some(Self::Sd6Ac),
            20 => Some(Self::Hh6Ac),
            21 => Some(Self::Oh6Ac),
            22 => Some(Self::Cl6Ac),
            23 => Some(Self::Tm6Ac),
            24 => Some(Self::Oh606),
            25 => Some(Self::Rift),
            26 => Some(Self::OneShot),
            27 => Some(Self::Reverb),
            28 => Some(Self::Delay),
            29 => Some(Self::Rumble),
            _ => None,
        }
    }

    /// Steepness of the filter envelope decay stage for voices that use a fixed
    /// filter curve. `None` means the curve is dynamic (`decay_curve`).
    pub fn filter_env_curve(self) -> Option<f32> {
        match self {
            DrumVoice::Kick => Some(KickVoice::FILTER_ENV_CURVE),
            DrumVoice::Snare => Some(SnareVoice::FILTER_ENV_CURVE),
            DrumVoice::HiHat => Some(HiHatVoice::FILTER_ENV_CURVE),
            DrumVoice::Tom1 | DrumVoice::Tom2 | DrumVoice::Tom3 => Some(TomVoice::FILTER_ENV_CURVE),
            DrumVoice::Snare606 => Some(Snare606Voice::FILTER_ENV_CURVE),
            // [174/F2] Fixed too — their DSP no longer reads the (now bipolar)
            // amp decay_curve for the filter envelope.
            DrumVoice::Perc1 => Some(Perc1Voice::FILTER_ENV_CURVE),
            DrumVoice::Bd606 => Some(Bd606Voice::FILTER_ENV_CURVE),
            DrumVoice::Sd606 => Some(Sd606Voice::FILTER_ENV_CURVE),
            DrumVoice::Ch606 => Some(Ch606Voice::FILTER_ENV_CURVE),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct VoiceSettings {
    pub frequency: f32,
    pub decay: f32,
    pub volume: f32,
    pub filter_freq: f32,
    /// Attack ramp length in seconds for the amplitude envelope. This is both
    /// the audible attack shape and the anti-click retrigger ramp.
    pub attack: f32,
    /// Slow release tail length in seconds. The amplitude envelope is bi-stage:
    /// `amp = max(decay_env, release_env)`. The decay_env starts at 1.0 and drops
    /// fast with `decay`; the release_env starts at a fixed shelf (~30 % of peak)
    /// and decays slowly with `release`, taking over once the decay phase falls
    /// below the shelf. Set `release` to 0 to get a single-stage decay.
    pub release: f32,
    /// Steepness of the decay stage (typical range 2..10). Low values give a
    /// near-linear early fall, high values a steep punchy drop.
    pub decay_curve: f32,
    /// Steepness of the release stage (typical range 2..10). Lower values give
    /// a long flat tail, higher values a quick falloff.
    pub release_curve: f32,
    /// Hold time in seconds. After the attack ramp the envelope stays at its
    /// peak for `hold` seconds before the decay starts. Used by snare and
    /// hi-hat voices to add a short sustain phase. 0 = no hold.
    pub hold: f32,
    /// Filter envelope amount (0..1). Scales the per-voice maximum depth so
    /// that 0 disables the filter envelope and 1 gives the full effect.
    pub filter_env_amount: f32,
    /// Filter envelope decay time in seconds (typically 1 ms .. 200 ms).
    pub filter_env_decay: f32,
    /// Analog drift amount (0..1). 1.0 = full analog behavior (phase continuous,
    /// pitch envelope persistent, filter state retained). 0.0 = digital stable
    /// (phase reset, pitch envelope reset, filter state reset on trigger).
    pub analog: f32,
    /// Stereo width (0..1). 1.0 = stereo (independent L/R noise generators).
    /// 0.0 = mono. Only affects noise-based voices.
    pub stereo: f32,
    /// Synthesis algorithm index (interpreted per instrument).
    pub algo: u8,
    /// Special parameters, indexed per instrument convention.
    pub special: [f32; 32],
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            frequency: 60.0,
            decay: 0.5,
            volume: 0.8,
            filter_freq: 100.0,
            attack: 0.0015,
            release: 0.0,
            decay_curve: 5.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special: [0.0; 32],
        }
    }
}

impl VoiceSettings {
    /// Read one parameter by its canonical id ([184]).
    ///
    /// Audio-thread safe: a plain match, no allocation. Replaces the
    /// hand-written `read_morph_value` index list that used to live in `lib.rs`.
    #[inline]
    pub fn get(&self, id: crate::param_id::ParamId) -> f32 {
        use crate::instrument_registry::StandardField as F;
        use crate::param_id::ParamId;
        match id {
            ParamId::Std(F::Freq) => self.frequency,
            ParamId::Std(F::Decay) => self.decay,
            ParamId::Std(F::Volume) => self.volume,
            ParamId::Std(F::FilterFreq) => self.filter_freq,
            ParamId::Std(F::Attack) => self.attack,
            ParamId::Std(F::Release) => self.release,
            ParamId::Std(F::DecayCurve) => self.decay_curve,
            ParamId::Std(F::ReleaseCurve) => self.release_curve,
            ParamId::Std(F::Hold) => self.hold,
            ParamId::Std(F::FilterEnvAmount) => self.filter_env_amount,
            ParamId::Std(F::FilterEnvDecay) => self.filter_env_decay,
            ParamId::Std(F::Analog) => self.analog,
            ParamId::Std(F::Stereo) => self.stereo,
            ParamId::Algo => self.algo as f32,
            ParamId::Special(index) => self.special.get(index).copied().unwrap_or(0.0),
            // A display mode has no place in the synthesis settings, and a send
            // level belongs to the track layout ([286]).
            ParamId::FreqMode | ParamId::FxSend(_) => 0.0,
        }
    }

    /// Write one parameter by its canonical id ([184]). Audio-thread safe.
    #[inline]
    pub fn set(&mut self, id: crate::param_id::ParamId, value: f32) {
        use crate::instrument_registry::StandardField as F;
        use crate::param_id::ParamId;
        match id {
            ParamId::Std(F::Freq) => self.frequency = value,
            ParamId::Std(F::Decay) => self.decay = value,
            ParamId::Std(F::Volume) => self.volume = value,
            ParamId::Std(F::FilterFreq) => self.filter_freq = value,
            ParamId::Std(F::Attack) => self.attack = value,
            ParamId::Std(F::Release) => self.release = value,
            ParamId::Std(F::DecayCurve) => self.decay_curve = value,
            ParamId::Std(F::ReleaseCurve) => self.release_curve = value,
            ParamId::Std(F::Hold) => self.hold = value,
            ParamId::Std(F::FilterEnvAmount) => self.filter_env_amount = value,
            ParamId::Std(F::FilterEnvDecay) => self.filter_env_decay = value,
            ParamId::Std(F::Analog) => self.analog = value,
            ParamId::Std(F::Stereo) => self.stereo = value,
            ParamId::Algo => self.algo = value.round().clamp(0.0, 255.0) as u8,
            ParamId::Special(index) => {
                if index < self.special.len() {
                    self.special[index] = value;
                }
            }
            ParamId::FreqMode | ParamId::FxSend(_) => {}
        }
    }

    pub fn kick() -> Self {
        Self {
            frequency: 60.0,
            decay: 0.5,
            volume: 1.0,
            filter_freq: 30.0,
            attack: 0.0015,
            release: 0.5,
            decay_curve: 5.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special: [
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn snare() -> Self {
        Self {
            frequency: 200.0,
            decay: 0.47,
            volume: 0.6,
            filter_freq: 200.0,
            attack: 0.0003,
            release: 0.2,
            decay_curve: 5.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.03,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            special: [
                0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn hihat() -> Self {
        Self {
            frequency: 8000.0,
            decay: 0.36,
            volume: 0.2,
            filter_freq: 5000.0,
            attack: 0.0003,
            release: 0.0,
            decay_curve: 8.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.04,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            special: [
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn open_hihat() -> Self {
        Self {
            frequency: 6000.0,
            decay: 0.66,
            volume: 0.3,
            filter_freq: 8000.0,
            attack: 0.0003,
            release: 0.4,
            decay_curve: 5.5,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special: [
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn tom1() -> Self {
        Self {
            frequency: 196.0,
            decay: 0.35,
            volume: 0.7,
            filter_freq: 600.0,
            attack: 0.0015,
            release: 0.25,
            decay_curve: 4.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.06,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special: [
                0.3, 0.0, 0.0, 0.0, 1.0, 0.0, 0.7071, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn tom2() -> Self {
        Self {
            frequency: 150.0,
            decay: 0.3,
            volume: 0.7,
            filter_freq: 650.0,
            attack: 0.0015,
            release: 0.2,
            decay_curve: 4.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.06,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special: [
                0.3, 0.0, 0.0, 0.0, 1.0, 0.0, 0.7071, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn tom3() -> Self {
        Self {
            frequency: 100.0,
            decay: 0.45,
            volume: 0.7,
            filter_freq: 500.0,
            attack: 0.0015,
            release: 0.35,
            decay_curve: 4.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.06,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special: [
                0.3, 0.0, 0.0, 0.0, 1.0, 0.0, 0.7071, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn clap() -> Self {
        Self {
            frequency: 1200.0,
            decay: 0.03,
            volume: 1.0,
            filter_freq: 1000.0,
            attack: 0.0015,
            release: 0.12,
            decay_curve: 6.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            special: [
                0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn ride() -> Self {
        Self {
            frequency: 8000.0,
            decay: 1.2,
            volume: 0.35,
            filter_freq: 10000.0,
            attack: 0.002,
            release: 1.5,
            decay_curve: 3.5,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            special: [
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn cymbal() -> Self {
        Self {
            frequency: 6000.0,
            decay: 2.0,
            volume: 0.4,
            filter_freq: 8000.0,
            attack: 0.002,
            release: 2.5,
            decay_curve: 2.8,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            special: [
                15.0, 0.0, 0.15, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn snare606() -> Self {
        Self {
            frequency: 220.0,
            decay: 0.08,
            volume: 0.7,
            filter_freq: 3000.0,
            attack: 0.0003,
            release: 0.15,
            decay_curve: 5.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            // special[0] = Resonance (Q)
            // special[1] = Tone (body vs wires balance)
            // special[2] = Snap (crispness of wires layer)
            special: [
                4.5, 0.55, 0.3, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn kick808() -> Self {
        Self {
            frequency: 50.0,
            decay: 0.4,
            volume: 1.0,
            filter_freq: 3000.0,
            attack: 0.0015,
            release: 0.0,
            decay_curve: 3.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            // special[0] = Accent (click level)
            // special[1] = Snap (pitch sweep depth)
            // special[2] = Pitch Drop amount
            // Defaults at 0 so the user hears the difference when raising sliders.
            special: [
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn perc1() -> Self {
        Self {
            frequency: 2000.0,
            decay: 0.15,
            volume: 0.6,
            filter_freq: 6000.0,
            attack: 0.0005,
            release: 0.0,
            decay_curve: 5.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.7,
            filter_env_decay: 0.03,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            // special[0] = Sweep amount (-1..1)
            // special[1] = Sweep speed (ms)
            // special[2] = Bite (FM amount)
            // special[3] = Width (stereo + slap delay)
            special: [
                0.5, 80.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn bd606() -> Self {
        Self {
            frequency: 0.0,
            decay: 0.4,
            volume: 1.0,
            filter_freq: 20000.0,
            attack: 0.002,
            release: 0.0,
            decay_curve: 4.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.15,
            analog: 1.0,
            stereo: 0.0,
            algo: 0,
            // special[0] = Analog Mode (1 = random multisample, 0 = fixed sample)
            // special[1] = Sample index (1..=8, used when Analog Mode is off)
            // special[2] = One Shot (1 = play to sample end, bypass amp env)
            // special[3] = Start offset (fraction of the sample length, 0..1)
            // special[10] = Pitch format (1 = relative semitones, 0 = legacy Hz)
            // special[11] = End (fraction of the sample length, default 1)
            special: [
                1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn sd606() -> Self {
        Self {
            frequency: 0.0,
            decay: 0.3,
            volume: 0.8,
            filter_freq: 20000.0,
            attack: 0.002,
            release: 0.0,
            decay_curve: 4.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.15,
            analog: 1.0,
            stereo: 0.0,
            algo: 0,
            // special[0] = Analog Mode (1 = random multisample, 0 = fixed sample)
            // special[1] = Sample index (1..=8, used when Analog Mode is off)
            // special[2] = One Shot (1 = play to sample end, bypass amp env)
            // special[3] = Start offset (fraction of the sample length, 0..1)
            // special[10] = Pitch format (1 = relative semitones, 0 = legacy Hz)
            // special[11] = End (fraction of the sample length, default 1)
            special: [
                1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    /// [221] Rift: a 40 ms slice of the noise-field texture, filter wide
    /// open, no saturation - the neutral starting point from which the offset
    /// does the work.
    pub fn rift() -> Self {
        Self {
            frequency: 0.0,
            decay: 0.12,
            volume: 0.8,
            // Repris de Buzz : filtre ferme au repos, ouvert par l'enveloppe.
            filter_freq: 20000.0,
            attack: 0.05,
            release: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.12,
            analog: 0.0,
            stereo: 0.0,
            algo: 0,
            // texture, offset, wander, grain, grain shape, filter type,
            // resonance, then the saturation pack (7..11), then Loop at 12.
            special: [
                0.0, 0.05, 0.0, 0.35, 0.15, 0.0, 0.9, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                // 16 dec curve, 17 pitch fine, 18 pitch env, 19 pitch env time,
                // 20 advance, 21 lfo shape, 22 free phase, 23/24 pitch LFO,
                // 25/26 filter LFO.
                0.6, 0.0, 0.0, 0.08, 0.0, 0.0, 0.0, 5.0, 0.0, 5.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    /// [243] One-Shot: the lane's file, whole; envelope times are fractions of
    /// the played region (decay 1.0 = rings to its end); filter wide open, no
    /// saturation.
    /// [286] The delay's settings: Volume is the return level; the knobs live in
    /// `special[]` (see `delay::sp`; 4 and 31 are reserved by the p-lock layout).
    pub fn rumble() -> Self {
        Self {
            frequency: 1000.0,
            decay: 1.0,
            volume: 1.0,
            filter_freq: 20000.0,
            attack: 0.0,
            release: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            // decay s, low pass Hz, duck, release ms, (4 reserved), drive, (6 retired), glide,
            // (8 restart), high pass Hz, width, echo mix, echo ms, echo sync, echo feedback, duck curve, duck trigger, reverb size, pre-delay, motion, blur
            special: [
                3.0, 120.0, 0.8, 250.0, 0.0, 0.25, 0.0, 0.0, 0.0, 30.0, 0.0, 0.0, 250.0, 0.0, 0.3, 0.5,
                0.0, 2.0, 0.0, 0.7, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    /// [286] The delay's settings (see `delay::sp`).
    pub fn delay() -> Self {
        Self {
            frequency: 1000.0,
            decay: 1.0,
            volume: 1.0,
            filter_freq: 20000.0,
            attack: 0.0,
            release: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            // time ms, feedback, damping, low cut Hz, (4 reserved), ping-pong, -, glide
            special: [
                375.0, 0.4, 0.4, 100.0, 0.0, 0.5, 0.0, 0.0, 0.0, 20000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    /// [286] The reverb's settings: Volume is the return level; the six knobs
    /// live in `special[]` (see `reverb::sp`; 4 and 31 are reserved by the
    /// p-lock layout). The standard fields are unused except Volume.
    pub fn reverb() -> Self {
        Self {
            frequency: 1000.0,
            decay: 1.0,
            volume: 1.0,
            filter_freq: 20000.0,
            attack: 0.0,
            release: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            // size, decay s, damping, pre-delay ms, (4 reserved), low cut Hz, width
            special: [
                1.0, 2.5, 0.5, 20.0, 0.0, 120.0, 1.0, 0.0, 0.0, 20000.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn oneshot() -> Self {
        Self {
            frequency: 0.0,
            decay: 1.0,
            volume: 0.8,
            filter_freq: 20000.0,
            attack: 0.0,
            release: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.12,
            analog: 0.0,
            stereo: 0.0,
            algo: 0,
            // texture, reverse, pitch fine, pitch env, pitch env A/H/D/curves,
            // filter type, resonance, filter A/H/curves, saturation pack 15..19.
            special: [
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.0, 0.6, 0.0, 0.9, 0.0, 0.0, 0.0, 0.6, 0.0,
                0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn ch606() -> Self {
        // TR-606 closed hi-hat sampler: same sampler engine as sd606, tighter
        // amp decay + a touch lower level (hats sit under the kit).
        Self {
            frequency: 0.0,
            decay: 0.2,
            volume: 0.6,
            filter_freq: 20000.0,
            attack: 0.001,
            release: 0.0,
            decay_curve: 4.0,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.15,
            analog: 1.0,
            stereo: 0.0,
            algo: 0,
            special: [
                1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0,
                0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn oh606() -> Self {
        // [208] TR-606 OPEN hi-hat sampler: the closed hat's engine and
        // defaults, with the decay opened up - the whole point of an open hat
        // is that it rings. One Shot is on, like every sampler ([206]), so the
        // slice plays to its end.
        Self {
            decay: 0.9,
            ..Self::ch606()
        }
    }

    pub fn buzz() -> Self {
        // Tonal percussion + adjustable noise, chopped by a fast amplitude gate.
        // Mirror of the registry `sound_settings_default` + special defaults.
        Self {
            frequency: 200.0,
            decay: 0.5,
            volume: 1.3,
            filter_freq: 1200.0,
            attack: 0.0005,
            release: 0.2,
            decay_curve: 4.0,
            release_curve: 0.0, // [146] linear: Buzz's attack before it had an Attack Curve
            hold: 0.0,
            filter_env_amount: 0.6,
            filter_env_decay: 0.12,
            analog: 0.5,
            stereo: 1.0,
            algo: 0,
            // [0 gate_rate, 1 gate_depth, 2 gate_shape, 3 noise_amount,
            //  4 noise_type, 5 pitch_sweep, 6 sat_type, 7 sat_amount, 8 sat_mix,
            //  9 sat_gain, 10 sat_pre, 11 wave, 12 filt_attack, 13 filt_hold,
            //  14 filt_type, 15 filt_dec_curve, 16 filt_atk_curve, …]
            special: [
                55.0, 0.85, 0.55, 0.3, 0.0, 0.3, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.6,
                0.0, 0.9, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    pub fn sdrex() -> Self {
        // Metallic snare (body + HP noise + ring-mod metal) through a flanger.
        // Mirror of the registry `sound_settings_default` + special defaults.
        Self {
            frequency: 185.0,
            decay: 0.15,
            volume: 0.9,
            filter_freq: 20000.0,
            attack: 0.0005,
            release: 0.0,
            decay_curve: 0.0,
            release_curve: 0.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            // [0 fl_rate, 1 fl_min_delay, 2 fl_depth, 3 fl_feedback, 4 fl_wet,
            //  5 sat_type, 6 sat_amount, 7 sat_mix, 8 sat_gain, 9 sat_pre,
            //  10 (retired: was noise_level), 11 noise_type, 12 modulation_free_phase,
            //  13 filt_attack, 14 filt_atk_curve, 15 filt_dec_curve,
            //  16 filt_hold, 17 modulation_type, …]
            special: [
                5.7, 0.7, 1.8, 0.38, 0.32, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.6,
                0.0, 0.0, 0.7071, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
        }
    }

    /// AC606 voices (ported analogcode engines) — one helper, six kinds.
    /// Mirrors of the registry `sound_settings_default` + special defaults
    /// (exploration params [196] + saturation pack at indices 10-13).
    fn ac6(
        frequency: f32,
        decay: f32,
        volume: f32,
        attack: f32,
        decay_curve: f32,
        specials: &[(usize, f32)],
    ) -> Self {
        let mut special = [0.0f32; 32];
        for &(idx, val) in specials {
            special[idx] = val;
        }
        // Saturation pack defaults: Mix at center, Output Gain at range
        // center (0.5..2.0 → 1.25). Type None + Amount 0 = bypass.
        special[12] = 0.5;
        special[13] = 1.25;
        Self {
            frequency,
            decay,
            volume,
            filter_freq: 20000.0,
            attack,
            release: 0.0,
            decay_curve,
            release_curve: 3.0,
            hold: 0.0,
            filter_env_amount: 0.0,
            filter_env_decay: 0.05,
            analog: 0.5,
            stereo: 0.0,
            algo: 0,
            special,
        }
    }

    pub fn bd6ac() -> Self {
        // [1 Sweep, 2 Bend, 3 Click, 4 Click Tone, 5 Punch, 7 Tone, 8 Drive]
        Self::ac6(
            60.0,
            0.5,
            1.0,
            0.002,
            0.0,
            &[
                (1, 0.5),
                (2, 0.5),
                (3, 0.2),
                (4, 1400.0),
                (5, 0.2),
                (7, 0.34),
                (8, 0.18),
            ],
        )
    }

    pub fn sd6ac() -> Self {
        // [0 Snap, 1 Wire Color, 2 Shell Bend, 3 Impact, 4 Ring]
        Self::ac6(
            201.09,
            0.25,
            0.9,
            0.0005,
            5.0,
            &[(0, 0.75), (1, 1.0), (2, 0.5), (3, 0.5), (4, 0.5)],
        )
    }

    pub fn hh6ac() -> Self {
        // [0 Metal, 1 Click, 2 Bell, 3 Wobble, 4 Spread, 5 Brightness]
        Self::ac6(
            8000.0,
            0.15,
            0.6,
            0.0003,
            5.0,
            &[
                (0, 0.11),
                (1, 0.35),
                (2, 0.7),
                (3, 0.25),
                (4, 0.0),
                (5, 0.0),
            ],
        )
    }

    pub fn oh6ac() -> Self {
        Self::ac6(
            8000.0,
            0.8,
            0.7,
            0.0003,
            5.0,
            &[
                (0, 0.11),
                (1, 0.35),
                (2, 0.7),
                (3, 0.25),
                (4, 0.0),
                (5, 0.0),
            ],
        )
    }

    pub fn cl6ac() -> Self {
        // [0 Noise, 1 Spread, 2 Tail, 3 Air]
        Self::ac6(
            1000.0,
            0.25,
            1.0,
            0.0005,
            5.0,
            &[(0, 0.5), (1, 1.0), (2, 0.5), (3, 0.0)],
        )
    }

    pub fn tm6ac() -> Self {
        // [0 Model, 1 Strike, 2 Snap, 3 Glide, 4 Modes, 5 Tail Noise]
        Self::ac6(
            124.435,
            0.35,
            0.8,
            0.0005,
            5.0,
            &[(0, 1.0), (1, 0.5), (2, 0.5), (3, 0.5), (4, 0.5), (5, 0.5)],
        )
    }
}

pub trait Voice: Send + Sync {
    fn trigger(&mut self);
    /// Hard machine-gun retrigger: envelope restarts from zero. Default
    /// falls back to regular trigger for voices that don't override.
    fn trigger_hard(&mut self) {
        self.trigger();
    }
    /// [227] Which hit in a row this is, for voices that step something per
    /// hit (Rift's Advance). The plugin sets it BEFORE `trigger()`; voices
    /// that count nothing ignore it.
    fn set_hit_index(&mut self, _index: u32) {}
    /// [228] The instance's pool of lane textures and which lane this voice
    /// sits on, for voices that read the file the user loaded on their lane
    /// (Rift). Set once after creation; an `Arc` clone, so safe on the audio
    /// thread. Voices without samples ignore it.
    fn set_texture_pool(&mut self, _pool: std::sync::Arc<sample_bank::TexturePool>, _lane: usize) {}
    /// [253] Hand the texture the voice holds across hits (Rift/One-Shot) to
    /// the pool's retirement queue. Called on the audio thread before the
    /// voice is swapped out in `reinitialize_slot`, so the freed `Arc` never
    /// runs its deallocation here. Voices without samples keep the default.
    fn release_held_texture(&mut self) {}
    fn process_sample(&mut self) -> f32;
    /// Stereo version. Default returns duplicated mono.
    fn process_sample_stereo(&mut self) -> (f32, f32) {
        let m = self.process_sample();
        (m, m)
    }
    /// [269] Test-only: the engine tracks activity per slot itself.
    #[cfg_attr(not(test), allow(dead_code))]
    fn is_active(&self) -> bool;
    fn reset(&mut self);
    fn set_settings(&mut self, settings: VoiceSettings);

    /// Set synthesis algorithm by index.
    fn set_algo(&mut self, algo: u8);
    /// Set a special parameter by index (0..7). [269] Test-only: the engine
    /// delivers specials through `set_settings`.
    #[cfg_attr(not(test), allow(dead_code))]
    fn set_special_param(&mut self, index: usize, value: f32);
}

/// [269] The single list of voice kinds. Each line below generates a variant
/// of `DrumVoiceKind` AND its arm in every method of the `Voice` dispatch, so
/// adding a voice is one line here instead of one arm per method. Every call
/// is forwarded to every variant: the trait's default methods are no-ops for
/// the voices that do not override them (e.g. `set_hit_index`, Rift only).
macro_rules! drum_voice_kinds {
    ($($(#[$attr:meta])* $variant:ident($voice:ty),)*) => {
        /// Concrete enum wrapping all drum voice types.
        /// Eliminates dynamic dispatch from the audio path.
        pub enum DrumVoiceKind {
            $($(#[$attr])* $variant($voice),)*
        }

        impl Voice for DrumVoiceKind {
            fn trigger(&mut self) {
                match self { $(Self::$variant(v) => v.trigger(),)* }
            }

            fn trigger_hard(&mut self) {
                match self { $(Self::$variant(v) => v.trigger_hard(),)* }
            }

            fn set_hit_index(&mut self, index: u32) {
                match self { $(Self::$variant(v) => v.set_hit_index(index),)* }
            }

            fn set_texture_pool(&mut self, pool: std::sync::Arc<sample_bank::TexturePool>, lane: usize) {
                match self { $(Self::$variant(v) => v.set_texture_pool(pool, lane),)* }
            }

            fn release_held_texture(&mut self) {
                match self { $(Self::$variant(v) => v.release_held_texture(),)* }
            }

            fn process_sample(&mut self) -> f32 {
                match self { $(Self::$variant(v) => v.process_sample(),)* }
            }

            fn process_sample_stereo(&mut self) -> (f32, f32) {
                match self { $(Self::$variant(v) => v.process_sample_stereo(),)* }
            }

            fn is_active(&self) -> bool {
                match self { $(Self::$variant(v) => v.is_active(),)* }
            }

            fn reset(&mut self) {
                match self { $(Self::$variant(v) => v.reset(),)* }
            }

            fn set_settings(&mut self, settings: VoiceSettings) {
                match self { $(Self::$variant(v) => v.set_settings(settings),)* }
            }

            fn set_algo(&mut self, algo: u8) {
                match self { $(Self::$variant(v) => v.set_algo(algo),)* }
            }

            fn set_special_param(&mut self, index: usize, value: f32) {
                match self { $(Self::$variant(v) => v.set_special_param(index, value),)* }
            }
        }
    };
}

drum_voice_kinds! {
    Kick(KickVoice),
    Snare(SnareVoice),
    HiHat(HiHatVoice),
    OpenHiHat(OpenHiHatVoice),
    Tom(TomVoice),
    Clap(ClapVoice),
    Ride(RideVoice),
    Cymbal(CymbalVoice),
    Snare606(Snare606Voice),
    BassDrum808(Kick808Voice),
    Perc1(Perc1Voice),
    Bd606(Bd606Voice),
    Sd606(Sd606Voice),
    Ch606(Ch606Voice),
    /// [208] OH6smp: the same hat engine as `Ch606`, on the open-hat bank.
    Oh606(Ch606Voice),
    Rift(RiftVoice),
    OneShot(OneShotVoice),
    /// [286] Carried by an effect slot; the DSP is `DrumSynthesizer::fx`.
    Reverb(ReverbVoice),
    /// [286] Carried by an effect slot; the DSP is `DrumSynthesizer::fx_delay`.
    Delay(DelayVoice),
    /// [286] Carried by an effect slot; the DSP is `DrumSynthesizer::fx_rumble`.
    Rumble(RumbleVoice),
    Buzz(BuzzVoice),
    Sdrex(SdrexVoice),
    Bd6Ac(AcVoice),
    Sd6Ac(AcVoice),
    Hh6Ac(AcVoice),
    Oh6Ac(AcVoice),
    Cl6Ac(AcVoice),
    Tm6Ac(AcVoice),
}

fn create_voice_for_kind(
    kind: crate::track::TrackInstrumentKind,
    sample_rate: f32,
) -> DrumVoiceKind {
    use crate::track::TrackInstrumentKind as K;
    match kind {
        K::Kick => DrumVoiceKind::Kick(KickVoice::new(
            sample_rate,
            KickSettings::from(VoiceSettings::kick()),
        )),
        K::Snare => DrumVoiceKind::Snare(SnareVoice::new(
            sample_rate,
            SnareSettings::from(VoiceSettings::snare()),
        )),
        K::HiHat => DrumVoiceKind::HiHat(HiHatVoice::new(
            sample_rate,
            HiHatSettings::from(VoiceSettings::hihat()),
        )),
        K::OpenHiHat => DrumVoiceKind::OpenHiHat(OpenHiHatVoice::new(
            sample_rate,
            OpenHiHatSettings::from(VoiceSettings::open_hihat()),
        )),
        K::Tom => DrumVoiceKind::Tom(TomVoice::new(
            sample_rate,
            TomSettings::from(VoiceSettings::tom1()),
        )),
        K::Clap => DrumVoiceKind::Clap(ClapVoice::new(
            sample_rate,
            ClapSettings::from(VoiceSettings::clap()),
        )),
        K::Ride => DrumVoiceKind::Ride(RideVoice::new(
            sample_rate,
            RideSettings::from(VoiceSettings::ride()),
        )),
        K::Cymbal => DrumVoiceKind::Cymbal(CymbalVoice::new(
            sample_rate,
            CymbalSettings::from(VoiceSettings::cymbal()),
        )),
        K::Snare606 => DrumVoiceKind::Snare606(Snare606Voice::new(
            sample_rate,
            Snare606Settings::from(VoiceSettings::snare606()),
        )),
        K::BassDrum808 => DrumVoiceKind::BassDrum808(Kick808Voice::new(
            sample_rate,
            Kick808Settings::from(VoiceSettings::kick808()),
        )),
        K::Perc1 => DrumVoiceKind::Perc1(Perc1Voice::new(
            sample_rate,
            Perc1Settings::from(VoiceSettings::perc1()),
        )),
        K::Bd6smp => DrumVoiceKind::Bd606(Bd606Voice::new(
            sample_rate,
            Bd606Settings::from(VoiceSettings::bd606()),
        )),
        K::Sd6smp => DrumVoiceKind::Sd606(Sd606Voice::new(
            sample_rate,
            Sd606Settings::from(VoiceSettings::sd606()),
        )),
        K::Ch6smp => DrumVoiceKind::Ch606(Ch606Voice::new(
            sample_rate,
            Ch606Settings::from(VoiceSettings::ch606()),
        )),
        K::Oh6smp => DrumVoiceKind::Oh606(Ch606Voice::with_bank(
            sample_rate,
            Ch606Settings::from(VoiceSettings::oh606()),
            sample_bank::oh606(),
        )),
        K::Rift => DrumVoiceKind::Rift(RiftVoice::new(
            sample_rate,
            RiftSettings::from(VoiceSettings::rift()),
        )),
        K::OneShot => DrumVoiceKind::OneShot(OneShotVoice::new(
            sample_rate,
            OneShotSettings::from(VoiceSettings::oneshot()),
        )),
        K::Reverb => DrumVoiceKind::Reverb(ReverbVoice),
        K::Delay => DrumVoiceKind::Delay(DelayVoice),
        K::Rumble => DrumVoiceKind::Rumble(RumbleVoice),
        K::Buzz => DrumVoiceKind::Buzz(BuzzVoice::new(
            sample_rate,
            BuzzSettings::from(VoiceSettings::buzz()),
        )),
        K::Sdrex => DrumVoiceKind::Sdrex(SdrexVoice::new(
            sample_rate,
            SdrexSettings::from(VoiceSettings::sdrex()),
        )),
        K::Bd6Ac => DrumVoiceKind::Bd6Ac(AcVoice::new(
            sample_rate,
            AcEngineKind::BassDrum,
            AcVoiceSettings::from(VoiceSettings::bd6ac()),
        )),
        K::Sd6Ac => DrumVoiceKind::Sd6Ac(AcVoice::new(
            sample_rate,
            AcEngineKind::Snare,
            AcVoiceSettings::from(VoiceSettings::sd6ac()),
        )),
        K::Hh6Ac => DrumVoiceKind::Hh6Ac(AcVoice::new(
            sample_rate,
            AcEngineKind::ClosedHat,
            AcVoiceSettings::from(VoiceSettings::hh6ac()),
        )),
        K::Oh6Ac => DrumVoiceKind::Oh6Ac(AcVoice::new(
            sample_rate,
            AcEngineKind::OpenHat,
            AcVoiceSettings::from(VoiceSettings::oh6ac()),
        )),
        K::Cl6Ac => DrumVoiceKind::Cl6Ac(AcVoice::new(
            sample_rate,
            AcEngineKind::Clap,
            AcVoiceSettings::from(VoiceSettings::cl6ac()),
        )),
        K::Tm6Ac => DrumVoiceKind::Tm6Ac(AcVoice::new(
            sample_rate,
            AcEngineKind::Tom,
            AcVoiceSettings::from(VoiceSettings::tm6ac()),
        )),
    }
}

/// [286] Values the Lane Editor shows for an effect bus ("Playing now").
pub const FX_LIVE_FIELDS: usize = 10;

/// [286] What an effect bus is using for this moment, whichever effect it hosts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FxParams {
    Reverb(ReverbParams),
    Delay(DelayParams),
    Rumble(RumbleParams),
}

impl FxParams {
    /// The values the Lane Editor's "Playing now" module shows, in the order of
    /// that effect's rows (see `fx_live_rows`).
    pub fn live_values(&self) -> [f32; FX_LIVE_FIELDS] {
        match self {
            FxParams::Reverb(p) => [
                p.size,
                p.decay_s,
                p.damping,
                p.predelay_ms,
                p.low_cut_hz,
                p.high_cut_hz,
                p.width,
                p.level,
                p.gate_ms,
                p.gate_release_ms,
            ],
            FxParams::Delay(p) => [
                p.time_ms,
                p.feedback,
                p.damping,
                p.low_cut_hz,
                p.high_cut_hz,
                p.pingpong,
                p.width,
                p.level,
                0.0,
                0.0,
            ],
            FxParams::Rumble(p) => [
                p.decay_s,
                p.high_pass_hz,
                p.low_pass_hz,
                p.duck,
                p.release_ms,
                p.drive,
                p.width,
                p.echo_mix,
                p.level,
                0.0,
            ],
        }
    }

    #[cfg(test)]
    pub fn reverb(self) -> Option<ReverbParams> {
        match self {
            FxParams::Reverb(p) => Some(p),
            FxParams::Delay(_) | FxParams::Rumble(_) => None,
        }
    }
}

pub struct DrumSynthesizer {
    voices: Box<[Option<Box<DrumVoiceKind>>; crate::track::MAX_TRACKS]>,
    sample_rate: f32,
    velocities: [f32; crate::track::MAX_TRACKS],
    /// One-pole smoothers on each voice's velocity, absorbing gain jumps when
    /// retriggering a voice while its tail is still ringing.
    velocity_smoothers: [dsp::OnePoleSmoother; crate::track::MAX_TRACKS],
    /// Per-slot active flag. All voices are preallocated in `initialize_with_layout`
    /// so that the audio thread never allocates, but only active slots are
    /// triggered and processed.
    active: [bool; crate::track::MAX_TRACKS],
    /// [228] The instance's user textures, handed to every voice that reads
    /// samples. `None` in the test harness: embedded textures only.
    texture_pool: Option<std::sync::Arc<sample_bank::TexturePool>>,
    /// [286] A reverb AND a delay per effect bus (Fx1, Fx2): the lane can change
    /// kind while playing, so both are allocated up front, in
    /// `initialize_with_layout` (non-RT).
    fx_reverb: [ReverbCore; crate::track::FX_SLOT_COUNT],
    fx_delay: [DelayCore; crate::track::FX_SLOT_COUNT],
    fx_rumble: [RumbleCore; crate::track::FX_SLOT_COUNT],
    /// A bus that was muted: its tail was flushed once, not every sample.
    fx_bypassed: [bool; crate::track::FX_SLOT_COUNT],
    /// Smoothed send level of every lane to every bus.
    send_state: [[f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
    send_coef: f32,
    /// [286] The send levels p-locked on the step a lane last fired: they replace the
    /// lane's own sends until its next hit (`None` = follow the lane).
    send_override: [[Option<f32>; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
    /// [300] Sends switched off (they win over a step's send lock).
    send_muted: [[bool; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
    /// [303] Each instrument lane's inserts (no heap inside: rebuilt in
    /// `initialize_with_layout` for the sample rate, never in `process`).
    inserts: Box<[inserts::InsertRack; crate::track::MAX_TRACKS]>,
}

const VELOCITY_SMOOTH_MS: f32 = 1.5;
const SEND_SMOOTH_MS: f32 = 5.0;

impl DrumSynthesizer {
    pub fn new() -> Self {
        Self {
            texture_pool: None,
            fx_reverb: std::array::from_fn(|_| ReverbCore::new(44100.0)),
            fx_delay: std::array::from_fn(|_| DelayCore::new(44100.0)),
            fx_rumble: std::array::from_fn(|_| RumbleCore::new(44100.0)),
            inserts: Box::new(std::array::from_fn(|_| inserts::InsertRack::new(44100.0))),
            send_muted: [[false; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
            fx_bypassed: [false; crate::track::FX_SLOT_COUNT],
            send_state: [[0.0; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
            send_override: [[None; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
            send_coef: 1.0 - (-1.0 / (44100.0 * SEND_SMOOTH_MS * 0.001f32)).exp(),
            voices: Box::new(std::array::from_fn(|_| None)),
            sample_rate: 44100.0,
            velocities: [1.0; crate::track::MAX_TRACKS],
            velocity_smoothers: std::array::from_fn(|_| {
                dsp::OnePoleSmoother::new(44100.0, VELOCITY_SMOOTH_MS, 1.0)
            }),
            active: [false; crate::track::MAX_TRACKS],
        }
    }

    /// [269] Tests and the `test_standalone` harness; the plugin calls
    /// `initialize_with_layout`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn initialize(&mut self, sample_rate: f32) {
        let legacy_layout = crate::track::TrackLayoutState::from_legacy_13();
        self.initialize_with_layout(sample_rate, &legacy_layout);
    }

    pub fn initialize_with_layout(
        &mut self,
        sample_rate: f32,
        layout: &crate::track::TrackLayoutState,
    ) {
        self.sample_rate = sample_rate;
        // [286] The effects' buffers: allocated here (non-RT), never in `process`.
        self.fx_reverb = std::array::from_fn(|_| ReverbCore::new(sample_rate));
        self.fx_delay = std::array::from_fn(|_| DelayCore::new(sample_rate));
        self.fx_rumble = std::array::from_fn(|_| RumbleCore::new(sample_rate));
        self.inserts = Box::new(std::array::from_fn(|_| inserts::InsertRack::new(sample_rate)));
        self.send_coef = 1.0 - (-1.0 / (sample_rate * SEND_SMOOTH_MS * 0.001)).exp();
        for smoother in self.velocity_smoothers.iter_mut() {
            *smoother = dsp::OnePoleSmoother::new(sample_rate, VELOCITY_SMOOTH_MS, 1.0);
        }

        // Pre-warm the embedded multisample banks here (non-RT) so a later
        // kind change to a multisample voice on the audio thread never
        // allocates.
        let _ = sample_bank::bd606();
        let _ = sample_bank::sd606();
        let _ = sample_bank::ch606();
        let _ = sample_bank::oh606();
        sample_bank::prewarm_textures();
        // Pre-build the AC606 clap's reconstruction table (non-RT too).
        ac606::prewarm();

        // Preallocate a voice for every slot so the audio thread never has to
        // allocate when a slot is activated or its instrument changes. Inactive
        // slots keep a placeholder voice and are gated out by `active`.
        for (i, slot) in layout.slots.iter().enumerate() {
            let placeholder_kind = crate::track::TrackInstrumentKind::from_drum_voice_index(i)
                .unwrap_or(crate::track::TrackInstrumentKind::Perc1);
            let kind = if slot.active {
                slot.kind
            } else {
                placeholder_kind
            };
            let mut voice = create_voice_for_kind(kind, sample_rate);
            if let Some(pool) = &self.texture_pool {
                voice.set_texture_pool(pool.clone(), i);
            }
            self.voices[i] = Some(Box::new(voice));
            self.active[i] = slot.active;
        }
    }

    /// [228] Give the voices access to the user textures. Call before
    /// `initialize_with_layout`; later voice swaps inherit it.
    pub fn set_texture_pool(&mut self, pool: std::sync::Arc<sample_bank::TexturePool>) {
        self.texture_pool = Some(pool);
    }
    pub fn trigger(&mut self, slot_idx: usize, velocity: f32) {
        if !self.active.get(slot_idx).copied().unwrap_or(false) {
            return;
        }
        if let Some(Some(voice)) = self.voices.get_mut(slot_idx) {
            voice.trigger();
            self.velocities[slot_idx] = velocity;
        }
    }

    pub fn trigger_hard(&mut self, slot_idx: usize, velocity: f32) {
        if !self.active.get(slot_idx).copied().unwrap_or(false) {
            return;
        }
        if let Some(Some(voice)) = self.voices.get_mut(slot_idx) {
            voice.trigger_hard();
            self.velocities[slot_idx] = velocity;
        }
    }

    /// [227] Hand the voice its hit index, before the trigger that follows.
    pub fn set_hit_index(&mut self, slot_idx: usize, index: u32) {
        if let Some(Some(voice)) = self.voices.get_mut(slot_idx) {
            voice.set_hit_index(index);
        }
    }

    /// [269] Mono mix for the `test_standalone` harness (the plugin renders
    /// per slot in stereo through `process_voice_samples_stereo`).
    #[allow(dead_code)]
    pub fn process_sample(&mut self, output: &mut f32) {
        let mut mixed = 0.0f32;
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if self.active[i] {
                if let Some(voice) = voice {
                    mixed += voice.process_sample();
                }
            }
        }
        *output = mixed;
    }

    pub fn process_voice_samples_stereo(
        &mut self,
        outputs: &mut [[f32; 2]; crate::track::MAX_TRACKS],
    ) {
        for (i, (voice, output)) in self.voices.iter_mut().zip(outputs.iter_mut()).enumerate() {
            if !self.active[i] {
                output[0] = 0.0;
                output[1] = 0.0;
                continue;
            }
            if let Some(voice) = voice {
                let vel = self.velocity_smoothers[i].process(self.velocities[i]);
                let (l, r) = voice.process_sample_stereo();
                output[0] = l * vel;
                output[1] = r * vel;
            } else {
                output[0] = 0.0;
                output[1] = 0.0;
            }
        }
    }

    /// [286] The sends of a lane for the hit that is about to fire: the step's p-locks,
    /// or `None` to follow the lane's own levels.
    pub fn set_send_override(&mut self, slot: usize, bus: usize, level: Option<f32>) {
        if let Some(cell) = self.send_override.get_mut(slot).and_then(|row| row.get_mut(bus)) {
            *cell = level.map(|v| v.clamp(0.0, 1.0));
        }
    }

    /// [286] Back to the lanes' own sends (the transport stopped: a slider moved while
    /// stopped must be heard, not shadowed by the last hit's locks).
    pub fn clear_send_overrides(&mut self) {
        self.send_override = [[None; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
    }

    /// [286] Hand a bus its parameters for this moment (the p-lock evaluation
    /// runs at control rate and replaces the slider values with the locked ones).
    pub fn set_fx_params(&mut self, bus: usize, params: FxParams) {
        if bus >= crate::track::FX_SLOT_COUNT {
            return;
        }
        match params {
            FxParams::Reverb(p) => self.fx_reverb[bus].set_params(p),
            FxParams::Delay(p) => self.fx_delay[bus].set_params(p),
            FxParams::Rumble(p) => self.fx_rumble[bus].set_params(p),
        }
    }

    /// [303] The largest gain reduction of `slot`'s compressor insert since the last call.
    pub fn take_insert_gr(&mut self, slot: usize) -> f32 {
        self.inserts.get_mut(slot).map_or(0.0, |rack| rack.take_gr_peak())
    }

    /// [300] Which sends are switched off (once per block).
    pub fn set_send_mutes(&mut self, muted: [[bool; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS]) {
        self.send_muted = muted;
    }

    /// [303] Hand a lane its inserts (once per block, from the layout's atomics).
    pub fn set_inserts(&mut self, slot: usize, settings: inserts::InsertSettings) {
        if let Some(rack) = self.inserts.get_mut(slot) {
            rack.set_settings(settings);
        }
    }

    /// [303] Run every instrument lane through its inserts. Call right after
    /// `process_voice_samples_stereo` and before `process_fx`: an insert sits on the lane
    /// itself, so the sends hear it. Effect buses have none.
    pub fn process_inserts(&mut self, outputs: &mut [[f32; 2]; crate::track::MAX_TRACKS]) {
        for slot in 0..crate::track::FX_FIRST_SLOT {
            if !self.active[slot] {
                continue;
            }
            let rack = &mut self.inserts[slot];
            if rack.is_active() {
                let (l, r) = rack.process(outputs[slot][0], outputs[slot][1]);
                outputs[slot] = [l, r];
            }
        }
    }

    /// [286] The Rumble's duck gain on bus `bus` (1 = untouched), `None` when the bus is not a
    /// Rumble. For the Lane Editor's "Playing now".
    pub fn rumble_duck_gain(&self, bus: usize) -> Option<f32> {
        let slot = crate::track::FX_FIRST_SLOT + *Some(&bus).filter(|b| **b < crate::track::FX_SLOT_COUNT)?;
        match self.voices[slot].as_deref() {
            Some(DrumVoiceKind::Rumble(_)) if self.active[slot] => Some(self.fx_rumble[bus].duck_gain()),
            _ => None,
        }
    }

    /// [286] Start the Rumble's duck on bus `bus` (the transport's beat), when its Duck Trigger
    /// is a beat division. A no-op for any other bus.
    pub fn rumble_trigger_duck(&mut self, bus: usize) {
        if bus >= crate::track::FX_SLOT_COUNT {
            return;
        }
        let slot = crate::track::FX_FIRST_SLOT + bus;
        if self.active[slot]
            && matches!(self.voices[slot].as_deref(), Some(DrumVoiceKind::Rumble(_)))
            && self.fx_rumble[bus].duck_source() != rumble::DUCK_SOURCE_KICK
        {
            self.fx_rumble[bus].trigger_duck();
        }
    }

    /// [286] The effect buses. Call right after `process_voice_samples_stereo`:
    /// every instrument lane (slots below `FX_FIRST_SLOT`) feeds each bus by its
    /// send level (post-fader: `outputs` already carries the lane Volume, and a
    /// lane routed away from Main still feeds the bus), the bus runs its reverb,
    /// and the wet return replaces the bus slot's output. A slot that is not an
    /// active Reverb (empty, or a grandfathered instrument) is left untouched.
    /// `bypass` mutes a bus: its tail is flushed once and it returns silence.
    pub fn process_fx(
        &mut self,
        outputs: &mut [[f32; 2]; crate::track::MAX_TRACKS],
        sends: &[[f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS],
        bypass: [bool; crate::track::FX_SLOT_COUNT],
    ) {
        for bus in 0..crate::track::FX_SLOT_COUNT {
            let slot = crate::track::FX_FIRST_SLOT + bus;
            // 0 = not an effect (empty, or a grandfathered instrument), 1 = reverb,
            // 2 = delay, 3 = rumble.
            let which = if self.active[slot] {
                match self.voices[slot].as_deref() {
                    Some(DrumVoiceKind::Reverb(_)) => 1,
                    Some(DrumVoiceKind::Delay(_)) => 2,
                    Some(DrumVoiceKind::Rumble(_)) => 3,
                    _ => 0,
                }
            } else {
                0
            };
            if which == 0 {
                continue;
            }
            if bypass[bus] {
                if !self.fx_bypassed[bus] {
                    self.fx_reverb[bus].reset();
                    self.fx_delay[bus].reset();
                    self.fx_rumble[bus].reset();
                    self.fx_bypassed[bus] = true;
                }
                outputs[slot] = [0.0, 0.0];
                continue;
            }
            self.fx_bypassed[bus] = false;
            let (mut in_l, mut in_r) = (0.0f32, 0.0f32);
            for lane in 0..crate::track::FX_FIRST_SLOT {
                let level = &mut self.send_state[lane][bus];
                let wanted = if self.send_muted[lane][bus] {
                    0.0
                } else {
                    self.send_override[lane][bus].unwrap_or(sends[lane][bus])
                };
                *level += (wanted - *level) * self.send_coef;
                in_l += outputs[lane][0] * *level;
                in_r += outputs[lane][1] * *level;
            }
            let (l, r) = match which {
                1 => self.fx_reverb[bus].process(in_l, in_r),
                2 => self.fx_delay[bus].process(in_l, in_r),
                _ => self.fx_rumble[bus].process(in_l, in_r),
            };
            outputs[slot] = [l, r];
        }
    }

    pub fn reset(&mut self) {
        for voice in self.voices.iter_mut().flatten() {
            voice.reset();
        }
        for fx in self.fx_reverb.iter_mut() {
            fx.reset();
        }
        for fx in self.fx_delay.iter_mut() {
            fx.reset();
        }
        for fx in self.fx_rumble.iter_mut() {
            fx.reset();
        }
        for rack in self.inserts.iter_mut() {
            rack.reset();
        }
        self.send_state = [[0.0; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
        self.send_override = [[None; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
    }

    pub fn set_voice_settings(&mut self, slot_idx: usize, settings: VoiceSettings) {
        if !self.active.get(slot_idx).copied().unwrap_or(false) {
            return;
        }
        // [286] An effect slot's reverb or delay is driven by the same settings push.
        if let Some(Some(voice)) = self.voices.get_mut(slot_idx) {
            if crate::track::is_fx_slot(slot_idx) {
                let bus = slot_idx - crate::track::FX_FIRST_SLOT;
                match &**voice {
                    DrumVoiceKind::Reverb(_) => self.fx_reverb[bus].set_params(ReverbParams::from(&settings)),
                    DrumVoiceKind::Delay(_) => self.fx_delay[bus].set_params(DelayParams::from(&settings)),
                    DrumVoiceKind::Rumble(_) => self.fx_rumble[bus].set_params(RumbleParams::from(&settings)),
                    _ => {}
                }
            }
            voice.set_settings(settings);
        }
    }

    pub fn set_algo(&mut self, slot_idx: usize, algo: u8) {
        if !self.active.get(slot_idx).copied().unwrap_or(false) {
            return;
        }
        if let Some(Some(voice)) = self.voices.get_mut(slot_idx) {
            voice.set_algo(algo);
        }
    }

    pub fn reset_voice(&mut self, slot_idx: usize) {
        if !self.active.get(slot_idx).copied().unwrap_or(false) {
            return;
        }
        if let Some(Some(voice)) = self.voices.get_mut(slot_idx) {
            voice.reset();
        }
    }

    pub fn set_slot_active(&mut self, slot_idx: usize, active: bool) {
        if slot_idx < crate::track::MAX_TRACKS {
            self.active[slot_idx] = active;
        }
    }

    pub fn reinitialize_slot(&mut self, slot_idx: usize, kind: crate::track::TrackInstrumentKind) {
        if slot_idx >= crate::track::MAX_TRACKS {
            return;
        }
        let mut new_voice = create_voice_for_kind(kind, self.sample_rate);
        if let Some(pool) = &self.texture_pool {
            // An Arc clone: one atomic increment, fine on the audio thread.
            new_voice.set_texture_pool(pool.clone(), slot_idx);
        }
        if let Some(existing) = self.voices[slot_idx].as_mut() {
            // [253] The old voice may hold the last `Arc` of a user texture
            // (Rift/One-Shot): retire it to the UI-drained queue instead of
            // freeing it here when `**existing` is overwritten.
            existing.release_held_texture();
            **existing = new_voice;
            self.active[slot_idx] = true;
        } else {
            // All slots should be preallocated in `initialize_with_layout`. Do
            // not allocate on the audio thread.
        }
        self.velocity_smoothers[slot_idx] =
            dsp::OnePoleSmoother::new(self.sample_rate, VELOCITY_SMOOTH_MS, 1.0);
        // [286] A new kind on an effect slot starts with a clean tail.
        if crate::track::is_fx_slot(slot_idx) {
            let bus = slot_idx - crate::track::FX_FIRST_SLOT;
            self.fx_reverb[bus].reset();
            self.fx_delay[bus].reset();
            self.fx_rumble[bus].reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: sum absolute output of a single voice after triggering it.
    fn sum_voice_output(synth: &mut DrumSynthesizer, voice_idx: usize, samples: usize) -> f32 {
        let mut sum = 0.0f32;
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for _ in 0..samples {
            synth.process_voice_samples_stereo(&mut outputs);
            sum += outputs[voice_idx][0].abs() + outputs[voice_idx][1].abs();
        }
        sum
    }

    /// [286] Run a kick through the effect bus and return the energy of the bus slot.
    fn fx_return_energy(send: f32, bypass: bool) -> f32 {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot, FX_FIRST_SLOT};
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(K::Kick);
        layout.slots[FX_FIRST_SLOT] = TrackSlot::active_with_kind(K::Reverb);
        let mut synth = DrumSynthesizer::new();
        synth.initialize_with_layout(44100.0, &layout);
        synth.set_voice_settings(0, VoiceSettings::kick());
        synth.set_voice_settings(FX_FIRST_SLOT, VoiceSettings::reverb());
        synth.trigger(0, 1.0);
        let mut sends = [[0.0f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
        sends[0][0] = send;
        let mut energy = 0.0f32;
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for _ in 0..44_100 {
            synth.process_voice_samples_stereo(&mut outputs);
            synth.process_fx(&mut outputs, &sends, [bypass, false]);
            energy += outputs[FX_FIRST_SLOT][0].powi(2) + outputs[FX_FIRST_SLOT][1].powi(2);
        }
        energy
    }

    #[test]
    fn the_send_level_feeds_the_reverb_bus() {
        assert_eq!(fx_return_energy(0.0, false), 0.0, "no send, no return");
        let half = fx_return_energy(0.5, false);
        let full = fx_return_energy(1.0, false);
        assert!(half > 0.01, "a send must produce a tail: {half}");
        assert!(full > half * 2.0, "more send, more return: {half} -> {full}");
    }

    /// [286] The real kick through the real bus: at any send level the Rumble's duck hears the
    /// hit (its gain drops), then it swells back to full.
    #[test]
    fn a_real_kick_ducks_the_rumble_bus_at_any_send_level() {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot, FX_FIRST_SLOT};
        for send in [1.0f32, 0.3, 0.05] {
            let mut layout = TrackLayoutState::empty_layout();
            layout.slots[0] = TrackSlot::active_with_kind(K::Kick);
            layout.slots[FX_FIRST_SLOT + 1] = TrackSlot::active_with_kind(K::Rumble);
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(44100.0, &layout);
            synth.set_voice_settings(0, VoiceSettings::kick());
            let mut rumble = VoiceSettings::rumble();
            rumble.special[rumble_sp::DUCK_SOURCE] = rumble::DUCK_SOURCE_KICK as f32; // listen to the kick
            synth.set_voice_settings(FX_FIRST_SLOT + 1, rumble);
            let mut sends = [[0.0f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
            sends[0][1] = send;
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            // Half a second of silence, then the kick.
            for _ in 0..22_050 {
                synth.process_voice_samples_stereo(&mut outputs);
                synth.process_fx(&mut outputs, &sends, [false, false]);
            }
            assert_eq!(synth.rumble_duck_gain(1), Some(1.0), "nothing ducks before the kick");
            synth.trigger(0, 1.0);
            let (mut lowest, mut peak_in) = (1.0f32, 0.0f32);
            let mut back = 0.0f32;
            for i in 0..22_050 {
                synth.process_voice_samples_stereo(&mut outputs);
                peak_in = peak_in.max(outputs[0][0].abs() * send);
                synth.process_fx(&mut outputs, &sends, [false, false]);
                let g = synth.rumble_duck_gain(1).unwrap();
                lowest = lowest.min(g);
                if i == 21_000 {
                    back = g;
                }
            }
            println!("send {send}: bus input peak {peak_in:.3}, duck gain down to {lowest:.3}, {back:.3} at 475 ms");
            assert!(lowest < 0.4, "the kick at send {send} did not duck the rumble ({lowest})");
            assert!(back > 0.9, "the rumble must be back after the Release ({back})");
        }
    }

    /// [303] A lane's inserts act on the lane's own output (so its main mix, aux and sends
    /// hear them): a 150 Hz low-pass takes the highs out of a hi-hat; all off, the lane is
    /// untouched, bit for bit.
    #[test]
    fn lane_inserts_shape_the_lane_and_all_off_change_nothing() {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot};
        let run = |filter_on: bool| {
            let mut layout = TrackLayoutState::empty_layout();
            layout.slots[2] = TrackSlot::active_with_kind(K::HiHat);
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(44100.0, &layout);
            synth.set_voice_settings(2, VoiceSettings::hihat());
            let mut ins = inserts::InsertSettings::default();
            ins.on[inserts::FILTER] = filter_on;
            ins.values[inserts::FILTER] = [0.0, 150.0, 0.0, 1.0, 0.0, 0.0];
            synth.set_inserts(2, ins);
            synth.trigger(2, 1.0);
            let mut out = Vec::new();
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            for _ in 0..8000 {
                synth.process_voice_samples_stereo(&mut outputs);
                synth.process_inserts(&mut outputs);
                out.push(outputs[2][0]);
            }
            out
        };
        let (dry, filtered) = (run(false), run(true));
        let e = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>();
        assert!(e(&dry) > 1.0e-3, "the hi-hat plays");
        assert!(e(&filtered) < 0.1 * e(&dry), "a 150 Hz low-pass guts a hi-hat");
        // Off is a wire: the same voice with no insert on is exactly the raw output.
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[2] = TrackSlot::active_with_kind(K::HiHat);
        let mut raw = DrumSynthesizer::new();
        raw.initialize_with_layout(44100.0, &layout);
        raw.set_voice_settings(2, VoiceSettings::hihat());
        raw.trigger(2, 1.0);
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for i in 0..8000 {
            raw.process_voice_samples_stereo(&mut outputs);
            assert_eq!(outputs[2][0], dry[i]);
        }
    }

    /// [300] A muted send feeds nothing, even with a step's send lock on the hit.
    #[test]
    fn a_muted_send_feeds_nothing_even_when_locked() {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot, FX_FIRST_SLOT};
        let energy = |muted: bool, locked: bool| {
            let mut layout = TrackLayoutState::empty_layout();
            layout.slots[0] = TrackSlot::active_with_kind(K::Kick);
            layout.slots[FX_FIRST_SLOT] = TrackSlot::active_with_kind(K::Reverb);
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(44100.0, &layout);
            synth.set_voice_settings(0, VoiceSettings::kick());
            synth.set_voice_settings(FX_FIRST_SLOT, VoiceSettings::reverb());
            let mut mutes = [[false; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
            mutes[0][0] = muted;
            synth.set_send_mutes(mutes);
            if locked {
                synth.set_send_override(0, 0, Some(1.0));
            }
            synth.trigger(0, 1.0);
            let mut sends = [[0.0f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
            sends[0][0] = 1.0;
            let mut e = 0.0f32;
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            for _ in 0..22_050 {
                synth.process_voice_samples_stereo(&mut outputs);
                synth.process_fx(&mut outputs, &sends, [false, false]);
                e += outputs[FX_FIRST_SLOT][0].powi(2);
            }
            e
        };
        assert!(energy(false, false) > 0.01);
        assert_eq!(energy(true, false), 0.0);
        assert_eq!(energy(true, true), 0.0, "the mute wins over a step lock");
    }

    /// [286] A p-locked send replaces the lane's own for the hit, and `None` goes back.
    #[test]
    fn a_send_override_replaces_the_lane_send_for_the_hit() {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot, FX_FIRST_SLOT};
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(K::Kick);
        layout.slots[FX_FIRST_SLOT] = TrackSlot::active_with_kind(K::Reverb);
        let run = |lane_send: f32, locked: Option<f32>| {
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(44100.0, &layout);
            synth.set_voice_settings(0, VoiceSettings::kick());
            synth.set_voice_settings(FX_FIRST_SLOT, VoiceSettings::reverb());
            synth.set_send_override(0, 0, locked);
            synth.trigger(0, 1.0);
            let mut sends = [[0.0f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
            sends[0][0] = lane_send;
            let mut energy = 0.0f32;
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            for _ in 0..44_100 {
                synth.process_voice_samples_stereo(&mut outputs);
                synth.process_fx(&mut outputs, &sends, [false, false]);
                energy += outputs[FX_FIRST_SLOT][0].powi(2);
            }
            energy
        };
        assert_eq!(run(0.0, None), 0.0);
        assert!(run(0.0, Some(1.0)) > 0.01, "a lock sends even when the lane does not");
        assert_eq!(run(1.0, Some(0.0)), 0.0, "a lock of 0 silences a lane that sends");
    }

    /// [286] The same bus hosts a delay: a kick sent to it comes back later, and the lane
    /// can switch from one effect to the other while playing.
    #[test]
    fn an_effect_bus_can_host_a_delay_and_switch_effects() {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot, FX_FIRST_SLOT};
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(K::Kick);
        layout.slots[FX_FIRST_SLOT] = TrackSlot::active_with_kind(K::Delay);
        let mut synth = DrumSynthesizer::new();
        synth.initialize_with_layout(44100.0, &layout);
        synth.set_voice_settings(0, VoiceSettings::kick());
        synth.set_voice_settings(FX_FIRST_SLOT, VoiceSettings::delay());
        synth.trigger(0, 1.0);
        let mut sends = [[0.0f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
        sends[0][0] = 1.0;
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        let (mut early, mut late) = (0.0f32, 0.0f32);
        for n in 0..44_100 {
            synth.process_voice_samples_stereo(&mut outputs);
            synth.process_fx(&mut outputs, &sends, [false, false]);
            let e = outputs[FX_FIRST_SLOT][0].powi(2) + outputs[FX_FIRST_SLOT][1].powi(2);
            // Default time 375 ms: nothing before ~350 ms, the repeat after.
            if n < 14_000 {
                early += e;
            } else if n < 20_000 {
                late += e;
            }
        }
        assert!(early < 1.0e-6, "the delay must wait before repeating: {early}");
        assert!(late > 0.01, "the repeat must arrive: {late}");
        // The lane becomes a reverb: its tail starts clean.
        synth.reinitialize_slot(FX_FIRST_SLOT, K::Reverb);
        synth.set_voice_settings(FX_FIRST_SLOT, VoiceSettings::reverb());
        synth.process_voice_samples_stereo(&mut outputs);
        synth.process_fx(&mut outputs, &sends, [false, false]);
        assert!(outputs[FX_FIRST_SLOT][0].is_finite());
    }

    #[test]
    fn a_muted_bus_returns_silence() {
        assert_eq!(fx_return_energy(1.0, true), 0.0);
    }

    /// The kick itself stays untouched by the bus.
    #[test]
    fn the_bus_does_not_change_the_dry_lane() {
        use crate::track::{TrackInstrumentKind as K, TrackLayoutState, TrackSlot, FX_FIRST_SLOT};
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(K::Kick);
        layout.slots[FX_FIRST_SLOT] = TrackSlot::active_with_kind(K::Reverb);
        let dry = |send: f32| {
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(44100.0, &layout);
            synth.set_voice_settings(0, VoiceSettings::kick());
            synth.trigger(0, 1.0);
            let mut sends = [[0.0f32; crate::track::FX_SLOT_COUNT]; crate::track::MAX_TRACKS];
            sends[0][0] = send;
            let mut out = Vec::new();
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            for _ in 0..4_000 {
                synth.process_voice_samples_stereo(&mut outputs);
                synth.process_fx(&mut outputs, &sends, [false, false]);
                out.push(outputs[0]);
            }
            out
        };
        assert_eq!(dry(0.0), dry(1.0));
    }

    #[test]
    fn bd606_fine_tune_changes_playback_rate_through_synthesizer() {
        let duration = |fine: f32| -> usize {
            let mut layout = crate::track::TrackLayoutState::empty_layout();
            layout.slots[0] = crate::track::TrackSlot::active_with_kind(
                crate::track::TrackInstrumentKind::Bd6smp,
            );
            let mut synth = DrumSynthesizer::new();
            synth.initialize_with_layout(44100.0, &layout);
            let mut settings = VoiceSettings::bd606();
            settings.special[2] = 1.0; // one shot: duration = hit length / rate
            settings.special[9] = fine;
            synth.set_voice_settings(0, settings);
            synth.trigger(0, 1.0);
            let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
            let mut last_nonzero = 0usize;
            for n in 0..44100 * 4 {
                synth.process_voice_samples_stereo(&mut outputs);
                if outputs[0][0].abs() > 1e-6 {
                    last_nonzero = n;
                }
            }
            last_nonzero
        };

        let flat = duration(-100.0);
        let sharp = duration(100.0);
        assert!(
            flat > sharp + 1000,
            "fine -100 should play clearly longer than +100 (flat={flat}, sharp={sharp})"
        );
    }

    #[test]
    fn bassdrum808_produces_sound_after_cymbal_settings_change() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let b8_idx = DrumVoice::BassDrum808 as usize;
        let cy_idx = DrumVoice::Cymbal as usize;

        // 1. Trigger B8 and verify it produces sound
        synth.trigger(b8_idx, 1.0);
        let out1 = sum_voice_output(&mut synth, b8_idx, 100);
        assert!(
            out1 > 0.0,
            "B8 should produce sound on first trigger: {}",
            out1
        );

        // 2. Change Cymbal settings while B8 is potentially still ringing
        let mut cymbal_settings = VoiceSettings::cymbal();
        cymbal_settings.decay = 1.0;
        cymbal_settings.volume = 0.5;
        synth.set_voice_settings(cy_idx, cymbal_settings);

        // 3. Trigger B8 again and verify it STILL produces sound
        synth.trigger(b8_idx, 1.0);
        let out2 = sum_voice_output(&mut synth, b8_idx, 100);
        assert!(
            out2 > 0.0,
            "B8 should still produce sound after Cymbal set_voice_settings: {}",
            out2
        );
    }

    #[test]
    fn bassdrum808_produces_sound_after_all_voice_settings_change() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let b8_idx = DrumVoice::BassDrum808 as usize;

        // 1. Trigger B8 and verify it produces sound
        synth.trigger(b8_idx, 1.0);
        let out1 = sum_voice_output(&mut synth, b8_idx, 100);
        assert!(
            out1 > 0.0,
            "B8 should produce sound on first trigger: {}",
            out1
        );

        // 2. Call set_voice_settings on ALL voices (like lib.rs does when sound_settings_state.version changes)
        let all_settings = [
            (DrumVoice::Kick, VoiceSettings::kick()),
            (DrumVoice::Snare, VoiceSettings::snare()),
            (DrumVoice::HiHat, VoiceSettings::hihat()),
            (DrumVoice::OpenHiHat, VoiceSettings::open_hihat()),
            (DrumVoice::Tom1, VoiceSettings::tom1()),
            (DrumVoice::Tom2, VoiceSettings::tom2()),
            (DrumVoice::Tom3, VoiceSettings::tom3()),
            (DrumVoice::Clap, VoiceSettings::clap()),
            (DrumVoice::Ride, VoiceSettings::ride()),
            (DrumVoice::Cymbal, VoiceSettings::cymbal()),
            (DrumVoice::Snare606, VoiceSettings::snare606()),
            (DrumVoice::BassDrum808, VoiceSettings::kick808()),
            (DrumVoice::Perc1, VoiceSettings::perc1()),
        ];

        for (voice, settings) in all_settings.iter() {
            synth.set_voice_settings(*voice as usize, *settings);
        }

        // 3. Trigger B8 again and verify it STILL produces sound
        synth.trigger(b8_idx, 1.0);
        let out2 = sum_voice_output(&mut synth, b8_idx, 100);
        assert!(
            out2 > 0.0,
            "B8 should still produce sound after all set_voice_settings: {}",
            out2
        );
    }

    #[test]
    fn bassdrum808_does_not_go_silent_when_cymbal_triggered_and_settings_changed() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let b8_idx = DrumVoice::BassDrum808 as usize;
        let cy_idx = DrumVoice::Cymbal as usize;

        // Trigger Cymbal first
        synth.trigger(cy_idx, 1.0);

        // Trigger B8 while Cymbal is ringing
        synth.trigger(b8_idx, 1.0);
        let out1 = sum_voice_output(&mut synth, b8_idx, 50);
        assert!(
            out1 > 0.0,
            "B8 should produce sound when triggered alongside Cymbal: {}",
            out1
        );

        // Now modify Cymbal settings while both are active
        let mut cymbal_settings = VoiceSettings::cymbal();
        cymbal_settings.decay = 0.5;
        cymbal_settings.filter_freq = 6000.0;
        synth.set_voice_settings(DrumVoice::Cymbal as usize, cymbal_settings);

        // Trigger B8 again
        synth.trigger(b8_idx, 1.0);
        let out2 = sum_voice_output(&mut synth, b8_idx, 100);
        assert!(
            out2 > 0.0,
            "B8 should still produce sound after Cymbal settings changed while active: {}",
            out2
        );
    }

    /// Helper: checks whether any sample in the output is NaN.
    fn contains_nan(synth: &mut DrumSynthesizer, voice_idx: usize, samples: usize) -> bool {
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for _ in 0..samples {
            synth.process_voice_samples_stereo(&mut outputs);
            if outputs[voice_idx][0].is_nan() || outputs[voice_idx][1].is_nan() {
                return true;
            }
        }
        false
    }

    #[test]
    fn bassdrum808_never_produces_nan_after_cymbal_or_all_settings_changed() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let b8_idx = DrumVoice::BassDrum808 as usize;

        // Trigger B8
        synth.trigger(b8_idx, 1.0);
        assert!(
            !contains_nan(&mut synth, b8_idx, 10),
            "B8 should not produce NaN initially"
        );

        // Change Cymbal with extreme values
        let mut cy_settings = VoiceSettings::cymbal();
        cy_settings.attack = 0.0;
        cy_settings.decay = 0.001;
        cy_settings.release = 0.001;
        cy_settings.decay_curve = 0.1;
        cy_settings.release_curve = 0.1;
        synth.set_voice_settings(DrumVoice::Cymbal as usize, cy_settings);

        // Trigger B8 again
        synth.trigger(b8_idx, 1.0);
        assert!(
            !contains_nan(&mut synth, b8_idx, 200),
            "B8 should not produce NaN after Cymbal extreme settings"
        );

        // Now change ALL voices with edge-case values while B8 is active
        let edge = VoiceSettings {
            frequency: 20.0,
            decay: 0.001,
            volume: 1.5,
            filter_freq: 20000.0,
            attack: 0.0,
            release: 0.001,
            decay_curve: 0.1,
            release_curve: 0.1,
            hold: 0.0,
            filter_env_amount: 1.0,
            filter_env_decay: 0.001,
            analog: 0.0,
            stereo: 1.0,
            algo: 1,
            special: [1.0; 32],
        };

        for voice in [
            DrumVoice::Kick,
            DrumVoice::Snare,
            DrumVoice::HiHat,
            DrumVoice::OpenHiHat,
            DrumVoice::Tom1,
            DrumVoice::Tom2,
            DrumVoice::Tom3,
            DrumVoice::Clap,
            DrumVoice::Ride,
            DrumVoice::Cymbal,
            DrumVoice::Snare606,
            DrumVoice::BassDrum808,
            DrumVoice::Perc1,
        ]
        .iter()
        {
            synth.set_voice_settings(*voice as usize, edge);
        }

        synth.trigger(b8_idx, 1.0);
        assert!(
            !contains_nan(&mut synth, b8_idx, 200),
            "B8 should not produce NaN after all edge-case settings"
        );
    }

    #[test]
    fn bassdrum808_stays_audible_when_settings_changed_mid_envelope() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let b8_idx = DrumVoice::BassDrum808 as usize;

        // Trigger B8 and let it reach the middle of its decay
        synth.trigger(b8_idx, 1.0);
        let _ = sum_voice_output(&mut synth, b8_idx, 500); // ~11ms into decay

        // Now change Cymbal settings (should not affect B8)
        let mut cy = VoiceSettings::cymbal();
        cy.decay = 3.0;
        cy.volume = 0.2;
        synth.set_voice_settings(DrumVoice::Cymbal as usize, cy);

        // Continue processing B8 tail
        let tail = sum_voice_output(&mut synth, b8_idx, 500);
        assert!(
            tail > 0.0,
            "B8 tail should remain audible after Cymbal set_voice_settings: {}",
            tail
        );

        // Trigger B8 again after the tail
        synth.trigger(b8_idx, 1.0);
        let after = sum_voice_output(&mut synth, b8_idx, 100);
        assert!(
            after > 0.0,
            "B8 retrigger should still produce sound: {}",
            after
        );
    }

    #[test]
    fn modular_default_layout_initializes_slot_4_as_tom_not_open_hihat() {
        let mut synth = DrumSynthesizer::new();
        let layout = crate::track::TrackLayoutState::default_layout();

        synth.initialize_with_layout(44100.0, &layout);

        assert!(matches!(
            synth.voices[3].as_deref(),
            Some(DrumVoiceKind::Tom(_))
        ));
    }

    /// Test the specific attack_time=0 envelope corruption scenario.
    /// If attack_time is set to 0 while attack_remaining > 0, the envelope
    /// value becomes -inf. On the next trigger with normal attack_time,
    /// the envelope value becomes NaN and stays NaN forever.
    #[test]
    fn bassdrum808_attack_time_zero_during_attack_causes_nan() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let b8_idx = DrumVoice::BassDrum808 as usize;

        // 1. Trigger B8 with normal attack
        synth.trigger(b8_idx, 1.0);
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        // Process just a few samples (still in attack ramp)
        for _ in 0..5 {
            synth.process_voice_samples_stereo(&mut outputs);
        }
        assert!(
            outputs[b8_idx][0].is_finite(),
            "B8 should produce finite output during attack"
        );

        // 2. WHILE B8 is in attack phase, change its attack to 0.0
        // This simulates what would happen if set_voice_settings is called
        // with attack=0 while the envelope is attacking.
        let mut b8_zero_attack = VoiceSettings::kick808();
        b8_zero_attack.attack = 0.0;
        synth.set_voice_settings(DrumVoice::BassDrum808 as usize, b8_zero_attack);

        // Process a few more samples — the envelope should now be corrupted
        for _ in 0..10 {
            synth.process_voice_samples_stereo(&mut outputs);
        }

        // 3. Now set attack back to normal and trigger again
        let mut b8_normal = VoiceSettings::kick808();
        b8_normal.attack = 0.0015;
        synth.set_voice_settings(DrumVoice::BassDrum808 as usize, b8_normal);
        synth.trigger(b8_idx, 1.0);

        // 4. Check if B8 is permanently silent (NaN or 0)
        let mut has_finite_output = false;
        for _ in 0..100 {
            synth.process_voice_samples_stereo(&mut outputs);
            if outputs[b8_idx][0].is_finite() && outputs[b8_idx][0].abs() > 0.0001 {
                has_finite_output = true;
                break;
            }
        }
        assert!(
            has_finite_output,
            "B8 should recover and produce finite output after attack corruption"
        );
    }

    /// Regression test for the "plock click" bug.
    ///
    /// Scenario: lib.rs used to call `set_voice_settings` inside `iter_samples`,
    /// which meant a global settings update could overwrite a per-step plock
    /// in the middle of a buffer — producing a one-sample discontinuity.
    ///
    /// This test reproduces that exact situation:
    ///   1. Trigger kick at 60 Hz
    ///   2. Process 500 samples (tail develops)
    ///   3. `set_voice_settings` to 200 Hz (simulates the mid-buffer overwrite)
    ///   4. Process 200 more samples
    ///   5. Measure the spectral flux around the change point.
    ///
    /// With the old code (FREQ_SMOOTH_MS = 0.1 ms, no filter cutoff smoother)
    /// this produced a sharp HF spike (click).  With the fix it should be smooth.
    #[test]
    fn kick_no_click_when_settings_changed_mid_envelope() {
        use std::f32::consts::PI;

        let sample_rate = 44100.0;
        let mut synth = DrumSynthesizer::new();
        synth.initialize(sample_rate);

        let kick_idx = DrumVoice::Kick as usize;

        // Settings A: low frequency
        let mut settings_a = VoiceSettings::kick();
        settings_a.frequency = 60.0;
        settings_a.special[0] = 0.0; // disable click transient to isolate body
        settings_a.filter_freq = 2000.0;
        settings_a.filter_env_amount = 0.0;
        settings_a.decay = 0.3;
        settings_a.analog = 1.0;

        // Settings B: high frequency (the plock value)
        let mut settings_b = settings_a;
        settings_b.frequency = 200.0;

        // 1. Trigger at 60 Hz
        synth.set_voice_settings(DrumVoice::Kick as usize, settings_a);
        synth.trigger(kick_idx, 1.0);

        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        let mut samples: Vec<f32> = Vec::with_capacity(700);

        // 2. Process 500 samples
        for _ in 0..500 {
            synth.process_voice_samples_stereo(&mut outputs);
            samples.push(outputs[kick_idx][0]);
        }

        // 3. MID-ENVELOPE SETTINGS CHANGE — this is the bug scenario
        synth.set_voice_settings(DrumVoice::Kick as usize, settings_b);

        // 4. Process 200 more samples
        for _ in 0..200 {
            synth.process_voice_samples_stereo(&mut outputs);
            samples.push(outputs[kick_idx][0]);
        }

        // 5. Detect click: compute high-frequency energy in a 10 ms window
        //    centred on the change point (sample 500).
        let window = (sample_rate * 0.01) as usize; // 10 ms
        let start = 500usize.saturating_sub(window / 2);
        let end = (500 + window / 2).min(samples.len());

        // Simple 1-pole HP @ 3 kHz
        let mut hp_state = 0.0_f32;
        let alpha = 1.0 - (-2.0 * PI * 3000.0 / sample_rate).exp();
        let mut energy_low = 0.0_f32;
        let mut energy_high = 0.0_f32;
        for s in &samples[start..end] {
            energy_low += s * s;
            hp_state += alpha * (s - hp_state);
            let hp = s - hp_state;
            energy_high += hp * hp;
        }
        let hf_ratio = if energy_low > 0.0 {
            energy_high / energy_low
        } else {
            0.0
        };

        // Also compute maximum sample-to-sample delta in the window
        let mut max_delta = 0.0_f32;
        for i in start..end.saturating_sub(1) {
            max_delta = max_delta.max((samples[i + 1] - samples[i]).abs());
        }

        eprintln!("\n=== kick_no_click_when_settings_changed_mid_envelope ===");
        eprintln!("HF ratio around change: {}", hf_ratio);
        eprintln!("Max sample delta:       {}", max_delta);

        // With the fix the HF ratio should stay below 0.05 and delta below 0.05
        // (empirically: with FREQ_SMOOTH_MS=0.1ms, hf_ratio ~0.15 and delta ~0.25)
        assert!(
            hf_ratio < 0.08,
            "Click detected: HF energy spike at settings change. hf_ratio={}",
            hf_ratio
        );
        assert!(
            max_delta < 0.08,
            "Click detected: large sample delta at settings change. max_delta={}",
            max_delta
        );
    }

    /// [254] Real-time safety net: playing any kind — trigger, retrigger on a
    /// ringing tail, settings pushed mid-play — must not allocate. Voices are
    /// preallocated at initialize; an allocation here is a dropout in the DAW.
    /// This is the test that would have caught the two RT defects of the
    /// 2026-09-24 audit ([244] pattern-bank save, [253] texture drop).
    #[test]
    fn every_kind_plays_without_allocations() {
        fn default_settings_for(kind: crate::track::TrackInstrumentKind) -> VoiceSettings {
            use crate::track::TrackInstrumentKind as K;
            match kind {
                K::Kick => VoiceSettings::kick(),
                K::Snare => VoiceSettings::snare(),
                K::HiHat => VoiceSettings::hihat(),
                K::OpenHiHat => VoiceSettings::open_hihat(),
                K::Tom => VoiceSettings::tom1(),
                K::Clap => VoiceSettings::clap(),
                K::Ride => VoiceSettings::ride(),
                K::Cymbal => VoiceSettings::cymbal(),
                K::Snare606 => VoiceSettings::snare606(),
                K::BassDrum808 => VoiceSettings::kick808(),
                K::Perc1 => VoiceSettings::perc1(),
                K::Bd6smp => VoiceSettings::bd606(),
                K::Sd6smp => VoiceSettings::sd606(),
                K::Ch6smp => VoiceSettings::ch606(),
                K::Buzz => VoiceSettings::buzz(),
                K::Sdrex => VoiceSettings::sdrex(),
                K::Bd6Ac => VoiceSettings::bd6ac(),
                K::Sd6Ac => VoiceSettings::sd6ac(),
                K::Hh6Ac => VoiceSettings::hh6ac(),
                K::Oh6Ac => VoiceSettings::oh6ac(),
                K::Cl6Ac => VoiceSettings::cl6ac(),
                K::Tm6Ac => VoiceSettings::tm6ac(),
                K::Oh6smp => VoiceSettings::oh606(),
                K::Rift => VoiceSettings::rift(),
                K::OneShot => VoiceSettings::oneshot(),
                K::Reverb => VoiceSettings::reverb(),
                K::Delay => VoiceSettings::delay(),
                K::Rumble => VoiceSettings::rumble(),
            }
        }

        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for kind in crate::track::TrackInstrumentKind::ALL {
            let mut layout = crate::track::TrackLayoutState::empty_layout();
            layout.slots[0] = crate::track::TrackSlot::active_with_kind(kind);
            let mut synth = DrumSynthesizer::new();
            // Non-RT: preallocates voices and prewarms the embedded banks.
            synth.initialize_with_layout(48000.0, &layout);

            assert_no_alloc::assert_no_alloc(|| {
                synth.trigger(0, 1.0);
                for _ in 0..4800 {
                    synth.process_voice_samples_stereo(&mut outputs);
                }
                // Retrigger on a ringing tail, then push settings mid-play.
                synth.trigger(0, 0.8);
                synth.set_voice_settings(0, default_settings_for(kind));
                for _ in 0..4800 {
                    synth.process_voice_samples_stereo(&mut outputs);
                }
            });
        }
    }

    /// Lightweight sanity check: every engine voice renders finite, non-silent audio
    /// with its default settings. This catches global regressions (silence, NaN, stereo
    /// collapse) after refactors like per-slot instances or special-param changes.
    #[test]
    fn all_voices_render_finite_non_silent_output() {
        const SAMPLES: usize = 200; // ~4.5 ms @ 44100 Hz — enough to catch silence/NaN
        const MIN_PEAK: f32 = 1e-4;

        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let default_settings: [(DrumVoice, fn() -> VoiceSettings); 13] = [
            (DrumVoice::Kick, VoiceSettings::kick),
            (DrumVoice::Snare, VoiceSettings::snare),
            (DrumVoice::HiHat, VoiceSettings::hihat),
            (DrumVoice::OpenHiHat, VoiceSettings::open_hihat),
            (DrumVoice::Tom1, VoiceSettings::tom1),
            (DrumVoice::Tom2, VoiceSettings::tom2),
            (DrumVoice::Tom3, VoiceSettings::tom3),
            (DrumVoice::Clap, VoiceSettings::clap),
            (DrumVoice::Ride, VoiceSettings::ride),
            (DrumVoice::Cymbal, VoiceSettings::cymbal),
            (DrumVoice::Snare606, VoiceSettings::snare606),
            (DrumVoice::BassDrum808, VoiceSettings::kick808),
            (DrumVoice::Perc1, VoiceSettings::perc1),
        ];

        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for (voice, settings_fn) in default_settings.iter() {
            let voice_idx = *voice as usize;
            synth.set_voice_settings(voice_idx, settings_fn());
            synth.trigger(voice_idx, 1.0);

            let mut peak = 0.0f32;
            for _ in 0..SAMPLES {
                synth.process_voice_samples_stereo(&mut outputs);
                let left = outputs[voice_idx][0];
                let right = outputs[voice_idx][1];
                assert!(
                    left.is_finite() && right.is_finite(),
                    "{:?} produced non-finite samples (NaN/Inf) with default settings",
                    voice
                );
                peak = peak.max(left.abs()).max(right.abs());
            }

            assert!(
                peak >= MIN_PEAK,
                "{:?} is silent with default settings (peak = {})",
                voice,
                peak
            );
        }
    }

    /// Regression guard: retriggering any voice twice in quick succession must stay
    /// finite. Some bugs only appear on the second trigger (phase/envelope state).
    #[test]
    fn all_voices_stay_finite_on_retrigger() {
        const SAMPLES: usize = 100;

        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        let voices = [
            DrumVoice::Kick,
            DrumVoice::Snare,
            DrumVoice::HiHat,
            DrumVoice::OpenHiHat,
            DrumVoice::Tom1,
            DrumVoice::Tom2,
            DrumVoice::Tom3,
            DrumVoice::Clap,
            DrumVoice::Ride,
            DrumVoice::Cymbal,
            DrumVoice::Snare606,
            DrumVoice::BassDrum808,
            DrumVoice::Perc1,
        ];

        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for voice in voices.iter() {
            let voice_idx = *voice as usize;
            synth.trigger(voice_idx, 1.0);
            // First trigger
            for _ in 0..SAMPLES {
                synth.process_voice_samples_stereo(&mut outputs);
            }
            // Immediate retrigger while tail may be ringing
            synth.trigger(voice_idx, 1.0);
            for _ in 0..SAMPLES {
                synth.process_voice_samples_stereo(&mut outputs);
                let left = outputs[voice_idx][0];
                let right = outputs[voice_idx][1];
                assert!(
                    left.is_finite() && right.is_finite(),
                    "{:?} produced non-finite sample on retrigger",
                    voice
                );
            }
        }
    }

    /// Regression guard for [AUDIT-RT1]: activating an inactive slot must not
    /// allocate on the audio thread. All slots are preallocated at init, so
    /// reinitializing an inactive slot only replaces the existing enum variant.
    #[test]
    fn reinitialize_inactive_slot_reuses_preallocated_voice() {
        use crate::track::{TrackInstrumentKind, TrackLayoutState, TrackSlot};

        let mut synth = DrumSynthesizer::new();
        let mut layout = TrackLayoutState::default_layout();
        // Ensure slot 13 is inactive and has no dedicated legacy voice.
        layout.slots[13] = TrackSlot::inactive();
        synth.initialize_with_layout(44100.0, &layout);
        assert!(
            !synth.active[13],
            "slot 13 should be inactive after init with default layout"
        );
        assert!(
            synth.voices[13].is_some(),
            "slot 13 should still have a preallocated placeholder voice"
        );

        // Activate slot 13 with a Kick — this must not allocate a new Box.
        synth.reinitialize_slot(13, TrackInstrumentKind::Kick);
        assert!(synth.active[13]);
        assert!(
            matches!(synth.voices[13].as_deref(), Some(DrumVoiceKind::Kick(_))),
            "slot 13 should now hold a Kick voice"
        );

        synth.trigger(13, 1.0);
        let out = sum_voice_output(&mut synth, 13, 100);
        assert!(out > 0.0, "reinitialized slot 13 should produce sound");
    }

    /// Ensure that deactivating a slot via `set_slot_active` prevents triggers.
    #[test]
    fn set_slot_active_gates_trigger_and_output() {
        let mut synth = DrumSynthesizer::new();
        synth.initialize(44100.0);

        synth.set_slot_active(0, false);
        synth.trigger(0, 1.0);
        let out = sum_voice_output(&mut synth, 0, 100);
        assert_eq!(out, 0.0, "deactivated slot should remain silent");

        synth.set_slot_active(0, true);
        synth.trigger(0, 1.0);
        let out = sum_voice_output(&mut synth, 0, 100);
        assert!(out > 0.0, "reactivated slot should play again");
    }
}
