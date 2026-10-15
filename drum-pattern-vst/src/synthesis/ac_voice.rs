//! AC606 voices — the six Flash Drum instruments wrapping the ported
//! analogcode "606-Inspired-Synth-Drums" engines (MIT, (c) 2026 Matthew
//! Fecher — see `ac606/LICENSE-MIT.txt` and the About section in Settings).
//!
//! One struct covers all six kinds; a per-kind config maps our standard
//! params onto each engine's fitted knobs:
//! - **Frequency/Tone** → pitch ratio (1.0 = the fitted sound, at each
//!   kind's default frequency),
//! - **Decay** (seconds) → the engine's normalized decay, where 100 % is the
//!   fitted full length,
//! - **Attack** (BD6(AC) only) → transient amount (10 ms+ = max),
//! - **Analog** → per-hit pitch jitter (the engines' noise/phases already
//!   make every hit unique),
//! - **Volume** → final level (post-saturation, invariant).
//!
//! [196] The kind-specific `special[]` slots expose the engines' internals
//! for sound exploration — defaults are the fitted values, so the original
//! sound sits in the middle of every range (see `mod bd` / `sd` / `hh` /
//! `cl` / `tm` below for the index maps). The saturation pack lives at
//! indices `SAT_*` on every kind.
//!
//! Anti-click: the engines restart from a fresh fitted state on every hit,
//! cutting any ringing tail — `dsp::RetrigDeclick` fades the cut (same
//! contract as Kick/BD808 [179]).

use super::{ac606, dsp, saturation, settings::ac_voice::AcVoiceSettings, Voice, VoiceSettings};

/// BD6(AC) special indices.
pub mod bd {
    pub const SWEEP: usize = 1;
    pub const BEND: usize = 2;
    pub const CLICK: usize = 3;
    pub const CLICK_TONE: usize = 4;
    pub const PUNCH: usize = 5;
    // index 6 was Punch Decay, removed [196b] (near-inaudible) — slot left inert.
    pub const TONE: usize = 7;
    pub const DRIVE: usize = 8;
}

/// SD6(AC) special indices.
pub mod sd {
    pub const SNAP: usize = 0;
    pub const WIRE_COLOR: usize = 1;
    pub const SHELL_BEND: usize = 2;
    pub const IMPACT: usize = 3;
    pub const RING: usize = 4;
}

/// HH6(AC) / OH6(AC) special indices.
pub mod hh {
    pub const METAL: usize = 0;
    pub const CLICK: usize = 1;
    pub const BELL: usize = 2;
    pub const WOBBLE: usize = 3;
    pub const SPREAD: usize = 4;
    pub const BRIGHTNESS: usize = 5;
}

/// CL6(AC) special indices.
pub mod cl {
    pub const NOISE: usize = 0;
    pub const SPREAD: usize = 1;
    pub const TAIL: usize = 2;
    pub const AIR: usize = 3;
}

/// TM6(AC) special indices.
pub mod tm {
    pub const MODEL: usize = 0;
    pub const STRIKE: usize = 1;
    pub const SNAP: usize = 2;
    pub const GLIDE: usize = 3;
    pub const MODES: usize = 4;
    pub const TAIL_NOISE: usize = 5;
}

/// Saturation pack indices (shared by the six kinds). No Pre-Filter slot:
/// the AC voices have no filter stage to route around.
pub const SAT_TYPE: usize = 10;
pub const SAT_AMOUNT: usize = 11;
pub const SAT_MIX: usize = 12;
pub const SAT_GAIN: usize = 13;

/// Which ported engine this voice runs, with its fitted reference points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AcEngineKind {
    BassDrum,
    Snare,
    ClosedHat,
    OpenHat,
    Clap,
    Tom,
}

impl AcEngineKind {
    /// Frequency at which the engine sits at its fitted tuning (ratio 1.0).
    const fn ref_hz(self) -> f32 {
        match self {
            // BD: the fitted landing note is ~57.5 Hz — we reference the
            // engine's semitone knob instead (see trigger).
            Self::BassDrum => 57.5,
            Self::Snare => 201.09442,
            Self::ClosedHat | Self::OpenHat => 8000.0,
            Self::Clap => 1000.0,
            // TM: referenced per spec (124.435 / 208 Hz) in trigger.
            Self::Tom => 1.0,
        }
    }

    /// Decay (seconds) that maps to the engine's full fitted length (100 %).
    const fn decay_ref(self) -> f32 {
        match self {
            // BD: the knob is a shape control, not seconds — see trigger.
            Self::BassDrum => 2.0,
            Self::Snare => 0.370,
            Self::ClosedHat => 0.220,
            Self::OpenHat => 1.766,
            Self::Clap => 0.301,
            Self::Tom => 0.45,
        }
    }
}

