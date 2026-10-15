//! Tests for the ported analogcode engines and their integration as the six
//! AC606 instruments (`AcVoice`).
//!
//! The engines restart from a fresh fitted state on every trigger, cutting
//! any ringing tail — `AcVoice` wraps them in `dsp::RetrigDeclick` (same
//! contract as Kick/BD808 [179]), which the retrigger test below verifies.

use super::super::{AcEngineKind, AcVoice, AcVoiceSettings, Voice, VoiceSettings};
use super::{
    AcBassDrum, AcClap, AcMetalHat, AcSnare, AcTom, BdMods, HatMods, SnareMods, TomMods,
    CLOSED_HAT_SPEC, HIGH_TOM_SPEC, LOW_TOM_SPEC, OPEN_HAT_SPEC,
};

const SR: f32 = 44100.0;

/// FNV-1a over the f32 bit patterns of a rendered buffer — golden-render
/// fingerprint for the [196] exploration refactor (defaults must stay
/// bit-identical to the original fitted sound).
fn fnv_hash(buf: &[f32]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for s in buf {
        for b in s.to_bits().to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
    }
    h
}

/// [196] Exploration refactor: the new specials default to the fitted values,
/// so a default-configured voice must render EXACTLY the original sound.
/// Hashes captured on the pre-refactor code (2026-08-26).
///
/// Exception, measured and accepted: BD6 was re-captured post-refactor
/// (0xb90a…). The decoupling of Click/Punch from the Attack derivation
/// (`0.10 + transient*0.5` = 1 ulp above a direct `0.2`) shifted those two
/// levels by exactly one float ulp — inaudible, and the new direct mapping
/// is the intended semantics. The five other kinds match bit-for-bit.
///
/// Second exception, deliberate ([292]): TM6 was re-captured when its two models
/// were pushed apart (Low = longer, more glide; High = shorter, clickier) and
/// the default Model became Low (124.435 Hz) instead of Auto at 150 Hz, and Modes
/// / Tail Noise / Decay got their wider ranges. The engine at the fitted values
/// is untouched; the other five kinds still match bit-for-bit.
#[test]
fn golden_default_render_unchanged() {
    let n = (SR * 0.15) as usize;
    for (kind, settings, name, expected) in [
        (
            AcEngineKind::BassDrum,
            VoiceSettings::bd6ac(),
            "BD6",
            0xb90a3e82f2063091u64,
        ),
        (
            AcEngineKind::Snare,
            VoiceSettings::sd6ac(),
            "SD6",
            0xd7de79b1ab896c5fu64,
        ),
        (
            AcEngineKind::ClosedHat,
            VoiceSettings::hh6ac(),
            "HH6",
            0xeb689f09852b5226u64,
        ),
        (
            AcEngineKind::OpenHat,
            VoiceSettings::oh6ac(),
            "OH6",
            0x68a5a17b54a65772u64,
        ),
        (
            AcEngineKind::Clap,
            VoiceSettings::cl6ac(),
            "CL6",
            0xdc7b957179dc2784u64,
        ),
        (
            AcEngineKind::Tom,
            VoiceSettings::tm6ac(),
            "TM6",
            0x70e8a8eaca17e731u64,
        ),
    ] {
        let mut voice = make(kind, settings);
        voice.trigger();
        let buf = render_voice(&mut voice, n);
        let h = fnv_hash(&buf);
        assert_eq!(
            h, expected,
            "{name}: default render changed (got 0x{h:016x}, want 0x{expected:016x})"
        );
    }
}

// ── Engine smoke tests ──────────────────────────────────────────────────────

fn render_engine<F: FnMut() -> f32>(mut f: F, seconds: f32) -> Vec<f32> {
    (0..(SR * seconds) as usize).map(|_| f()).collect()
}

fn assert_sane_render(buf: &[f32], name: &str) {
    let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    assert!(peak > 0.02, "{name}: engine rendered silence (peak {peak})");
    assert!(
        buf.iter().all(|s| s.is_finite()),
        "{name}: non-finite sample in render"
    );
}

