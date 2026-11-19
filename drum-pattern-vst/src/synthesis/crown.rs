//! [334] CROWN — a synthesised 80s snare: a pitched tone with a sweep, filtered
//! noise with a peak, a click/snap attack and a gate.
//!
//! Ported from the reference module in `resources/snare80`: same equations, same
//! noise sequence and same filter order, evaluated one sample at a time. Two
//! deliberate differences, both forced by real-time playback:
//! * the reference normalised each hit to its own peak; here a fixed calibrated
//!   gain replaces that look-ahead (the default settings keep the reference
//!   level exactly);
//! * the reference rendered a fixed `len`; here the hit ends on its own, gate +
//!   closing time, so the duration follows the sound.
//! The machine-grain option is not ported: Flash Drum already has a bit crusher.
#[cfg(test)]
mod tests;

use super::dsp;
use super::{Voice, VoiceSettings};
use std::f64::consts::PI;

const TAU: f64 = 2.0 * PI;

/// [334] Fixed gain replacing the reference's whole-hit peak normalisation.
/// Calibrated on the default settings (measured by `raw_default_peak`).
const GAIN: f64 = 0.622;

pub struct CrownVoice {
    sample_rate: f64,
    settings: VoiceSettings,
    current: VoiceSettings,
    active: bool,
    position: usize,
    duration: usize,
    rng: u32,
    hp: Biquad,
    hp2: Biquad,
    lp: Biquad,
    lp2: Biquad,
    pk: Biquad,
    cb: Biquad,
    ph: f64,
    ph2: f64,
    declick: dsp::RetrigDeclick,
    last: f32,
    smooth: f64,
}

impl CrownVoice {
    pub fn new(sample_rate: f32) -> Self {
        let settings = VoiceSettings::crown();
        Self {
            sample_rate: sample_rate as f64,
            settings,
            current: settings,
            active: false,
            position: 0,
            duration: 0,
            rng: 0x80D5,
            hp: Biquad::default(),
            hp2: Biquad::default(),
            lp: Biquad::default(),
            lp2: Biquad::default(),
            pk: Biquad::default(),
            cb: Biquad::default(),
            ph: 0.0,
            ph2: 0.0,
            declick: dsp::RetrigDeclick::new(sample_rate),
            last: 0.0,
            smooth: 1.0 - (-1.0 / (sample_rate as f64 * 0.005)).exp(),
        }
    }

    fn advance_controls(&mut self) {
        let a = self.smooth;
        for (current, target) in [
            (&mut self.current.frequency, self.settings.frequency),
            (&mut self.current.attack, self.settings.attack),
            (&mut self.current.decay, self.settings.decay),
            (&mut self.current.volume, self.settings.volume),
        ] {
            *current += (a as f32) * (target - *current);
        }
        for i in 0..32 {
            self.current.special[i] += (a as f32) * (self.settings.special[i] - self.current.special[i]);
        }
    }

    fn filter_params(&self) -> (f64, f64, f64, f64, f64, f64) {
        let s = &self.current.special;
        (
            s[crown_sp::N_HP] as f64,
            s[crown_sp::N_LP] as f64,
            s[crown_sp::N_PK_F] as f64,
            s[crown_sp::N_PK_G] as f64,
            s[crown_sp::C_F] as f64,
            0.0,
        )
    }

    fn rebuild_filters(&mut self) {
        let (n_hp, n_lp, n_pk_f, n_pk_g, c_f, _) = self.filter_params();
        let sr = self.sample_rate;
        let hp = Biquad::new(Kind::Hp, n_hp, 0.707, sr, 0.0);
        let lp = Biquad::new(Kind::Lp, n_lp, 0.707, sr, 0.0);
        let pk = Biquad::new(Kind::Pk, n_pk_f, 1.2, sr, n_pk_g);
        let cb = Biquad::new(Kind::Bp, c_f, 0.8, sr, 0.0);
        self.hp.set_coeffs(&hp);
        self.hp2.set_coeffs(&hp);
        self.lp.set_coeffs(&lp);
        self.lp2.set_coeffs(&lp);
        self.pk.set_coeffs(&pk);
        self.cb.set_coeffs(&cb);
    }

    fn duration_samples(&self) -> usize {
        let s = &self.current.special;
        let gate = s[crown_sp::GATE] as f64;
        let rel = s[crown_sp::G_REL] as f64;
        let ms = (gate + 5.0 * rel).clamp(80.0, 700.0);
        ((ms / 1000.0 * self.sample_rate).ceil() as usize).max(64)
    }

