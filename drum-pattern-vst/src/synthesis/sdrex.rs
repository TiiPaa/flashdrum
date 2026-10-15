//! SDrex — metallic snare (user recipe "drex_snare").
//!
//! Three layers, mixed 0.50 / 0.80 / 0.18:
//! - **Body**: sine with a fast pitch drop (~+95 Hz → base, exp rate 45) and a
//!   short exp envelope (rate 32).
//! - **Noise**: selectable coloured noise high-passed by LP subtraction
//!   (α = 0.08 one-pole), exp envelope (rate 18) — the main tail; the `decay`
//!   param scales all three envelope rates.
//! - **Metal**: ring-mod of two sines (620 × 910 Hz, scaled by the base
//!   frequency), exp envelope (rate 25).
//!
//! The modulation LFO targets either a **flanger** (rate / min_delay / depth /
//! feedback / wet) or the low-pass cutoff (rate / depth / wet). Free Phase
//! controls the shared LFO. The chain also includes an A-H-D low-pass sweep and
//! a fixed tanh drive (×2.2, ×0.8). Volume and filter envelopes are time
//! formulas, so `set_settings` has no state to rebuild.

use super::{dsp, saturation, settings::sdrex::SdrexSettings, Voice, VoiceSettings};

/// Reference decay (s) at which the three envelope rates match the recipe.
const DECAY_REF_SECS: f32 = 0.15;
/// Body pitch-drop height above the base frequency (Hz).
const BODY_SWEEP_HZ: f32 = 95.0;
/// Recipe body base frequency (Hz) — the metal pair scales with `freq / this`.
const BODY_REF_HZ: f32 = 185.0;
/// Eight milliseconds at 192 kHz, plus two samples for interpolation.
const FLANGER_BUFFER_LEN: usize = 1538;
/// Flanger minimum delay, in ms. Was a user parameter ("Delay") until [181]
/// replaced that slider with the modulation Fade-in; kept at its former default
/// so the flanger keeps the exact same character.
const FLANGER_MIN_DELAY_MS: f32 = 0.7;
/// Octaves covered by the Filter LFO at full Depth × Wet ([183]). The Depth
/// slider tops out at 3.0 (it is shared with the flanger, where it means ms of
/// delay), so 2.0 here means the filter sweeps up to 6 octaves above its base —
/// 20 Hz → 1280 Hz at the very bottom of the Filter range. Measured: the voice
/// then peaks 4.8 dB under the wide-open filter instead of 15.8 dB under it.
const FILTER_MOD_OCTAVE_SCALE: f32 = 2.0;

pub struct SdrexVoice {
    settings: SdrexSettings,
    sample_rate: f32,

    /// Seconds since the last trigger (drives the three exp envelopes).
    env_time: f32,
    /// Common volume-attack state. On a regular retrigger the ramp starts from
    /// the current tail level; hard retriggers restart it from zero.
    attack_start_value: f32,
    amp_proxy: f32,
    body_phase: f32,
    metal_phase_1: f32,
    metal_phase_2: f32,

    noise: dsp::NoiseSource,
    noise_lp: dsp::OnePoleFilter,

    /// Low-pass on the mix, swept by an A-H-D envelope with bipolar curves:
    /// cutoff = base × (20000/base)^(env × amount).
    filter: dsp::Biquad,
    filter_env_time: f32,

    /// Flanger state: circular delay, free-running LFO phase. Fixed-size
    /// buffer (8 ms @ up to 192 kHz) — a `Vec` here would allocate on the
    /// audio thread when the voice is created by a hot kind-change.
    flanger_buf: [f32; FLANGER_BUFFER_LEN],
    flanger_len: usize,
    flanger_pos: usize,
    flanger_phase: f32,

    saturation: saturation::SaturationConfig,
    drift: dsp::AnalogDrift,
    dc_block: dsp::DcBlocker,

    active: bool,
}

