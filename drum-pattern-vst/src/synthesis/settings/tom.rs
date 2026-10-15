use crate::synthesis::VoiceSettings;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TomSettings {
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
    pub analog: f32,
    pub stick_attack: f32,
    /// Filter Q (0.7071 = the former fixed Butterworth response).
    pub resonance: f32,
    pub saturation_type: u8,
    pub saturation_amount: f32,
    pub saturation_mix: f32,
    pub saturation_output_gain: f32,
    pub saturation_pre_filter: f32,
    pub algo: u8,
}

impl From<VoiceSettings> for TomSettings {
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
            analog: v.analog,
            stick_attack: v.special[0],
            resonance: super::resonance_or(v.special[6], std::f32::consts::FRAC_1_SQRT_2),
            saturation_type: v.special[1] as u8,
            saturation_amount: v.special[2],
            saturation_mix: v.special[3],
            saturation_output_gain: v.special[4],
            saturation_pre_filter: v.special[5],
            algo: v.algo,
        }
    }
}

impl From<TomSettings> for VoiceSettings {
    fn from(t: TomSettings) -> Self {
        let mut special = [0.0f32; 32];
        special[0] = t.stick_attack;
        special[1] = t.saturation_type as f32;
        special[2] = t.saturation_amount;
        special[3] = t.saturation_mix;
        special[4] = t.saturation_output_gain;
        special[5] = t.saturation_pre_filter;
        special[6] = t.resonance;
        Self {
            frequency: t.frequency,
            attack: t.attack,
            decay: t.decay,
            decay_curve: t.decay_curve,
            release: t.release,
            release_curve: t.release_curve,
            volume: t.volume,
            filter_freq: t.filter_freq,
            filter_env_amount: t.filter_env_amount,
            filter_env_decay: t.filter_env_decay,
            hold: 0.0,
            analog: t.analog,
            stereo: 0.0,
            algo: t.algo,
            special,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::settings_roundtrip_test!(tom1_settings_roundtrip, tom1, TomSettings);
    crate::settings_roundtrip_test!(tom2_settings_roundtrip, tom2, TomSettings);
    crate::settings_roundtrip_test!(tom3_settings_roundtrip, tom3, TomSettings);
}
