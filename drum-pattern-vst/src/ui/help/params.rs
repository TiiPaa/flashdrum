//! [283] Help texts of the sound parameters (the Lane Editor rows). A label
//! does not always mean the same thing from one instrument to the next (Snap,
//! Tone, Resonance...), so standard rows are keyed by (voice, field) and
//! special rows by their registry name.

use crate::instrument_registry::{StandardField as F, INSTRUMENTS};
use crate::synthesis::DrumVoice as V;

fn is(voice: usize, voices: &[V]) -> bool {
    voices.iter().any(|v| *v as usize == voice)
}

const SAMPLERS: [V; 4] = [V::Bd606, V::Sd606, V::Ch606, V::Oh606];
const TOMS: [V; 3] = [V::Tom1, V::Tom2, V::Tom3];

const NOTE_SWITCH: &str = "\nHz | Note on the right: show it in hertz, or as a musical note set by semitones (the note of the body, after the sweep).";

/// Help of a standard row (Frequency, Decay, Filter...) on `voice`.
pub fn standard(voice: usize, field: F) -> String {
    let rift_like = is(voice, &[V::Rift, V::OneShot]);
    let sampler = is(voice, &SAMPLERS);
    let text: &str = match field {
        F::Freq => {
            if sampler || rift_like {
                "Pitch, in semitones (0 = the original pitch). Pitch Fine adds cents."
            } else if voice == V::Kick as usize {
                return format!("Frequency: where the kick's pitch sweep starts; the body settles lower, about a third of it.{NOTE_SWITCH}");
            } else if voice == V::BassDrum808 as usize {
                return format!("Frequency: pitch of the kick's body.{NOTE_SWITCH}");
            } else if voice == V::Bd6Ac as usize {
                return format!("Frequency: pitch of the kick's body.{NOTE_SWITCH}");
            } else if is(voice, &[V::HiHat, V::OpenHiHat]) {
                "Tone: centre of the boost that colours the noise. Higher = brighter, thinner hats."
            } else if is(voice, &[V::Hh6Ac, V::Oh6Ac]) {
                "Tone: tuning of the metallic sound. Higher = brighter."
            } else if voice == V::Cl6Ac as usize {
                "Tone: colour of the clap's noise. Higher = brighter."
            } else if is(voice, &TOMS) {
                "Frequency: pitch of the tom. It drops a little at the start of each hit (the Deep algorithm plays lower)."
            } else if voice == V::Snare as usize {
                "Frequency: pitch of the snare's body, the tone under the noise."
            } else if voice == V::Snare606 as usize {
                "Frequency: pitch of the resonant body."
            } else if voice == V::Ride as usize {
                "Frequency: base pitch of the ride's metallic tones."
            } else if voice == V::Perc1 as usize {
                "Frequency: starting pitch of the oscillator, before the Sweep."
            } else if voice == V::Sdrex as usize {
                "Frequency: pitch of the body; the metallic layer follows it."
            } else {
                "Frequency: pitch of the sound."
            }
        }
        F::Attack => {
            if sampler || voice == V::Rift as usize {
                "Attack: fade-in at the start of each hit (up to 80 ms). 0 = instant, the punchiest."
            } else if voice == V::OneShot as usize {
                "Attack: fade-in at the start of each hit, as a share of the played part of the file."
            } else {
                "Attack: fade-in time at the start of each hit. 0 = instant, the punchiest."
            }
        }
        F::Hold => {
            if voice == V::OneShot as usize {
                "Hold: how long the sound stays at full level before the decay, as a share of the played part of the file."
            } else {
                "Hold: how long the sound stays at full level before the decay starts."
            }
        }
        F::Decay => {
            if sampler || voice == V::OneShot as usize {
                "Decay: length of the fade-out, as a share of the sample."
            } else {
                "Decay: how long the sound takes to die away."
            }
        }
        F::DecayCurve => {
            "Decay Curve: bends the fade-out. Middle = a straight line; the Amp graph shows the shape."
        }
        F::ReleaseCurve => {
            "Attack Curve: bends the fade-in. Middle = a straight line; the Amp graph shows the shape."
        }
        F::Volume => "Volume of this lane (after the saturation). The grid's Vol slider is the same setting.",
        F::FilterFreq => {
            let label = INSTRUMENTS.get(voice).map_or("", |d| d.filter_type_label);
            if label == "HP" {
                "Filter cutoff (high-pass): higher = thinner, less low end."
            } else if label.is_empty() && is(voice, &[V::Buzz, V::Rift, V::OneShot]) {
                "Filter cutoff. The kind of filter (low-pass, high-pass, band-pass) is chosen in Filter Type."
            } else {
                "Filter cutoff (low-pass): lower = darker, rounder; higher = brighter."
            }
        }
        F::FilterEnvAmount => {
            "Filter Env: how far the filter opens at each hit before closing again over Filter Decay. 0 = the filter does not move."
        }
        F::FilterEnvDecay => "Filter Decay: how fast the filter closes again after each hit.",
        F::Analog => {
            if is(voice, &[V::HiHat, V::OpenHiHat, V::Clap, V::Cymbal, V::Ride]) {
                "Analog: each hit varies slightly in tone, level and timing, like a vintage machine. More to the right, more variation."
            } else {
                "Analog: from the middle up, each hit varies a little in pitch, level and length, like a vintage machine. Below the middle, every hit is identical."
            }
        }
        F::Stereo => {
            if voice == V::Buzz as usize {
                "Stereo: slightly detunes the right channel, for a wider sound. Off = mono."
            } else if voice == V::Perc1 as usize {
                "Stereo: detunes the right channel (by Width), for a wider sound. Off = mono."
            } else {
                "Stereo: a different noise on the left and the right, for a wider sound. Off = mono."
            }
        }
        F::Release => "",
    };
    text.to_string()
}