impl SdrexVoice {
    pub fn new(sample_rate: f32, settings: SdrexSettings) -> Self {
        // Noise LP subtraction: recipe α = 0.08 at 44.1 kHz → keep the same
        // coefficient at any session rate (f = −ln(1−α)·sr/2π ≈ 0.01327·sr).
        let mut noise_lp = dsp::OnePoleFilter::new(dsp::FilterMode::LowPass);
        noise_lp.set_cutoff(0.013_27 * sample_rate, sample_rate);

        Self {
            settings,
            sample_rate,
            env_time: 0.0,
            attack_start_value: 0.0,
            amp_proxy: 0.0,
            body_phase: 0.0,
            metal_phase_1: 0.0,
            metal_phase_2: 0.0,
            noise: dsp::NoiseSource::new(settings.noise_type, 0xD3EC_1A00),
            noise_lp,
            filter: {
                let mut f = dsp::Biquad::new();
                f.set_lowpass(
                    settings.filter_freq.max(20.0),
                    settings.resonance,
                    sample_rate,
                );
                f
            },
            filter_env_time: 0.0,
            // 8 ms > min_delay(3) + depth(3) max, +2 for interpolation;
            // clamped to the fixed buffer capacity at high session rates.
            flanger_buf: [0.0; FLANGER_BUFFER_LEN],
            flanger_len: ((sample_rate.max(1.0) * 0.008) as usize)
                .saturating_add(2)
                .clamp(2, FLANGER_BUFFER_LEN),
            flanger_pos: 0,
            flanger_phase: 0.0,
            saturation: saturation::SaturationConfig {
                saturation_type: saturation::SaturationType::from(settings.saturation_type),
                amount: settings.saturation_amount,
                mix: settings.saturation_mix,
                output_gain: settings.saturation_output_gain,
                pre_filter: settings.saturation_pre_filter > 0.5,
                compensation_gain: 1.0,
            },
            drift: dsp::AnalogDrift::new(0x5D3E_0001),
            dc_block: dsp::DcBlocker::default(),
            active: false,
        }
        .with_saturation_compensation()
    }

    fn with_saturation_compensation(mut self) -> Self {
        self.saturation.update_compensation();
        self
    }

    /// Envelope rate scale: `decay` stretches/shrinks the whole hit.
    fn time_factor(&self) -> f32 {
        DECAY_REF_SECS / self.settings.decay.clamp(0.03, 1.5)
    }
}

