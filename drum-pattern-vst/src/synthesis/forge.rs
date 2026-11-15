//! Nine Percussion Forge instruments. No rendering, allocation or freeing of
//! buffers at trigger/kind-change time: each slot lends its preallocated memory.
mod core;
#[cfg(test)]
mod tests;

use super::{dsp, DrumVoice, Voice, VoiceSettings};
use core::{Channel, Parameters};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ForgeKind {
    IonBloom,
    Ferroskin,
    PrismHat,
    OrbitClay,
    NeonTom,
    ArcZap,
    TidalAlloy,
    MobiusSkin,
    AbyssFm,
}
impl ForgeKind {
    #[cfg(test)]
    pub const ALL: [Self; 9] = [
        Self::IonBloom,
        Self::Ferroskin,
        Self::PrismHat,
        Self::OrbitClay,
        Self::NeonTom,
        Self::ArcZap,
        Self::TidalAlloy,
        Self::MobiusSkin,
        Self::AbyssFm,
    ];
    pub fn voice(self) -> DrumVoice {
        match self {
            Self::IonBloom => DrumVoice::IonBloom,
            Self::Ferroskin => DrumVoice::Ferroskin,
            Self::PrismHat => DrumVoice::PrismHat,
            Self::OrbitClay => DrumVoice::OrbitClay,
            Self::NeonTom => DrumVoice::NeonTom,
            Self::ArcZap => DrumVoice::ArcZap,
            Self::TidalAlloy => DrumVoice::TidalAlloy,
            Self::MobiusSkin => DrumVoice::MobiusSkin,
            Self::AbyssFm => DrumVoice::AbyssFm,
        }
    }
    fn stereo(self) -> bool {
        !matches!(self, Self::ArcZap | Self::AbyssFm)
    }
    /// Calibration from the source's default hit (seed 42), not a future-peak
    /// scan in the audio callback. Extremes are protected before lane Volume.
    fn gain(self) -> f64 {
        0.88 / match self {
            Self::Ferroskin => 1.6224318742752075,
            Self::PrismHat => 1.0020943880081177,
            Self::OrbitClay => 4.5635762214660645,
            Self::MobiusSkin => 1.2030802965164185,
            _ => 1.0,
        }
    }
    fn widening(self) -> (f64, f64, f64, f64) {
        match self {
            Self::IonBloom => (2.6, 0.7, 0.06, 0.55),
            Self::Ferroskin => (2.2, 1.4, 0.12, 0.35),
            Self::PrismHat => (2.4, 0.9, 0.18, 0.38),
            Self::OrbitClay => (3.2, 1.1, 0.08, 0.32),
            Self::NeonTom => (3.0, 0.8, 0.07, 0.5),
            Self::TidalAlloy => (1.5, 1.8, 0.15, 0.22),
            _ => (2.0, 1.0, 0.1, 0.3),
        }
    }
}

