//! Super 606 Low & High Toms.
//!
//! Ported from `Toms.hpp` of analogcode's "606-Inspired-Synth-Drums"
//! — MIT License, Copyright (c) 2026 Matthew Fecher (see `LICENSE-MIT.txt`).
//!
//! A few quieter resonances around the main note instead of a single clean
//! sine. The low tom falls into pitch over its first few cycles, while the
//! high tom leaves a lower ring hanging behind the body. The lower ring was
//! kept separate because it hangs on after the main note; only the four
//! upper peaks that stayed visible in the tail were kept.

use super::common::{
    clampf, decay_coef_t60, flush_denormal, one_pole_coef, Biquad, Random, TWO_PI,
};

const TOM_SILENCE_THRESHOLD: f32 = 5.0e-6;

#[derive(Clone, Copy)]
pub struct TomMode {
    pub ratio: f32,
    pub level: f32,
    pub decay_t60_seconds: f32,
    pub start_phase: f32,
}

const fn mode(ratio: f32, level: f32, decay_t60_seconds: f32, start_phase: f32) -> TomMode {
    TomMode {
        ratio,
        level,
        decay_t60_seconds,
        start_phase,
    }
}

pub struct TomSpec {
    pub main_hz: f32,
    pub main_glide_hz: f32,
    pub main_glide_time_seconds: f32,
    pub main_decay_t60_seconds: f32,
    pub main_start_phase: f32,
    pub body_attack_seconds: f32,

    pub lower_mode_ratio: f32,
    pub lower_mode_level: f32,
    pub lower_mode_decay_t60_seconds: f32,
    pub lower_mode_start_phase: f32,

    pub second_harmonic_level: f32,
    pub third_harmonic_level: f32,

    pub strike_hz: f32,
    pub strike_level: f32,
    pub strike_decay_t60_seconds: f32,
    pub strike_start_phase: f32,
    pub strike_rise_seconds: f32,

    pub upper_modes: [TomMode; 4],

    pub snap_decay_t60_seconds: f32,
    pub burst_decay_t60_seconds: f32,
    pub tail_noise_decay_t60_seconds: f32,
    pub noise_rise_seconds: f32,

    pub low_snap_level: f32,
    pub low_burst_level: f32,
    pub low_high_pass_hz: f32,
    pub low_low_pass_hz: f32,

    pub high_snap_level: f32,
    pub high_burst_level: f32,
    pub high_high_pass_hz: f32,
    pub high_low_pass_hz: f32,

    pub focused_strike_level: f32,
    pub focused_strike_high_pass_hz: f32,
    pub focused_strike_low_pass_hz: f32,

    pub tail_noise_level: f32,
    pub tail_noise_high_pass_hz: f32,
    pub tail_noise_low_pass_hz: f32,

    pub output_trim: f32,
}

pub static HIGH_TOM_SPEC: TomSpec = TomSpec {
    main_hz: 208.0,
    main_glide_hz: 0.0,             // high tom stays on one note
    main_glide_time_seconds: 0.050, // unused when glide is zero
    main_decay_t60_seconds: 0.218,  // time to fall 60 dB
    main_start_phase: 2.674,
    body_attack_seconds: 0.00012,
    lower_mode_ratio: 135.616 / 208.0, // measured 135.616 Hz ring
    lower_mode_level: 0.090,
    lower_mode_decay_t60_seconds: 0.315,
    lower_mode_start_phase: 1.059,
    second_harmonic_level: 0.0095,
    third_harmonic_level: 0.0032,
    strike_hz: 285.0,
    strike_level: 0.71, // backed off so it joins the body
    strike_decay_t60_seconds: 0.0125,
    strike_start_phase: 0.420,
    strike_rise_seconds: 0.00003,
    upper_modes: [
        mode(305.2 / 208.0, 0.002617, 0.800, -2.987),
        mode(345.7 / 208.0, 0.001954, 0.800, 0.618),
        mode(367.0 / 208.0, 0.003381, 0.400, 1.002),
        mode(386.7 / 208.0, 0.003546, 0.360, 0.597),
    ],
    snap_decay_t60_seconds: 0.012,
    burst_decay_t60_seconds: 0.080,
    tail_noise_decay_t60_seconds: 0.300,
    noise_rise_seconds: 0.0015,
    low_snap_level: 0.500,
    low_burst_level: 0.030,
    low_high_pass_hz: 300.0,
    low_low_pass_hz: 3000.0,
    high_snap_level: 0.030,
    high_burst_level: 0.009,
    high_high_pass_hz: 700.0,
    high_low_pass_hz: 12000.0,
    focused_strike_level: 0.0, // low tom needs it but high does not
    focused_strike_high_pass_hz: 700.0,
    focused_strike_low_pass_hz: 1100.0,
    tail_noise_level: 0.010,
    tail_noise_high_pass_hz: 300.0,
    tail_noise_low_pass_hz: 1500.0,
    output_trim: 0.80,
};