impl Voice for SdrexVoice {
    fn trigger(&mut self) {
        let was_active = self.active;
        self.attack_start_value = if was_active {
            self.amp_proxy.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.active = true;
        self.env_time = 0.0;
        self.filter_env_time = 0.0;
        // Oscillator phases keep the normal anti-click policy: reset only on a
        // cold start. Never reset the noise RNG (analog continuity convention).
        if !was_active {
            self.body_phase = 0.0;
            self.metal_phase_1 = 0.0;
            self.metal_phase_2 = 0.0;
        }
        // Free Phase belongs to the flanger: fixed mode restarts the LFO on
        // every hit, while free mode preserves its current phase.
        if self.settings.modulation_free_phase < 0.5 {
            self.flanger_phase = 0.0;
        }
        self.drift.trigger(self.settings.analog >= 0.5);
    }

    fn trigger_hard(&mut self) {
        self.amp_proxy = 0.0;
        self.trigger();
    }

    fn process_sample(&mut self) -> f32 {
        if !self.active {
            return 0.0;
        }

        let t = self.env_time;
        self.env_time += 1.0 / self.sample_rate;
        let tfac = self.time_factor();
        let dt = 1.0 / self.sample_rate;

        // One LFO, routed either to the flanger delay or to the low-pass
        // cutoff. In Filter LFO mode, Depth × Wet × `FILTER_MOD_OCTAVE_SCALE` is
        // how many octaves the cutoff climbs ABOVE the Filter setting ([183]);
        // the flanger reads the same Depth as milliseconds of delay.
        let modulation_rate = self.settings.flanger_rate.clamp(0.1, 20.0);
        // [290] Bipolar: the sign is the direction of the sweep. Positive opens
        // the cutoff above its base / grows the delay from its minimum; negative
        // closes the cutoff below its base / starts the delay long and shrinks it.
        let modulation_depth = self.settings.flanger_depth.clamp(-3.0, 3.0);
        // [181] Modulation fade-in: Wet ramps from 0 to its setting over
        // `Fade-in` ms after the hit, so the flanger sweep / filter LFO swells in
        // instead of being there on the attack. Applies to BOTH modes.
        let fade_s = self.settings.modulation_fade_ms.clamp(0.0, 300.0) * 0.001;
        let modulation_fade = if fade_s <= 0.0 {
            1.0
        } else {
            (self.env_time / fade_s).clamp(0.0, 1.0)
        };
        let modulation_wet = self.settings.flanger_wet.clamp(0.0, 1.0) * modulation_fade;
        let filter_mod = self.settings.modulation_type >= 1;
        let modulation_bipolar = self.flanger_phase.sin();
        let modulation_unipolar = 0.5 + 0.5 * modulation_bipolar;
        self.flanger_phase += 2.0 * std::f32::consts::PI * modulation_rate * dt;
        if self.flanger_phase > 2.0 * std::f32::consts::PI {
            self.flanger_phase -= 2.0 * std::f32::consts::PI;
        }

        // Common volume attack. `release_curve` is the project's persisted
        // bipolar Attack Curve field; zero attack bypasses the ramp.
        let attack = self.settings.attack.clamp(0.0, 0.2);
        let hold = self.settings.hold.clamp(0.0, 1.0);
        let attack_gain = if attack <= 0.0 || t >= attack {
            1.0
        } else {
            let shaped = dsp::shape_curve(t / attack, self.settings.release_curve);
            self.attack_start_value + (1.0 - self.attack_start_value) * shaped
        };
        let amp_decay_t = (t - attack - hold).max(0.0);

        // 1. Body: sine, fast pitch drop (base + 95 Hz → base, rate 45).
        let base = self.settings.frequency.max(20.0) * self.drift.pitch;
        let body_freq = base + BODY_SWEEP_HZ * (-t * 45.0 * tfac).exp();
        self.body_phase += 2.0 * std::f32::consts::PI * body_freq * dt;
        if self.body_phase >= 2.0 * std::f32::consts::PI {
            self.body_phase -= 2.0 * std::f32::consts::PI;
        }
        let body_env = dsp::shape_curve(
            (-amp_decay_t * 32.0 * tfac).exp(),
            self.settings.decay_curve,
        );
        let body = self.body_phase.sin() * body_env;

        // 2. Noise: coloured noise high-passed by LP subtraction, exp tail,
        // level set by the Noise slider.
        let white = self.noise.next();
        let noise_hp = white - self.noise_lp.process(white);
        let noise_env = dsp::shape_curve(
            (-amp_decay_t * 18.0 * tfac).exp(),
            self.settings.decay_curve,
        );
        let noise = noise_hp * noise_env * self.settings.noise_level.clamp(0.0, 1.0) / 0.80;

        // 3. Metal: ring-mod pair (620 × 910 Hz at the reference base).
        let mscale = base / BODY_REF_HZ;
        self.metal_phase_1 += 2.0 * std::f32::consts::PI * 620.0 * mscale * dt;
        self.metal_phase_2 += 2.0 * std::f32::consts::PI * 910.0 * mscale * dt;
        if self.metal_phase_1 >= 2.0 * std::f32::consts::PI {
            self.metal_phase_1 -= 2.0 * std::f32::consts::PI;
        }
        if self.metal_phase_2 >= 2.0 * std::f32::consts::PI {
            self.metal_phase_2 -= 2.0 * std::f32::consts::PI;
        }
        let metal_env = dsp::shape_curve(
            (-amp_decay_t * 25.0 * tfac).exp(),
            self.settings.decay_curve,
        );
        let metal = self.metal_phase_1.sin() * self.metal_phase_2.sin() * metal_env;

        // Silence detection on the longest envelope (noise tail).
        if noise_env <= 0.0005 {
            self.active = false;
        }

        self.amp_proxy = attack_gain * noise_env;
        let mixed = (body * 0.50 + noise * 0.80 + metal * 0.18) * attack_gain * self.drift.level;

        // 3b. LP filter with A-D envelope (bipolar curves), exponential sweep
        // toward 20 kHz (same law as Tom/Buzz).
        let f_attack = self.settings.filter_attack.clamp(0.0, 0.5);
        let f_hold = self.settings.filter_hold.clamp(0.0, 1.0);
        let f_decay = self.settings.filter_env_decay.clamp(0.001, 1.5);
        let ft = self.filter_env_time;
        self.filter_env_time += dt;
        let env = if ft < f_attack {
            dsp::shape_curve(ft / f_attack.max(0.0001), self.settings.filter_atk_curve)
        } else if ft < f_attack + f_hold {
            1.0
        } else {
            let p = ((ft - f_attack - f_hold) / f_decay).clamp(0.0, 1.0);
            dsp::shape_curve(1.0 - p, self.settings.filter_dec_curve)
        };
        let f_base = self.settings.filter_freq.clamp(20.0, 20000.0);
        let f_sweep = (env * self.settings.filter_env_amount.clamp(0.0, 1.0)).clamp(0.0, 1.0);
        let cutoff = f_base * (20000.0 / f_base).powf(f_sweep);
        let cutoff = if filter_mod {
            // [183] The LFO opens the filter UPWARD from its base: the base is the
            // FLOOR of the sweep, not its centre. With the previous bipolar swing,
            // half of every cycle fell below the base and was eaten by the 20 Hz
            // clamp, so "Filter at minimum + Depth at max" never opened past
            // 160 Hz — 15.8 dB below the wide-open voice, i.e. inaudible, which is
            // the opposite of what a full-depth modulation should sound like.
            let octaves = modulation_depth * modulation_wet * FILTER_MOD_OCTAVE_SCALE;
            (cutoff * 2.0f32.powf(modulation_unipolar * octaves)).clamp(20.0, 20000.0)
        } else {
            cutoff
        };
        self.filter
            .set_lowpass(cutoff, self.settings.resonance, self.sample_rate);
        let mixed = self.filter.process(self.saturation.process_at(true, mixed));

        // 4. Flanger target. Filter Mod bypasses the delay line completely.
        let min_delay = FLANGER_MIN_DELAY_MS;
        let feedback = self.settings.flanger_feedback.clamp(0.0, 0.9);
        let flanged = if filter_mod {
            mixed
        } else {
            let len = self.flanger_len;
            let sweep = if modulation_depth >= 0.0 {
                modulation_unipolar
            } else {
                1.0 - modulation_unipolar
            };
            let delay_samples =
                ((min_delay + modulation_depth.abs() * sweep) * 0.001 * self.sample_rate)
                    .clamp(0.0, (len - 2) as f32);
            let delay_int = delay_samples.floor() as usize;
            let frac = delay_samples - delay_int as f32;
            let p1 = (self.flanger_pos + len - delay_int.min(len - 1)) % len;
            let p2 = (self.flanger_pos + len - delay_int.min(len - 1) - 1) % len;
            let delayed = self.flanger_buf[p1] * (1.0 - frac) + self.flanger_buf[p2] * frac;
            self.flanger_buf[self.flanger_pos] = mixed + delayed * feedback;
            self.flanger_pos = (self.flanger_pos + 1) % len;
            mixed * (1.0 - modulation_wet) + delayed * modulation_wet
        };

        // 5. Recipe drive (fixed tanh) inside the standard saturation chain.
        let driven = (flanged * 2.2).tanh() * 0.8;

        // Volume post-saturation: the knob sets the final level, not the drive.
        self.dc_block
            .process(self.saturation.process_at(false, driven))
            * self.settings.volume
    }

    fn is_active(&self) -> bool {
        self.active
    }

    fn reset(&mut self) {
        self.active = false;
        self.env_time = 0.0;
        self.attack_start_value = 0.0;
        self.amp_proxy = 0.0;
        self.filter_env_time = 0.0;
        self.flanger_buf.fill(0.0);
        self.flanger_pos = 0;
        self.flanger_phase = 0.0;
        self.noise_lp.reset();
        self.filter.reset();
        self.dc_block.reset();
    }

    fn set_settings(&mut self, settings: VoiceSettings) {
        // No envelopes to rebuild (time-formula envelopes); saturation and the
        // flanger read the new values live. The noise source is swapped only
        // when its colour actually changes.
        let new = SdrexSettings::from(settings);
        let modulation_changed = new.modulation_type != self.settings.modulation_type;
        if new.noise_type != self.settings.noise_type {
            self.noise = dsp::NoiseSource::new(new.noise_type, 0xD3EC_1A00);
        }
        self.settings = new;
        if modulation_changed {
            self.flanger_buf.fill(0.0);
            self.flanger_pos = 0;
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
            0 => self.settings.flanger_rate = value,
            1 => self.settings.modulation_fade_ms = value,
            2 => self.settings.flanger_depth = value,
            3 => self.settings.flanger_feedback = value,
            4 => self.settings.flanger_wet = value,
            5 => {
                self.settings.saturation_type = value as u8;
                self.saturation.saturation_type =
                    saturation::SaturationType::from(self.settings.saturation_type);
            }
            6 => {
                self.settings.saturation_amount = value;
                self.saturation.amount = value;
            }
            7 => {
                self.settings.saturation_mix = value;
                self.saturation.mix = value;
            }
            8 => {
                self.settings.saturation_output_gain = value;
                self.saturation.output_gain = value;
            }
            9 => {
                self.settings.saturation_pre_filter = value;
                self.saturation.pre_filter = value > 0.5;
            }
            10 => self.settings.noise_level = value,
            11 => {
                let nt = value as u8;
                if self.settings.noise_type != nt {
                    self.settings.noise_type = nt;
                    self.noise = dsp::NoiseSource::new(nt, 0xD3EC_1A00);
                }
            }
            12 => self.settings.modulation_free_phase = value,
            13 => self.settings.filter_attack = value,
            14 => self.settings.filter_atk_curve = value,
            15 => self.settings.filter_dec_curve = value,
            16 => self.settings.filter_hold = value,
            17 => {
                let modulation_type = value as u8;
                if self.settings.modulation_type != modulation_type {
                    self.settings.modulation_type = modulation_type;
                    self.flanger_buf.fill(0.0);
                    self.flanger_pos = 0;
                }
            }
            _ => {}
        }
        self.saturation.update_compensation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_with(settings: VoiceSettings) -> SdrexVoice {
        SdrexVoice::new(44100.0, SdrexSettings::from(settings))
    }

    #[test]
    fn produces_sound_on_trigger() {
        let mut voice = voice_with(VoiceSettings::sdrex());
        voice.trigger();
        let mut peak = 0.0f32;
        for _ in 0..44100 {
            peak = peak.max(voice.process_sample().abs());
        }
        assert!(peak > 0.05, "SDrex should produce sound, peak {peak}");
    }

    #[test]
    fn output_stays_finite_and_stops() {
        let mut voice = voice_with(VoiceSettings::sdrex());
        for _ in 0..20 {
            voice.trigger();
            for _ in 0..44100 * 2 {
                let s = voice.process_sample();
                assert!(s.is_finite(), "non-finite sample");
            }
        }
        let mut tail = 0.0f32;
        for _ in 0..44100 {
            tail = tail.max(voice.process_sample().abs());
        }
        assert!(
            tail < 1e-3,
            "voice should be silent after hits, tail {tail}"
        );
        assert!(!voice.is_active());
    }

    #[test]
    fn flanger_wet_changes_the_output() {
        let render = |wet: f32| -> Vec<f32> {
            let mut settings = VoiceSettings::sdrex();
            settings.special[4] = wet;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..22050).map(|_| voice.process_sample()).collect()
        };
        let dry = render(0.0);
        let wet = render(0.8);
        let max_diff = dry
            .iter()
            .zip(wet.iter())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_diff > 0.01,
            "flanger wet should change the output (max diff {max_diff})"
        );
    }

