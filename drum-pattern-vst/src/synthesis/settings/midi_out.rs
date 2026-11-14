use crate::synthesis::VoiceSettings;

/// [95] MIDI Out controls. Each CC occupies `(enabled, number, value)`;
/// Program Change occupies `(enabled, program)` after the four CCs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MidiOutSettings {
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
    pub cc_enabled: [f32; 4],
    pub cc_number: [f32; 4],
    pub cc_value: [f32; 4],
    pub note: f32,
    /// 0 = follow the global channel; 1..16 = per-instrument override.
    pub channel: f32,
    pub program_enabled: f32,
    pub program: f32,
    pub algo: u8,
}

impl From<VoiceSettings> for MidiOutSettings {
    fn from(v: VoiceSettings) -> Self {
        let mut cc_enabled = [0.0; 4];
        let mut cc_number = [0.0; 4];
        let mut cc_value = [0.0; 4];
        for i in 0..4 {
            cc_enabled[i] = v.special[i * 3];
            cc_number[i] = v.special[i * 3 + 1];
            cc_value[i] = v.special[i * 3 + 2];
        }
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
            cc_enabled,
            cc_number,
            cc_value,
            note: v.special[14],
            channel: v.special[15],
            program_enabled: v.special[12],
            program: v.special[13],
            algo: v.algo,
        }
    }
}

impl From<MidiOutSettings> for VoiceSettings {
    fn from(s: MidiOutSettings) -> Self {
        let mut special = [0.0f32; 32];
        for i in 0..4 {
            special[i * 3] = s.cc_enabled[i];
            special[i * 3 + 1] = s.cc_number[i];
            special[i * 3 + 2] = s.cc_value[i];
        }
        special[12] = s.program_enabled;
        special[13] = s.program;
        special[14] = s.note;
        special[15] = s.channel;
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
    crate::settings_roundtrip_test!(midi_out_settings_roundtrip, midi_out, MidiOutSettings);
}