enum AcEngine {
    BassDrum(ac606::AcBassDrum),
    Snare(ac606::AcSnare),
    Hat(ac606::AcMetalHat),
    Clap(ac606::AcClap),
    Tom(ac606::AcTom),
}

pub struct AcVoice {
    kind: AcEngineKind,
    settings: AcVoiceSettings,
    engine: AcEngine,
    saturation: saturation::SaturationConfig,
    declick: dsp::RetrigDeclick,
    last_out: f32,
    drift: dsp::AnalogDrift,
    active: bool,
}

impl AcVoice {
    pub fn new(sample_rate: f32, kind: AcEngineKind, settings: AcVoiceSettings) -> Self {
        let engine = match kind {
            AcEngineKind::BassDrum => {
                AcEngine::BassDrum(ac606::AcBassDrum::new(sample_rate, 0x606606))
            }
            AcEngineKind::Snare => AcEngine::Snare(ac606::AcSnare::new(sample_rate, 0x6063)),
            AcEngineKind::ClosedHat => {
                AcEngine::Hat(ac606::AcMetalHat::new(sample_rate, 0x60C6_6066))
            }
            AcEngineKind::OpenHat => {
                AcEngine::Hat(ac606::AcMetalHat::new(sample_rate, 0x6066_0660))
            }
            AcEngineKind::Clap => AcEngine::Clap(ac606::AcClap::new(sample_rate, 0x0606_C1A9)),
            AcEngineKind::Tom => AcEngine::Tom(ac606::AcTom::new(sample_rate, 0x6061)),
        };
        let mut voice = Self {
            kind,
            settings,
            engine,
            saturation: saturation::SaturationConfig {
                saturation_type: saturation::SaturationType::None,
                amount: 0.0,
                mix: 1.0,
                output_gain: 1.0,
                pre_filter: false,
                compensation_gain: 1.0,
            },
            declick: dsp::RetrigDeclick::new(sample_rate),
            last_out: 0.0,
            drift: dsp::AnalogDrift::new(0xAC60_6006),
            active: false,
        };
        voice.update_saturation();
        voice
    }

    /// Slider → multiplier for the "x0..x2 with fitted in the middle" knobs.
    fn times2(v: f32) -> f32 {
        v.clamp(0.0, 1.0) * 2.0
    }

    /// A knob whose centre is the fitted sound (x1) and whose right half climbs
    /// to x`top`: for levels so small in the fitted spec that a plain x2 is
    /// inaudible ([292], TM6 Modes and Tail Noise).
    fn boost(v: f32, top: f32) -> f32 {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.5 {
            v * 2.0
        } else {
            1.0 + (v - 0.5) * 2.0 * (top - 1.0)
        }
    }

    fn update_saturation(&mut self) {
        let s = &self.settings.special;
        self.saturation.saturation_type = saturation::SaturationType::from(s[SAT_TYPE] as u8);
        self.saturation.amount = s[SAT_AMOUNT];
        self.saturation.mix = s[SAT_MIX];
        self.saturation.output_gain = s[SAT_GAIN];
        self.saturation.pre_filter = false;
        self.saturation.update_compensation();
    }
}

