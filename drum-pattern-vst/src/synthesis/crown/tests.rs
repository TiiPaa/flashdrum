//! [334] CROWN against the reference module in `resources/snare80`.
//! The oracle repeats the reference equations and stops before its look-ahead
//! normalisation (the voice uses a fixed calibrated gain instead).
use super::*;
use crate::synthesis::{DrumVoice, Voice, VoiceSettings};
use std::f64::consts::PI;

const TAU: f64 = 2.0 * PI;

#[derive(Clone, Copy)]
enum Kind {
    Lp,
    Hp,
    Pk,
    Bp,
}

#[derive(Clone, Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    z1: f64,
    z2: f64,
}
impl Biquad {
    fn new(kind: Kind, fc: f64, q: f64, sr: f64, gain_db: f64) -> Self {
        let w = TAU * fc.min(0.49 * sr) / sr;
        let (c, s) = (w.cos(), w.sin());
        let al = s / (2.0 * q);
        let (mut a0, a1, mut a2) = (1.0 + al, -2.0 * c, 1.0 - al);
        let (b0, b1, b2) = match kind {
            Kind::Lp => ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0),
            Kind::Hp => ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0),
            Kind::Pk => {
                let a = 10f64.powf(gain_db / 40.0);
                a0 = 1.0 + al / a;
                a2 = 1.0 - al / a;
                (1.0 + al * a, -2.0 * c, 1.0 - al * a)
            }
            Kind::Bp => (al, 0.0, -al),
        };
        Biquad { b0: b0 / a0, b1: b1 / a0, b2: b2 / a0, a1: a1 / a0, a2: a2 / a0, z1: 0.0, z2: 0.0 }
    }
    fn p(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

struct Rng(u32);
impl Rng {
    fn new() -> Self {
        Rng(0x80D5)
    }
    fn next(&mut self) -> f64 {
        let mut s = self.0;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.0 = s;
        s as f64 / 2147483648.0 - 1.0
    }
}

/// Reference equations, default settings, without the peak normalisation.
fn reference(sr: f64, samples: usize) -> Vec<f64> {
    use crate::synthesis::crown::crown_sp as sp;
    let s = VoiceSettings::crown();
    let mut rnd = Rng::new();
    let f0 = s.frequency as f64;
    let t_atk = s.attack as f64;
    let t_dec = s.decay as f64;
    let sweep = s.special[sp::SWEEP] as f64;
    let sweep_t = s.special[sp::SWEEP_T] as f64 / 1000.0;
    let drift = s.special[sp::DRIFT] as f64;
    let t_shape = s.special[sp::T_SHAPE] as f64;
    let t2r = s.special[sp::T2R] as f64;
    let t2 = s.special[sp::T2] as f64;
    let drive = s.special[sp::DRIVE] as f64;
    let n_hp = s.special[sp::N_HP] as f64;
    let n_lp = s.special[sp::N_LP] as f64;
    let n_pk_f = s.special[sp::N_PK_F] as f64;
    let n_pk_g = s.special[sp::N_PK_G] as f64;
    let n_atk = s.special[sp::N_ATK] as f64 / 1000.0;
    let n_dec = s.special[sp::N_DEC] as f64 / 1000.0;
    let n_shape = s.special[sp::N_SHAPE] as f64;
    let n_mod = s.special[sp::N_MOD] as f64;
    let tail = s.special[sp::TAIL] as f64;
    let tail_dec = s.special[sp::TAIL_DEC] as f64 / 1000.0;
    let snap = s.special[sp::SNAP] as f64;
    let snap_w = s.special[sp::SNAP_W] as f64;
    let click = s.special[sp::CLICK] as f64;
    let c_f = s.special[sp::C_F] as f64;
    let c_dec = s.special[sp::C_DEC] as f64 / 1000.0;
    let dly = s.special[sp::DLY] as f64 / 1000.0;
    let gate = s.special[sp::GATE] as f64 / 1000.0;
    let g_rel = s.special[sp::G_REL] as f64 / 1000.0;
    let tone = s.special[sp::TONE] as f64;
    let noise = s.special[sp::NOISE] as f64;
    let ph0 = s.special[sp::PH0] as f64;

    let f_start = f0 * 2f64.powf(sweep / 12.0);
    let dr = 1.0 + drive * 5.0;
    let dn = dr.tanh();
    let mut hp = Biquad::new(Kind::Hp, n_hp, 0.707, sr, 0.0);
    let mut hp2 = hp;
    let mut lp = Biquad::new(Kind::Lp, n_lp, 0.707, sr, 0.0);
    let mut lp2 = lp;
    let mut pk = Biquad::new(Kind::Pk, n_pk_f, 1.2, sr, n_pk_g);
    let mut cb = Biquad::new(Kind::Bp, c_f, 0.8, sr, 0.0);
    let snap_w = if snap_w > 0.0 { snap_w } else { 0.5 };
    let snap_n = (((snap_w / 1000.0 * sr) + 0.5).floor() as usize).max(1);
    let mut ph = TAU * ph0;
    let mut ph2 = 0.0f64;
    let mut out = Vec::with_capacity(samples);
    for i in 0..samples {
        let ta = i as f64 / sr;
        let t = ta - dly;
        let ck = cb.p(rnd.next()) * (-ta / c_dec).exp();
        let sp_i = if i < snap_n {
            snap * (PI * (i as f64 + 0.5) / snap_n as f64).sin()
        } else {
            0.0
        };
        if t < 0.0 {
            rnd.next();
            out.push(click * ck + sp_i);
            continue;
        }
        let f = ((f0 + (f_start - f0) * (-t / sweep_t).exp()) * 2f64.powf(drift * t * 10.0 / 12.0))
            .max(30.0);
        ph += TAU * f / sr;
        ph2 += TAU * f * t2r / sr;
        let e_t = (1.0 - (-t / t_atk).exp()) * (-(t / t_dec).powf(t_shape)).exp();
        let mut t_one = ph.sin() + t2 * ph2.sin() * (-t / (t_dec * 0.5)).exp();
        t_one = (dr * t_one * e_t).tanh() / dn;
        let e_n = (1.0 - (-t / n_atk).exp()) * (-(t / n_dec).powf(n_shape)).exp()
            + tail * (-t / tail_dec).exp();
        let w = rnd.next();
        let nz = pk.p(lp2.p(lp.p(hp2.p(hp.p(w))))) * e_n * (1.0 + n_mod * ph.sin());
        let mut x = tone * t_one + noise * nz + click * ck + sp_i;
        if ta > gate {
            x *= (-(ta - gate) / g_rel).exp();
        }
        out.push(x);
    }
    out
}

#[test]
fn crown_plays_decays_and_retriggers_without_a_click() {
    let mut v = CrownVoice::new(48_000.0);
    v.set_settings(VoiceSettings::crown());
    v.trigger();
    let mut energy = 0.0f64;
    let mut peak = 0.0f32;
    for _ in 0..48_000 {
        let x = v.process_sample();
        energy += (x as f64).abs();
        peak = peak.max(x.abs());
    }
    assert!(energy > 1.0, "the hit must sound: {energy}");
    assert!(peak <= 1.0, "the hit must stay in range: {peak}");
    assert!(!v.is_active(), "the hit must end by itself");

    // A retrigger during the tail continues from the previous sample.
    v.trigger();
    for _ in 0..2_000 {
        v.process_sample();
    }
    let before = v.process_sample();
    v.trigger();
    let after = v.process_sample();
    assert!((after - before).abs() < 0.2, "retrigger jumped: {before} -> {after}");
}

#[test]
fn crown_every_control_changes_the_hit() {
    use crate::instrument_registry::{StandardField, INSTRUMENTS};
    let def = &INSTRUMENTS[DrumVoice::Crown as usize];
    let render = |s: VoiceSettings| {
        let mut v = CrownVoice::new(48_000.0);
        v.set_settings(s);
        v.trigger();
        (0..96_000).map(|_| v.process_sample()).collect::<Vec<_>>()
    };
    let base = VoiceSettings::crown();
    for p in def.special_params {
        let mut low = base;
        low.special[p.special_index] = p.min;
        let mut high = base;
        // Gate Release only shapes the end when Gate comes before the hit is over.
        if p.name == "crown_gate_release" {
            low.special[super::crown_sp::GATE] = 120.0;
            high.special[super::crown_sp::GATE] = 120.0;
        }
        // Start Phase is in turns: 0 and 1 are the same phase, compare 0.5.
        high.special[p.special_index] = if p.name == "crown_phase" { 0.5 } else { p.max };
        let a = render(low);
        let b = render(high);
        let diff: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum();
        assert!(diff > 0.001, "{} does nothing", p.name);
    }
    for (field, value) in [
        (StandardField::Freq, 200.0),
        (StandardField::Decay, 0.05),
        (StandardField::Attack, 0.004),
        (StandardField::Volume, 0.3),
    ] {
        let mut s = base;
        s.set(crate::param_id::ParamId::Std(field), value);
        let a = render(base);
        let b = render(s);
        let diff: f32 = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).sum();
        assert!(diff > 0.001, "{field:?} does nothing");
    }
}

