use super::*;
use crate::instrument_registry::{StandardField, INSTRUMENTS};
use crate::track::{TrackInstrumentKind, TrackLayoutState};
use crate::synthesis::settings::forge::OrbitClaySettings;

fn voice(kind: ForgeKind, sr: f32) -> ForgeVoice {
    let mut v = ForgeVoice::new(sr, kind);
    v.memory = Some(Box::new(ForgeMemory::new(sr)));
    v
}
fn fixtures() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../tests/fixtures/percussion-forge.json"
    ))
    .unwrap()
}
fn settings(f: &serde_json::Value, kind: ForgeKind) -> VoiceSettings {
    let mut s = VoiceSettings::forge(kind.voice());
    s.frequency = f["frequency"].as_f64().unwrap() as f32;
    s.decay = f["decay"].as_f64().unwrap() as f32;
    for pair in f["special"].as_array().unwrap() {
        s.special[pair[0].as_u64().unwrap() as usize] = pair[1].as_f64().unwrap() as f32;
    }
    s
}

/// Compare engine equations, NOT the deliberate streaming gain calibration.
/// Source peak scaling is applied after the test render, exactly as in the app.
#[test]
fn forge_all_mono_source_oracles() {
    let root = fixtures();
    let mut checked = 0;
    for f in root["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["left"].is_null())
    {
        let kind = ForgeKind::ALL[f["kind"].as_u64().unwrap() as usize];
        let s = settings(f, kind);
        let p = Parameters::new(kind, s);
        let mut c = Channel::new(kind, 42);
        let mut mem = ChannelMemory::new(48000.0);
        let velocity = f["velocity"].as_f64().unwrap_or(1.0);
        let n = f["samples"].as_u64().unwrap() as usize;
        let (dur, release) = duration(kind, &s);
        assert!(
            ((dur * 48000.0).floor() as i64 - n as i64).abs() <= 1,
            "{kind:?} duration {}",
            f["name"]
        );
        let gain = 0.88 / f["rawPeak"].as_f64().unwrap().max(1.0);
        let prefix = f["prefix"].as_array().unwrap();
        let mut error = 0.0;
        let mut power = 0.0;
        let mut square = 0.0;
        let mut peak = 0.0f64;
        let mut sum = 0.0;
        for i in 0..n {
            let x =
                (c.sample(p, i as f64 / 48000.0, 48000.0, &mut mem, 0.0) * velocity) as f32 as f64;
            let x = (x * fade(kind, i, n, release, 48000.0)) as f32 as f64 * gain;
            square += x * x;
            peak = peak.max(x.abs());
            sum += x;
            if i < prefix.len() {
                let expected = prefix[i].as_f64().unwrap();
                error += (x - expected).powi(2);
                power += expected * expected;
            }
        }
        let nrmse = (error / power.max(1e-20)).sqrt();
        // Float32 parameter storage can move a hard square-wave edge by one
        // sample; 2% NRMSE permits that but not a changed synthesis equation.
        assert!(nrmse < 0.02, "{kind:?} {} prefix NRMSE {nrmse}", f["name"]);
        let expected = f["rms"].as_f64().unwrap();
        let rms = (square / n as f64).sqrt();
        assert!(
            (rms - expected).abs() < expected * 0.015 + 1e-6,
            "{kind:?} {} RMS {rms} vs {expected}",
            f["name"]
        );
        let expected_peak = f["peak"].as_f64().unwrap();
        assert!(
            (peak - expected_peak).abs() < expected_peak * 0.025 + 1e-5,
            "{kind:?} {} peak {peak} vs {expected_peak}",
            f["name"]
        );
        assert!(
            (sum / n as f64 - f["mean"].as_f64().unwrap()).abs() < 0.002,
            "{kind:?} {} DC",
            f["name"]
        );
        checked += 1;
    }
    assert_eq!(checked, 190);
}

#[test]
fn forge_seven_stereo_source_oracles() {
    let root = fixtures();
    let mut checked = 0;
    for f in root["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| !f["left"].is_null())
    {
        let kind = ForgeKind::ALL[f["kind"].as_u64().unwrap() as usize];
        let s = settings(f, kind);
        let p = Parameters::new(kind, s);
        let (_, release) = duration(kind, &s);
        let n = f["samples"].as_u64().unwrap() as usize;
        let gains = [
            0.88 / f["rawLeftPeak"].as_f64().unwrap().max(1.0),
            0.88 / f["rawRightPeak"].as_f64().unwrap().max(1.0),
        ];
        let wide_gain = f["wideGain"].as_f64().unwrap();
        let mut channels = [Channel::new(kind, 42), Channel::new(kind, 7961)];
        let mut mem = ForgeMemory::new(48000.0);
        let mut low = 0.0;
        let mut error = [0.0; 2];
        let mut power = [0.0; 2];
        let mut square = [0.0; 2];
        let prefixes = [
            f["prefix"].as_array().unwrap(),
            f["rightPrefix"].as_array().unwrap(),
        ];
        for i in 0..n {
            let mut raw = [0.0; 2];
            for side in 0..2 {
                let x = channels[side].sample(
                    p,
                    i as f64 / 48000.0,
                    48000.0,
                    &mut mem.channels[side],
                    if side == 0 { -1.0 } else { 1.0 },
                ) as f32 as f64;
                let x = (x * fade(kind, i, n, release, 48000.0)) as f32 as f64;
                raw[side] = (x * gains[side]) as f32 as f64;
            }
            let out = mem.widen(kind, raw[0], raw[1], 48000.0, &mut low);
            for side in 0..2 {
                let x = (out[side] as f32 as f64 * wide_gain) as f32 as f64;
                let x = (x * fade(ForgeKind::IonBloom, i, n, 0.022, 48000.0)) as f32 as f64;
                square[side] += x * x;
                if i < prefixes[side].len() {
                    let expected = prefixes[side][i].as_f64().unwrap();
                    error[side] += (x - expected).powi(2);
                    power[side] += expected * expected;
                }
            }
        }
        for side in 0..2 {
            let nrmse = (error[side] / power[side].max(1e-20)).sqrt();
            assert!(nrmse < 0.02, "{kind:?} side {side} prefix {nrmse}");
            let expected = f[if side == 0 { "left" } else { "right" }]["rms"]
                .as_f64()
                .unwrap();
            let rms = (square[side] / n as f64).sqrt();
            assert!(
                (rms - expected).abs() < expected * 0.015 + 1e-6,
                "{kind:?} side {side} RMS"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 7);
}

#[test]
fn forge_registry_has_all_68_source_controls_and_safe_indices() {
    let mut controls = 0;
    for (i, kind) in ForgeKind::ALL.into_iter().enumerate() {
        let d = &INSTRUMENTS[kind.voice() as usize];
        let track = TrackInstrumentKind::from_drum_voice_index(kind.voice() as usize).unwrap();
        assert_eq!(track.index(), 29 + i);
        assert_eq!(d.index, 31 + i);
        assert_eq!(d.midi_note, 61 + i as u8);
        assert_eq!(track.default_midi_note(), d.midi_note);
        assert_eq!(track.default_name(), d.full_name);
        assert_eq!(track.default_label(), d.label);
        assert!(!track.is_effect());
        assert_eq!(d.algo_count, 1);
        controls += 2 + d.special_params.len();
        for p in d.special_params {
            assert_ne!(p.special_index, 4, "Attack aliases special 4");
            assert!(p.continuous);
            assert!((p.min..=p.max).contains(&p.default));
        }
        assert_eq!(
            d.standard_params
                .iter()
                .filter(|p| p.field == StandardField::Stereo)
                .count(),
            usize::from(kind.stereo())
        );
    }
    assert_eq!(controls, 68);
    assert_eq!(TrackInstrumentKind::MidiOut.index(), 28);
    assert_eq!(DrumVoice::MidiOut as usize, 30);
    assert_eq!(crate::track::MAX_TRACKS, 14);
}

#[test]
fn forge_each_source_control_changes_its_engine() {
    let root = fixtures();
    for kind in ForgeKind::ALL {
        let rows: Vec<_> = root["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["kind"].as_u64() == Some(kind as u64) && f["left"].is_null())
            .collect();
        let d = &INSTRUMENTS[kind.voice() as usize];
        for param in d.special_params {
            // These two parameters intentionally only affect stereo, as in source.
            let stereo_only = param.family == crate::instrument_registry::ParamFamily::Stereo;
            assert_eq!(stereo_only, param.name.starts_with("tidal_lfo_") || param.name == "mobius_width",
                "{}: stereo-only controls must be in the Stereo module", param.name);
            let mut a = voice(kind, 48000.0);
            let mut b = voice(kind, 48000.0);
            let mut sa = settings(rows[0], kind);
            sa.special[param.special_index] = param.min;
            sa.stereo = if stereo_only { 1.0 } else { 0.0 };
            let mut sb = sa;
            sb.special[param.special_index] = param.max;
            a.set_settings(sa);
            b.set_settings(sb);
            a.trigger();
            b.trigger();
            let mut difference = 0.0;
            let samples = if param.name == "orbit_release" {
                100000
            } else {
                24000
            };
            for _ in 0..samples {
                let (al, ar) = a.process_sample_stereo();
                let (bl, br) = b.process_sample_stereo();
                difference += (al - bl).abs() + (ar - br).abs();
            }
            assert!(difference > 0.001, "{kind:?} {} has no effect", param.name);
            if stereo_only {
                // Audit ALL nine engines against their real mono output too:
                // the only stereo-only specials must be exactly unchanged.
                sa.stereo = 0.0;
                sb.stereo = 0.0;
                a.reset();
                b.reset();
                a.set_settings(sa);
                b.set_settings(sb);
                a.set_hit_index(42);
                b.set_hit_index(42);
                a.trigger();
                b.trigger();
                for _ in 0..samples {
                    assert_eq!(a.process_sample_stereo(), b.process_sample_stereo(), "{} must be mono-inactive", param.name);
                }
            }
        }
    }
}

#[test]
fn forge_sample_rates_stereo_live_edits_and_silence() {
    for sr in [44100.0, 48000.0, 96000.0, 192000.0] {
        for kind in ForgeKind::ALL {
            let mut v = voice(kind, sr);
            let mut s = VoiceSettings::forge(kind.voice());
            s.stereo = 1.0;
            v.set_settings(s);
            v.trigger();
            let mut energy = 0.0;
            let mut side = 0.0;
            for i in 0..(sr as usize / 8) {
                if i == 1000 {
                    s.frequency *= 1.05;
                    v.set_settings(s);
                }
                let (l, r) = v.process_sample_stereo();
                assert!(l.is_finite() && r.is_finite());
                energy += l.abs() + r.abs();
                side += (l - r).abs();
            }
            assert!(energy > 0.1, "{kind:?} silent at {sr}");
            if kind.stereo() {
                assert!(side > 0.01, "{kind:?} not stereo");
            } else {
                assert_eq!(side, 0.0);
            }
            // Jump near the native final fade; do not spend seconds rendering
            // silence in every sample-rate test.
            v.position = (duration(kind, &s).0 * sr as f64) as usize - 1;
            for _ in 0..4 {
                v.process_sample_stereo();
            }
            assert!(!v.is_active());
            assert_eq!(v.process_sample_stereo(), (0.0, 0.0));
        }
    }
}

#[test]
fn forge_kind_changes_triggers_and_controls_never_allocate_or_free() {
    use crate::synthesis::DrumSynthesizer;
    let mut synth = DrumSynthesizer::new();
    synth.initialize_with_layout(48000.0, &TrackLayoutState::from_legacy_13());
    assert_no_alloc::assert_no_alloc(|| {
        for _ in 0..3 {
            for kind in ForgeKind::ALL {
                let track =
                    TrackInstrumentKind::from_drum_voice_index(kind.voice() as usize).unwrap();
                synth.reinitialize_slot(0, track);
                synth.set_voice_settings(0, VoiceSettings::forge(kind.voice()));
                synth.trigger(0, 1.0);
                let mut outputs = [[0.0; 2]; crate::track::MAX_TRACKS];
                for _ in 0..256 {
                    synth.process_voice_samples_stereo(&mut outputs);
                }
                synth.trigger(0, 0.7);
                for _ in 0..64 {
                    synth.process_voice_samples_stereo(&mut outputs);
                }
                synth.reinitialize_slot(0, TrackInstrumentKind::Kick);
            }
        }
    });
}

#[test]
fn forge_volume_is_post_coloration_and_slots_are_independent() {
    for kind in ForgeKind::ALL {
        let mut a = voice(kind, 48000.0);
        let mut b = voice(kind, 48000.0);
        let mut s = VoiceSettings::forge(kind.voice());
        s.volume = 0.25;
        b.set_settings(s);
        a.trigger();
        b.trigger();
        for _ in 0..1024 {
            let x = a.process_sample();
            let y = b.process_sample();
            assert!((x * 0.25 - y).abs() < 1e-6);
        }
        a.reset();
        assert_eq!(a.process_sample(), 0.0);
        assert!(b.is_active());
    }
}

#[test]
fn forge_volume_keeps_the_shared_internal_zero_to_two_range() {
    for kind in ForgeKind::ALL {
        let mut unity = voice(kind, 48000.0);
        let mut doubled = voice(kind, 48000.0);
        let mut s = VoiceSettings::forge(kind.voice());
        s.volume = 2.0;
        doubled.set_settings(s);
        unity.trigger();
        doubled.trigger();
        for _ in 0..2048 {
            assert!(
                (unity.process_sample() * 2.0 - doubled.process_sample()).abs() < 1e-6,
                "{kind:?}: the upper half of the shared Volume control must work"
            );
        }
    }
}

#[test]
fn ferroskin_impact_keeps_the_default_and_leaves_the_tail_alone() {
    for n in 0..4800 {
        let t = n as f64 / 48000.0;
        assert_eq!(ferroskin_impact_gain(52.0, t), 1.0, "preserve the source default exactly");
        let low = ferroskin_impact_gain(0.0, t);
        let high = ferroskin_impact_gain(100.0, t);
        assert!(low >= 0.0 && low <= 1.0 && high >= 1.0 && high <= 2.0);
        if n >= 576 {
            assert_eq!(low, 1.0);
            assert_eq!(high, 1.0);
        }
    }
}

#[test]
fn forge_stereo_controls_share_a_module_for_all_nine_instruments() {
    use crate::instrument_registry::ParamFamily;
    let mut switches = 0;
    let mut stereo_only = Vec::new();
    for kind in ForgeKind::ALL {
        let def = &INSTRUMENTS[kind.voice() as usize];
        let switch = def.standard_params.iter().find(|p| p.field == StandardField::Stereo);
        assert_eq!(switch.is_some(), kind.stereo(), "{kind:?}");
        if let Some(switch) = switch {
            assert_eq!(switch.family, ParamFamily::Stereo);
            switches += 1;
        }
        for param in def.special_params.iter().filter(|p| p.family == ParamFamily::Stereo) {
            assert!(switch.is_some(), "{} has no Stereo switch", param.name);
            stereo_only.push(param.name);
        }
    }
    assert_eq!(switches, 7);
    assert_eq!(stereo_only, ["tidal_lfo_rate", "tidal_lfo_depth", "mobius_width"]);
}

#[test]
fn forge_color_and_drop_adaptations_preserve_defaults_and_extend_their_action() {
    assert_eq!(tidal_drop_time(42.0), 0.5 - 0.42 * 0.44);
    assert!((tidal_drop_time(0.0) - 2.0).abs() < 1e-12);
    assert!((tidal_drop_time(100.0) - 0.012).abs() < 1e-12);
    for speed in 1..=100 {
        assert!(tidal_drop_time(speed as f64) < tidal_drop_time((speed - 1) as f64));
    }
    assert!(tidal_drop_time(0.0) / tidal_drop_time(100.0) > 160.0);
    for sr in [44100.0, 48000.0, 96000.0, 192000.0] {
        let coef = 1.0 - (-std::f64::consts::TAU * 2500.0 / sr).exp();
        let mut low = 0.0;
        for n in 0..2000 {
            let sample = (n as f64 * 0.37).sin();
            assert_eq!(prism_color(sample, 66.0, &mut low, coef), sample);
        }
        for kind in [ForgeKind::TidalAlloy, ForgeKind::PrismHat, ForgeKind::OrbitClay] {
            let settings = VoiceSettings::forge(kind.voice());
            let mut native = Channel::new(kind, 42);
            let mut production = Channel::new(kind, 42);
            let mut a = ChannelMemory::new(sr);
            let mut b = ChannelMemory::new(sr);
            for n in 0..2000 {
                let t = n as f64 / sr;
                assert_eq!(native.sample(Parameters::new(kind, settings), t, sr, &mut a, 0.0),
                    production.sample(production_parameters(kind, settings), t, sr, &mut b, 0.0),
                    "{kind:?}: native defaults must remain unchanged");
            }
        }
    }
}

#[test]
fn orbit_gravity_and_deformation_adaptations_have_clear_ranges() {
    fn adapted(gravity: f32, drive: f32) -> OrbitClaySettings {
        let mut settings = VoiceSettings::forge(DrumVoice::OrbitClay);
        settings.special[2] = gravity;
        settings.special[7] = drive;
        match production_parameters(ForgeKind::OrbitClay, settings) {
            Parameters::Orbit(p) => p,
            _ => unreachable!(),
        }
    }
    let earth = adapted(47.0, 10.0);
    assert_eq!(earth.bounce_interval, 0.045 + (1.0 - 0.47) * 0.085);
    assert_eq!(earth.bounce_growth, 1.18 + 0.47 * 0.42);
    assert_eq!(earth.mode_deformation, 0.0);
    assert_eq!(earth.bounce_smoothing, 0.0);
    assert_eq!(earth.soft_drive, 0.1f64.powf(1.5) * 0.3);
    let light = adapted(0.0, 0.0);
    let heavy = adapted(100.0, 100.0);
    assert!((light.bounce_interval - 0.22).abs() < 1e-12);
    assert!((light.bounce_growth - 1.9).abs() < 1e-12);
    assert!((heavy.bounce_interval - 0.018).abs() < 1e-12);
    assert!((heavy.bounce_growth - 1.12).abs() < 1e-12);
    assert!((light.bounce_smoothing - 0.018).abs() < 1e-12);
    assert_eq!(heavy.bounce_smoothing, 0.0);
    let third_light = light.bounce_interval * (1.0 + light.bounce_growth);
    let third_heavy = heavy.bounce_interval * (1.0 + heavy.bounce_growth);
    assert!(third_light > 0.6 && third_heavy < 0.04);
    assert!(third_light / third_heavy > 16.0);
    assert_eq!(light.mode_deformation, -1.0);
    assert_eq!(heavy.mode_deformation, 1.0);
    // No extra drive at the extremes: this control now bends resonances.
    assert_eq!(light.soft_drive, earth.soft_drive);
    assert_eq!(heavy.soft_drive, earth.soft_drive);
    for gravity in 1..=100 {
        let previous = adapted((gravity - 1) as f32, 10.0);
        let current = adapted(gravity as f32, 10.0);
        assert!(current.bounce_interval < previous.bounce_interval);
        assert!(current.bounce_growth < previous.bounce_growth);
    }
}

#[test]
fn forge_color_and_drop_live_controls_remain_smoothed_and_allocation_free() {
    for (kind, index) in [(ForgeKind::PrismHat, 2), (ForgeKind::TidalAlloy, 3)] {
        let mut v = voice(kind, 48000.0);
        v.trigger();
        for _ in 0..1000 { v.process_sample_stereo(); }
        let old = v.current.special[index];
        assert_no_alloc::assert_no_alloc(|| {
            v.set_special_param(index, 100.0);
            let (l, r) = v.process_sample_stereo();
            assert!(l.is_finite() && r.is_finite());
            assert!(v.current.special[index] > old && v.current.special[index] < old + 1.0);
            for _ in 0..256 { v.process_sample_stereo(); }
            v.set_special_param(index, 0.0);
            for _ in 0..256 { v.process_sample_stereo(); }
            v.trigger();
            v.process_sample_stereo();
        });
    }
}

#[test]
fn forge_invalid_host_values_and_retrigger_are_safe() {
    for kind in ForgeKind::ALL {
        let mut v = voice(kind, 48000.0);
        let mut s = VoiceSettings::forge(kind.voice());
        s.frequency = f32::NAN;
        s.decay = f32::INFINITY;
        s.volume = f32::NEG_INFINITY;
        s.special.fill(f32::NAN);
        v.set_settings(s);
        v.trigger();
        for _ in 0..1024 {
            assert!(v.process_sample().is_finite());
        }
        let before = v.last;
        v.trigger();
        let after = v.process_sample_stereo();
        // First hits retain their native transients. Retriggers fade the new
        // hit from zero while continuing the previous output exactly.
        assert!(after.0.is_finite() && after.1.is_finite());
        assert!(
            (after.0 - before[0]).abs() < 1e-6,
            "{kind:?} retrigger step"
        );
    }
}
