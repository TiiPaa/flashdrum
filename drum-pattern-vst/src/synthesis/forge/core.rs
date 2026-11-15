//! Percussion Forge equations, evaluated one sample at a time.
//! f64 phases/filters follow JavaScript Number; delay storage follows Float32Array.
use super::{ChannelMemory, ForgeKind};
use crate::synthesis::settings::forge::*;
use crate::synthesis::VoiceSettings;
use std::f64::consts::{PI, TAU};

#[derive(Clone, Copy)]
pub(super) enum Parameters {
    Ion(IonBloomSettings),
    Ferro(FerroskinSettings),
    Prism(PrismHatSettings),
    Orbit(OrbitClaySettings),
    Neon(NeonTomSettings),
    Arc(ArcZapSettings),
    Tidal(TidalAlloySettings),
    Mobius(MobiusSkinSettings),
    Abyss(AbyssFmSettings),
}
impl Parameters {
    pub fn new(kind: ForgeKind, settings: VoiceSettings) -> Self {
        match kind {
            ForgeKind::IonBloom => Self::Ion(settings.into()),
            ForgeKind::Ferroskin => Self::Ferro(settings.into()),
            ForgeKind::PrismHat => Self::Prism(settings.into()),
            ForgeKind::OrbitClay => Self::Orbit(settings.into()),
            ForgeKind::NeonTom => Self::Neon(settings.into()),
            ForgeKind::ArcZap => Self::Arc(settings.into()),
            ForgeKind::TidalAlloy => Self::Tidal(settings.into()),
            ForgeKind::MobiusSkin => Self::Mobius(settings.into()),
            ForgeKind::AbyssFm => Self::Abyss(settings.into()),
        }
    }
}