#[test]
fn crown_renders_at_every_supported_rate_without_allocating() {
    for sr in [44_100.0, 48_000.0, 96_000.0, 192_000.0] {
        let mut v = CrownVoice::new(sr);
        v.set_settings(VoiceSettings::crown());
        v.trigger();
        assert_no_alloc::assert_no_alloc(|| {
            for _ in 0..(sr as usize / 4) {
                let x = v.process_sample();
                assert!(x.is_finite(), "non-finite sample at {sr}");
            }
            v.trigger();
            for _ in 0..256 {
                assert!(v.process_sample().is_finite());
            }
        });
    }
}

#[test]
fn crown_matches_the_reference_before_its_peak_normalisation() {
    for sr in [44_100.0, 48_000.0, 96_000.0] {
        let s = VoiceSettings::crown();
        let samples = (0.6 * sr) as usize;
        let mut voice = CrownVoice::new(sr as f32);
        voice.set_settings(s);
        voice.trigger();
        let oracle = reference(sr, samples);
        let mut peak = 0.0f64;
        for (i, expected) in oracle.iter().enumerate() {
            let got = voice.process_sample() as f64 / (s.volume as f64 * GAIN);
            peak = peak.max(expected.abs());
            assert!(
                (got - expected).abs() < 1e-5,
                "sample {i} at {sr} Hz: got {got}, expected {expected}"
            );
        }
        println!("{sr} Hz: reference peak {peak:.6}");
    }
}