    /// [183] Regression guard for the user-reported symptom: "Filter at 20 Hz +
    /// Depth at max and I hear nothing, where I should get the maximum effect".
    ///
    /// The Filter LFO must lift the cutoff ABOVE its base, so even at the very
    /// bottom of the Filter range a full-depth modulation stays audible. With the
    /// old bipolar swing this measured 15.8 dB under the wide-open voice (half of
    /// every LFO cycle fell under the 20 Hz clamp); it now sits within ~6 dB.
    #[test]
    fn filter_lfo_at_the_lowest_base_stays_audible_at_full_depth() {
        let peak = |filter_hz: f32, depth: f32, wet: f32, mode: f32| {
            let mut settings = VoiceSettings::sdrex();
            settings.analog = 0.0;
            settings.filter_freq = filter_hz;
            settings.filter_env_amount = 0.0;
            settings.special[2] = depth;
            settings.special[4] = wet;
            settings.special[17] = mode;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..24_000)
                .map(|_| voice.process_sample())
                .fold(0.0f32, |m, x| m.max(x.abs()))
        };

        let open = peak(20_000.0, 0.0, 0.0, 1.0);
        let closed = peak(20.0, 0.0, 1.0, 1.0);
        let modulated = peak(20.0, 3.0, 1.0, 1.0);