#[derive(Clone, Default)]
pub(super) struct Channel {
    phase: [f64; 12],
    tone_phase: [f64; 2],
    energy: [f64; 5],
    low: f64,
    noise_low: f64,
    damping: [f64; 3],
    writes: [usize; 3],
    chorus_write: usize,
    rng: u32,
    next_bounce: f64,
    bounce: usize,
    interval: f64,
    pulse_pos: usize,
    pulse_len: usize,
    pulse_amp: f64,
}
impl Channel {
    pub fn new(kind: ForgeKind, seed: u32) -> Self {
        let mut c = Self {
            rng: seed,
            ..Self::default()
        };
        if kind == ForgeKind::TidalAlloy {
            for i in 0..12 {
                c.phase[i] = c.rand() * TAU;
            }
            for i in 0..2 {
                c.tone_phase[i] = c.rand() * TAU;
            }
        }
        c
    }
    fn rand(&mut self) -> f64 {
        self.rng = self.rng.wrapping_add(0x6D2B79F5);
        let mut t = (self.rng ^ (self.rng >> 15)).wrapping_mul(1 | self.rng);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }
    pub fn sample(
        &mut self,
        p: Parameters,
        t: f64,
        sr: f64,
        mem: &mut ChannelMemory,
        side: f64,
    ) -> f64 {
        match p {
            Parameters::Ion(p) => {
                let pitch = p.common.frequency as f64;
                let dc = p.common.decay as f64;
                let bloom = p.bloom / 100.0;
                let e = (-t / (dc * 0.38)).exp();
                let f = pitch + p.drop * (-t / (0.012 + bloom * 0.035)).exp();
                self.phase[0] += TAU * f / sr;
                self.phase[1] += PI * (pitch * 0.997) / sr;
                let raw = if t < 0.018 {
                    (self.rand() * 2.0 - 1.0) * (-t / 0.0038).exp() * p.impact / 100.0
                } else {
                    0.0
                };
                self.low += coef(0.22, sr) * (raw - self.low);
                let click = (raw - self.low) * (-t / 0.005).exp();
                let late = (1.0 - (-t / 0.045).exp()) * (-t / (dc * 0.65)).exp();
                let attack = 1.0 - (-t / 0.0007).exp();
                let ph = self.phase[0];
                let x = ((ph + 0.18 * (ph * 2.0).sin()).sin() * e
                    + self.phase[1].sin() * late * bloom * 0.52
                    + (ph * 0.5).sin() * e * e * 0.13)
                    * attack
                    + click * 0.42;
                soft(x * 0.78, p.pressure / 100.0)
            }
            Parameters::Ferro(p) => {
                let pitch = p.common.frequency as f64;
                let dc = p.common.decay as f64;
                let ten = p.tension / 100.0;
                let cpl = p.coupling / 100.0;
                let body_e = (-t / (dc * 0.58)).exp();
                let noise_e = (-t / (dc * 0.31)).exp();
                let fa = pitch * (1.0 + ten * 0.32);
                let fb = pitch * (1.43 + ten * 0.53);
                self.phase[0] += TAU * (fa + self.phase[1].sin() * cpl * 11.0) / sr;
                self.phase[1] += TAU * (fb + self.phase[0].sin() * cpl * 17.0) / sr;
                let z = self.rand() * 2.0 - 1.0;
                self.low += coef(0.22, sr) * (z - self.low);
                let hp = z - self.low;
                self.phase[2] += TAU * (2100.0 + ten * 3300.0) / sr;
                let wl = self.phase[2];
                let wires = (wl.sin() + 0.55 * (wl * 1.71).sin() + 0.3 * (wl * 2.43).sin())
                    * p.wire
                    / 100.0;
                let impact_e = (1.0 - (-t / 0.00018).exp()) * (-t / (0.0018 + ten * 0.0017)).exp();
                let crack =
                    (hp * 0.95 + (self.phase[0] * 2.17).sin() * 0.32) * impact_e * p.impact / 100.0;
                let membrane =
                    (self.phase[0].sin() + self.phase[1].sin() * (0.48 + cpl * 0.2)) * body_e;
                let body =
                    membrane * 0.48 + hp * noise_e * p.noise / 100.0 * 1.2 + wires * noise_e * 0.16;
                soft(body, 0.2 + cpl * 0.32) + crack * 1.4
            }
            Parameters::Prism(p) => {
                const RATIOS: [f64; 6] = [1.0, 1.337, 1.843, 2.317, 2.881, 3.73];
                let pitch = p.common.frequency as f64;
                let dc = p.common.decay as f64;
                let sp = p.spread / 100.0;
                let den = p.density / 100.0;
                let bite = p.bite / 100.0;
                let mut bank = 0.0;
                for (k, ratio) in RATIOS.iter().enumerate() {
                    let r = 1.0 + (ratio - 1.0) * (0.35 + sp * 1.25);
                    self.phase[k] += TAU * pitch * r / sr;
                    let sq = if self.phase[k].sin() > 0.0 { 1.0 } else { -1.0 };
                    bank += sq * (1.0 - k as f64 * 0.085);
                }
                bank /= 4.5;
                let ring = self.phase[1].sin() * self.phase[4].sin()
                    + 0.7 * self.phase[2].sin() * self.phase[5].sin();
                let z = self.rand() * 2.0 - 1.0;
                self.low += coef(0.015 + p.color / 100.0 * 0.16, sr) * (z - self.low);
                let hp = z - self.low;
                let mut x =
                    (bank * (0.25 + den * 0.18) + ring * den * 0.32 + hp * (1.0 - den) * 0.5)
                        * (-t / (dc * 0.42)).exp();
                let quant = 5.0 + bite * 38.0;
                let gate = if bite > 0.0 {
                    (x * quant + 0.5).floor() / quant
                } else {
                    x
                };
                x = x * (1.0 - bite * 0.5) + gate * bite * 0.7;
                soft(x * 1.25, bite * 0.45)
            }
            Parameters::Orbit(p) => {
                const WOOD: [f64; 5] = [1.0, 1.92, 2.83, 3.76, 4.71];
                const METAL: [f64; 5] = [1.0, 1.57, 2.36, 3.31, 4.64];
                let pitch = p.common.frequency as f64;
                let dc = p.common.decay as f64;
                let mat = p.material / 100.0;
                let grav = p.gravity / 100.0;
                let drift = (p.drift / 100.0).powf(0.8);
                if t == 0.0 {
                    self.interval = 0.045 + (1.0 - grav) * 0.085;
                    self.pulse_len = (sr * (0.0035 + mat * 0.0015)).floor().max(1.0) as usize;
                }
                let bounces = 1 + (p.bounce / 100.0 * 5.0).floor() as usize;
                if self.bounce < bounces && t >= self.next_bounce {
                    self.pulse_pos = 0;
                    self.pulse_amp = 0.58f64.powi(self.bounce as i32);
                    self.bounce += 1;
                    self.next_bounce += self.interval;
                    self.interval *= 1.18 + grav * 0.42;
                }
                if self.pulse_pos < self.pulse_len {
                    let excite = (PI * self.pulse_pos as f64 / self.pulse_len as f64).sin()
                        * self.pulse_amp
                        / self.pulse_len as f64
                        * 7.5;
                    for k in 0..5 {
                        self.energy[k] +=
                            excite / (1.0 + k as f64 * 0.62) * (0.96 + self.rand() * 0.04);
                    }
                    self.pulse_pos += 1;
                }
                let mut sum = 0.0;
                for k in 0..5 {
                    let fk = k as f64;
                    let mode = WOOD[k] * (1.0 - mat) + METAL[k] * mat;
                    let wobble = (TAU * p.mod_rate * (0.7 + mat * 0.7 + fk * 0.13) * t + fk * 0.91)
                        .sin()
                        + (TAU * p.mod_rate * (0.23 + grav * 0.22) * t + fk * 1.63).sin() * 0.35;
                    let tilt = 1.0
                        - drift
                            * (0.12 + fk * 0.09)
                            * (0.5
                                + 0.5
                                    * (TAU * p.mod_rate * (0.4 + fk * 0.08) * t + fk * 0.72).sin());
                    self.phase[k] +=
                        TAU * pitch * mode * (1.0 + wobble * drift * (0.011 + fk * 0.004)) / sr;
                    self.energy[k] *= (-1.0
                        / (sr * (dc * (1.0 - fk * 0.11) * (1.0 + mat * 0.35)).max(0.035)))
                    .exp();
                    sum += self.phase[k].sin() * self.energy[k] * tilt;
                }
                let cutoff = (2100.0 - drift * 1500.0
                    + (TAU * p.mod_rate * (0.5 + mat * 0.3) * t).sin() * drift * 700.0)
                    .max(380.0);
                self.low += (1.0 - (-TAU * cutoff / sr).exp()) * (sum - self.low);
                sum = sum * (1.0 - drift * 0.82) + self.low * drift * 0.82;
                clean_soft(sum * 0.27, (p.drive / 100.0).powf(1.5) * 0.3)
            }
            Parameters::Neon(p) => {
                let dc = p.common.decay as f64;
                let shape = p.shape / 100.0;
                let e = (-t / (dc * 0.48)).exp();
                let f = p.common.frequency as f64 + p.drop * (-t / (0.018 + shape * 0.055)).exp();
                self.phase[0] += TAU * f / sr;
                self.phase[1] += TAU * f * 1.997 / sr;
                let sine = self.phase[0].sin();
                let tri = 2.0 / PI * sine.asin();
                let body = sine * (1.0 - shape) + tri * shape;
                let raw = (self.rand() * 2.0 - 1.0) * (-t / 0.004).exp() * p.snap / 100.0;
                self.low += coef(0.16, sr) * (raw - self.low);
                let x = (body + 0.14 * self.phase[1].sin() * e) * e + (raw - self.low) * 0.18;
                clean_soft(x * 0.62, p.drive / 100.0)
            }
            Parameters::Arc(p) => {
                let dc = p.common.decay as f64;
                let sweep_dc = 0.9 - p.speed / 100.0 * 0.84;
                let pitch_env = (-t / (sweep_dc * 0.34).max(0.025)).exp();
                let f = p.common.frequency as f64 * 2.0f64.powf(p.amount / 12.0 * pitch_env);
                self.phase[1] += TAU * f * 1.618 / sr;
                self.phase[0] += TAU * f / sr;
                let sine = (self.phase[0] + self.phase[1].sin() * p.fm / 100.0 * 0.85).sin();
                let tri = 2.0 / PI * self.phase[0].sin().asin();
                (sine * (1.0 - p.shape / 100.0) + tri * p.shape / 100.0)
                    * (-t / (dc * 0.42)).exp()
                    * 0.72
            }
            Parameters::Tidal(p) => {
                const RATIOS: [f64; 12] = [
                    1.0, 1.274, 1.537, 1.911, 2.317, 2.863, 3.421, 4.097, 4.883, 5.677, 6.619,
                    7.731,
                ];
                let pitch = p.common.frequency as f64;
                let dc = p.common.decay as f64;
                let morph = p.morph / 100.0;
                let spread = p.spread / 100.0;
                let tide = p.tide / 100.0;
                let body_e = (-t / (dc * (0.32 + morph * 0.2))).exp();
                let crash_e = (-t / (dc * (0.2 + morph * 0.22))).exp();
                let bell_e = (-t / (0.12 + dc * 0.09)).exp();
                let drop_time = 0.5 - p.drop_speed / 100.0 * 0.44;
                let drop_ratio = 2.0f64.powf(p.tone_drop / 12.0 * (-t / drop_time).exp());
                let mut bank = 0.0;
                let mut bell = 0.0;
                for (k, ratio) in RATIOS.iter().enumerate() {
                    let fk = k as f64;
                    let drift = 1.0
                        + (t * (2.1 + fk * 0.13) + if k % 2 == 1 { 1.7 } else { 0.0 }).sin()
                            * tide
                            * (0.0012 + fk * 0.00016);
                    let r = 1.0 + (ratio - 1.0) * (0.78 + spread * 0.44);
                    self.phase[k] += TAU * pitch * r * drift / sr;
                    let s = self.phase[k].sin();
                    bank += if s > 0.0 {
                        1.0
                    } else if s < 0.0 {
                        -1.0
                    } else {
                        0.0
                    } / (1.0 + fk * 0.09);
                    if k < 2 {
                        let f = (pitch * r * 1.997 * drift * drop_ratio).min(sr * 0.44);
                        self.tone_phase[k] += TAU * f / sr;
                        bell += self.tone_phase[k].sin() / (1.0 + fk * 0.4);
                    }
                }
                bank /= 7.2;
                self.low += coef(0.055 + spread * 0.055, sr) * (bank - self.low);
                let cymbal_hp = bank - self.low;
                let z = self.rand() * 2.0 - 1.0;
                self.noise_low += coef(0.035, sr) * (z - self.noise_low);
                let noise_hp = z - self.noise_low;
                let diffuse = cymbal_hp * 0.42 + noise_hp * (0.18 + morph * 0.2);
                let raw = clean_soft(
                    diffuse * body_e * 0.34 * (1.0 - morph)
                        + (diffuse + noise_hp * 0.22) * crash_e * 0.32 * morph
                        + bell * bell_e * p.bell / 100.0 * 0.1
                        + noise_hp * (-t / 0.014).exp() * (0.06 + morph * 0.13),
                    0.04 + tide * 0.05,
                );
                mem.chorus[self.chorus_write] = raw as f32;
                let depth = p.lfo_depth / 100.0;
                let out = if side != 0.0 && depth > 0.0 {
                    let lfo = (TAU * p.lfo_rate * t).sin() * side;
                    let delay = sr * (0.0012 + depth * (0.0021 + 0.0019 * lfo));
                    let delayed = read_delay(&mem.chorus, self.chorus_write, delay);
                    let wet = depth * 0.68;
                    raw * (1.0 - wet) + delayed * wet
                } else {
                    raw
                };
                self.chorus_write = (self.chorus_write + 1) % mem.chorus.len();
                out
            }
            Parameters::Mobius(p) => {
                let pitch = p.common.frequency as f64;
                let dc = p.common.decay as f64;
                let mat = p.material / 100.0;
                let tw = p.twist / 100.0;
                let base = sr / pitch;
                let delays = [base, base * 0.741, base * 0.527];
                let theta = tw * TAU / 3.0;
                let rc = theta.cos();
                let rs = theta.sin();
                let rd = (1.0 + 2.0 * rc) / 3.0;
                let rf = (1.0 - rc) / 3.0 - rs / 3.0f64.sqrt();
                let rb = (1.0 - rc) / 3.0 + rs / 3.0f64.sqrt();
                let strike = p.strike / 100.0;
                let exc = (self.rand() * 2.0 - 1.0)
                    * (-t / (0.0015 + strike * 0.003)).exp()
                    * (0.25 + strike * 0.9);
                let mut reads = [0.0; 3];
                for line in 0..3 {
                    let modulation = (TAU * p.rotation * (0.17 + line as f64 * 0.07) * t
                        + line as f64 * 2.094)
                        .sin()
                        * p.chaos
                        / 100.0
                        * 3.2
                        * sr
                        / 48000.0;
                    reads[line] = read_delay(
                        &mem.delays[line],
                        self.writes[line],
                        delays[line] + modulation,
                    );
                    self.damping[line] +=
                        coef(0.08 + mat * 0.5, sr) * (reads[line] - self.damping[line]);
                }
                for (line, delay) in delays.iter().enumerate() {
                    let next = (line + 1) % 3;
                    let prev = (line + 2) % 3;
                    let circ =
                        self.damping[line] * rd + self.damping[next] * rf + self.damping[prev] * rb;
                    let pol = if line == 2 { -1.0 } else { 1.0 };
                    let input = circ + exc * pol * (1.0 - line as f64 * 0.18);
                    let drive = 1.0 + mat * 0.8;
                    let color = mat * 0.45;
                    let stable = input * (1.0 - color) + (input * drive).tanh() / drive * color;
                    let feedback = (-delay / (sr * dc * 1.35)).exp();
                    mem.delays[line][self.writes[line]] = (stable * feedback) as f32;
                    self.writes[line] = (self.writes[line] + 1) % mem.delays[line].len();
                }
                let angle = TAU * p.rotation * t + side * p.width / 100.0 * PI * 0.7;
                let res = reads[0] * angle.cos()
                    + reads[1] * (angle + 2.094).cos()
                    + reads[2] * (angle + 4.189).cos();
                let avg = (reads[0] + reads[1] + reads[2]) / 3.0;
                let body = avg * 0.8 + res * 1.8 + exc * (-t / 0.006).exp() * 0.25;
                clean_soft(body, mat * 0.08 + tw * 0.05)
            }
            Parameters::Abyss(p) => {
                let dc = p.common.decay as f64;
                let length = p.length / 1000.0;
                let closing = (length * 0.12).clamp(0.04, 0.12);
                let f = p.common.frequency as f64 * 2.0f64.powf(p.drop / 12.0 * (-t / 0.065).exp());
                self.phase[0] += TAU * f / sr;
                self.phase[1] += TAU * f * p.ratio / sr;
                let close_pos = ((length - t) / closing).clamp(0.0, 1.0);
                let amp =
                    (1.0 - (-t / 0.0012).exp()) * (-t / (dc * 0.48)).exp() * smoothstep(close_pos);
                let max_index = p.index / 100.0 * 12.0;
                let idx = max_index * (-t / (0.025 + p.fm_tail / 100.0 * dc * 0.7)).exp();
                let pure = self.phase[0].sin();
                let fm = (self.phase[0] + self.phase[1].sin() * idx).sin();
                let fundamental = p.fundamental / 100.0;
                let mut x = (pure * (0.28 + fundamental * 0.72) + fm * (0.92 - fundamental * 0.52))
                    / (1.2 + max_index * 0.035);
                let fold = p.fold / 100.0;
                if fold > 0.0 {
                    x = x * (1.0 - fold * 0.65) + (x * (1.0 + fold * 7.0)).sin() * fold * 0.65;
                }
                clean_soft(x * amp * 0.9, p.drive / 100.0 * 0.48)
            }
        }
    }
}

pub(super) fn smoothstep(x: f64) -> f64 {
    x * x * (3.0 - 2.0 * x)
}
fn soft(x: f64, d: f64) -> f64 {
    let g = 1.0 + d * 7.0;
    (x * g).tanh() / g.tanh()
}
fn clean_soft(x: f64, d: f64) -> f64 {
    if d <= 0.0 {
        return x;
    }
    let mix = d * 0.72;
    let g = 1.0 + d * 3.5;
    x * (1.0 - mix) + (x * g).tanh() / g.tanh() * mix
}
fn coef(at_48k: f64, sr: f64) -> f64 {
    if sr == 48000.0 {
        at_48k
    } else {
        1.0 - (1.0 - at_48k).powf(48000.0 / sr)
    }
}
fn read_delay(buffer: &[f32], write: usize, delay: f64) -> f64 {
    let pos = (write as f64 - delay).rem_euclid(buffer.len() as f64);
    let a = pos.floor() as usize;
    let b = (a + 1) % buffer.len();
    let frac = pos - a as f64;
    buffer[a] as f64 * (1.0 - frac) + buffer[b] as f64 * frac
}