impl Voice for AcVoice {
    fn trigger(&mut self) {
        self.declick.arm(self.last_out);
        self.active = true;
        self.drift.trigger(self.settings.analog >= 0.5);
        let s = &self.settings;
        let sp = &s.special;
        let freq = s.frequency.max(1.0);
        match &mut self.engine {
            AcEngine::BassDrum(e) => {
                // Frequency → semitones vs the fitted landing note; analog
                // drift → per-hit pitch jitter in semitones.
                let semis =
                    (12.0 * (freq / AcEngineKind::BassDrum.ref_hz()).log2()).clamp(-24.0, 24.0);
                let jitter_semis = 12.0 * self.drift.pitch.log2();
                // The engine's body amp decay is 0.22 + d*0.12 s with
                // d = lerpf(-1.75, 3.0, pct^2) — solve so the slider reads as
                // the decay time it produces.
                let d = (s.decay - 0.22) / 0.12;
                let decay_pct = ((d + 1.75) / 4.75).clamp(0.0, 1.0).sqrt();
                let mods = ac606::BdMods {
                    click: sp[bd::CLICK],
                    click_tone_hz: sp[bd::CLICK_TONE],
                    punch: sp[bd::PUNCH],
                    tone: sp[bd::TONE],
                    drive: sp[bd::DRIVE],
                    sweep: sp[bd::SWEEP],
                    bend: sp[bd::BEND],
                    decay_curve: s.decay_curve,
                };
                e.trigger(decay_pct, semis, jitter_semis, &mods);
            }
            AcEngine::Snare(e) => {
                let ratio =
                    (freq / AcEngineKind::Snare.ref_hz() * self.drift.pitch).clamp(0.25, 4.0);
                let decay_pct = (s.decay / AcEngineKind::Snare.decay_ref()).clamp(0.01, 1.0);
                let mods = ac606::SnareMods {
                    shell_bend: Self::times2(sp[sd::SHELL_BEND]),
                    impact: Self::times2(sp[sd::IMPACT]),
                    ring: Self::times2(sp[sd::RING]),
                };
                e.trigger(
                    decay_pct,
                    ratio,
                    sp[sd::SNAP].clamp(0.0, 1.0),
                    sp[sd::WIRE_COLOR].clamp(0.25, 4.0),
                    &mods,
                );
            }
            AcEngine::Hat(e) => {
                let ratio = (freq / self.kind.ref_hz() * self.drift.pitch).clamp(1.0 / 16.0, 16.0);
                let decay_pct = (s.decay / self.kind.decay_ref()).clamp(0.0, 1.0);
                let spec = if self.kind == AcEngineKind::ClosedHat {
                    &ac606::CLOSED_HAT_SPEC
                } else {
                    &ac606::OPEN_HAT_SPEC
                };
                let mods = ac606::HatMods {
                    metal: sp[hh::METAL],
                    click: sp[hh::CLICK],
                    bell: sp[hh::BELL],
                    wobble: sp[hh::WOBBLE] * 4.0,
                    spread: sp[hh::SPREAD],
                    brightness: sp[hh::BRIGHTNESS],
                };
                e.trigger(spec, decay_pct, ratio, &mods);
            }
            AcEngine::Clap(e) => {
                let ratio = (freq / AcEngineKind::Clap.ref_hz() * self.drift.pitch).clamp(0.5, 2.0);
                let decay_pct = (s.decay / AcEngineKind::Clap.decay_ref()).clamp(0.05, 1.0);
                e.trigger(
                    decay_pct,
                    ratio,
                    sp[cl::NOISE].clamp(0.0, 1.0),
                    sp[cl::SPREAD].clamp(0.25, 3.0),
                    Self::times2(sp[cl::TAIL]),
                    sp[cl::AIR].clamp(0.0, 1.0),
                );
            }
            AcEngine::Tom(e) => {
                // Model: 1 = the Low tom, 2 = the High tom (mains 124.435 and
                // 208 Hz). 0 is the retired Auto (sessions saved before [292]):
                // `seed_late_specials` converts it; until then it still picks by
                // pitch, as it always did.
                let low = match sp[tm::MODEL].round() as i32 {
                    1 => true,
                    2 => false,
                    _ => freq < 166.0,
                };
                let spec = if low {
                    &ac606::LOW_TOM_SPEC
                } else {
                    &ac606::HIGH_TOM_SPEC
                };
                // [292] The two models are pushed apart on top of their fitted
                // specs: Low = long, booming, with a pitch fall and a purer
                // body; High = short, tight, clicky and ringing.
                let (decay_x, strike_x, snap_x, glide_x, modes_x) = if low {
                    (1.35, 1.2, 1.0, 1.5, 0.6)
                } else {
                    (0.7, 0.8, 1.6, 1.0, 1.5)
                };
                let ratio = (freq / spec.main_hz * self.drift.pitch).clamp(0.25, 4.0);
                let decay_pct =
                    (s.decay / AcEngineKind::Tom.decay_ref() * decay_x).clamp(0.05, 5.0);
                let mods = ac606::TomMods {
                    strike: Self::times2(sp[tm::STRIKE]) * strike_x,
                    snap: Self::times2(sp[tm::SNAP]) * snap_x,
                    glide: Self::times2(sp[tm::GLIDE]) * glide_x,
                    modes: Self::boost(sp[tm::MODES], 60.0) * modes_x,
                    tail_noise: Self::boost(sp[tm::TAIL_NOISE], 200.0),
                };
                e.trigger(spec, decay_pct, ratio, &mods);
            }
        }
    }

    /// Machine-gun retrigger: identical to a normal trigger (fresh state +
    /// declick), same contract as Kick [179].
    fn trigger_hard(&mut self) {
        self.trigger();
    }

