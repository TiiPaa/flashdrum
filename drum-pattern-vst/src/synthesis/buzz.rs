//! Buzz — tonal percussion with a built-in fast amplitude gate/retrigger.
//!
//! A pitched oscillator (sine / square / saw, with a short percussive pitch
//! sweep) blended with an adjustable, colour-selectable noise layer, fed
//! through a rate-controlled GATE. Two gate flavours:
//! - **Smooth** = a raised-cosine tremolo (soft pumping),
//! - **Razor** = a short exponential spike re-articulated each cycle (hard chop).
//!
//! Envelopes: the amp is A-H-D (attack/hold/decay, no release), and the filter
//! cutoff is swept by its own A-H-D envelope. The base filter is a 2-pole
//! Biquad selectable as low-pass / high-pass / band-pass.
//!
//! Signal path: source (tonal + noise) → × gate → filter → × amp env →
//! saturation → DC blocker → × volume. Stereo shares one gate phase so the
//! buzz stays coherent across channels.

use super::{dsp, saturation, settings::buzz::BuzzSettings, Voice, VoiceSettings};
use std::f32::consts::TAU;

/// Anti-click floor for the amp attack (a true 0 ms attack is a step).
const MIN_AMP_ATTACK_MS: f32 = 0.3;
/// Anti-click floor for the filter-envelope attack (seconds).
const MIN_FILTER_ATTACK_S: f32 = 0.0005;
/// Fixed attack ramp of the Razor gate spike — softens its front.
const GATE_ATTACK_MS: f32 = 0.3;
/// Upper bound of the gate rate; keeps the gate period well above the attack.
const GATE_RATE_MAX: f32 = 500.0;
/// Percussive pitch-sweep duration.
const SWEEP_TIME_S: f32 = 0.05;
/// Default Q for LP/HP; the band-pass is tighter by `FILTER_Q_BP / FILTER_Q`.
const FILTER_Q: f32 = 0.9;
const FILTER_Q_BP: f32 = 2.5;
const NOISE_SEED_L: u32 = 0xB022_1111;
const NOISE_SEED_R: u32 = 0xB022_2222;

/// Tonal oscillator sample from a phase in [0,1). 0 = sine, 1 = square, 2 = saw.
#[inline]
fn osc_wave(phase: f32, waveform: u8) -> f32 {
    match waveform {
        1 => {
            if phase < 0.5 {
                1.0
            } else {
                -1.0
            }
        }
        2 => 2.0 * phase - 1.0,
        _ => (phase * TAU).sin(),
    }
}

pub struct BuzzVoice {
    settings: BuzzSettings,
    sample_rate: f32,

    // Tonal source: one running phase per channel (waveform picked at render).
    osc_phase_l: f32,
    osc_phase_r: f32,
    // Noise layer (per channel)
    noise_l: dsp::NoiseSource,
    noise_r: dsp::NoiseSource,

    // Percussive pitch sweep + A-H-D amp envelope (no release).
    sweep_env: dsp::PitchEnvelope,
    // Amp A-H-D envelope with bipolar decay-curve shaping (shared engine).
    amp_env: dsp::DecayReleaseEnvelope,
    // Manual A-H-D filter envelope (independent attack/decay curve shaping):
    // seconds since the last trigger.
    filter_env_time: f32,

    // Gate: a phasor drives either a cosine tremolo (Smooth) or a short
    // exponential spike (Razor), re-articulated each cycle.
    gate_env: dsp::ExpDecayEnvelope,
    gate_phase: f32,

    // Control smoothers (absorb slider-drag zipper / sweep reset jumps)
    depth_smoother: dsp::OnePoleSmoother,
    cutoff_smoother: dsp::OnePoleSmoother,
    freq_smoother: dsp::OnePoleSmoother,

    // 2-pole filter (per channel), post-gate — type-selectable (LP/HP/BP).
    filter_l: dsp::Biquad,
    filter_r: dsp::Biquad,

