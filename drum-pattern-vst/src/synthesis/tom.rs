//! Tom synthesizer — refactored with shared DSP primitives.
//!
//! Architecture:
//! - Sine oscillator + 2nd harmonic overtone (×0.22)
//! - Exponential pitch sweep to 55 % of fundamental
//! - One-pole lowpass filter
//! - Exponential amplitude envelope
//! - Optional stick attack (short noise burst) for realism
//! - [94] Pitch LFO (vibrato) on the body, restarted on each hit, with a fade-in

use super::{dsp, saturation, settings::tom::TomSettings, Voice, VoiceSettings};

/// Anti-click floor for the amplitude attack (a true 0 ms attack is a step = click).
const MIN_AMP_ATTACK_MS: f32 = 0.5;

pub struct TomVoice {
    settings: TomSettings,
    sample_rate: f32,

    osc: dsp::SineOsc,
    pitch_env: dsp::PitchEnvelope,
    // 2-pole (12 dB/oct) LowPass filter — the envelope sweeps the cutoff
    // EXPONENTIALLY from the base toward 20 kHz:
    // cutoff = filter_freq × (20000/filter_freq)^(filter_env × amount).
    filter: dsp::Biquad,
    /// The stick attack goes through the SAME cutoff (its own biquad, since
    /// a filter can't carry two signals) — otherwise it bypasses the Filter
    /// knob entirely (e.g. Filter at 20 Hz still let the hit through).
    stick_filter: dsp::Biquad,
    /// Last computed modulated cutoff, reused by `stick_filter` (the stick
    /// rings on after the body is done).
    last_cutoff: f32,
    amp_env: dsp::DecayReleaseEnvelope,
    // Filter envelope for natural "bouum" decay.
    filter_env: dsp::ExpDecayEnvelope,
    stick_attack: dsp::ClickGenerator,
    // Saturation stage
    saturation: saturation::SaturationConfig,
    // Per-hit analog drift (breathing) + DC blocker.
    drift: dsp::AnalogDrift,
    dc_block: dsp::DcBlocker,
    /// [94] Pitch LFO, and the time since the hit (for its fade-in).
    pitch_lfo: dsp::Lfo,
    hit_time: f32,

    active: bool,
}

impl TomVoice {
    /// Steepness of the filter envelope decay stage used by the engine and the UI graph.
    pub const FILTER_ENV_CURVE: f32 = 6.0;

    pub fn new(sample_rate: f32, settings: TomSettings) -> Self {
        let sweep_time = 0.14f32.min(settings.decay);
        let mut osc = dsp::SineOsc::new(sample_rate);
        osc.set_freq(settings.frequency);

        let mut filter = dsp::Biquad::new();
        filter.set_lowpass(settings.filter_freq, settings.resonance, sample_rate);
        let mut stick_filter = dsp::Biquad::new();
        stick_filter.set_lowpass(settings.filter_freq, settings.resonance, sample_rate);

        let mut voice = Self {
            settings,
            sample_rate,
            osc,
            pitch_env: dsp::PitchEnvelope::new(sample_rate, 1.0, 0.55, sweep_time),
            filter,
            stick_filter,
            last_cutoff: settings.filter_freq,
            amp_env: dsp::DecayReleaseEnvelope::new(
                sample_rate,
                settings.decay_curve,
                settings.decay,
                settings.release_curve,
                settings.release,
            )
            .with_attack_ms((settings.attack * 1000.0).max(MIN_AMP_ATTACK_MS)),
            filter_env: dsp::ExpDecayEnvelope::new(
                sample_rate,
                Self::FILTER_ENV_CURVE,
                settings.filter_env_decay.max(0.001),
            )
            .with_attack_ms(0.5),
            stick_attack: dsp::ClickGenerator::new(sample_rate, 8.0, 0.5, 0.6),
            saturation: saturation::SaturationConfig {
                saturation_type: saturation::SaturationType::None,
                amount: 0.0,
                mix: 1.0,
                output_gain: 1.0,
                pre_filter: false,
                compensation_gain: 1.0,
            },
            drift: dsp::AnalogDrift::new(0x5151_2222),
            dc_block: dsp::DcBlocker::default(),
            pitch_lfo: dsp::Lfo::new(sample_rate, 0x7011_1F0E),
            hit_time: 0.0,
            active: false,
        };
        voice.update_derived_params();
        voice
    }