    fn sample(&mut self) -> f64 {
        let s = self.current;
        let sp = &s.special;
        let gain = GAIN;
        let ta = self.position as f64 / self.sample_rate;
        let dly = sp[crown_sp::DLY] as f64 / 1000.0;
        let t = ta - dly;
        let c_dec = sp[crown_sp::C_DEC] as f64 / 1000.0;

        let noise_seed = self.rand();
        let ck = self.cb.p(noise_seed) * (-ta / c_dec.max(1e-6)).exp();
        let snap_w = if sp[crown_sp::SNAP_W] > 0.0 {
            sp[crown_sp::SNAP_W] as f64
        } else {
            0.5
        };
        let snap_n = (((snap_w / 1000.0 * self.sample_rate) + 0.5).floor() as usize).max(1);
        let impulse = if self.position < snap_n {
            sp[crown_sp::SNAP] as f64 * (PI * (self.position as f64 + 0.5) / snap_n as f64).sin()
        } else {
            0.0
        };
        let click = sp[crown_sp::CLICK] as f64;
        if t < 0.0 {
            self.rand();
            return (click * ck + impulse) * gain;
        }

        let f0 = s.frequency as f64;
        let sweep = sp[crown_sp::SWEEP] as f64;
        let sweep_t = sp[crown_sp::SWEEP_T] as f64 / 1000.0;
        let drift = sp[crown_sp::DRIFT] as f64;
        let t_atk = s.attack as f64;
        let t_dec = s.decay as f64;
        let t_shape = sp[crown_sp::T_SHAPE] as f64;
        let t2r = sp[crown_sp::T2R] as f64;
        let t2 = sp[crown_sp::T2] as f64;
        let drive = sp[crown_sp::DRIVE] as f64;

        let f_start = f0 * 2f64.powf(sweep / 12.0);
        let f = ((f0 + (f_start - f0) * (-t / sweep_t.max(1e-6)).exp())
            * 2f64.powf(drift * t * 10.0 / 12.0))
        .max(30.0);
        self.ph += TAU * f / self.sample_rate;
        self.ph2 += TAU * f * t2r / self.sample_rate;

        let dr = 1.0 + drive * 5.0;
        let dn = dr.tanh();
        let e_t = (1.0 - (-t / t_atk.max(1e-6)).exp()) * (-(t / t_dec.max(1e-6)).powf(t_shape)).exp();
        let mut tone = self.ph.sin() + t2 * self.ph2.sin() * (-t / (t_dec * 0.5).max(1e-6)).exp();
        tone = (dr * tone * e_t).tanh() / dn;

        let n_atk = sp[crown_sp::N_ATK] as f64 / 1000.0;
        let n_dec = sp[crown_sp::N_DEC] as f64 / 1000.0;
        let n_shape = sp[crown_sp::N_SHAPE] as f64;
        let tail = sp[crown_sp::TAIL] as f64;
        let tail_dec = sp[crown_sp::TAIL_DEC] as f64 / 1000.0;
        let n_mod = sp[crown_sp::N_MOD] as f64;
        let e_n = (1.0 - (-t / n_atk.max(1e-6)).exp())
            * (-(t / n_dec.max(1e-6)).powf(n_shape)).exp()
            + tail * (-t / tail_dec.max(1e-6)).exp();
        let w = self.rand();
        let nz = self
            .pk
            .p(self.lp2.p(self.lp.p(self.hp2.p(self.hp.p(w)))))
            * e_n
            * (1.0 + n_mod * self.ph.sin());

        let mut x = sp[crown_sp::TONE] as f64 * tone
            + sp[crown_sp::NOISE] as f64 * nz
            + click * ck
            + impulse;
        let gate = sp[crown_sp::GATE] as f64 / 1000.0;
        if ta > gate {
            x *= (-(ta - gate) / (sp[crown_sp::G_REL] as f64 / 1000.0).max(1e-6)).exp();
        }
        // [334] Density: the reference scaled by the hit's own peak; with a fixed
        // gain the same shape is applied to the calibrated signal instead.
        let sat = sp[crown_sp::SAT] as f64;
        if sat > 0.0 {
            let k = 1.0 + sat * 6.0;
            x = (k * x).tanh() / k.tanh();
        }
        x * gain
    }

    fn rand(&mut self) -> f64 {
        let mut s = self.rng;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.rng = s;
        s as f64 / 2147483648.0 - 1.0
    }
}

impl Voice for CrownVoice {
    fn trigger(&mut self) {
        self.current = self.settings;
        self.position = 0;
        self.active = true;
        self.rng = 0x80D5;
        self.ph = TAU * (self.current.special[crown_sp::PH0] as f64);
        self.ph2 = 0.0;
        self.rebuild_filters();
        self.hp.reset();
        self.hp2.reset();
        self.lp.reset();
        self.lp2.reset();
        self.pk.reset();
        self.cb.reset();
        self.duration = self.duration_samples();
        self.declick.arm(self.last);
    }