#[test]
fn ac_engines_render_at_44100_and_48000() {
    for sr in [44100.0f32, 48000.0] {
        let mut kick = AcBassDrum::new(sr, 0x606606);
        kick.trigger(0.80, 2.22, 0.0, &BdMods::default());
        assert_sane_render(&render_engine(|| kick.process(), 1.0), "kick");

        let mut snare = AcSnare::new(sr, 0x6063);
        snare.trigger(0.80, 1.0, 0.75, 1.0, &SnareMods::default());
        assert_sane_render(&render_engine(|| snare.process(), 0.5), "snare");

        let mut hat = AcMetalHat::new(sr, 0x606606);
        hat.trigger(&CLOSED_HAT_SPEC, 0.70, 1.0, &HatMods::default());
        assert_sane_render(&render_engine(|| hat.process(), 0.3), "closed hat");

        let mut ohat = AcMetalHat::new(sr, 0x606606);
        ohat.trigger(&OPEN_HAT_SPEC, 0.70, 1.0, &HatMods::default());
        assert_sane_render(&render_engine(|| ohat.process(), 1.0), "open hat");

        let mut clap = AcClap::new(sr, 0x0606C1A9);
        clap.trigger(0.80, 1.0, 0.5, 1.0, 1.0, 0.0);
        assert_sane_render(&render_engine(|| clap.process(), 0.4), "clap");

        let mut ltom = AcTom::new(sr, 0x6061);
        ltom.trigger(&LOW_TOM_SPEC, 0.80, 1.0, &TomMods::default());
        assert_sane_render(&render_engine(|| ltom.process(), 0.5), "low tom");

        let mut htom = AcTom::new(sr, 0x6062);
        htom.trigger(&HIGH_TOM_SPEC, 0.80, 1.0, &TomMods::default());
        assert_sane_render(&render_engine(|| htom.process(), 0.5), "high tom");
    }
}

#[test]
fn ac_engines_go_silent() {
    let mut snare = AcSnare::new(SR, 0x6063);
    snare.trigger(0.5, 1.0, 0.75, 1.0, &SnareMods::default());
    for _ in 0..(SR * 1.5) as usize {
        snare.process();
    }
    assert!(!snare.is_active(), "snare should be done after its decay");

    let mut hat = AcMetalHat::new(SR, 0x606606);
    hat.trigger(&CLOSED_HAT_SPEC, 0.5, 1.0, &HatMods::default());
    for _ in 0..(SR * 1.0) as usize {
        hat.process();
    }
    assert!(
        !hat.is_active(),
        "closed hat should be done after its decay"
    );

    let mut clap = AcClap::new(SR, 0x0606C1A9);
    clap.trigger(0.8, 1.0, 0.5, 1.0, 1.0, 0.0);
    for _ in 0..(SR * 1.0) as usize {
        clap.process();
    }
    assert!(!clap.is_active(), "clap should stop at the voice duration");

    let mut tom = AcTom::new(SR, 0x6061);
    tom.trigger(&LOW_TOM_SPEC, 0.5, 1.0, &TomMods::default());
    for _ in 0..(SR * 2.0) as usize {
        tom.process();
    }
    assert!(!tom.is_active(), "tom should decay to silence");

    let mut kick = AcBassDrum::new(SR, 0x606606);
    kick.trigger(0.5, 0.0, 0.0, &BdMods::default());
    for _ in 0..(SR * 8.0) as usize {
        kick.process();
    }
    assert!(!kick.is_active(), "kick silence gate should have closed");
}