    fn update_derived_params(&mut self) {
        self.osc.set_freq(self.settings.frequency);
        self.filter
            .set_lowpass(self.settings.filter_freq, self.settings.resonance, self.sample_rate);
        self.amp_env.set_decay(self.settings.decay);
        self.amp_env
            .set_attack_ms((self.settings.attack * 1000.0).max(MIN_AMP_ATTACK_MS));
        self.amp_env.set_release(self.settings.release);
        self.amp_env.set_decay_curve(self.settings.decay_curve);
        self.amp_env.set_release_curve(self.settings.release_curve);
        self.filter_env
            .set_decay(self.settings.filter_env_decay.max(0.001));
        let sweep_time = 0.14f32.min(self.settings.decay);
        self.pitch_env.set_sweep_time(sweep_time);
        self.pitch_lfo
            .set_rate(self.settings.pitch_lfo_rate.clamp(0.05, 40.0));
    }

    /// [94] The pitch LFO's frequency ratio for this sample (1 when its depth is 0). The
    /// depth ramps up linearly over the fade-in, from the hit.
    fn pitch_lfo_ratio(&mut self) -> f32 {
        let shape = dsp::LfoShape::from_index(self.settings.lfo_shape.round().clamp(0.0, 3.0) as u8);
        let lfo = self.pitch_lfo.next(shape);
        let fade_s = self.settings.pitch_lfo_fade_ms.clamp(0.0, 1000.0) * 0.001;
        let fade = if fade_s <= 0.0 {
            1.0
        } else {
            (self.hit_time / fade_s).clamp(0.0, 1.0)
        };
        self.hit_time += 1.0 / self.sample_rate;
        let semis = lfo * self.settings.pitch_lfo_depth.clamp(0.0, 12.0) * fade;
        if semis.abs() > 1e-4 {
            2f32.powf(semis / 12.0)
        } else {
            1.0
        }
    }

    fn stick_amount(&self) -> f32 {
        self.settings.stick_attack
    }
}

impl Voice for TomVoice {
    fn trigger(&mut self) {
        let was_active = self.active;
        self.active = true;
        // Cold start only (voice was silent): reset phase + filter for a clean,
        // consistent attack. Never on a ringing-tail retrigger — that phase jump
        // is the click parasite (tonal voices click hard on it). See kick.rs.
        if !was_active {
            self.osc.phase = 0.0;
            self.filter.reset();
            self.stick_filter.reset();
            self.dc_block.reset();
        }
        // analog = per-hit drift (breathing) ; digital = bit-identical hits.
        self.drift.trigger(self.settings.analog >= 0.5);
        self.amp_env
            .set_decay(self.settings.decay * self.drift.time);
        self.amp_env
            .set_release(self.settings.release * self.drift.time);
        self.pitch_env.trigger();
        self.pitch_lfo.retrigger();
        self.hit_time = 0.0;
        self.amp_env.trigger();
        self.filter_env.trigger();
        if self.stick_amount() > 0.0 {
            self.stick_attack.trigger();
        }
    }

    fn trigger_hard(&mut self) {
        self.active = true;
        self.amp_env.trigger_hard();
    }

    fn process_sample(&mut self) -> f32 {
        let mut tone = 0.0f32;

        if self.active {
            // Pitch sweep — advanced ONCE per sample (it was previously also
            // advanced inside the algo branches → 2× speed).
            let pitch_ratio = self.pitch_env.next() * self.pitch_lfo_ratio();

            // Amplitude envelope
            let env = self.amp_env.next();
            if env <= 0.0 {
                self.active = false;
            } else {
                let (body, modulated_cutoff) = match self.settings.algo {
                    1 => {
                        // Deep: lower pitch, darker tone, less overtone
                        let deep_freq = self.settings.frequency * 0.7;
                        self.osc
                            .set_freq(deep_freq * pitch_ratio * self.drift.pitch);
                        let fundamental = self.osc.next();
                        let overtone =
                            ((self.osc.phase * 2.0) * 2.0 * std::f32::consts::PI).sin() * 0.12;
                        let filter_env_val = self.filter_env.next();
                        // Exponential sweep from the base cutoff toward fully
                        // open (20 kHz), same law as Buzz: amount = how far
                        // toward open the env pushes (0 = static filter,
                        // 1 = base → 20 kHz → base). Was base×(1+env×amount×4)
                        // → a 20 Hz base only swept to 100 Hz.
                        let base = self.settings.filter_freq * 0.7;
                        let sweep =
                            (filter_env_val * self.settings.filter_env_amount).clamp(0.0, 1.0);
                        let cutoff = base * (20000.0 / base.max(20.0)).powf(sweep);
                        (fundamental + overtone, cutoff.max(20.0))
                    }
                    _ => {
                        // Standard: sine + overtone, pitch sweep
                        self.osc
                            .set_freq(self.settings.frequency * pitch_ratio * self.drift.pitch);
                        let fundamental = self.osc.next();
                        let overtone =
                            ((self.osc.phase * 2.0) * 2.0 * std::f32::consts::PI).sin() * 0.22;
                        let filter_env_val = self.filter_env.next();
                        let base = self.settings.filter_freq;
                        let sweep =
                            (filter_env_val * self.settings.filter_env_amount).clamp(0.0, 1.0);
                        let cutoff = base * (20000.0 / base.max(20.0)).powf(sweep);
                        (fundamental + overtone, cutoff.max(20.0))
                    }
                };
                self.filter
                    .set_lowpass(modulated_cutoff, self.settings.resonance, self.sample_rate);
                self.last_cutoff = modulated_cutoff;
                let filtered = self.filter.process(self.saturation.process_at(true, body));
                tone = filtered * env * self.drift.level;
            }
        }

        // Stick attack — allowed to ring out even if body finished, but routed
        // through the same cutoff so the Filter knob governs the WHOLE voice.
        let attack = if self.stick_amount() > 0.0 && self.stick_attack.is_active() {
            self.stick_filter
                .set_lowpass(self.last_cutoff, self.settings.resonance, self.sample_rate);
            self.stick_filter.process(self.stick_attack.next()) * self.stick_amount()
        } else {
            0.0
        };

        // Volume post-saturation: the knob sets the final level, not the drive.
        self.dc_block
            .process(self.saturation.process_at(false, tone + attack))
            * self.settings.volume
    }

