//! [95] MIDI Out: an instrument lane with no audio engine.
//!
//! Notes and enabled CC / Program Change messages are emitted by the plugin's
//! trigger path. This voice only keeps the lane silent on the audio side.

use super::{Voice, VoiceSettings};

pub struct MidiOutVoice;

impl Voice for MidiOutVoice {
    fn trigger(&mut self) {}

    fn process_sample(&mut self) -> f32 {
        0.0
    }

    fn process_sample_stereo(&mut self) -> (f32, f32) {
        (0.0, 0.0)
    }

    fn is_active(&self) -> bool {
        false
    }

    fn reset(&mut self) {}

    fn set_settings(&mut self, _settings: VoiceSettings) {}

    fn set_algo(&mut self, _algo: u8) {}

    fn set_special_param(&mut self, _index: usize, _value: f32) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_out_is_always_silent() {
        let mut voice = MidiOutVoice;
        voice.set_settings(VoiceSettings::midi_out());
        voice.trigger();
        assert!(!voice.is_active());
        assert_eq!(voice.process_sample_stereo(), (0.0, 0.0));
        voice.reset();
        assert_eq!(voice.process_sample(), 0.0);
    }
}