// The low tom was fit on its own instead of pitching the high tom down.
// The body starts about 40 Hz high, then falls into the 124.435 Hz note.
// The narrow noise strike was added after the body was right so the hit had
// weight.
pub static LOW_TOM_SPEC: TomSpec = TomSpec {
    main_hz: 124.435,      // settled note in the Kit1 recording
    main_glide_hz: 39.921, // added at the start
    main_glide_time_seconds: 0.050622,
    main_decay_t60_seconds: 0.3276, // time to fall 60 dB
    main_start_phase: 2.2826,
    body_attack_seconds: 0.00012,
    lower_mode_ratio: 1.0, // unused here
    lower_mode_level: 0.0,
    lower_mode_decay_t60_seconds: 0.100,
    lower_mode_start_phase: 0.0,
    second_harmonic_level: 0.0095,
    third_harmonic_level: 0.0032,
    strike_hz: 367.0,
    strike_level: 1.05,
    strike_decay_t60_seconds: 0.005,
    strike_start_phase: 0.200,
    strike_rise_seconds: 0.00015,
    upper_modes: [
        mode(1.0, 0.0, 0.100, 0.0),
        mode(1.0, 0.0, 0.100, 0.0),
        mode(1.0, 0.0, 0.100, 0.0),
        mode(1.0, 0.0, 0.100, 0.0),
    ],
    snap_decay_t60_seconds: 0.012,
    burst_decay_t60_seconds: 0.080,
    tail_noise_decay_t60_seconds: 0.300,
    noise_rise_seconds: 0.0015,
    low_snap_level: 0.500,
    low_burst_level: 0.030,
    low_high_pass_hz: 300.0,
    low_low_pass_hz: 3000.0,
    high_snap_level: 0.030,
    high_burst_level: 0.009,
    high_high_pass_hz: 700.0,
    high_low_pass_hz: 12000.0,
    focused_strike_level: 1.100,
    focused_strike_high_pass_hz: 700.0,
    focused_strike_low_pass_hz: 1100.0,
    tail_noise_level: 0.010,
    tail_noise_high_pass_hz: 300.0,
    tail_noise_low_pass_hz: 1500.0,
    output_trim: 0.80,
};

/// Exploration knobs ([196]): multipliers on the fitted spec values.
/// `Default` reproduces the fitted sound.
#[derive(Clone, Copy, Debug)]
pub struct TomMods {
    /// Strike (attack partial + focused noise strike) level (fitted 1.0).
    pub strike: f32,
    /// Noise snap level, low and high bands (fitted 1.0).
    pub snap: f32,
    /// Pitch glide depth. Past 1.0 the high tom gains the low tom's fall
    /// (fitted 1.0, i.e. the spec's own glide).
    pub glide: f32,
    /// Upper resonance modes level (fitted 1.0).
    pub modes: f32,
    /// Tail noise level (fitted 1.0).
    pub tail_noise: f32,
}