    fn is_active(&self) -> bool {
        self.active || self.stick_attack.is_active()
    }

    fn reset(&mut self) {
        self.active = false;
        self.amp_env.reset();
        self.filter_env.reset();
        self.stick_attack.reset();
        self.filter.reset();
        self.stick_filter.reset();
        self.dc_block.reset();
        self.pitch_lfo.reset();
        self.hit_time = 0.0;
    }

    fn set_settings(&mut self, settings: VoiceSettings) {
        self.settings = TomSettings::from(settings);
        self.update_derived_params();
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
            0 => self.settings.stick_attack = value,
            1 => {
                self.settings.saturation_type = value as u8;
                self.saturation.saturation_type =
                    saturation::SaturationType::from(self.settings.saturation_type);
            }
            2 => {
                self.settings.saturation_amount = value;
                self.saturation.amount = value;
            }
            3 => {
                self.settings.saturation_mix = value;
                self.saturation.mix = value;
            }
            4 => {
                self.settings.saturation_output_gain = value;
                self.saturation.output_gain = value;
            }
            5 => {
                self.settings.saturation_pre_filter = value;
                self.saturation.pre_filter = value > 0.5;
            }
            7 => self.settings.lfo_shape = value,
            8 => {
                self.settings.pitch_lfo_rate = value;
                self.pitch_lfo.set_rate(value.clamp(0.05, 40.0));
            }
            9 => self.settings.pitch_lfo_depth = value,
            10 => self.settings.pitch_lfo_fade_ms = value,
            _ => {}
        }
        self.saturation.update_compensation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(settings: VoiceSettings, n: usize) -> Vec<f32> {
        let mut voice = TomVoice::new(44100.0, TomSettings::from(settings));
        voice.trigger();
        (0..n).map(|_| voice.process_sample()).collect()
    }

    fn base() -> VoiceSettings {
        let mut settings = VoiceSettings::tom1();
        settings.analog = 0.0;
        settings.special[0] = 0.0; // no stick: the body alone
        settings
    }

    /// [94] Depth 0 = the tom as before; a depth moves the pitch; the fade-in holds the
    /// vibrato back at the start of the hit.
    #[test]
    fn the_pitch_lfo_bends_the_body_and_fades_in() {
        let n = 8820; // 200 ms
        let plain = render(base(), n);
        let mut zero = base();
        zero.special[9] = 0.0;
        zero.special[8] = 7.0;
        assert_eq!(render(zero, n), plain, "depth 0 must leave the tom untouched");

        let mut lfo = base();
        lfo.special[8] = 12.0;
        lfo.special[9] = 6.0;
        let wobbly = render(lfo, n);
        let diff = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| (x - y).abs()).fold(0.0f32, f32::max);
        assert!(diff(&wobbly, &plain) > 0.05, "a 6-semitone LFO must change the sound");
        assert!(wobbly.iter().all(|x| x.is_finite()));

        let mut faded = lfo;
        faded.special[10] = 500.0;
        let faded = render(faded, n);
        // The first 5 ms: the fade-in has barely started, so it sounds like the plain tom.
        let early = 220;
        assert!(
            diff(&faded[..early], &plain[..early]) < diff(&wobbly[..early], &plain[..early]),
            "the fade-in must hold the vibrato back at the start"
        );
    }

    /// [94] Every shape runs and stays finite.
    #[test]
    fn every_lfo_shape_plays() {
        for shape in 0..4 {
            let mut settings = base();
            settings.special[7] = shape as f32;
            settings.special[9] = 3.0;
            assert!(render(settings, 4410).iter().all(|x| x.is_finite()));
        }
    }
}