/// Every hit must be unique (free-running noise / randomized phases) — the
/// analog hardware behavior these engines model.
#[test]
fn ac_hits_differ_between_triggers() {
    let mut hat = AcMetalHat::new(SR, 0x606606);
    hat.trigger(&CLOSED_HAT_SPEC, 0.7, 1.0, &HatMods::default());
    let hit1 = render_engine(|| hat.process(), 0.1);
    hat.trigger(&CLOSED_HAT_SPEC, 0.7, 1.0, &HatMods::default());
    let hit2 = render_engine(|| hat.process(), 0.1);
    let diff = hit1
        .iter()
        .zip(hit2.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(diff > 1.0e-4, "two hits should not be bit-identical");
}

// ── AcVoice integration (the six new instruments) ───────────────────────────

fn render_voice(voice: &mut AcVoice, n: usize) -> Vec<f32> {
    (0..n).map(|_| voice.process_sample()).collect()
}

fn make(kind: AcEngineKind, settings: VoiceSettings) -> AcVoice {
    AcVoice::new(SR, kind, AcVoiceSettings::from(settings))
}

#[test]
fn ac_voices_render_all_six_kinds() {
    let mut bd = make(AcEngineKind::BassDrum, VoiceSettings::bd6ac());
    bd.trigger();
    assert_sane_render(&render_voice(&mut bd, (SR * 0.5) as usize), "BD6(AC)");

    let mut sd = make(AcEngineKind::Snare, VoiceSettings::sd6ac());
    sd.trigger();
    assert_sane_render(&render_voice(&mut sd, (SR * 0.5) as usize), "SD6(AC)");

    let mut hh = make(AcEngineKind::ClosedHat, VoiceSettings::hh6ac());
    hh.trigger();
    assert_sane_render(&render_voice(&mut hh, (SR * 0.3) as usize), "HH6(AC)");

    let mut oh = make(AcEngineKind::OpenHat, VoiceSettings::oh6ac());
    oh.trigger();
    assert_sane_render(&render_voice(&mut oh, (SR * 1.0) as usize), "OH6(AC)");

    let mut cl = make(AcEngineKind::Clap, VoiceSettings::cl6ac());
    cl.trigger();
    assert_sane_render(&render_voice(&mut cl, (SR * 0.4) as usize), "CL6(AC)");

    let mut tm = make(AcEngineKind::Tom, VoiceSettings::tm6ac());
    tm.trigger();
    assert_sane_render(&render_voice(&mut tm, (SR * 0.5) as usize), "TM6(AC)");
}

/// [196] Every exploration special must (a) be safe at both extremes (finite
/// output, no panic) and (b) audibly change the render vs the fitted default.
#[test]
fn ac_exploration_params_are_wired() {
    use super::super::ac_voice::{bd, cl, hh, sd, tm};

    struct Case {
        kind: AcEngineKind,
        settings: fn() -> VoiceSettings,
        specials: &'static [usize],
        name: &'static str,
    }
    let cases = [
        Case {
            kind: AcEngineKind::BassDrum,
            settings: VoiceSettings::bd6ac,
            name: "BD6",
            specials: &[
                bd::SWEEP,
                bd::BEND,
                bd::CLICK,
                bd::CLICK_TONE,
                bd::PUNCH,
                bd::TONE,
                bd::DRIVE,
            ],
        },
        Case {
            kind: AcEngineKind::Snare,
            settings: VoiceSettings::sd6ac,
            name: "SD6",
            specials: &[
                sd::SNAP,
                sd::WIRE_COLOR,
                sd::SHELL_BEND,
                sd::IMPACT,
                sd::RING,
            ],
        },
        Case {
            kind: AcEngineKind::ClosedHat,
            settings: VoiceSettings::hh6ac,
            name: "HH6",
            specials: &[
                hh::METAL,
                hh::CLICK,
                hh::BELL,
                hh::WOBBLE,
                hh::SPREAD,
                hh::BRIGHTNESS,
            ],
        },
        Case {
            kind: AcEngineKind::OpenHat,
            settings: VoiceSettings::oh6ac,
            name: "OH6",
            specials: &[
                hh::METAL,
                hh::CLICK,
                hh::BELL,
                hh::WOBBLE,
                hh::SPREAD,
                hh::BRIGHTNESS,
            ],
        },
        Case {
            kind: AcEngineKind::Clap,
            settings: VoiceSettings::cl6ac,
            name: "CL6",
            specials: &[cl::NOISE, cl::SPREAD, cl::TAIL, cl::AIR],
        },
        Case {
            kind: AcEngineKind::Tom,
            settings: VoiceSettings::tm6ac,
            name: "TM6",
            specials: &[
                tm::MODEL,
                tm::STRIKE,
                tm::SNAP,
                tm::GLIDE,
                tm::MODES,
                tm::TAIL_NOISE,
            ],
        },
    ];

    let n = (SR * 0.1) as usize;
    for case in cases {
        let base = (case.settings)();
        for &idx in case.specials {
            let original = base.special[idx];
            // Discrete selector (TM Model): 0/1/2. Click Tone is in Hz.
            // Continuous sliders: 0 and the registry max (<= 4).
            let extremes: [f32; 2] = if case.kind == AcEngineKind::Tom && idx == tm::MODEL {
                [1.0, 2.0]
            } else if case.kind == AcEngineKind::BassDrum && idx == bd::CLICK_TONE {
                [400.0, 4000.0]
            } else {
                [0.0, 4.0]
            };
            // Wiring check: the two extremes must render differently from
            // EACH OTHER (a dead param renders identically at both).
            let bufs: [Vec<f32>; 2] = extremes.map(|extreme| {
                let mut s = base;
                s.analog = 0.0; // deterministic
                                // Sensitizer: Click Tone is only measurable when the click
                                // stage is loud.
                if case.kind == AcEngineKind::BassDrum && idx == bd::CLICK_TONE {
                    s.special[bd::CLICK] = 1.0;
                }
                // The low tom spec has no upper modes — force the High spec
                // to measure the Modes knob.
                if case.kind == AcEngineKind::Tom && idx == tm::MODES {
                    s.special[tm::MODEL] = 2.0;
                }
                s.special[idx] = extreme;
                let mut v = make(case.kind, s);
                v.trigger();
                render_voice(&mut v, n)
            });
            for (buf, extreme) in bufs.iter().zip(extremes) {
                assert!(
                    buf.iter().all(|x| x.is_finite()),
                    "{}: special[{idx}]={extreme} produced non-finite output",
                    case.name
                );
            }
            let max_diff = bufs[0]
                .iter()
                .zip(bufs[1].iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(
                max_diff > 0.003,
                "{}: special[{idx}] (default {original}) renders identically at both extremes (max diff {max_diff})",
                case.name
            );
        }
    }
}

/// Repro for the song-load crash report (2026-09-02): build layouts covering
/// EVERY instrument kind (including all six AC kinds), serde-roundtrip them
/// like a state restore does, initialize the engine, trigger and process.
/// `panic = "abort"` in release means any panic here would kill the host.
#[test]
fn all_kinds_survive_a_song_load_cycle() {
    use crate::synthesis::DrumSynthesizer;
    use crate::track::{TrackInstrumentKind, TrackLayoutState};

    // Every kind gets a slot at least once (22 kinds over 14 slots → two kits).
    let kits: [&[TrackInstrumentKind]; 2] = [
        &TrackInstrumentKind::ALL[..14],
        &TrackInstrumentKind::ALL[14..],
    ];
    for kit in kits {
        let layout = TrackLayoutState::from_kinds(kit);
        // State-restore equivalent: serialize then deserialize the layout.
        let json = serde_json::to_string(&layout).unwrap();
        let layout: TrackLayoutState = serde_json::from_str(&json).unwrap();

        let mut synth = DrumSynthesizer::new();
        synth.initialize_with_layout(44100.0, &layout);
        for slot in 0..kit.len() {
            synth.trigger(slot, 1.0);
        }
        let mut outputs = [[0.0f32; 2]; crate::track::MAX_TRACKS];
        for _ in 0..4410 {
            synth.process_voice_samples_stereo(&mut outputs);
        }
        for (i, ch) in outputs.iter().enumerate() {
            assert!(
                ch[0].is_finite() && ch[1].is_finite(),
                "slot {i} produced non-finite output"
            );
        }
    }
}

/// [196e] BD6(AC) Decay Curve: bipolar shaping of the body amp envelope,
/// neutral at 0 (fitted). Curve -1 (convex, longer feel) must leave MORE
/// energy in the tail than curve +1 (concave, punchy).
#[test]
fn bd_decay_curve_is_wired() {
    let render = |curve: f32| -> Vec<f32> {
        let mut s = VoiceSettings::bd6ac();
        s.analog = 0.0;
        s.decay_curve = curve;
        let mut v = make(AcEngineKind::BassDrum, s);
        v.trigger();
        render_voice(&mut v, (SR * 0.2) as usize)
    };
    let concave = render(1.0);
    let convex = render(-1.0);
    for buf in [&concave, &convex] {
        assert!(buf.iter().all(|x| x.is_finite()), "non-finite output");
    }
    let max_diff = concave
        .iter()
        .zip(convex.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(
        max_diff > 0.01,
        "decay curve has no effect (max diff {max_diff})"
    );
    // Tail energy (last 50 ms): convex > concave.
    let tail = |b: &[f32]| b[b.len() - 2205..].iter().map(|s| s * s).sum::<f32>();
    assert!(
        tail(&convex) > tail(&concave) * 1.2,
        "convex tail ({}) should clearly exceed concave tail ({})",
        tail(&convex),
        tail(&concave)
    );
}

/// [196] The shared saturation chain is wired on the AC voices.
#[test]
fn ac_saturation_is_wired() {
    let render = |sat_on: bool| -> Vec<f32> {
        let mut s = VoiceSettings::sd6ac();
        s.analog = 0.0;
        if sat_on {
            s.special[super::super::ac_voice::SAT_TYPE] = 4.0; // HardClip
            s.special[super::super::ac_voice::SAT_AMOUNT] = 1.0;
            s.special[super::super::ac_voice::SAT_MIX] = 1.0;
            s.special[super::super::ac_voice::SAT_GAIN] = 1.0;
        }
        let mut v = make(AcEngineKind::Snare, s);
        v.trigger();
        render_voice(&mut v, 2000)
    };
    let dry = render(false);
    let wet = render(true);
    let max_diff = dry
        .iter()
        .zip(wet.iter())
        .fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
    assert!(
        max_diff > 0.01,
        "saturation has no effect on AC voices (max diff {max_diff})"
    );
}

/// Retrigger click-safety: two hits 31 ms apart must not produce a
/// sample-to-sample step much steeper than the same voice's own cold attack
/// (auto-calibrated, same method as `retrig_tests.rs` [179]).
#[test]
fn ac_voice_retrigger_is_click_free() {
    fn max_step(buf: &[f32]) -> f32 {
        buf.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    }

    let gap = (SR * 0.031) as usize;

    for (kind, settings, name) in [
        (AcEngineKind::BassDrum, VoiceSettings::bd6ac(), "BD6(AC)"),
        (AcEngineKind::Snare, VoiceSettings::sd6ac(), "SD6(AC)"),
        (AcEngineKind::ClosedHat, VoiceSettings::hh6ac(), "HH6(AC)"),
        (AcEngineKind::OpenHat, VoiceSettings::oh6ac(), "OH6(AC)"),
        (AcEngineKind::Clap, VoiceSettings::cl6ac(), "CL6(AC)"),
        (AcEngineKind::Tom, VoiceSettings::tm6ac(), "TM6(AC)"),
    ] {
        let mut s = settings;
        s.analog = 0.0; // digital: isolate the retrigger behavior
        let mut voice = make(kind, s);

        // Cold reference: steepest step of an isolated attack.
        voice.trigger();
        let cold = render_voice(&mut voice, (SR * 0.02) as usize);
        let cold_step = max_step(&cold);

        voice.reset();
        voice.trigger();
        let mut buf = render_voice(&mut voice, gap);
        voice.trigger();
        buf.extend(render_voice(&mut voice, (SR * 0.02) as usize));
        let retrig_step = max_step(&buf[gap..]);

        assert!(
            retrig_step <= cold_step * 1.5 + 0.02,
            "{name}: retrigger step {retrig_step} vs cold attack {cold_step}"
        );
    }
}