impl Default for TomMods {
    fn default() -> Self {
        Self {
            strike: 1.0,
            snap: 1.0,
            glide: 1.0,
            modes: 1.0,
            tail_noise: 1.0,
        }
    }
}

pub struct AcTom {
    spec: &'static TomSpec,
    sample_rate: f32,
    active: bool,

    main_hz: f32,
    main_glide_hz: f32,
    lower_mode_hz: f32,
    strike_hz: f32,
    main_phase: f32,
    lower_mode_phase: f32,
    strike_phase: f32,
    /// The modes actually played: the spec's own, or the High tom's when the
    /// spec has none (the Low tom), so the Modes knob works on both.
    upper_modes: [TomMode; 4],
    upper_mode_hz: [f32; 4],
    upper_mode_phase: [f32; 4],
    upper_mode_envelope: [f32; 4],
    upper_mode_pole: [f32; 4],
    main_envelope: f32,
    lower_mode_envelope: f32,
    strike_envelope: f32,
    snap_envelope: f32,
    burst_envelope: f32,
    tail_noise_envelope: f32,
    main_pole: f32,
    main_glide_pole: f32,
    lower_mode_pole: f32,
    strike_pole: f32,
    snap_pole: f32,
    burst_pole: f32,
    tail_noise_pole: f32,
    body_rise: f32,
    body_rise_coefficient: f32,
    strike_rise: f32,
    strike_rise_coefficient: f32,
    snap_rise: f32,
    snap_rise_coefficient: f32,
    noise_rise: f32,
    noise_rise_coefficient: f32,

    // The three noise bands get their own stream so changing one does not
    // shuffle the others.
    low_noise_random: Random,
    high_noise_random: Random,
    tail_noise_random: Random,
    low_noise_high_pass: Biquad,
    low_noise_low_pass: Biquad,
    high_noise_high_pass: Biquad,
    high_noise_low_pass: Biquad,
    focused_strike_high_pass: Biquad,
    focused_strike_low_pass: Biquad,
    tail_noise_high_pass: Biquad,
    tail_noise_low_pass: Biquad,

    // [196] Exploration multipliers (1.0 = fitted).
    mod_strike: f32,
    mod_snap: f32,
    mod_modes: f32,
    mod_tail_noise: f32,
}