    saturation: saturation::SaturationConfig,
    dc_block_l: dsp::DcBlocker,
    dc_block_r: dsp::DcBlocker,
    drift: dsp::AnalogDrift,

    active: bool,
}

impl BuzzVoice {
    pub fn new(sample_rate: f32, settings: BuzzSettings) -> Self {
        let (sweep_start, sweep_end) = Self::sweep_ratios(&settings);
        let sweep_env = dsp::PitchEnvelope::new(sample_rate, sweep_start, sweep_end, SWEEP_TIME_S);

        let decay = settings.decay.max(0.01).min(5.0);
        let mut amp_env =
            dsp::DecayReleaseEnvelope::new(sample_rate, settings.decay_curve, decay, 0.0, 0.0)
                .with_attack_ms((settings.attack * 1000.0).max(MIN_AMP_ATTACK_MS));
        amp_env.set_hold(settings.hold);

        let gate_env =
            dsp::ExpDecayEnvelope::new(sample_rate, 6.0, 0.005).with_attack_ms(GATE_ATTACK_MS);

        let mut voice = Self {
            settings,
            sample_rate,
            osc_phase_l: 0.0,
            osc_phase_r: 0.25,
            noise_l: dsp::NoiseSource::new(settings.noise_type, NOISE_SEED_L),
            noise_r: dsp::NoiseSource::new(settings.noise_type, NOISE_SEED_R),
            sweep_env,
            amp_env,
            filter_env_time: 1.0e6,
            gate_env,
            gate_phase: 0.0,
            depth_smoother: dsp::OnePoleSmoother::new(sample_rate, 5.0, settings.gate_depth),
            cutoff_smoother: dsp::OnePoleSmoother::new(
                sample_rate,
                2.0,
                settings.filter_freq.max(20.0).min(20000.0),
            ),
            freq_smoother: dsp::OnePoleSmoother::new(
                sample_rate,
                1.5,
                settings.frequency.max(20.0),
            ),
            filter_l: dsp::Biquad::new(),
            filter_r: dsp::Biquad::new(),
            saturation: saturation::SaturationConfig {
                saturation_type: saturation::SaturationType::from(settings.saturation_type),
                amount: settings.saturation_amount,
                mix: settings.saturation_mix,
                output_gain: settings.saturation_output_gain,
                pre_filter: settings.saturation_pre_filter > 0.5,
                compensation_gain: 1.0,
            },
            dc_block_l: dsp::DcBlocker::default(),
            dc_block_r: dsp::DcBlocker::default(),
            drift: dsp::AnalogDrift::new(0xB033_4455),
            active: false,
        };
        voice.saturation.update_compensation();
        voice.recompute_gate_env();
        voice
    }

    /// Pitch-sweep endpoints: a downward drop whose depth grows with the amount
    /// (up to ~2 octaves), settling at the tuned pitch.
    fn sweep_ratios(settings: &BuzzSettings) -> (f32, f32) {
        let amount = settings.pitch_sweep.clamp(0.0, 1.0);
        (1.0 + amount * 4.0, 1.0)
    }

    /// Bipolar curve shaping of the normalised filter envelope value.
    /// `curve` -1 = concave (slow → fast), 0 = natural, +1 = convex (snappy).
    /// Mirrors `dsp::shape_curve` (exponent 1+5|c|, [170]).
    #[inline]
    fn shape_curve(e: f32, curve: f32) -> f32 {
        let e = e.clamp(0.0, 1.0);
        let c = curve.clamp(-1.0, 1.0);
        if c >= 0.0 {
            e.powf(1.0 + c * 5.0)
        } else {
            1.0 - (1.0 - e).powf(1.0 - c * 5.0)
        }
    }

    /// Razor gate spike shape: decay length + curve relative to the gate period.
    fn recompute_gate_env(&mut self) {
        let period = 1.0 / self.settings.gate_rate.clamp(1.0, GATE_RATE_MAX);
        let shape = self.settings.gate_shape.clamp(0.0, 1.0);
        let ratio = 0.8 * 0.05_f32.powf(shape); // 0.8·T … 0.04·T
        self.gate_env.set_decay((period * ratio).max(0.0002));
        self.gate_env.set_curve(3.0 + 9.0 * shape);
    }

