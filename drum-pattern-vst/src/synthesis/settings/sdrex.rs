use crate::synthesis::VoiceSettings;

/// Typed settings for the SDrex voice — a metallic snare (sine body with fast
/// pitch drop + HP noise + ring-mod metal pair) fed through a built-in
/// flanger and a fixed tanh drive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SdrexSettings {
    pub frequency: f32,
    pub attack: f32,
    pub decay: f32,
    pub decay_curve: f32,
    pub release: f32,
    pub release_curve: f32,
    pub volume: f32,
    pub filter_freq: f32,
    pub filter_env_amount: f32,
    pub filter_env_decay: f32,
    pub hold: f32,
    pub analog: f32,
    pub stereo: f32,
    /// Flanger LFO rate in Hz (default 5.7).
    pub flanger_rate: f32,
    /// Flanger minimum delay in ms (default 0.7).
    /// [181] Modulation fade-in in ms: the LFO's effect ramps in over this time
    /// after each hit, in BOTH modulation modes. Replaces the flanger's minimum
    /// delay, which is now the fixed `FLANGER_MIN_DELAY_MS` constant.
    pub modulation_fade_ms: f32,
    /// Flanger LFO depth in ms (default 1.8).
    pub flanger_depth: f32,
    /// Flanger feedback 0..0.9 (default 0.38).
    pub flanger_feedback: f32,
    /// Flanger dry/wet 0..1 (default 0.32).
    pub flanger_wet: f32,
    /// Balance between the noise layer (0) and the synth layers, body + metal (1);
    /// 0.5 = the recipe, both at full level.
    pub balance: f32,
    /// Noise colour: 0 = white, 1 = pink, 2 = brown, 3 = blue.
    pub noise_type: u8,
    /// 1.0 = preserve the flanger LFO phase between hits; 0 = restart the LFO
    /// phase from zero on every trigger.
    pub modulation_free_phase: f32,
    /// 0 = flanger, 1 = LFO modulation of the low-pass cutoff.
    pub modulation_type: u8,
    /// Filter A-H-D envelope: attack time (seconds); decay =
    /// `filter_env_decay`, depth = `filter_env_amount`.
    pub filter_attack: f32,
    /// Filter envelope hold time in seconds.
    pub filter_hold: f32,
    /// Bipolar filter-ATTACK curve: -1 = concave, 0 = linear, +1 = convex.
    pub filter_atk_curve: f32,
    /// Bipolar filter-DECAY curve.
    pub filter_dec_curve: f32,
    /// Low-pass Q (0.7071 = the former fixed Butterworth response).
    pub resonance: f32,
    pub saturation_type: u8,
    pub saturation_amount: f32,
    pub saturation_mix: f32,
    pub saturation_output_gain: f32,
    pub saturation_pre_filter: f32,
    pub algo: u8,
}

impl From<VoiceSettings> for SdrexSettings {
    fn from(v: VoiceSettings) -> Self {
        Self {
            frequency: v.frequency,
            attack: v.attack,
            decay: v.decay,
            decay_curve: v.decay_curve,
            release: v.release,
            release_curve: v.release_curve,
            volume: v.volume,
            filter_freq: v.filter_freq,
            filter_env_amount: v.filter_env_amount,
            filter_env_decay: v.filter_env_decay,
            hold: v.hold,
            analog: v.analog,
            stereo: v.stereo,
            flanger_rate: v.special[0],
            modulation_fade_ms: v.special[1],
            flanger_depth: v.special[2],
            flanger_feedback: v.special[3],
            flanger_wet: v.special[4],
            balance: v.special[19],
            noise_type: v.special[11] as u8,
            modulation_free_phase: v.special[12],
            modulation_type: v.special[17] as u8,
            filter_attack: v.special[13],
            filter_atk_curve: v.special[14],
            filter_dec_curve: v.special[15],
            resonance: super::resonance_or(v.special[18], std::f32::consts::FRAC_1_SQRT_2),
            filter_hold: v.special[16],
            saturation_type: v.special[5] as u8,
            saturation_amount: v.special[6],
            saturation_mix: v.special[7],
            saturation_output_gain: v.special[8],
            saturation_pre_filter: v.special[9],
            algo: v.algo,
        }
    }
}

impl From<SdrexSettings> for VoiceSettings {
    fn from(s: SdrexSettings) -> Self {
        let mut special = [0.0f32; 32];
        special[0] = s.flanger_rate;
        special[1] = s.modulation_fade_ms;
        special[2] = s.flanger_depth;
        special[3] = s.flanger_feedback;
        special[4] = s.flanger_wet;
        special[19] = s.balance;
        special[11] = s.noise_type as f32;
        special[12] = s.modulation_free_phase;
        special[17] = s.modulation_type as f32;
        special[13] = s.filter_attack;
        special[14] = s.filter_atk_curve;
        special[15] = s.filter_dec_curve;
        special[18] = s.resonance;
        special[16] = s.filter_hold;
        special[5] = s.saturation_type as f32;
        special[6] = s.saturation_amount;
        special[7] = s.saturation_mix;
        special[8] = s.saturation_output_gain;
        special[9] = s.saturation_pre_filter;
        Self {
            frequency: s.frequency,
            attack: s.attack,
            decay: s.decay,
            decay_curve: s.decay_curve,
            release: s.release,
            release_curve: s.release_curve,
            volume: s.volume,
            filter_freq: s.filter_freq,
            filter_env_amount: s.filter_env_amount,
            filter_env_decay: s.filter_env_decay,
            hold: s.hold,
            analog: s.analog,
            stereo: s.stereo,
            algo: s.algo,
            special,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::settings_roundtrip_test!(sdrex_settings_roundtrip, sdrex, SdrexSettings);

    #[test]
    fn modulation_phase_target_and_filter_hold_roundtrip() {
        let mut source = VoiceSettings::sdrex();
        source.hold = 1.25;
        source.filter_env_decay = 1.5;
        source.special[12] = 1.0;
        source.special[16] = 1.5;
        source.special[17] = 1.0;

        let restored = VoiceSettings::from(SdrexSettings::from(source));
        assert_eq!(restored.hold, 1.25);
        assert_eq!(restored.filter_env_decay, 1.5);
        assert_eq!(restored.special[12], 1.0);
        assert_eq!(restored.special[16], 1.5);
        assert_eq!(restored.special[17], 1.0);
    }
}