struct ChannelMemory {
    delays: [Vec<f32>; 3],
    chorus: Vec<f32>,
}
impl ChannelMemory {
    fn new(sr: f64) -> Self {
        let delay_len = (sr / 55.0 + sr / 48000.0 * 16.0 + 16.0).ceil() as usize;
        Self {
            delays: std::array::from_fn(|_| vec![0.0; delay_len.max(2)]),
            chorus: vec![0.0; (sr * 0.012).ceil().max(2.0) as usize],
        }
    }
    fn clear(&mut self) {
        for d in &mut self.delays {
            d.fill(0.0);
        }
        self.chorus.fill(0.0);
    }
}
pub(crate) struct ForgeMemory {
    channels: [ChannelMemory; 2],
    decor: Vec<f32>,
    decor_write: usize,
}
impl ForgeMemory {
    pub(super) fn allocated_bytes(&self) -> usize {
        (self
            .channels
            .iter()
            .map(|c| c.delays.iter().map(|d| d.capacity()).sum::<usize>() + c.chorus.capacity())
            .sum::<usize>()
            + self.decor.capacity())
            * std::mem::size_of::<f32>()
    }
    pub fn new(sample_rate: f32) -> Self {
        let sr = sample_rate as f64;
        Self {
            channels: std::array::from_fn(|_| ChannelMemory::new(sr)),
            decor: vec![0.0; (sr * 0.002).ceil().max(2.0) as usize + 1],
            decor_write: 0,
        }
    }
    fn clear(&mut self) {
        for c in &mut self.channels {
            c.clear();
        }
        self.decor.fill(0.0);
        self.decor_write = 0;
    }
    fn widen(&mut self, kind: ForgeKind, l: f64, r: f64, sr: f64, low: &mut f64) -> [f64; 2] {
        let (side_gain, delay_ms, low_coef, decor_amount) = kind.widening();
        let mid = (l + r) * 0.5;
        let side = (l - r) * 0.5 * side_gain;
        let a = if sr == 48000.0 {
            low_coef
        } else {
            1.0 - (1.0 - low_coef).powf(48000.0 / sr)
        };
        *low += a * (mid - *low);
        let high = mid - *low;
        let write = self.decor_write;
        self.decor[write] = high as f32;
        let delay = (sr * delay_ms / 1000.0).round().max(1.0) as usize;
        let read = (write + self.decor.len() - delay.min(self.decor.len() - 1)) % self.decor.len();
        let decor = (high - self.decor[read] as f64) * decor_amount;
        self.decor_write = (write + 1) % self.decor.len();
        [mid + side + decor, mid - side - decor]
    }
}

pub struct ForgeVoice {
    kind: ForgeKind,
    sample_rate: f64,
    settings: VoiceSettings,
    current: VoiceSettings,
    channels: [Channel; 2],
    pub(crate) memory: Option<Box<ForgeMemory>>,
    position: usize,
    active: bool,
    seed: u32,
    width_low: f64,
    color_low: [f64; 2],
    color_coef: f64,
    smooth: f32,
    retrigger_ramp: usize,
    declick: [dsp::RetrigDeclick; 2],
    last: [f32; 2],
}
impl ForgeVoice {
    pub(crate) fn new(sample_rate: f32, kind: ForgeKind) -> Self {
        let s = VoiceSettings::forge(kind.voice());
        Self {
            kind,
            sample_rate: sample_rate as f64,
            settings: s,
            current: s,
            channels: std::array::from_fn(|i| Channel::new(kind, 1337 + i as u32 * 7919)),
            memory: None,
            position: 0,
            active: false,
            seed: 1337,
            width_low: 0.0,
            color_low: [0.0; 2],
            color_coef: 1.0 - (-std::f64::consts::TAU * 2500.0 / sample_rate as f64).exp(),
            retrigger_ramp: 0,
            smooth: 1.0 - (-1.0 / (sample_rate * 0.005)).exp(),
            declick: [dsp::RetrigDeclick::new(sample_rate); 2],
            last: [0.0; 2],
        }
    }
    fn advance_controls(&mut self) {
        let a = self.smooth;
        for (current, target) in [
            (&mut self.current.frequency, self.settings.frequency),
            (&mut self.current.decay, self.settings.decay),
            (&mut self.current.volume, self.settings.volume),
            (&mut self.current.stereo, self.settings.stereo),
        ] {
            *current += a * (target - *current);
        }
        for p in crate::instrument_registry::INSTRUMENTS[self.kind.voice() as usize].special_params
        {
            let i = p.special_index;
            self.current.special[i] += a * (self.settings.special[i] - self.current.special[i]);
        }
    }
}