    /// Advance the per-sample state shared by both channels. Returns
    /// (amp, gate_mod, freq, cutoff), or None once the voice has gone silent.
    fn tick_shared(&mut self) -> Option<(f32, f32, f32, f32)> {
        let amp = self.amp_env.next();
        if amp <= 0.0 && !self.sweep_env.is_active() {
            self.active = false;
            return None;
        }

        // Gate phasor.
        self.gate_phase += self.settings.gate_rate.clamp(1.0, GATE_RATE_MAX) / self.sample_rate;
        let wrapped = self.gate_phase >= 1.0;
        if wrapped {
            self.gate_phase -= 1.0;
        }
        let shape = self.settings.gate_shape.clamp(0.0, 1.0);
        let g = if self.settings.algo == 0 {
            // Smooth: raised-cosine tremolo, peaking at phase 0; Shape narrows
            // the pulse (0 = pure sine tremolo, 1 = peaky).
            let raw = 0.5 + 0.5 * (self.gate_phase * TAU).cos();
            raw.powf(1.0 + shape * 4.0)
        } else {
            // Razor: sharp exponential spike, restarted from zero each cycle.
            if wrapped {
                self.gate_env.trigger_from_zero(1.0);
            }
            self.gate_env.next()
        };
        let depth = self
            .depth_smoother
            .process(self.settings.gate_depth.clamp(0.0, 1.0));
        let gate_mod = 1.0 - depth * (1.0 - g);

        // Pitch (sweep) — smoothed so the sweep reset never jumps in one sample.
        let base = self.settings.frequency.max(20.0);
        let ratio = self.sweep_env.next();
        let freq = self.freq_smoother.process(base * ratio * self.drift.pitch);

        // Filter cutoff: A-H-D envelope sweeps EXPONENTIALLY from the base
        // cutoff up toward fully open (20 kHz). amount = how far toward open the
        // env pushes (0 = no sweep, 1 = base → 20 kHz → base). Base at minimum +
        // amount at max → a full-range percussive filter drop.
        let base_cutoff = self
            .cutoff_smoother
            .process(self.settings.filter_freq.max(20.0).min(20000.0));
        // Manual A-H-D filter envelope: linear attack & decay ramps, each shaped
        // independently by its own bipolar curve (atk vs dec).
        let atk = self.settings.filter_env_attack.max(MIN_FILTER_ATTACK_S);
        let hold = self.settings.filter_env_hold.max(0.0);
        let dec = self.settings.filter_env_decay.max(0.001);
        let t = self.filter_env_time;
        self.filter_env_time += 1.0 / self.sample_rate;
        let env = if t < atk {
            Self::shape_curve(t / atk, self.settings.filter_atk_curve)
        } else if t < atk + hold {
            1.0
        } else {
            let p = ((t - atk - hold) / dec).clamp(0.0, 1.0);
            Self::shape_curve(1.0 - p, self.settings.filter_curve)
        };
        let amt = (env * self.settings.filter_env_amount).clamp(0.0, 1.0);
        let cutoff = (base_cutoff * (20000.0 / base_cutoff).powf(amt)).clamp(20.0, 20000.0);

        Some((amp, gate_mod, freq, cutoff))
    }

