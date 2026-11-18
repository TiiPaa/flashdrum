//! Production-path regressions for the controls reported in [325].
use super::*;
use crate::instrument_registry::{StandardField, INSTRUMENTS};
use crate::param_id::ParamId;
use crate::track::{TrackInstrumentKind as Kind, TrackLayoutState, TrackSlot, MAX_TRACKS};
use crate::ui::param_source::{AlgoSink, GlobalSource, ParamSource, PlockSource};

struct Algo;
impl AlgoSink for Algo {
    fn get(&self) -> u8 { 0 }
    fn set(&self, _: u8) {}
}

fn special(kind: Kind, name: &str) -> ParamId {
    let def = &INSTRUMENTS[kind.drum_voice_index()];
    ParamId::Special(def.special_params.iter().find(|p| p.name == name).unwrap().special_index)
}

fn render(kind: Kind, stereo: bool, values: &[(&str, f32)], locked: bool, live: bool) -> Vec<[f32; 2]> {
    let mut vst = DrumFlashVst::default();
    let voice = kind.drum_voice_index();
    let mut layout = TrackLayoutState::empty_layout();
    layout.slots[0] = TrackSlot::active_with_kind(kind);
    PersistentField::<TrackLayoutState>::set(&vst.params.track_layout, layout.clone());
    vst.synthesizer.initialize_with_layout(48000.0, &layout);
    vst.sound_settings_state.reset_slot_to_defaults(0, kind, 0.0);
    let mut voices = [None; MAX_TRACKS];
    voices[0] = Some(voice);
    let mut global = GlobalSource::new(&vst.sound_settings_state, 0, voice, &Algo);
    global.set(ParamId::Std(StandardField::Stereo), if stereo { 1.0 } else { 0.0 });
    global.commit();
    let change = |vst: &mut DrumFlashVst| {
        let global = GlobalSource::new(&vst.sound_settings_state, 0, voice, &Algo);
        let mut src: Box<dyn ParamSource> = if locked {
            Box::new(PlockSource::new(&vst.params.plock_state.state, 7, global))
        } else {
            Box::new(global)
        };
        for &(name, value) in values { src.set(special(kind, name), value); }
        src.commit();
        drop(src);
        vst.push_sound_settings(&voices);
        let settings = vst.voice_settings_at_step(0, voice, 7);
        for &(name, value) in values {
            assert_eq!(settings.get(special(kind, name)), value, "{name} must reach the voice");
        }
        // This is the same merge passed to the production trigger path.
        if !live { vst.synthesizer.set_voice_settings(0, settings); }
    };
    if !live { change(&mut vst); }
    vst.push_sound_settings(&voices);
    vst.synthesizer.set_hit_index(0, 42);
    vst.synthesizer.trigger(0, 1.0);
    let mut result = Vec::with_capacity(38400);
    let mut out = [[0.0; 2]; MAX_TRACKS];
    for n in 0..38400 {
        if live && n == 4800 { change(&mut vst); }
        if n % 64 == 0 { vst.push_sound_settings(&voices); }
        vst.synthesizer.process_voice_samples_stereo(&mut out);
        let main = main_mix(&out, &[1.0; MAX_TRACKS], 1.0);
        assert!(main.0.is_finite() && main.1.is_finite());
        result.push([main.0, main.1]);
    }
    result
}

fn relative_difference(a: &[[f32; 2]], b: &[[f32; 2]]) -> f64 {
    let (mut error, mut power) = (0.0, 0.0);
    for (a, b) in a.iter().zip(b) {
        for ch in 0..2 {
            error += (a[ch] as f64 - b[ch] as f64).powi(2);
            power += (a[ch] as f64).powi(2);
        }
    }
    (error / power.max(1e-30)).sqrt()
}

// Broad-band spectral contrast, not a claim about subjective audibility.
fn brightness(samples: &[[f32; 2]]) -> f64 {
    let (mut lows, mut highs) = ([0.0; 2], [0.0; 2]);
    let (mut low_power, mut high_power) = (0.0, 0.0);
    let low_coef = 1.0 - (-std::f64::consts::TAU * 1200.0 / 48000.0).exp();
    let high_coef = 1.0 - (-std::f64::consts::TAU * 4500.0 / 48000.0).exp();
    for pair in samples {
        for ch in 0..2 {
            let sample = pair[ch] as f64;
            lows[ch] += low_coef * (sample - lows[ch]);
            highs[ch] += high_coef * (sample - highs[ch]);
            low_power += lows[ch].powi(2);
            high_power += (sample - highs[ch]).powi(2);
        }
    }
    10.0 * (high_power / low_power.max(1e-30)).log10()
}

#[test]
fn prism_color_changes_the_whole_spectrum_including_at_full_density() {
    for density in [72.0, 100.0] {
        for (stereo, locked, live) in [(false, false, false), (true, true, false), (true, false, true)] {
            // For live edits, Density stays at its default in both runs until
            // the same edit frame, as it does in the real editor.
            let low = render(Kind::PrismHat, stereo, &[("prism_density", density), ("prism_color", 0.0)], locked, live);
            let high = render(Kind::PrismHat, stereo, &[("prism_density", density), ("prism_color", 100.0)], locked, live);
            let start = if live { 6000 } else { 0 };
            let tilt = brightness(&high[start..]) - brightness(&low[start..]);
            eprintln!("Prism spectral contrast: density={density}, stereo={stereo}, locked={locked}, live={live}: {tilt:.2} dB");
            assert!(tilt > 6.0, "Color must tilt the spectrum, not only change a few noise samples ({tilt:.2} dB)");
            if live { assert_eq!(&low[..4800], &high[..4800], "no edit before the UI writes"); }
        }
    }
}