    fn trigger_hard(&mut self) {
        self.trigger();
    }

    fn process_sample(&mut self) -> f32 {
        if !self.active && !self.declick.is_active() {
            return 0.0;
        }
        self.advance_controls();
        self.duration = self.duration_samples();
        let mut out = 0.0f32;
        if self.active && self.position < self.duration {
            out = self.sample() as f32 * self.current.volume;
            self.position += 1;
        } else {
            self.active = false;
        }
        out += self.declick.next();
        self.last = out;
        out
    }

    fn is_active(&self) -> bool {
        self.active || self.declick.is_active()
    }

    fn reset(&mut self) {
        self.active = false;
        self.position = 0;
        self.last = 0.0;
        self.declick.reset();
        self.hp.reset();
        self.hp2.reset();
        self.lp.reset();
        self.lp2.reset();
        self.pk.reset();
        self.cb.reset();
    }

    fn set_settings(&mut self, mut s: VoiceSettings) {
        use crate::instrument_registry::{ParamWidget, INSTRUMENTS};
        let def = &INSTRUMENTS[self.voice_index()];
        for p in def.standard_params {
            let id = crate::param_id::ParamId::Std(p.field);
            let mut v = s.get(id);
            if !v.is_finite() {
                v = def.sound_settings_default[p.field as usize];
            }
            v = match p.widget {
                ParamWidget::Slider { min, max, .. } => v.clamp(min, max),
                ParamWidget::Checkbox => v.clamp(0.0, 1.0),
            };
            s.set(id, v);
        }
        for p in def.special_params {
            let v = &mut s.special[p.special_index];
            *v = if v.is_finite() {
                v.clamp(p.min, p.max)
            } else {
                p.default
            };
        }
        s.algo = 0;
        self.settings = s;
        if !self.active {
            self.current = s;
        }
        self.rebuild_filters();
    }

    fn set_algo(&mut self, _algo: u8) {}

    fn set_special_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.settings.special.get_mut(index) {
            *v = value;
        }
        self.set_settings(self.settings);
    }
}

impl CrownVoice {
    fn voice_index(&self) -> usize {
        super::DrumVoice::Crown as usize
    }
}

/// [334] `special[]` indices of the CROWN controls (index 4 stays reserved by
/// the p-lock layout).
pub mod crown_sp {
    pub const SWEEP: usize = 0;
    pub const SWEEP_T: usize = 1;
    pub const DRIFT: usize = 2;
    pub const T_SHAPE: usize = 3;
    pub const T2R: usize = 5;
    pub const T2: usize = 6;
    pub const DRIVE: usize = 7;
    pub const TONE: usize = 8;
    pub const NOISE: usize = 9;
    pub const N_HP: usize = 10;
    pub const N_LP: usize = 11;
    pub const N_PK_F: usize = 12;
    pub const N_PK_G: usize = 13;
    pub const N_ATK: usize = 14;
    pub const N_DEC: usize = 15;
    pub const N_SHAPE: usize = 16;
    pub const N_MOD: usize = 17;
    pub const TAIL: usize = 18;
    pub const TAIL_DEC: usize = 19;
    pub const PH0: usize = 20;
    pub const SNAP: usize = 21;
    pub const SNAP_W: usize = 22;
    pub const CLICK: usize = 23;
    pub const C_F: usize = 24;
    pub const C_DEC: usize = 25;
    pub const DLY: usize = 26;
    pub const GATE: usize = 27;
    pub const G_REL: usize = 28;
    pub const SAT: usize = 29;
}

#[derive(Clone, Copy, Default)]
enum Kind {
    #[default]
    Lp,
    Hp,
    Pk,
    Bp,
}

/// RBJ biquad, transposed direct form II — the reference filter, kept in f64.
#[derive(Clone, Copy, Default)]
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
        let mut b = Self::default();
        b.set(kind, fc, q, sr, gain_db);
        b
    }
    fn set(&mut self, kind: Kind, fc: f64, q: f64, sr: f64, gain_db: f64) {
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
        self.b0 = b0 / a0;
        self.b1 = b1 / a0;
        self.b2 = b2 / a0;
        self.a1 = a1 / a0;
        self.a2 = a2 / a0;
    }
    fn set_coeffs(&mut self, other: &Self) {
        self.b0 = other.b0;
        self.b1 = other.b1;
        self.b2 = other.b2;
        self.a1 = other.a1;
        self.a2 = other.a2;
    }
    fn reset(&mut self) {
        self.z1 = 0.0;
        self.z2 = 0.0;
    }
    #[inline]
    fn p(&mut self, x: f64) -> f64 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}