    #[allow(clippy::too_many_arguments)]
    fn render_channel(
        phase: &mut f32,
        noise: &mut dsp::NoiseSource,
        filter: &mut dsp::Biquad,
        dc: &mut dsp::DcBlocker,
        sat: &saturation::SaturationConfig,
        sample_rate: f32,
        settings: &BuzzSettings,
        drift_level: f32,
        amp: f32,
        gate_mod: f32,
        freq: f32,
        cutoff: f32,
    ) -> f32 {
        let tonal = osc_wave(*phase, settings.waveform);
        *phase += freq / sample_rate;
        *phase -= phase.floor();

        let n = settings.noise_amount.clamp(0.0, 1.0);
        let src = tonal * (1.0 - n) + noise.next() * n;
        let gated = src * gate_mod;
        // source → gate → filter → amp → saturation → dc → volume
        let pre = sat.process_at(true, gated);
        match settings.filter_type {
            1 => filter.set_highpass(cutoff, settings.resonance, sample_rate),
            2 => filter.set_bandpass(
                cutoff,
                settings.resonance * (FILTER_Q_BP / FILTER_Q),
                sample_rate,
            ),
            _ => filter.set_lowpass(cutoff, settings.resonance, sample_rate),
        }
        let filtered = filter.process(pre);
        let enveloped = filtered * amp * drift_level;
        let post = sat.process_at(false, enveloped);
        dc.process(post) * settings.volume
    }
}

impl Voice for BuzzVoice {
    fn trigger(&mut self) {
        let was_active = self.active;
        self.active = true;
        self.drift.trigger(self.settings.analog >= 0.5);
        self.amp_env
            .set_decay((self.settings.decay.max(0.01)) * self.drift.time);
        // Machine-gun retrigger: every cell restarts the full A-H-D volume
        // envelope from zero, so a ringing tail from the previous cell doesn't
        // swallow the new envelope (each consecutive hit is fully re-articulated).
        self.amp_env.trigger_hard();
        // The filter envelope re-articulates on every hit (the cutoff sweep).
        self.filter_env_time = 0.0;

        // Re-fire the pitch sweep on every hit (the "pew"). The freq smoother
        // absorbs the ratio reset, so it stays click-safe even mid-tail.
        let (start, end) = Self::sweep_ratios(&self.settings);
        self.sweep_env = dsp::PitchEnvelope::new(self.sample_rate, start, end, SWEEP_TIME_S);
        self.sweep_env.trigger();

        // Re-sync the GATE on every hit so each cell starts with a fresh, identical
        // amplitude burst (the "attack" of the gated volume) instead of catching
        // the free-running gate at a random phase.
        self.gate_phase = 0.0;
        self.gate_env.trigger_at_peak(1.0);

        // Oscillator phase is reset on COLD START only — resetting it mid-tail
        // would be the click parasite for the tonal part.
        if !was_active {
            self.osc_phase_l = 0.0;
            self.osc_phase_r = 0.25;
        }
    }

    fn trigger_hard(&mut self) {
        self.active = true;
        self.amp_env.trigger_hard();
        self.filter_env_time = 0.0;
    }