#[test]
fn reported_forge_controls_work_through_step_locks_and_live_polls() {
    for (kind, name, low, high) in [
        (Kind::TidalAlloy, "tidal_drop_speed", 0.0, 100.0),
        (Kind::TidalAlloy, "tidal_tide", 0.0, 100.0),
        (Kind::TidalAlloy, "tidal_lfo_rate", 0.05, 5.0),
        (Kind::TidalAlloy, "tidal_lfo_depth", 0.0, 100.0),
        (Kind::MobiusSkin, "mobius_width", 0.0, 100.0),
    ] {
        for (locked, live) in [(true, false), (false, true)] {
            let a = render(kind, true, &[(name, low)], locked, live);
            let b = render(kind, true, &[(name, high)], locked, live);
            let start = if live { 6000 } else { 0 };
            let difference = relative_difference(&a[start..], &b[start..]);
            assert!(difference > 0.05, "{name}: locked={locked} live={live} difference={difference}");
            if live { assert_eq!(&a[..4800], &b[..4800]); }
        }
    }
    // Keep the bell/no-drop cases explicit: Speed must never become Volume.
    for (name, value) in [("tidal_bell", 0.0), ("tidal_tone_drop", 0.0)] {
        let a = render(Kind::TidalAlloy, false, &[(name, value), ("tidal_drop_speed", 0.0)], false, false);
        let b = render(Kind::TidalAlloy, false, &[(name, value), ("tidal_drop_speed", 100.0)], false, false);
        assert_eq!(a, b, "no tonal drop means no effect of its speed");
    }
}

#[test]
fn orbit_material_extremes_do_not_hit_the_safety_ceiling() {
    for stereo in [false, true] {
        for (material, gravity) in [(0.0, 47.0), (38.0, 47.0), (100.0, 47.0), (0.0, 0.0), (100.0, 0.0)] {
            let audio = render(Kind::OrbitClay, stereo, &[
                ("orbit_material", material), ("orbit_gravity", gravity), ("orbit_bounce", 100.0),
            ], false, false);
            let peak = audio.iter().fold(0.0f32, |peak, x| peak.max(x[0].abs()).max(x[1].abs()));
            eprintln!("Orbit peak: material={material} gravity={gravity} stereo={stereo}: {peak:.6}");
            assert!(peak < 0.99, "Material must not saturate after reverting [328]: peak {peak}");
        }
    }
}

#[test]
fn orbit_light_rebounds_have_no_sample_discontinuity() {
    for stereo in [false, true] {
        let audio = render(Kind::OrbitClay, stereo, &[("orbit_gravity", 0.0)], false, false);
        for ms in [0.0, 220.0, 638.0] {
            let centre = (ms * 48.0) as usize;
            let mut peak = 0.0f64;
            for n in centre.saturating_sub(24).max(1)..(centre + 25).min(audio.len()) {
                for channel in 0..2 {
                    let delta = (audio[n][channel] as f64 - audio[n - 1][channel] as f64).abs();
                    peak = peak.max(delta);
                }
            }
            assert!(peak < 0.006, "click at {ms} ms, stereo={stereo}: sample delta {peak}");
        }
    }
}

#[test]
fn orbit_gravity_and_timbre_deformation_are_clear_in_production() {
    for (stereo, locked, live) in [(false, false, false), (true, true, false), (true, false, true)] {
        for (name, low, high) in [("orbit_gravity", 0.0, 100.0), ("orbit_drive", 0.0, 100.0)] {
            let a = render(Kind::OrbitClay, stereo, &[(name, low)], locked, live);
            let b = render(Kind::OrbitClay, stereo, &[(name, high)], locked, live);
            let start = if live { 6000 } else { 0 };
            let difference = relative_difference(&a[start..], &b[start..]);
            assert!(difference > 0.15, "{name}: stereo={stereo} locked={locked} live={live} difference={difference}");
            if name == "orbit_drive" {
                let tilt = brightness(&b[start..]) - brightness(&a[start..]);
                assert!(tilt > 1.0, "Deformation must lift the upper resonances: {tilt:.2} dB");
            }
            if live { assert_eq!(&a[..4800], &b[..4800]); }
        }
    }
}

#[test]
fn reported_forge_controls_production_contrast() {
    let cases = [
        (Kind::TidalAlloy, "tidal_drop_speed", 0.0, 100.0),
        (Kind::TidalAlloy, "tidal_tide", 0.0, 100.0),
        (Kind::TidalAlloy, "tidal_lfo_rate", 0.05, 5.0),
        (Kind::TidalAlloy, "tidal_lfo_depth", 0.0, 100.0),
        (Kind::PrismHat, "prism_color", 0.0, 100.0),
        (Kind::MobiusSkin, "mobius_width", 0.0, 100.0),
    ];
    for (kind, name, low, high) in cases {
        for stereo in [false, true] {
            let a = render(kind, stereo, &[(name, low)], false, false);
            let b = render(kind, stereo, &[(name, high)], false, false);
            let difference = relative_difference(&a, &b);
            eprintln!("{name}, stereo={stereo}, delta RMS / signal RMS={difference:.5} ({:.2} dB)",
                20.0 * difference.max(1e-30).log10());
            let stereo_only = name.starts_with("tidal_lfo_") || name == "mobius_width";
            if !stereo && stereo_only { assert_eq!(a, b); }
            else { assert!(difference > 0.001, "{name}: no effective production change"); }
        }
    }
}