impl AcTom {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        let sr = if sample_rate.is_finite() && sample_rate >= 8000.0 {
            sample_rate
        } else {
            44100.0
        };
        let safe_seed = if seed == 0 { 0x606606 } else { seed };
        Self {
            spec: &HIGH_TOM_SPEC,
            sample_rate: sr,
            active: false,
            main_hz: 208.0,
            main_glide_hz: 0.0,
            lower_mode_hz: 135.616,
            strike_hz: 285.0,
            main_phase: 0.0,
            lower_mode_phase: 0.0,
            strike_phase: 0.0,
            upper_modes: HIGH_TOM_SPEC.upper_modes,
            upper_mode_hz: [0.0; 4],
            upper_mode_phase: [0.0; 4],
            upper_mode_envelope: [0.0; 4],
            upper_mode_pole: [0.0; 4],
            main_envelope: 0.0,
            lower_mode_envelope: 0.0,
            strike_envelope: 0.0,
            snap_envelope: 0.0,
            burst_envelope: 0.0,
            tail_noise_envelope: 0.0,
            main_pole: 0.0,
            main_glide_pole: 0.0,
            lower_mode_pole: 0.0,
            strike_pole: 0.0,
            snap_pole: 0.0,
            burst_pole: 0.0,
            tail_noise_pole: 0.0,
            body_rise: 0.0,
            body_rise_coefficient: 0.0,
            strike_rise: 0.0,
            strike_rise_coefficient: 0.0,
            snap_rise: 0.0,
            snap_rise_coefficient: 0.0,
            noise_rise: 0.0,
            noise_rise_coefficient: 0.0,
            low_noise_random: Random::new(safe_seed ^ 0x60001604),
            high_noise_random: Random::new(safe_seed ^ 0x60001607),
            tail_noise_random: Random::new(safe_seed ^ 0x60001605),
            low_noise_high_pass: Biquad::new(sr as f64),
            low_noise_low_pass: Biquad::new(sr as f64),
            high_noise_high_pass: Biquad::new(sr as f64),
            high_noise_low_pass: Biquad::new(sr as f64),
            focused_strike_high_pass: Biquad::new(sr as f64),
            focused_strike_low_pass: Biquad::new(sr as f64),
            tail_noise_high_pass: Biquad::new(sr as f64),
            tail_noise_low_pass: Biquad::new(sr as f64),
            mod_strike: 1.0,
            mod_snap: 1.0,
            mod_modes: 1.0,
            mod_tail_noise: 1.0,
        }
    }

    /// `spec`: low or high tom, `decay_percent` 0.05..5 (1 = the fitted decay),
    /// `pitch_ratio` (1 = original tuning), `mods`: exploration knobs.
    pub fn trigger(
        &mut self,
        spec: &'static TomSpec,
        decay_percent: f32,
        pitch_ratio: f32,
        mods: &TomMods,
    ) {
        if !decay_percent.is_finite() || !pitch_ratio.is_finite() || pitch_ratio <= 0.0 {
            self.reset();
            return;
        }

        self.spec = spec;
        self.upper_modes = if spec.upper_modes.iter().any(|m| m.level != 0.0) {
            spec.upper_modes
        } else {
            HIGH_TOM_SPEC.upper_modes
        };
        let decay = clampf(decay_percent, 0.05, 5.0);
        let ratio = clampf(pitch_ratio, 0.25, 4.0);
        self.mod_strike = mods.strike.max(0.0);
        self.mod_snap = mods.snap.max(0.0);
        self.mod_modes = mods.modes.max(0.0);
        self.mod_tail_noise = mods.tail_noise.max(0.0);

        self.main_hz = self.spec.main_hz * ratio;
        // [196] Glide: the spec's own glide is scaled; past 1.0 the high tom
        // gains the low tom's 40 Hz fall.
        let glide_scale = mods.glide.max(0.0);
        let spec_glide = if self.spec.main_glide_hz > 0.0 {
            self.spec.main_glide_hz * glide_scale
        } else {
            (glide_scale - 1.0).max(0.0) * 39.921
        };
        self.main_glide_hz = spec_glide * ratio;
        self.main_glide_pole =
            one_pole_coef(self.sample_rate as f64, self.spec.main_glide_time_seconds);
        self.lower_mode_hz = self.main_hz * self.spec.lower_mode_ratio;
        self.strike_hz = self.spec.strike_hz * ratio;
        self.main_phase = self.spec.main_start_phase;
        self.lower_mode_phase = self.spec.lower_mode_start_phase;
        self.strike_phase = self.spec.strike_start_phase;
        self.main_envelope = 1.0;
        self.lower_mode_envelope = 1.0;
        self.strike_envelope = 1.0;

        for index in 0..self.upper_modes.len() {
            let m = &self.upper_modes[index];
            self.upper_mode_hz[index] = self.main_hz * m.ratio;
            self.upper_mode_phase[index] = m.start_phase;
            self.upper_mode_envelope[index] = 1.0;
            self.upper_mode_pole[index] =
                decay_coef_t60(self.sample_rate as f64, m.decay_t60_seconds * decay);
        }

        self.snap_envelope = 1.0;
        self.burst_envelope = 1.0;
        self.tail_noise_envelope = 1.0;
        let sr = self.sample_rate as f64;
        self.main_pole = decay_coef_t60(sr, self.spec.main_decay_t60_seconds * decay);
        self.lower_mode_pole = decay_coef_t60(sr, self.spec.lower_mode_decay_t60_seconds * decay);
        self.strike_pole = decay_coef_t60(sr, self.spec.strike_decay_t60_seconds * decay);
        self.snap_pole = decay_coef_t60(sr, self.spec.snap_decay_t60_seconds * decay);
        self.burst_pole = decay_coef_t60(sr, self.spec.burst_decay_t60_seconds * decay);
        self.tail_noise_pole = decay_coef_t60(sr, self.spec.tail_noise_decay_t60_seconds * decay);

        self.body_rise = 0.0;
        self.body_rise_coefficient = 1.0 - one_pole_coef(sr, self.spec.body_attack_seconds);
        self.strike_rise = 0.0;
        self.strike_rise_coefficient = 1.0 - one_pole_coef(sr, self.spec.strike_rise_seconds);
        self.snap_rise = 0.0;
        self.snap_rise_coefficient = 1.0 - one_pole_coef(sr, 0.0004);
        self.noise_rise = 0.0;
        self.noise_rise_coefficient = 1.0 - one_pole_coef(sr, self.spec.noise_rise_seconds);

        self.low_noise_high_pass.set_high_pass(
            self.spec.low_high_pass_hz * ratio,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        self.low_noise_low_pass.set_low_pass(
            self.spec.low_low_pass_hz * ratio,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        self.high_noise_high_pass.set_high_pass(
            self.spec.high_high_pass_hz * ratio,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        self.high_noise_low_pass.set_low_pass(
            self.spec.high_low_pass_hz * ratio,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        if self.spec.focused_strike_level != 0.0 {
            self.focused_strike_high_pass.set_high_pass(
                self.spec.focused_strike_high_pass_hz * ratio,
                std::f32::consts::FRAC_1_SQRT_2,
            );
            self.focused_strike_low_pass.set_low_pass(
                self.spec.focused_strike_low_pass_hz * ratio,
                std::f32::consts::FRAC_1_SQRT_2,
            );
        }
        self.tail_noise_high_pass.set_high_pass(
            self.spec.tail_noise_high_pass_hz * ratio,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        self.tail_noise_low_pass.set_low_pass(
            self.spec.tail_noise_low_pass_hz * ratio,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        self.low_noise_high_pass.reset();
        self.low_noise_low_pass.reset();
        self.high_noise_high_pass.reset();
        self.high_noise_low_pass.reset();
        self.focused_strike_high_pass.reset();
        self.focused_strike_low_pass.reset();
        self.tail_noise_high_pass.reset();
        self.tail_noise_low_pass.reset();
        self.active = true;
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        self.main_phase = wrap_phase(
            self.main_phase + TWO_PI * (self.main_hz + self.main_glide_hz) / self.sample_rate,
        );
        if self.spec.lower_mode_level != 0.0 {
            self.lower_mode_phase =
                wrap_phase(self.lower_mode_phase + TWO_PI * self.lower_mode_hz / self.sample_rate);
        }
        self.strike_phase =
            wrap_phase(self.strike_phase + TWO_PI * self.strike_hz / self.sample_rate);
        for index in 0..self.upper_modes.len() {
            if self.upper_modes[index].level == 0.0 {
                continue;
            }
            self.upper_mode_phase[index] = wrap_phase(
                self.upper_mode_phase[index]
                    + TWO_PI * self.upper_mode_hz[index] / self.sample_rate,
            );
        }

        self.body_rise += (1.0 - self.body_rise) * self.body_rise_coefficient;
        self.strike_rise += (1.0 - self.strike_rise) * self.strike_rise_coefficient;
        self.snap_rise += (1.0 - self.snap_rise) * self.snap_rise_coefficient;
        self.noise_rise += (1.0 - self.noise_rise) * self.noise_rise_coefficient;

        let main = (self.main_phase.sin()
            + self.spec.second_harmonic_level * (2.0 * self.main_phase).sin()
            + self.spec.third_harmonic_level * (3.0 * self.main_phase).sin())
            * self.main_envelope;
        let lower_mode = if self.spec.lower_mode_level != 0.0 {
            self.spec.lower_mode_level * self.lower_mode_phase.sin() * self.lower_mode_envelope
        } else {
            0.0
        };
        let strike = self.spec.strike_level
            * self.mod_strike
            * self.strike_phase.sin()
            * self.strike_envelope
            * self.strike_rise;

        let mut upper_modes = 0.0f32;
        for index in 0..self.upper_modes.len() {
            if self.upper_modes[index].level == 0.0 {
                continue;
            }
            upper_modes += self.upper_modes[index].level
                * self.mod_modes
                * self.upper_mode_phase[index].sin()
                * self.upper_mode_envelope[index];
        }

        let mut low_noise = self.low_noise_random.bipolar24();
        low_noise = self
            .low_noise_low_pass
            .process(self.low_noise_high_pass.process(low_noise));
        let high_noise_source = self.high_noise_random.bipolar24();
        let high_noise = self
            .high_noise_low_pass
            .process(self.high_noise_high_pass.process(high_noise_source));
        let mut tail_noise = self.tail_noise_random.bipolar24();
        tail_noise = self
            .tail_noise_low_pass
            .process(self.tail_noise_high_pass.process(tail_noise));

        let snap = self.snap_rise * self.snap_envelope;
        let burst = self.noise_rise * self.burst_envelope;
        let mut excitation = low_noise
            * (self.spec.low_snap_level * self.mod_snap * snap + self.spec.low_burst_level * burst)
            + high_noise
                * (self.spec.high_snap_level * self.mod_snap * snap
                    + self.spec.high_burst_level * burst)
            + tail_noise
                * self.spec.tail_noise_level
                * self.mod_tail_noise
                * self.noise_rise
                * self.tail_noise_envelope;
        if self.spec.focused_strike_level != 0.0 {
            let mut focused_strike = self.focused_strike_high_pass.process(high_noise_source);
            focused_strike = self.focused_strike_low_pass.process(focused_strike);
            excitation += focused_strike * self.spec.focused_strike_level * self.mod_strike * snap;
        }

        let output = ((main + lower_mode + upper_modes) * self.body_rise + strike + excitation)
            * self.spec.output_trim;

        self.main_envelope = flush_denormal(self.main_envelope * self.main_pole);
        self.main_glide_hz = flush_denormal(self.main_glide_hz * self.main_glide_pole);
        self.lower_mode_envelope = flush_denormal(self.lower_mode_envelope * self.lower_mode_pole);
        self.strike_envelope = flush_denormal(self.strike_envelope * self.strike_pole);
        let mut upper_modes_are_silent = true;
        for index in 0..self.upper_modes.len() {
            self.upper_mode_envelope[index] =
                flush_denormal(self.upper_mode_envelope[index] * self.upper_mode_pole[index]);
            if self.upper_modes[index].level * self.mod_modes * self.upper_mode_envelope[index]
                > TOM_SILENCE_THRESHOLD
            {
                upper_modes_are_silent = false;
            }
        }
        self.snap_envelope = flush_denormal(self.snap_envelope * self.snap_pole);
        self.burst_envelope = flush_denormal(self.burst_envelope * self.burst_pole);
        self.tail_noise_envelope = flush_denormal(self.tail_noise_envelope * self.tail_noise_pole);
        if self.main_envelope <= TOM_SILENCE_THRESHOLD
            && self.spec.lower_mode_level * self.lower_mode_envelope <= TOM_SILENCE_THRESHOLD
            && upper_modes_are_silent
            && self.spec.tail_noise_level * self.mod_tail_noise * self.tail_noise_envelope
                <= TOM_SILENCE_THRESHOLD
        {
            self.active = false;
        }
        if !output.is_finite() {
            self.reset();
            return 0.0;
        }
        output
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.active = false;
    }
}

fn wrap_phase(mut phase: f32) -> f32 {
    while phase >= TWO_PI {
        phase -= TWO_PI;
    }
    while phase < 0.0 {
        phase += TWO_PI;
    }
    phase
}