        // A 20 Hz low-pass on its own IS silence for this voice — that part is
        // the filter doing its job.
        let closed_db = 20.0 * (closed / open).log10();
        assert!(
            closed_db < -25.0,
            "a 20 Hz low-pass should mute the voice (got {closed_db:.1} dB)"
        );
        // Full depth must bring it back to within 6 dB of the open filter.
        let modulated_db = 20.0 * (modulated / open).log10();
        assert!(
            modulated_db > -6.0,
            "full-depth Filter LFO at the lowest base is inaudible ({modulated_db:.1} dB under the open filter; the bipolar law measured -15.8 dB)"
        );
        // ...and it must still be a modulation, not a bypass.
        assert!(
            modulated_db < -1.0,
            "the Filter LFO should still shape the sound ({modulated_db:.1} dB)"
        );
    }

    /// [181] The modulation Fade-in replaced the flanger's Delay slider: the LFO
    /// effect must ramp in over the given time, in BOTH modulation modes.
    #[test]
    fn modulation_fade_in_ramps_the_modulation_in_both_modes() {
        let render = |mode: f32, fade_ms: f32| {
            let mut settings = VoiceSettings::sdrex();
            settings.analog = 0.0;
            settings.filter_freq = 800.0;
            settings.filter_env_amount = 0.0;
            settings.special[1] = fade_ms;
            settings.special[2] = 2.0; // depth
            settings.special[4] = 1.0; // wet
            settings.special[17] = mode;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..8192)
                .map(|_| voice.process_sample())
                .collect::<Vec<_>>()
        };
        for (mode, name) in [(0.0f32, "flanger"), (1.0, "filter LFO")] {
            let instant = render(mode, 0.0);
            let faded = render(mode, 200.0);
            // Early on, the faded version still has (almost) no modulation.
            let early = instant
                .iter()
                .zip(&faded)
                .take(4800) // first 100 ms at 48 kHz
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                early > 0.01,
                "{name}: a 200 ms fade-in must delay the modulation (max diff {early})"
            );
        }
    }

    #[test]
    fn negative_depth_sweeps_the_other_way() {
        let render = |mode: f32, depth: f32| {
            let mut settings = VoiceSettings::sdrex();
            settings.analog = 0.0;
            settings.filter_freq = 800.0;
            settings.filter_env_amount = 0.0;
            settings.special[0] = 7.0;
            settings.special[1] = 0.0;
            settings.special[2] = depth;
            settings.special[4] = 1.0;
            settings.special[17] = mode;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..8192)
                .map(|_| voice.process_sample())
                .collect::<Vec<f32>>()
        };
        let energy = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>();
        // Filter LFO: +3 opens the cutoff above its base, -3 closes it below.
        let up = render(1.0, 3.0);
        let down = render(1.0, -3.0);
        assert!(up.iter().chain(&down).all(|x| x.is_finite()));
        assert!(
            energy(&up) > energy(&down),
            "a negative Depth must close the filter, a positive one open it"
        );
        // Flanger: both signs stay finite and the sign changes the sound.
        let fl_up = render(0.0, 3.0);
        let fl_down = render(0.0, -3.0);
        assert!(fl_up.iter().chain(&fl_down).all(|x| x.is_finite()));
        let diff = fl_up.iter().zip(&fl_down).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
        assert!(diff > 0.001, "the flanger sweep direction must be audible");
    }

    #[test]
    fn filter_mod_routes_rate_depth_and_wet_to_the_cutoff() {
        let render = |mode: f32, depth: f32, wet: f32, delay: f32, feedback: f32| {
            let mut settings = VoiceSettings::sdrex();
            settings.analog = 0.0;
            settings.filter_freq = 800.0;
            settings.filter_env_amount = 0.0;
            settings.special[0] = 7.0;
            settings.special[1] = delay;
            settings.special[2] = depth;
            settings.special[3] = feedback;
            settings.special[4] = wet;
            settings.special[17] = mode;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..8192)
                .map(|_| voice.process_sample())
                .collect::<Vec<_>>()
        };
        let difference = |a: &[f32], b: &[f32]| {
            a.iter()
                .zip(b)
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max)
        };

        let filter_dry = render(1.0, 2.0, 0.0, 0.0, 0.0);
        let filter_wet = render(1.0, 2.0, 1.0, 0.0, 0.0);
        assert!(
            difference(&filter_dry, &filter_wet) > 0.01,
            "Filter Mod Wet should scale cutoff modulation"
        );
        let filter_zero_depth = render(1.0, 0.0, 1.0, 0.0, 0.0);
        assert!(
            difference(&filter_zero_depth, &filter_wet) > 0.01,
            "Filter Mod Depth should change cutoff modulation"
        );

        // [181] Feedback is the only flanger-specific control left; the former
        // Delay slot is now the modulation Fade-in, which applies in BOTH modes.
        let ignored_a = render(1.0, 2.0, 1.0, 0.0, 0.0);
        let ignored_b = render(1.0, 2.0, 1.0, 0.0, 0.9);
        assert_eq!(
            ignored_a, ignored_b,
            "Feedback must be ignored in Filter LFO mode"
        );

        let flanger = render(0.0, 2.0, 1.0, 0.7, 0.38);
        assert!(
            difference(&flanger, &filter_wet) > 0.01,
            "switching the modulation target should change the output"
        );
    }

    #[test]
    fn changing_modulation_target_clears_the_delay_line() {
        let mut voice = voice_with(VoiceSettings::sdrex());
        voice.trigger();
        for _ in 0..256 {
            voice.process_sample();
        }
        assert!(voice.flanger_pos > 0);
        assert!(voice.flanger_buf.iter().any(|sample| *sample != 0.0));

        voice.set_special_param(17, 1.0);
        assert_eq!(voice.flanger_pos, 0);
        assert!(voice.flanger_buf.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn noise_level_and_type_change_the_output() {
        let render = |level: f32, ntype: f32| -> Vec<f32> {
            let mut settings = VoiceSettings::sdrex();
            settings.special[10] = level;
            settings.special[11] = ntype;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..22050).map(|_| voice.process_sample()).collect()
        };
        let full_white = render(0.8, 0.0);
        let muted = render(0.0, 0.0);
        let brown = render(0.8, 2.0);
        let diff = |a: &[f32], b: &[f32]| {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| (x - y).abs())
                .fold(0.0f32, f32::max)
        };
        assert!(
            diff(&full_white, &muted) > 0.01,
            "noise level must change the output"
        );
        assert!(
            diff(&full_white, &brown) > 0.005,
            "noise type must change the output"
        );
    }

    #[test]
    fn filter_env_sweeps_the_output() {
        let render = |amount: f32| -> f32 {
            let mut settings = VoiceSettings::sdrex();
            settings.filter_freq = 500.0; // low resting cutoff
            settings.filter_env_amount = amount;
            settings.filter_env_decay = 0.2;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..4410).map(|_| voice.process_sample().abs()).sum()
        };
        let closed = render(0.0);
        let swept = render(1.0);
        assert!(
            swept > closed * 1.2,
            "filter env should brighten the hit (closed={closed}, swept={swept})"
        );
    }

    #[test]
    fn amp_and_filter_hold_delay_their_decays() {
        let amp_tail = |hold: f32| -> f32 {
            let mut settings = VoiceSettings::sdrex();
            settings.attack = 0.0;
            settings.hold = hold;
            settings.decay = 0.05;
            settings.analog = 0.0;
            settings.special[4] = 0.0;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..8820)
                .map(|index| (index, voice.process_sample().abs()))
                .filter(|(index, _)| *index >= 6615)
                .map(|(_, sample)| sample)
                .sum()
        };
        let no_amp_hold = amp_tail(0.0);
        let amp_hold = amp_tail(0.2);
        assert!(
            amp_hold > no_amp_hold * 5.0,
            "Amp Hold should postpone the tail (off={no_amp_hold}, on={amp_hold})"
        );

        let filtered_tail = |hold: f32| -> f32 {
            let mut settings = VoiceSettings::sdrex();
            settings.attack = 0.0;
            settings.hold = 0.2;
            settings.decay = 0.3;
            settings.filter_freq = 200.0;
            settings.filter_env_amount = 1.0;
            settings.filter_env_decay = 0.01;
            settings.special[13] = 0.0;
            settings.special[16] = hold;
            settings.analog = 0.0;
            settings.special[4] = 0.0;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..4410)
                .map(|index| (index, voice.process_sample().abs()))
                .filter(|(index, _)| *index >= 2205)
                .map(|(_, sample)| sample)
                .sum()
        };
        let no_filter_hold = filtered_tail(0.0);
        let filter_hold = filtered_tail(0.1);
        assert!(
            filter_hold > no_filter_hold * 1.3,
            "Filter Hold should keep the sweep open (off={no_filter_hold}, on={filter_hold})"
        );
    }

    #[test]
    fn volume_attack_and_curves_change_the_output() {
        let render = |attack: f32, attack_curve: f32, decay_curve: f32, count: usize| {
            let mut settings = VoiceSettings::sdrex();
            settings.analog = 0.0;
            settings.attack = attack;
            settings.release_curve = attack_curve;
            settings.decay_curve = decay_curve;
            settings.special[4] = 0.0; // isolate the envelope from the flanger
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..count)
                .map(|_| voice.process_sample().abs())
                .sum::<f32>()
        };

        let immediate = render(0.0, 0.0, 0.0, 256);
        let attacked = render(0.05, 0.0, 0.0, 256);
        assert!(
            attacked < immediate * 0.25,
            "Attack should soften the onset (immediate={immediate}, attacked={attacked})"
        );

        let attack_curve_a = render(0.05, -1.0, 0.0, 2205);
        let attack_curve_b = render(0.05, 1.0, 0.0, 2205);
        assert!(
            (attack_curve_a - attack_curve_b).abs() > 1.0,
            "Attack Curve should audibly change the ramp"
        );

        let decay_curve_a = render(0.0, 0.0, -1.0, 22050);
        let decay_curve_b = render(0.0, 0.0, 1.0, 22050);
        assert!(
            (decay_curve_a - decay_curve_b).abs() > 10.0,
            "Decay Curve should audibly change the tail"
        );
    }

    #[test]
    fn modulation_free_phase_controls_lfo_start() {
        let mut fixed_settings = VoiceSettings::sdrex();
        fixed_settings.special[12] = 0.0;
        let mut fixed = voice_with(fixed_settings);
        fixed.flanger_phase = 1.25;
        fixed.trigger();
        assert_eq!(fixed.flanger_phase, 0.0);
        for _ in 0..128 {
            fixed.process_sample();
        }
        assert!(fixed.flanger_phase > 0.0);
        fixed.trigger();
        assert_eq!(fixed.flanger_phase, 0.0);

        let mut free_settings = VoiceSettings::sdrex();
        free_settings.special[12] = 1.0;
        let mut free = voice_with(free_settings);
        free.flanger_phase = 1.25;
        free.body_phase = 1.0;
        free.trigger();
        assert_eq!(free.flanger_phase, 1.25);
        assert_eq!(
            free.body_phase, 0.0,
            "Free Phase must not affect oscillators"
        );
        for _ in 0..128 {
            free.process_sample();
        }
        let phase_before_retrigger = free.flanger_phase;
        free.trigger();
        assert_eq!(free.flanger_phase, phase_before_retrigger);
    }

    #[test]
    fn decay_stretches_the_tail() {
        let energy = |decay: f32| -> f32 {
            let mut settings = VoiceSettings::sdrex();
            settings.decay = decay;
            let mut voice = voice_with(settings);
            voice.trigger();
            (0..44100).map(|_| voice.process_sample().abs()).sum()
        };
        let short = energy(0.05);
        let long = energy(0.8);
        assert!(
            long > short * 1.5,
            "longer decay should carry more energy (short={short}, long={long})"
        );
    }

    #[test]
    fn extreme_settings_stay_finite_at_supported_sample_rates() {
        for sample_rate in [8_000.0, 44_100.0, 192_000.0, 384_000.0] {
            let mut settings = VoiceSettings::sdrex();
            settings.frequency = 600.0;
            settings.decay = 1.5;
            settings.hold = 2.0;
            settings.filter_freq = 20.0;
            settings.filter_env_amount = 1.0;
            settings.filter_env_decay = 1.5;
            settings.special[0] = 20.0;
            settings.special[1] = 3.0;
            settings.special[2] = 3.0;
            settings.special[3] = 0.9;
            settings.special[4] = 1.0;
            settings.special[10] = 1.0;
            settings.special[11] = 3.0;
            settings.special[13] = 0.5;
            settings.special[14] = -1.0;
            settings.special[15] = 1.0;
            settings.special[16] = 2.0;
            settings.special[17] = 1.0;

            let mut voice = SdrexVoice::new(sample_rate, SdrexSettings::from(settings));
            voice.trigger();
            for _ in 0..8192 {
                assert!(
                    voice.process_sample().is_finite(),
                    "non-finite output at {sample_rate} Hz"
                );
            }
            assert!(voice.flanger_pos < voice.flanger_len);
            assert!(voice.flanger_len <= FLANGER_BUFFER_LEN);
        }
    }
}
