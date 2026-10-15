use crate::synthesis::VoiceSettings;

/// Typed settings for the Buzz voice — a tonal percussion (pitched oscillator
/// + adjustable noise) fed through a fast amplitude gate/retrigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuzzSettings {
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
    /// Gate retrigger frequency in Hz (1..150).
    pub gate_rate: f32,
    /// Gate dry/wet: 0 = source passes untouched, 1 = full amplitude chop.
    pub gate_depth: f32,
    /// Gate envelope decay length + curve within one cycle (0 = smooth/long,
    /// 1 = razor/short).
    pub gate_shape: f32,
    /// Noise blend: 0 = pure tonal, 1 = pure noise.
    pub noise_amount: f32,
    /// Noise colour: 0=white, 1=pink, 2=brown, 3=blue.
    pub noise_type: u8,
    /// Percussive downward pitch sweep amount (0..1).
    pub pitch_sweep: f32,
    /// Tonal oscillator waveform: 0 = sine, 1 = square, 2 = saw.
    pub waveform: u8,
    /// Filter A-H-D envelope: attack + hold (seconds); decay = `filter_env_decay`,
    /// depth = `filter_env_amount`.
    pub filter_env_attack: f32,
    pub filter_env_hold: f32,
    /// Bipolar filter-DECAY curve: -1 = concave (holds then drops), 0 = linear,
    /// +1 = convex (fast → slow, snappy).
    pub filter_curve: f32,
    /// Bipolar filter-ATTACK curve: -1 = fast → slow, 0 = linear, +1 = slow → fast.
    pub filter_atk_curve: f32,
    /// Base filter type: 0 = low-pass, 1 = high-pass, 2 = band-pass.
    pub filter_type: u8,
    /// Filter Q for low-/high-pass (0.9 = the former fixed value); the band-pass
    /// keeps its tighter ratio on top of it.
    pub resonance: f32,
    pub saturation_type: u8,
    pub saturation_amount: f32,
    pub saturation_mix: f32,
    pub saturation_output_gain: f32,
    pub saturation_pre_filter: f32,
    /// 0 = Smooth (ramp-from-current gate retrigger), 1 = Razor (from zero).
    pub algo: u8,
}

impl From<VoiceSettings> for BuzzSettings {
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
            gate_rate: v.special[0],
            gate_depth: v.special[1],
            gate_shape: v.special[2],
            noise_amount: v.special[3],
            noise_type: v.special[4] as u8,
            pitch_sweep: v.special[5],
            waveform: v.special[11] as u8,
            filter_env_attack: v.special[12],
            filter_env_hold: v.special[13],
            filter_type: v.special[14] as u8,
            resonance: super::resonance_or(v.special[17], 0.9),
            filter_curve: v.special[15],
            filter_atk_curve: v.special[16],
            saturation_type: v.special[6] as u8,
            saturation_amount: v.special[7],
            saturation_mix: v.special[8],
            saturation_output_gain: v.special[9],
            saturation_pre_filter: v.special[10],
            algo: v.algo,
        }
    }
}

impl From<BuzzSettings> for VoiceSettings {
    fn from(b: BuzzSettings) -> Self {
        let mut special = [0.0f32; 32];
        special[0] = b.gate_rate;
        special[1] = b.gate_depth;
        special[2] = b.gate_shape;
        special[3] = b.noise_amount;
        special[4] = b.noise_type as f32;
        special[5] = b.pitch_sweep;
        special[11] = b.waveform as f32;
        special[12] = b.filter_env_attack;
        special[13] = b.filter_env_hold;
        special[14] = b.filter_type as f32;
        special[17] = b.resonance;
        special[15] = b.filter_curve;
        special[16] = b.filter_atk_curve;
        special[6] = b.saturation_type as f32;
        special[7] = b.saturation_amount;
        special[8] = b.saturation_mix;
        special[9] = b.saturation_output_gain;
        special[10] = b.saturation_pre_filter;
        Self {
            frequency: b.frequency,
            attack: b.attack,
            decay: b.decay,
            decay_curve: b.decay_curve,
            release: b.release,
            release_curve: b.release_curve,
            volume: b.volume,
            filter_freq: b.filter_freq,
            filter_env_amount: b.filter_env_amount,
            filter_env_decay: b.filter_env_decay,
            hold: b.hold,
            analog: b.analog,
            stereo: b.stereo,
            algo: b.algo,
            special,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    crate::settings_roundtrip_test!(buzz_settings_roundtrip, buzz, BuzzSettings);
}