/// Help of a special row, by its registry name (`kick_click`, ...).
pub fn special(name: &str) -> &'static str {
    let ends = |s: &str| name.ends_with(s);
    // Shared by many instruments: by suffix.
    if ends("_saturation_type") || ends("_sat_type") {
        return "Saturation Type: the kind of distortion. None = off. SoftClip and Tape round the peaks, Valve and Transistor add colour, HardClip cuts hard.";
    }
    if ends("_saturation_amount") || ends("_sat_amount") {
        return "Saturation Amount: how hard the sound is driven into the distortion.";
    }
    if ends("_saturation_mix") || ends("_sat_mix") {
        return "Saturation Mix: blend of the clean and the distorted sound (0 = clean only).";
    }
    if ends("_saturation_output_gain") || ends("_sat_gain") {
        return "Saturation Output Gain: level after the distortion, to make up for the volume it adds or takes.";
    }
    if name == "rift_saturation_pre_filter" {
        return "Pre-Filter: on = Decimate, Crush and Saturation come before the filter (the filter then smooths them); off = after it.";
    }
    if ends("_saturation_pre_filter") {
        return "Pre-Filter: on = the distortion comes before the filter (the filter then smooths it); off = after it.";
    }
    if ends("_noise_type") {
        return "Noise Type: colour of the noise. White = bright and even, Pink = softer, Brown = dark, Blue = very bright.";
    }
    if ends("_fine_tune") || ends("_pitch_fine") {
        return "Pitch Fine: fine tuning in cents (100 cents = 1 semitone).";
    }
    if ends("_filter_type") {
        return "Filter Type: Low-pass keeps the lows, High-pass keeps the highs, Band-pass keeps a band around the cutoff.";
    }
    if ends("_filter_attack") || ends("_filter_atk") {
        return "Filter Attack: time the filter takes to open at each hit.";
    }
    if ends("_filter_hold") || ends("_filter_hld") {
        return "Filter Hold: time the filter stays open before closing.";
    }
    if ends("_filter_atk_curve") {
        return "Filter Atk Curve: bends the filter's opening. Middle = a straight line.";
    }
    if ends("_filter_curve") || ends("_filter_dec_curve") {
        return "Filter Dec Curve: bends the filter's closing. Middle = a straight line.";
    }
    if ends("_analog_mode") {
        return "Analog Mode: each hit plays a different recording of the original TR-606 (never the same twice in a row), like a real machine. This variation is normal: no setting is moving.\n\
Off: every hit plays the Sample chosen below.";
    }
    if ends("_sample") {
        return "Sample: which of the 8 recordings is played (or which stereo pair when Stereo is on). Greyed while Analog Mode is on.";
    }
    if ends("_one_shot") {
        return "One Shot: plays the whole recording and ignores the Amp envelope (its rows are greyed).";
    }
    if ends("_start_offset") {
        return "Start: where playback starts in the recording.";
    }
    if ends("_end") {
        return "End: where playback stops in the recording.";
    }
    if ends("_reverse") {
        return "Reverse: plays the sound backwards.";
    }
    if ends("_texture") {
        return "Texture: the sound the slices are cut from: 4 built in, or Custom = the WAV file loaded in the File row.";
    }
    match name {
        "kick_click" => "Click Level: level of the click at the start of the kick (the beater).",
        "kick_click_type" => "Click Type: Soft = a short noisy click, Hard = a sharp, loud tick, Medium in between.",
        "snare_snap" => "Snap: balance between the noise (wires) and the body (tone). Right = more body. No effect with the Noise algorithm.",
        "hihat_resonance" | "openhihat_resonance" => "Resonance: sharpness of the Tone boost. Higher = more ringing, more metallic.",
        "hihat_shimmer" | "openhihat_shimmer" => "Shimmer: adds a bright, airy noise layer on top.",
        "tom_stick" => "Stick Attack: a short click at the start of the hit, like a stick on the skin.",
        "clap_echo" => "Echo: spacing of the clap's repeated bursts. 0 = one single clap.",
        "cymbal_shimmer_freq" => "Shimmer Freq: speed of the cymbal's shimmering movement.",
        "cymbal_shimmer_amount" => "Shimmer Amount: depth of that shimmering movement.",
        "snare606_resonance" => "Resonance: how much the body rings.",
        "snare606_tone" => "Tone: balance between the body and the wires.",
        "snare606_snap" => "Snap: crispness of the wires.",
        "bassdrum808_accent" => "Accent: level of the click at the start of the kick.",
        "bassdrum808_snap" => "Snap: a quick pitch jump at the start of the hit, for more attack.",
        "bassdrum808_pitch_drop" => "Pitch Drop: how far the pitch falls during the hit.",
        "bassdrum808_click_tone" | "ac_bd_click_tone" => "Click Tone: brightness of the click.",
        "perc1_sweep" => "Sweep: pitch glide on each hit. Right = falling (laser), left = rising, middle = none.",
        "perc1_speed" => "Speed: duration of the pitch sweep, in ms.",
        "perc1_bite" => "Bite: FM growl added to the tone.",
        "perc1_width" => "Width: stereo width (a short echo and a detuned right channel).",
        "buzz_gate_rate" => "Gate Rate: speed of the on/off chopping of the sound, in Hz. Fast = a buzz, slow = a machine gun.",
        "buzz_gate_depth" => "Gate Depth: how much the gate cuts the sound between pulses. 0 = no gating.",
        "buzz_gate_shape" => "Gate Shape: shape of each pulse. Smooth algorithm: narrows the pulse; Razor: shortens the spike.",
        "buzz_noise_amount" => "Noise: blend between the tone (left) and noise (right).",
        "buzz_pitch_sweep" => "Pitch Sweep: each hit starts higher and drops to the pitch within 50 ms.",
        "buzz_wave" => "Wave: waveform of the oscillator.",
        "sdrex_filter_mod" => "Modulation: what the LFO moves. Flanger = a sweeping comb effect; Filter LFO = sweeps the filter up.",
        "sdrex_flanger_rate" => "Rate: speed of the modulation, in Hz.",
        "sdrex_modulation_fade" => "Fade-in: time for the modulation to come in after each hit.",
        "sdrex_flanger_depth" => "Depth: amount of modulation.",
        "sdrex_flanger_feedback" => "Feedback: resonance of the flanger (Flanger mode only).",
        "sdrex_flanger_wet" => "Wet: level of the modulated sound.",
        "sdrex_noise_level" => "Noise: level of the noise layer.",
        "sdrex_free_phase" => "Free Phase: on = the modulation keeps running between hits; off = it restarts on each hit.",
        "ac_bd_sweep" => "Sweep: range of the pitch drop.",
        "ac_bd_bend" => "Bend: speed of the pitch drop.",
        "ac_bd_click" | "ac_hh_click" | "ac_oh_click" => "Click: level of the click at the start of the hit.",
        "ac_bd_punch" => "Punch: thump of the body at the hit.",
        "ac_bd_tone" => "Tone: brightness of the body; above the middle it opens up.",
        "ac_bd_drive" => "Drive: built-in overdrive.",
        "ac_sd_snap" => "Snap: amount of snare wires.",
        "ac_sd_wire_color" => "Wire Color: colour of the wires' noise.",
        "ac_sd_shell_bend" => "Shell Bend: pitch bend of the shell at the hit.",
        "ac_sd_impact" => "Impact: pitch dip at the start of the hit.",
        "ac_sd_ring" => "Ring: low ringing of the shell.",
        "ac_hh_metal" | "ac_oh_metal" => "Metal: blend of the metallic tones.",
        "ac_hh_bell" | "ac_oh_bell" => "Bell: accent on the bell-like tones.",
        "ac_hh_wobble" | "ac_oh_wobble" => "Wobble: movement of the metallic tones.",
        "ac_hh_spread" | "ac_oh_spread" => "Spread: detune between the metallic tones.",
        "ac_hh_brightness" | "ac_oh_brightness" => "Brightness: tilt of the spectrum. Right = brighter, left = darker.",
        "ac_cl_noise" => "Noise: amount of noise in the clap.",
        "ac_cl_spread" => "Spread: timing of the clap's bursts.",
        "ac_cl_tail" => "Tail: level of the clap's tail.",
        "ac_cl_air" => "Air: level of the airy layer.",
        "ac_tm_model" => "Model: 0 = Auto, 1 = Low tom, 2 = High tom.",
        "ac_tm_strike" => "Strike: attack of the stick.",
        "ac_tm_snap" => "Snap: noise snap at the hit.",
        "ac_tm_glide" => "Glide: pitch glide of the tom.",
        "ac_tm_modes" => "Modes: level of the drum's overtones.",
        "ac_tm_tail_noise" => "Tail Noise: noise in the tail.",
        "rift_offset" => "Offset: where in the texture the slice starts.",
        "oneshot_offset" => "Offset: where playback starts in the file (counted from the end in Reverse).",
        "rift_wander" => "Wander: random move of the start point on each hit, around Offset.",
        "rift_advance" => "Advance Step: how far the start point moves forward on each hit while Advance is on.",
        "rift_pitch_env" | "oneshot_pitch_env" => "Pitch Env Depth: pitch jump at the start of the hit, in semitones, falling back to the pitch.",
        "rift_pitch_env_time" => "Pitch Env Time: how fast the pitch falls back.",
        "oneshot_pitch_env_atk" => "Pitch Env Attack: time for the pitch to reach the Depth.",
        "oneshot_pitch_env_atk_curve" => "Pitch Env Atk Curve: bends that rise. Middle = a straight line.",
        "oneshot_pitch_env_hld" => "Pitch Env Hold: time the pitch stays at the Depth.",
        "oneshot_pitch_env_decay" => "Pitch Env Decay: time for the pitch to come back.",
        "oneshot_pitch_env_dec_curve" => "Pitch Env Dec Curve: bends that return. Middle = a straight line.",
        "rift_loop" => "Loop: repeats the slice (the grain) for as long as the sound lasts.",
        "rift_grain" => "Grain: length of the repeated slice (Loop only).",
        "rift_grain_shape" => "Grain Shape: smoothness of the grain's edges (Loop only).",
        "rift_stereo_spread" => "Stereo Spread: the right channel reads a little ahead, for width.",
        "rift_lfo_shape" => "LFO Shape: waveform of the LFO (S&H = random steps).",
        "rift_lfo_free_phase" => "LFO Free Phase: on = the LFO keeps running between hits; off = it restarts on each hit.",
        "rift_pitch_lfo_rate" => "Pitch LFO Rate: speed of the vibrato.",
        "rift_pitch_lfo_depth" => "Pitch LFO Depth: depth of the vibrato, in semitones.",
        "rift_filter_lfo_rate" => "Filter LFO Rate: speed of the filter sweep.",
        "rift_filter_lfo_depth" => "Filter LFO Depth: depth of the filter sweep, in octaves.",
        "rift_resonance" | "oneshot_resonance" => "Resonance: emphasis at the filter cutoff; high values ring.",
        "rift_crush" => "Crush: fewer bits, for a gritty lo-fi sound.",
        "rift_decimate" => "Decimate: lower sample rate, for an aliased lo-fi sound.",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every special parameter of every instrument has a help text.
    #[test]
    fn every_special_param_has_help() {
        for def in INSTRUMENTS.iter() {
            for sp in def.special_params {
                assert!(!special(sp.name).is_empty(), "no help for {}", sp.name);
            }
        }
    }

    /// Every standard row shown has a help text (Release is never shown).
    #[test]
    fn every_standard_param_has_help() {
        for def in INSTRUMENTS.iter() {
            for sp in def.standard_params {
                if sp.field != F::Release {
                    assert!(
                        !standard(def.index, sp.field).is_empty(),
                        "{} {:?}",
                        def.name,
                        sp.field
                    );
                }
            }
        }
    }
}