/// Duration includes the source's native final fade, which is NOT an AHD release.
fn duration(kind: ForgeKind, s: &VoiceSettings) -> (f64, f64) {
    let base = (s.decay as f64 * 1.45 + 0.12).max(0.18);
    match kind {
        ForgeKind::OrbitClay => {
            let release = (s.special[6] as f64 / 1000.0).max(0.03);
            ((base + release).min(3.5), release)
        }
        ForgeKind::MobiusSkin => (base.min(4.2), 0.022),
        ForgeKind::AbyssFm => ((s.special[1] as f64 / 1000.0).clamp(0.15, 4.2), 0.022),
        _ => (base.min(2.2), 0.022),
    }
}
fn fade(kind: ForgeKind, pos: usize, n: usize, release: f64, sr: f64) -> f64 {
    let samples = ((sr * release).floor() as usize).min(n).max(1);
    if pos < n.saturating_sub(samples) {
        1.0
    } else {
        let x = n.saturating_sub(1 + pos) as f64 / samples as f64;
        if kind == ForgeKind::OrbitClay {
            core::smoothstep(x)
        } else {
            x
        }
    }
}

/// [324] The native crack alone changed the first 12 ms by only ~1 dB in
/// production, where the full-level noise masks it. Give Impact an actual
/// soft-to-accented onset, around the unchanged source default of 52%.
/// This short window is exactly unity after 12 ms: never turn it into Volume.
fn ferroskin_impact_gain(impact: f64, time: f64) -> f64 {
    let delta = if impact <= 52.0 { (impact - 52.0) / 52.0 } else { (impact - 52.0) / 48.0 };
    let remaining = (1.0 - time / 0.012).clamp(0.0, 1.0);
    1.0 + delta.clamp(-1.0, 1.0) * core::smoothstep(remaining)
}

/// [325] Keep the original 315.2 ms time constant at 42%, but make the ends
/// clearly slow/fast (2 s / 12 ms, instead of 500 / 60 ms). This remains the
/// pitched bell's sweep, NOT an unrelated volume or whole-cymbal pitch knob.
fn tidal_drop_time(speed: f64) -> f64 {
    let speed = speed.clamp(0.0, 100.0);
    let native_default = 0.5 - 0.42 * 0.44;
    if speed <= 42.0 {
        native_default * (2.0f64 / native_default).powf((42.0 - speed) / 42.0)
    } else {
        native_default * (0.012 / native_default).powf((speed - 42.0) / 58.0)
    }
}

fn production_parameters(kind: ForgeKind, settings: VoiceSettings) -> Parameters {
    let mut params = Parameters::new(kind, settings);
    if let Parameters::Tidal(p) = &mut params {
        // Bypass the inverse conversion at the default: bit-identical native
        // equations there. Out-of-source-range values are internal only.
        if p.drop_speed != 42.0 {
            p.drop_speed = (0.5 - tidal_drop_time(p.drop_speed)) / 0.44 * 100.0;
        }
    }
    params
}

/// [325] Whole-hit tilt in addition to the native noise coloration, so Color
/// still works when Density hides/removes the noise. 66% is exactly neutral.
/// Complementary one-pole bands: cuts only, no extra resonance or makeup gain.
fn prism_color(sample: f64, color: f64, low: &mut f64, coef: f64) -> f64 {
    *low += coef * (sample - *low);
    if color == 66.0 {
        return sample;
    }
    let high = sample - *low;
    if color <= 66.0 {
        *low + high * (1.0 - (66.0 - color) / 66.0 * 0.96)
    } else {
        *low * (1.0 - (color - 66.0) / 34.0 * 0.90) + high
    }
}