    fn process_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let raw = match &mut self.engine {
            AcEngine::BassDrum(e) => e.process(),
            AcEngine::Snare(e) => e.process(),
            AcEngine::Hat(e) => e.process(),
            AcEngine::Clap(e) => e.process(),
            AcEngine::Tom(e) => e.process(),
        };
        let engine_active = match &self.engine {
            AcEngine::BassDrum(e) => e.is_active(),
            AcEngine::Snare(e) => e.is_active(),
            AcEngine::Hat(e) => e.is_active(),
            AcEngine::Clap(e) => e.is_active(),
            AcEngine::Tom(e) => e.is_active(),
        };
        if !engine_active && !self.declick.is_active() {
            self.active = false;
        }
        // Shared saturation chain, applied once: the AC voices have no filter
        // stage, so the pre/post routing of the other voices does not apply
        // (and no Pre-Filter param is exposed for these kinds).
        let wet = self.saturation.process(raw);
        let out = wet * self.drift.level * self.settings.volume + self.declick.next();
        self.last_out = out;
        out
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn reset(&mut self) {
        self.active = false;
        match &mut self.engine {
            AcEngine::BassDrum(e) => e.reset(),
            AcEngine::Snare(e) => e.reset(),
            AcEngine::Hat(e) => e.reset(),
            AcEngine::Clap(e) => e.reset(),
            AcEngine::Tom(e) => e.reset(),
        }
        self.declick.reset();
        self.last_out = 0.0;
    }

    fn set_settings(&mut self, settings: VoiceSettings) {
        self.settings = AcVoiceSettings::from(settings);
        self.update_saturation();
    }

    fn set_algo(&mut self, algo: u8) {
        self.settings.algo = algo;
    }

    fn set_special_param(&mut self, index: usize, value: f32) {
        if index < self.settings.special.len() {
            self.settings.special[index] = value;
        }
        if (SAT_TYPE..=SAT_GAIN).contains(&index) {
            self.update_saturation();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tom_tail(model: f32) -> f32 {
        let mut settings = VoiceSettings::tm6ac();
        settings.frequency = 150.0;
        settings.special[tm::MODEL] = model;
        let mut voice = AcVoice::new(44100.0, AcEngineKind::Tom, AcVoiceSettings::from(settings));
        voice.trigger();
        let out: Vec<f32> = (0..22050).map(|_| voice.process_sample()).collect();
        assert!(out.iter().all(|x| x.is_finite()));
        out[11025..].iter().map(|x| x * x).sum::<f32>()
    }

    /// Energy of the second half-second of a hit with `set` applied to the defaults.
    fn tom_energy(model: f32, window: std::ops::Range<usize>, set: impl Fn(&mut VoiceSettings)) -> f32 {
        let mut settings = VoiceSettings::tm6ac();
        settings.special[tm::MODEL] = model;
        settings.frequency = if model == 2.0 { 208.0 } else { 124.435 };
        set(&mut settings);
        let mut voice = AcVoice::new(44100.0, AcEngineKind::Tom, AcVoiceSettings::from(settings));
        voice.trigger();
        let out: Vec<f32> = (0..window.end).map(|_| voice.process_sample()).collect();
        assert!(out.iter().all(|x| x.is_finite()));
        out[window].iter().map(|x| x * x).sum::<f32>()
    }

    /// [292] Modes, Tail Noise and Decay must be audible across their whole range,
    /// on both models (they were inaudible or capped before).
    #[test]
    fn tom_modes_tail_noise_and_decay_all_act() {
        for model in [1.0f32, 2.0] {
            let window = 2000..22050;
            let base = tom_energy(model, window.clone(), |_| {});
            let modes = tom_energy(model, window.clone(), |s| s.special[tm::MODES] = 1.0);
            let tail = tom_energy(model, window.clone(), |s| s.special[tm::TAIL_NOISE] = 1.0);
            assert!(modes > base * 1.1, "model {model}: Modes at max {modes} vs centre {base}");
            assert!(tail > base * 1.1, "model {model}: Tail Noise at max {tail} vs centre {base}");
            // Decay past the former 0.45 s cap keeps lengthening the hit.
            let late = 30000..60000;
            let d_half = tom_energy(model, late.clone(), |s| s.decay = 0.5);
            let d_two = tom_energy(model, late.clone(), |s| s.decay = 2.0);
            assert!(d_two > d_half * 2.0, "model {model}: Decay 2 s {d_two} vs 0.5 s {d_half}");
        }
    }

    /// [292] At the same pitch, Low rings long and High is short: the model is a
    /// different drum, not a pitch offset.
    #[test]
    fn tom_models_differ_clearly_at_the_same_pitch() {
        let low = tom_tail(1.0);
        let high = tom_tail(2.0);
        assert!(
            low > high * 3.0,
            "Low tail {low} should clearly outlast High tail {high}"
        );
    }
}