    fn process_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }
        let Some((amp, gate_mod, freq, cutoff)) = self.tick_shared() else {
            return 0.0;
        };
        Self::render_channel(
            &mut self.osc_phase_l,
            &mut self.noise_l,
            &mut self.filter_l,
            &mut self.dc_block_l,
            &self.saturation,
            self.sample_rate,
            &self.settings,
            self.drift.level,
            amp,
            gate_mod,
            freq,
            cutoff,
        )
    }

    fn process_sample_stereo(&mut self) -> (f32, f32) {
        if !self.active {
            return (0.0, 0.0);
        }
        if self.settings.stereo < 0.5 {
            let m = self.process_sample();
            return (m, m);
        }
        let Some((amp, gate_mod, freq, cutoff)) = self.tick_shared() else {
            return (0.0, 0.0);
        };
        let l = Self::render_channel(
            &mut self.osc_phase_l,
            &mut self.noise_l,
            &mut self.filter_l,
            &mut self.dc_block_l,
            &self.saturation,
            self.sample_rate,
            &self.settings,
            self.drift.level,
            amp,
            gate_mod,
            freq,
            cutoff,
        );
        // Slight detune on the right for stereo width; same gate/amp/cutoff.
        let r = Self::render_channel(
            &mut self.osc_phase_r,
            &mut self.noise_r,
            &mut self.filter_r,
            &mut self.dc_block_r,
            &self.saturation,
            self.sample_rate,
            &self.settings,
            self.drift.level,
            amp,
            gate_mod,
            freq * 1.003,
            cutoff,
        );
        (l, r)
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn reset(&mut self) {
        self.active = false;
        self.gate_phase = 0.0;
        self.osc_phase_l = 0.0;
        self.osc_phase_r = 0.25;
        self.amp_env.reset();
        self.filter_env_time = 1.0e6;
        self.gate_env.reset();
        self.filter_l.reset();
        self.filter_r.reset();
        self.dc_block_l.reset();
        self.dc_block_r.reset();
    }

    fn set_settings(&mut self, settings: VoiceSettings) {
        let new = BuzzSettings::from(settings);
        let noise_changed = new.noise_type != self.settings.noise_type;
        self.settings = new;

        // Amp A-H-D envelope via setters — never recreate (would reset the tail).
        self.amp_env.set_decay(self.settings.decay.max(0.01));
        self.amp_env.set_decay_curve(self.settings.decay_curve);
        self.amp_env.set_hold(self.settings.hold);
        self.amp_env
            .set_attack_ms((self.settings.attack * 1000.0).max(MIN_AMP_ATTACK_MS));

        // Filter A-H-D envelope params are read per sample (manual envelope) —
        // nothing to push here.

        // Gate timing (does NOT touch gate_phase / gate_env value → tail stays live).
        self.recompute_gate_env();

        if noise_changed {
            self.noise_l = dsp::NoiseSource::new(self.settings.noise_type, NOISE_SEED_L);
            self.noise_r = dsp::NoiseSource::new(self.settings.noise_type, NOISE_SEED_R);
        }

        self.saturation.saturation_type =
            saturation::SaturationType::from(self.settings.saturation_type);
        self.saturation.amount = self.settings.saturation_amount;
        self.saturation.mix = self.settings.saturation_mix;
        self.saturation.output_gain = self.settings.saturation_output_gain;
        self.saturation.pre_filter = self.settings.saturation_pre_filter > 0.5;
        self.saturation.update_compensation();
    }

    fn set_algo(&mut self, algo: u8) {
        self.settings.algo = algo;
    }

    fn set_special_param(&mut self, index: usize, value: f32) {
        match index {
            0 => {
                self.settings.gate_rate = value;
                self.recompute_gate_env();
            }
            1 => self.settings.gate_depth = value,
            2 => {
                self.settings.gate_shape = value;
                self.recompute_gate_env();
            }
            3 => self.settings.noise_amount = value,
            4 => {
                self.settings.noise_type = value as u8;
                self.noise_l = dsp::NoiseSource::new(self.settings.noise_type, NOISE_SEED_L);
                self.noise_r = dsp::NoiseSource::new(self.settings.noise_type, NOISE_SEED_R);
            }
            5 => self.settings.pitch_sweep = value,
            11 => self.settings.waveform = value as u8,
            12 => self.settings.filter_env_attack = value,
            13 => self.settings.filter_env_hold = value,
            14 => self.settings.filter_type = value as u8,
            15 => self.settings.filter_curve = value,
            16 => self.settings.filter_atk_curve = value,
            6 => {
                self.settings.saturation_type = value as u8;
                self.saturation.saturation_type =
                    saturation::SaturationType::from(self.settings.saturation_type);
            }
            7 => {
                self.settings.saturation_amount = value;
                self.saturation.amount = value;
            }
            8 => {
                self.settings.saturation_mix = value;
                self.saturation.mix = value;
            }
            9 => {
                self.settings.saturation_output_gain = value;
                self.saturation.output_gain = value;
            }
            10 => {
                self.settings.saturation_pre_filter = value;
                self.saturation.pre_filter = value > 0.5;
            }
            _ => {}
        }
        self.saturation.update_compensation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_with(settings: VoiceSettings) -> BuzzVoice {
        BuzzVoice::new(44100.0, BuzzSettings::from(settings))
    }

    #[test]
    fn produces_sound_on_trigger() {
        let mut voice = voice_with(VoiceSettings::buzz());
        voice.trigger();
        let mut peak = 0.0f32;
        for _ in 0..22050 {
            peak = peak.max(voice.process_sample().abs());
        }
        assert!(peak > 0.05, "Buzz should produce sound, peak {peak}");
    }

    #[test]
    fn output_stays_finite_and_stops() {
        let mut voice = voice_with(VoiceSettings::buzz());
        for _ in 0..20 {
            voice.trigger();
            for _ in 0..22050 {
                let (l, r) = voice.process_sample_stereo();
                assert!(l.is_finite() && r.is_finite(), "non-finite sample");
            }
        }
        // Drain the last hit until the voice deactivates (bounded), then the
        // output must be exactly silent.
        let mut n = 0;
        while voice.is_active() && n < 44100 * 6 {
            voice.process_sample();
            n += 1;
        }
        assert!(!voice.is_active(), "voice never went silent");
        let mut tail = 0.0f32;
        for _ in 0..2000 {
            tail = tail.max(voice.process_sample().abs());
        }
        assert!(tail < 1e-6, "silent voice still outputs, tail {tail}");
    }

    #[test]
    fn gate_depth_modulates_the_output() {
        // Pure tonal, deterministic. Depth 0 = bypass, depth 1 = full chop.
        let render = |depth: f32| -> Vec<f32> {
            let mut s = VoiceSettings::buzz();
            s.special[1] = depth; // gate_depth
            s.special[3] = 0.0; // noise amount → pure tonal
            s.special[0] = 30.0; // gate_rate
            let mut v = voice_with(s);
            v.trigger();
            (0..8000).map(|_| v.process_sample()).collect()
        };
        let dry = render(0.0);
        let wet = render(1.0);
        let max_diff = dry
            .iter()
            .zip(&wet)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff > 0.05,
            "gate depth should change the output (max diff {max_diff})"
        );
    }

    #[test]
    fn smooth_and_razor_differ() {
        let render = |algo: u8| -> Vec<f32> {
            let mut s = VoiceSettings::buzz();
            s.special[3] = 0.0; // pure tonal, deterministic
            s.special[0] = 60.0; // gate rate
            s.special[1] = 1.0; // full depth
            s.algo = algo;
            let mut v = voice_with(s);
            v.trigger();
            (0..8000).map(|_| v.process_sample()).collect()
        };
        let smooth = render(0);
        let razor = render(1);
        let max_diff = smooth
            .iter()
            .zip(&razor)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff > 0.1,
            "Smooth and Razor should differ (max diff {max_diff})"
        );
    }

    #[test]
    fn waveform_changes_the_source() {
        let render = |wave: f32| -> Vec<f32> {
            let mut s = VoiceSettings::buzz();
            s.special[1] = 0.0; // gate depth 0 → bypass
            s.special[3] = 0.0; // no noise
            s.special[5] = 0.0; // no pitch sweep
            s.special[11] = wave;
            let mut v = voice_with(s);
            v.trigger();
            (0..4000).map(|_| v.process_sample()).collect()
        };
        let sine = render(0.0);
        let saw = render(2.0);
        let max_diff = sine
            .iter()
            .zip(&saw)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff > 0.1,
            "waveform should change the source (diff {max_diff})"
        );
    }

    #[test]
    fn filter_type_changes_the_output() {
        // LP vs HP on the same source must differ.
        let render = |ftype: f32| -> Vec<f32> {
            let mut s = VoiceSettings::buzz();
            s.special[1] = 0.0; // no gate
            s.special[3] = 0.0; // pure tonal
            s.special[14] = ftype; // filter type
            s.frequency = 400.0;
            s.filter_freq = 800.0;
            let mut v = voice_with(s);
            v.trigger();
            (0..4000).map(|_| v.process_sample()).collect()
        };
        let lp = render(0.0);
        let hp = render(1.0);
        let max_diff = lp
            .iter()
            .zip(&hp)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff > 0.05,
            "filter type should change the output (diff {max_diff})"
        );
    }
}