impl Voice for ForgeVoice {
    fn trigger(&mut self) {
        // Noise can be nonzero on the source's first sample. Fade the NEW hit
        // in while RetrigDeclick continues the old output; first hits stay intact.
        self.retrigger_ramp = if self.active {
            (self.sample_rate * 0.003).round() as usize
        } else {
            0
        };
        for i in 0..2 {
            self.declick[i].arm(self.last[i]);
        }
        self.current = self.settings;
        self.position = 0;
        self.active = true;
        self.width_low = 0.0;
        self.color_low = [0.0; 2];
        self.seed = self.seed.wrapping_add(0x9E3779B9);
        self.channels = std::array::from_fn(|i| {
            Channel::new(self.kind, self.seed.wrapping_add(i as u32 * 7919))
        });
        if let Some(memory) = &mut self.memory {
            memory.clear();
        }
    }
    fn set_hit_index(&mut self, index: u32) {
        self.seed = 1337u32.wrapping_add(index.wrapping_mul(0x85EBCA6B));
    }
    fn process_sample(&mut self) -> f32 {
        let (l, r) = self.process_sample_stereo();
        (l + r) * 0.5
    }
    fn process_sample_stereo(&mut self) -> (f32, f32) {
        if !self.active && !self.declick.iter().any(|d| d.is_active()) {
            return (0.0, 0.0);
        }
        self.advance_controls();
        let (seconds, release) = duration(self.kind, &self.current);
        let n = (seconds * self.sample_rate).floor() as usize;
        let mut out = [0.0; 2];
        if self.active && self.position < n {
            if let Some(mem) = &mut self.memory {
                let p = production_parameters(self.kind, self.current);
                let t = self.position as f64 / self.sample_rate;
                let stereo = self.kind.stereo();
                let width = if stereo {
                    self.current.stereo as f64
                } else {
                    0.0
                };
                let f = fade(self.kind, self.position, n, release, self.sample_rate);
                // The source stores each raw sample in Float32Array before fading.
                let l =
                    (self.channels[0].sample(p, t, self.sample_rate, &mut mem.channels[0], -width)
                        as f32 as f64
                        * f) as f32 as f64;
                let mut pair = [l, l];
                if stereo {
                    let r = (self.channels[1].sample(
                        p,
                        t,
                        self.sample_rate,
                        &mut mem.channels[1],
                        width,
                    ) as f32 as f64
                        * f) as f32 as f64;
                    let wide = mem.widen(self.kind, l, r, self.sample_rate, &mut self.width_low);
                    let extra = fade(
                        ForgeKind::IonBloom,
                        self.position,
                        n,
                        0.022,
                        self.sample_rate,
                    );
                    pair = [
                        l * (1.0 - width) + wide[0] * extra * width,
                        l * (1.0 - width) + wide[1] * extra * width,
                    ];
                }
                // Only an over-range safety ceiling, after native coloration.
                // Volume stays post-coloration and is never normalised away.
                let attack = if self.position < self.retrigger_ramp {
                    core::smoothstep(self.position as f64 / self.retrigger_ramp as f64)
                } else {
                    1.0
                };
                let impact_gain = match p {
                    Parameters::Ferro(p) => ferroskin_impact_gain(p.impact, t),
                    _ => 1.0,
                };
                if let Parameters::Prism(p) = p {
                    for (sample, low) in pair.iter_mut().zip(&mut self.color_low) {
                        *sample = prism_color(*sample, p.color, low, self.color_coef);
                    }
                }
                for (output, sample) in out.iter_mut().zip(pair) {
                    *output = (sample * impact_gain * self.kind.gain()).clamp(-1.0, 1.0) as f32
                        * self.current.volume
                        * attack as f32;
                }
            }
            self.position += 1;
        } else {
            self.active = false;
        }
        for (output, declick) in out.iter_mut().zip(&mut self.declick) {
            *output += declick.next();
        }
        self.last = out;
        (out[0], out[1])
    }
    fn is_active(&self) -> bool {
        self.active || self.declick.iter().any(|d| d.is_active())
    }
    fn reset(&mut self) {
        self.active = false;
        self.last = [0.0; 2];
        self.position = 0;
        self.width_low = 0.0;
        self.color_low = [0.0; 2];
        for d in &mut self.declick {
            d.reset();
        }
        if let Some(m) = &mut self.memory {
            m.clear();
        }
    }
    fn set_settings(&mut self, mut s: VoiceSettings) {
        use crate::instrument_registry::{ParamWidget, INSTRUMENTS};
        let def = &INSTRUMENTS[self.kind.voice() as usize];
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
    }
    fn set_algo(&mut self, _algo: u8) {}
    fn set_special_param(&mut self, index: usize, value: f32) {
        if let Some(v) = self.settings.special.get_mut(index) {
            *v = value;
        }
        self.set_settings(self.settings);
    }
}
