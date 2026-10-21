//! [303] Lane inserts: six light effects in series on an instrument lane, one of each,
//! each with its own On switch, in a fixed channel-strip order:
//!
//! **Transient → Filter → EQ → Drive → Bitcrush → Compressor**
//!
//! An insert that is off costs nothing (it is not computed); switching one on or off
//! crossfades over a few milliseconds, so it never clicks. The algorithms are the
//! standard good ones, not shortcuts:
//!
//! - **Filter** and **EQ**: the trapezoidal-integrated state-variable filter (Zavalishin,
//!   "The Art of VA Filter Design"; Simper, "Linear Trap SVF", Cytomic 2013). Unlike a
//!   direct-form biquad it stays stable and quiet while its cutoff moves, and one
//!   structure gives low/high/band/notch and the low shelf, bell and high shelf (Simper's
//!   mixing coefficients).
//! - **Drive**: memoryless shapers (tanh, hard clip, an asymmetric tube-like tanh) with
//!   first-order antiderivative anti-aliasing (Parker, Zavalishin, Le Bivic, "Reducing
//!   the aliasing of nonlinear waveshaping using continuous-time convolution", DAFx 2016):
//!   the harmonics a hard drive makes above Nyquist no longer fold back as inharmonic
//!   whistles. The dry path is delayed by the same half sample, so Mix does not comb.
//! - **Compressor**: log-domain gain computer with a soft knee and the smooth decoupled
//!   peak detector of Giannoulis, Massberg & Reiss, "Digital Dynamic Range Compressor
//!   Design - A Tutorial and Analysis" (JAES 2012); both channels linked; automatic makeup.
//! - **Transient**: two pairs of envelope followers, the transient-designer principle:
//!   fast vs slow ATTACK says "a hit is starting", fast vs slow RELEASE says "a tail is
//!   ringing"; Attack and Sustain turn those differences into gain, in dB, bounded.
//! - **Bitcrush**: mid-tread quantisation to a (continuous) number of bits, and a
//!   sample-and-hold with a fractional phase for the rate (the aliasing is the sound).
//!
//! Real-time: no allocation anywhere (all state is inline), parameters smoothed, filter
//! coefficients recomputed at control rate from smoothed values.

use serde::{Deserialize, Serialize};
use std::f32::consts::PI;

/// The six inserts, in chain order.
pub const INSERT_COUNT: usize = 6;
/// The most knobs an insert has (the EQ: Low, Mid, Mid Freq, High, Low Freq).
pub const INSERT_PARAMS: usize = 5;

pub const TRANSIENT: usize = 0;
pub const FILTER: usize = 1;
pub const EQ: usize = 2;
pub const DRIVE: usize = 3;
pub const BITCRUSH: usize = 4;
pub const COMPRESSOR: usize = 5;

/// One knob of an insert, for the editor.
#[derive(Clone, Copy, Debug)]
pub struct InsertParamDef {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub unit: Option<&'static str>,
    /// Drawn on an exponential slider (frequencies, times).
    pub log: bool,
    /// A menu instead of a slider (the value is the index).
    pub options: Option<&'static [&'static str]>,
    pub help: &'static str,
    /// Where the value is stored, when it is not the row's position (a knob added later
    /// takes the next free slot, so a lane saved before keeps its values).
    pub index: Option<usize>,
}

/// One insert, for the editor.
#[derive(Clone, Copy, Debug)]
pub struct InsertDef {
    pub name: &'static str,
    pub help: &'static str,
    pub params: &'static [InsertParamDef],
}

const fn knob(
    label: &'static str,
    min: f32,
    max: f32,
    default: f32,
    unit: Option<&'static str>,
    log: bool,
    help: &'static str,
) -> InsertParamDef {
    InsertParamDef { label, min, max, default, unit, log, options: None, help, index: None }
}

/// The same knob, stored in value slot `index`.
const fn at(def: InsertParamDef, index: usize) -> InsertParamDef {
    InsertParamDef { index: Some(index), ..def }
}

/// Where row `row` of an insert stores its value.
pub fn value_index(row: usize, def: &InsertParamDef) -> usize {
    def.index.unwrap_or(row)
}

/// The knob stored in value slot `slot` of insert `insert`, if any.
fn knob_at(insert: usize, slot: usize) -> Option<&'static InsertParamDef> {
    INSERTS[insert].params.iter().enumerate().find(|(row, d)| value_index(*row, d) == slot).map(|(_, d)| d)
}

const fn menu(label: &'static str, options: &'static [&'static str], help: &'static str) -> InsertParamDef {
    InsertParamDef {
        label,
        min: 0.0,
        max: (options.len() - 1) as f32,
        default: 0.0,
        unit: None,
        log: false,
        options: Some(options),
        help,
        index: None,
    }
}

pub const INSERTS: [InsertDef; INSERT_COUNT] = [
    InsertDef {
        name: "Transient",
        help: "Transient: reshapes the hit itself (up to 18 dB either way). Attack makes the start of each hit harder (up) or softer (down); Sustain makes the body and tail longer (up) or shorter (down). It follows the sound, whatever its level.",
        params: &[
            knob("Attack", -1.0, 1.0, 0.0, None, false, "Attack: up = a harder, snappier start of each hit; down = a softer one. 0 = untouched."),
            knob("Sustain", -1.0, 1.0, 0.0, None, false, "Sustain: up = a longer, fuller body and tail; down = a shorter, drier one. 0 = untouched."),
        ],
    },
    InsertDef {
        name: "Filter",
        help: "Filter: a resonant filter on the lane (low-pass, high-pass, band-pass or notch). It stays smooth when its cutoff moves.",
        params: &[
            menu("Type", &["Low-pass", "High-pass", "Band-pass", "Notch"], "Type: Low-pass keeps the lows, High-pass keeps the highs, Band-pass keeps a band around the cutoff, Notch removes one."),
            knob("Cutoff", 20.0, 20_000.0, 2_000.0, Some(" Hz"), true, "Cutoff: where the filter acts."),
            knob("Resonance", 0.0, 1.0, 0.2, None, false, "Resonance: a peak at the cutoff. High = a whistling, ringing filter."),
            knob("Mix", 0.0, 1.0, 1.0, None, false, "Mix: how much of the filtered sound replaces the dry one."),
        ],
    },
    InsertDef {
        name: "EQ",
        help: "EQ: three bands. Low is a shelf (everything under Low Freq), Mid is a bell (a band around Mid Freq), High is a shelf (everything over 6 kHz).",
        params: &[
            knob("Low", -15.0, 15.0, 0.0, Some(" dB"), false, "Low: a shelf. Raises or lowers everything under Low Freq."),
            at(
                knob("Low Freq", 30.0, 600.0, 150.0, Some(" Hz"), true, "Low Freq: where the Low shelf starts."),
                4,
            ),
            // (Rows after Low Freq are stored where they always were: slots 1, 2, 3.)
            at(knob("Mid", -15.0, 15.0, 0.0, Some(" dB"), false, "Mid: a bell (a peak). Raises or lowers a band around Mid Freq."), 1),
            at(knob("Mid Freq", 150.0, 8_000.0, 1_000.0, Some(" Hz"), true, "Mid Freq: the centre of the Mid band."), 2),
            at(knob("High", -15.0, 15.0, 0.0, Some(" dB"), false, "High: a shelf. Raises or lowers everything over about 6 kHz."), 3),
        ],
    },
    InsertDef {
        name: "Drive",
        help: "Drive: saturation, from warmth to a hard distortion, with anti-aliasing so a hard drive does not whistle. Its level is compensated: Drive changes the colour more than the volume.",
        params: &[
            menu("Type", &["Soft", "Hard", "Tube"], "Type: Soft = a round saturation; Hard = a clipped, aggressive one; Tube = asymmetric, adds even harmonics (warmer)."),
            knob("Drive", 0.0, 36.0, 12.0, Some(" dB"), false, "Drive: how hard the sound is pushed into the saturation."),
            knob("Tone", -1.0, 1.0, 0.0, None, false, "Tone: after the saturation, down = darker, up = brighter (a gentle tilt)."),
            knob("Mix", 0.0, 1.0, 1.0, None, false, "Mix: how much of the driven sound replaces the dry one (parallel saturation below 1)."),
        ],
    },
    InsertDef {
        name: "Bitcrush",
        help: "Bitcrush: lo-fi. Fewer bits = a grainy, noisy sound; a lower rate = a metallic, aliased one, like an early sampler.",
        params: &[
            knob("Bits", 1.0, 16.0, 8.0, None, false, "Bits: the resolution. 16 = clean, 1 = almost a square wave."),
            knob("Rate", 0.0, 1.0, 0.25, None, false, "Rate: lowers the sample rate. 0 = untouched, 1 = 64 times lower."),
            knob("Mix", 0.0, 1.0, 1.0, None, false, "Mix: how much of the crushed sound replaces the dry one."),
        ],
    },
    InsertDef {
        name: "Compressor",
        help: "Compressor: brings the loud parts down so the lane sounds denser and more even. Its makeup gain is automatic.",
        params: &[
            knob("Threshold", -48.0, 0.0, -18.0, Some(" dB"), false, "Threshold: the level above which the sound is compressed."),
            knob("Ratio", 1.0, 20.0, 4.0, None, true, "Ratio: how hard. 2 = gentle, 4 = clear, 10 and more = a limiter."),
            knob("Attack", 0.1, 100.0, 5.0, Some(" ms"), true, "Attack: how fast it reacts. Slow lets the start of each hit through (more punch)."),
            knob("Release", 10.0, 1_000.0, 120.0, Some(" ms"), true, "Release: how fast it lets go. Short = pumping, long = smooth."),
        ],
    },
];

/// What a lane's inserts are set to: which are on, and their knobs (unused knobs stay at 0).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InsertSettings {
    pub on: [bool; INSERT_COUNT],
    /// Knob values by insert, by value slot (see `value_index`). Read tolerantly: a lane
    /// saved with fewer slots per insert gets the defaults for the missing ones.
    #[serde(deserialize_with = "values_padded")]
    pub values: [[f32; INSERT_PARAMS]; INSERT_COUNT],
    /// The chain, first to last: insert indices (`TRANSIENT`, `FILTER`...).
    pub order: [u8; INSERT_COUNT],
}

/// The default chain order.
pub const DEFAULT_ORDER: [u8; INSERT_COUNT] = [0, 1, 2, 3, 4, 5];

fn default_value(insert: usize, slot: usize) -> f32 {
    knob_at(insert, slot).map_or(0.0, |d| d.default)
}

fn values_padded<'de, D>(deserializer: D) -> Result<[[f32; INSERT_PARAMS]; INSERT_COUNT], D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw: Vec<Vec<f32>> = Vec::deserialize(deserializer)?;
    Ok(std::array::from_fn(|i| {
        std::array::from_fn(|k| raw.get(i).and_then(|v| v.get(k)).copied().unwrap_or_else(|| default_value(i, k)))
    }))
}

impl Default for InsertSettings {
    fn default() -> Self {
        Self {
            on: [false; INSERT_COUNT],
            values: std::array::from_fn(|i| std::array::from_fn(|k| default_value(i, k))),
            order: DEFAULT_ORDER,
        }
    }
}

impl InsertSettings {
    /// Every value finite and inside its knob's range.
    pub fn sanitized(mut self) -> Self {
        for i in 0..INSERT_COUNT {
            for (k, v) in self.values[i].iter_mut().enumerate() {
                *v = match knob_at(i, k) {
                    Some(d) if v.is_finite() => v.clamp(d.min, d.max),
                    Some(d) => d.default,
                    None => 0.0,
                };
            }
        }
        // The order must hold every insert once.
        let mut seen = [false; INSERT_COUNT];
        let valid = self.order.iter().all(|&n| {
            let ok = (n as usize) < INSERT_COUNT && !seen[n as usize];
            if ok {
                seen[n as usize] = true;
            }
            ok
        });
        if !valid {
            self.order = DEFAULT_ORDER;
        }
        self
    }

    pub fn any_on(&self) -> bool {
        self.on.iter().any(|o| *o)
    }
}

/// Samples between two coefficient updates.
const CONTROL_PERIOD: usize = 16;
/// Time constant of the knob smoothing, and of the On/Off crossfade.
const SMOOTH_MS: f32 = 15.0;
const BYPASS_FADE_MS: f32 = 6.0;

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

#[inline]
fn db_to_gain(db: f32) -> f32 {
    10.0f32.powf(db / 20.0)
}

#[inline]
fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1.0e-9).log10()
}

// ------------------------------------------------------------------ the SVF core

/// Simper's linear trapezoidal SVF, one channel. `tick` returns (band, low).
#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

/// The coefficients of an SVF: `g` = tan(pi fc / fs) (possibly pre-scaled for shelves), `k`
/// = 1/Q, and the output mix `out = m0 * v0 + m1 * v1 + m2 * v2`.
#[derive(Clone, Copy, Debug)]
struct SvfCoefs {
    a1: f32,
    a2: f32,
    a3: f32,
    m0: f32,
    m1: f32,
    m2: f32,
}

impl SvfCoefs {
    fn new(g: f32, k: f32, m0: f32, m1: f32, m2: f32) -> Self {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        Self { a1, a2, a3, m0, m1, m2 }
    }

    /// Low-pass, high-pass, band-pass (unity at the centre) or notch.
    fn filter(kind: usize, fc: f32, q: f32, sr: f32) -> Self {
        let g = (PI * (fc / sr).clamp(1.0e-5, 0.49)).tan();
        let k = 1.0 / q;
        match kind {
            1 => Self::new(g, k, 1.0, -k, -1.0),
            2 => Self::new(g, k, 0.0, k, 0.0),
            3 => Self::new(g, k, 1.0, -k, 0.0),
            _ => Self::new(g, k, 0.0, 0.0, 1.0),
        }
    }

    fn low_shelf(fc: f32, db: f32, sr: f32) -> Self {
        let a = 10.0f32.powf(db / 40.0);
        let g = (PI * (fc / sr).clamp(1.0e-5, 0.49)).tan() / a.sqrt();
        let k = std::f32::consts::SQRT_2;
        Self::new(g, k, 1.0, k * (a - 1.0), a * a - 1.0)
    }

    fn high_shelf(fc: f32, db: f32, sr: f32) -> Self {
        let a = 10.0f32.powf(db / 40.0);
        let g = (PI * (fc / sr).clamp(1.0e-5, 0.49)).tan() * a.sqrt();
        let k = std::f32::consts::SQRT_2;
        Self::new(g, k, a * a, k * (1.0 - a) * a, 1.0 - a * a)
    }

    fn bell(fc: f32, db: f32, q: f32, sr: f32) -> Self {
        let a = 10.0f32.powf(db / 40.0);
        let g = (PI * (fc / sr).clamp(1.0e-5, 0.49)).tan();
        let k = 1.0 / (q * a);
        Self::new(g, k, 1.0, k * (a * a - 1.0), 0.0)
    }
}

impl Svf {
    #[inline]
    fn process(&mut self, c: &SvfCoefs, v0: f32) -> f32 {
        let v3 = v0 - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        c.m0 * v0 + c.m1 * v1 + c.m2 * v2
    }
}

// ------------------------------------------------------------------ the inserts

/// A one-pole smoother towards a target.
#[derive(Clone, Copy, Debug)]
struct Smooth {
    value: f32,
}

impl Smooth {
    #[inline]
    fn to(&mut self, target: f32, coef: f32) -> f32 {
        self.value += (target - self.value) * coef;
        self.value
    }
}

#[derive(Clone, Copy, Debug)]
struct EnvFollower {
    value: f32,
}

impl EnvFollower {
    #[inline]
    fn follow(&mut self, x: f32, attack: f32, release: f32) -> f32 {
        let coef = if x > self.value { attack } else { release };
        self.value = flush(x + coef * (self.value - x));
        self.value
    }
}

/// Coefficient of a one-pole follower with time constant `ms`.
fn time_coef(ms: f32, sr: f32) -> f32 {
    if ms <= 0.0 {
        0.0
    } else {
        (-1.0 / (ms * 0.001 * sr)).exp()
    }
}

#[derive(Clone, Copy, Debug)]
struct Transient {
    fast_atk: EnvFollower,
    slow_atk: EnvFollower,
    fast_rel: EnvFollower,
    slow_rel: EnvFollower,
    gain_db: f32,
}

#[derive(Clone, Copy, Debug)]
struct Filter {
    svf: [Svf; 2],
    coefs: SvfCoefs,
    log_fc: Smooth,
    res: Smooth,
    mix: Smooth,
}

#[derive(Clone, Copy, Debug)]
struct Eq3 {
    low: [Svf; 2],
    mid: [Svf; 2],
    high: [Svf; 2],
    c_low: SvfCoefs,
    c_mid: SvfCoefs,
    c_high: SvfCoefs,
    db: [Smooth; 3],
    log_mid: Smooth,
    log_low: Smooth,
}

#[derive(Clone, Copy, Debug)]
struct Drive {
    x1: [f32; 2],
    tone_lp: [f32; 2],
    drive_db: Smooth,
    tone: Smooth,
    mix: Smooth,
}

#[derive(Clone, Copy, Debug)]
struct Crush {
    phase: f32,
    held: [f32; 2],
    bits: Smooth,
    rate: Smooth,
    mix: Smooth,
}

#[derive(Clone, Copy, Debug)]
struct Compressor {
    y1: f32,
    yl: f32,
}

/// The high shelf's corner, and the bell's Q.
const EQ_HIGH_HZ: f32 = 6_000.0;
const EQ_MID_Q: f32 = 0.9;
/// The drive's tilt: corner and range.
const TONE_CORNER_HZ: f32 = 800.0;
const TONE_RANGE_DB: f32 = 6.0;
/// The level a drive's gain compensation is matched at (-12 dBFS).
const DRIVE_REF_LEVEL: f32 = 0.25;
const TUBE_BIAS: f32 = 0.3;
/// Compressor knee width.
const COMP_KNEE_DB: f32 = 6.0;
/// Transient: the bound on the gain, and the follower times.
const TRANSIENT_MAX_DB: f32 = 18.0;
/// How much of the followers' difference (dB) becomes gain at Attack / Sustain = 1.
const TRANSIENT_DEPTH: f32 = 1.0;

/// ln(cosh(x)), stable for any x.
#[inline]
fn ln_cosh(x: f32) -> f32 {
    let a = x.abs();
    a + (-2.0 * a).exp().ln_1p() - std::f32::consts::LN_2
}

/// The drive shapers: the function and its antiderivative.
#[inline]
fn shaper(kind: usize, x: f32) -> f32 {
    match kind {
        1 => x.clamp(-1.0, 1.0),
        2 => (x + TUBE_BIAS).tanh() - TUBE_BIAS.tanh(),
        _ => x.tanh(),
    }
}

#[inline]
fn shaper_antiderivative(kind: usize, x: f32) -> f32 {
    match kind {
        1 => {
            if x.abs() <= 1.0 {
                0.5 * x * x
            } else {
                x.abs() - 0.5
            }
        }
        2 => ln_cosh(x + TUBE_BIAS) - x * TUBE_BIAS.tanh(),
        _ => ln_cosh(x),
    }
}

/// First-order ADAA: the average of the shaper over the segment from the previous input.
#[inline]
fn adaa(kind: usize, x: f32, x1: f32) -> f32 {
    let dx = x - x1;
    if dx.abs() > 1.0e-4 {
        (shaper_antiderivative(kind, x) - shaper_antiderivative(kind, x1)) / dx
    } else {
        shaper(kind, 0.5 * (x + x1))
    }
}

/// One lane's six inserts.
pub struct InsertRack {
    sr: f32,
    target: InsertSettings,
    /// On/Off crossfade of each insert (0 = bypassed, 1 = in).
    fade: [f32; INSERT_COUNT],
    fade_coef: f32,
    smooth_coef: f32,
    counter: usize,
    transient: Transient,
    filter: Filter,
    eq: Eq3,
    drive: Drive,
    crush: Crush,
    comp: Compressor,
    /// Transient follower coefficients.
    t_coefs: [f32; 4],
    tone_coef: f32,
}

impl InsertRack {
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() { sample_rate.clamp(8000.0, 384_000.0) } else { 44_100.0 };
        let target = InsertSettings::default();
        let v = target.values;
        let mut rack = Self {
            sr,
            target,
            fade: [0.0; INSERT_COUNT],
            fade_coef: 1.0 - time_coef(BYPASS_FADE_MS, sr),
            smooth_coef: 1.0 - time_coef(SMOOTH_MS, sr / CONTROL_PERIOD as f32),
            counter: 0,
            transient: Transient {
                fast_atk: EnvFollower { value: 0.0 },
                slow_atk: EnvFollower { value: 0.0 },
                fast_rel: EnvFollower { value: 0.0 },
                slow_rel: EnvFollower { value: 0.0 },
                gain_db: 0.0,
            },
            filter: Filter {
                svf: [Svf::default(); 2],
                coefs: SvfCoefs::filter(0, v[FILTER][1], 0.7, sr),
                log_fc: Smooth { value: v[FILTER][1].log2() },
                res: Smooth { value: v[FILTER][2] },
                mix: Smooth { value: v[FILTER][3] },
            },
            eq: Eq3 {
                low: [Svf::default(); 2],
                mid: [Svf::default(); 2],
                high: [Svf::default(); 2],
                c_low: SvfCoefs::low_shelf(v[EQ][4].max(20.0), 0.0, sr),
                c_mid: SvfCoefs::bell(v[EQ][2], 0.0, EQ_MID_Q, sr),
                c_high: SvfCoefs::high_shelf(EQ_HIGH_HZ, 0.0, sr),
                db: [Smooth { value: 0.0 }; 3],
                log_mid: Smooth { value: v[EQ][2].log2() },
                log_low: Smooth { value: v[EQ][4].max(20.0).log2() },
            },
            drive: Drive {
                x1: [0.0; 2],
                tone_lp: [0.0; 2],
                drive_db: Smooth { value: v[DRIVE][1] },
                tone: Smooth { value: v[DRIVE][2] },
                mix: Smooth { value: v[DRIVE][3] },
            },
            crush: Crush {
                phase: 1.0,
                held: [0.0; 2],
                bits: Smooth { value: v[BITCRUSH][0] },
                rate: Smooth { value: v[BITCRUSH][1] },
                mix: Smooth { value: v[BITCRUSH][2] },
            },
            comp: Compressor { y1: 0.0, yl: 0.0 },
            t_coefs: [
                time_coef(0.1, sr),  // fast attack
                time_coef(15.0, sr), // slow attack
                time_coef(20.0, sr), // fast release
                time_coef(250.0, sr), // slow release
            ],
            tone_coef: 1.0 - time_coef(1000.0 / (2.0 * PI * TONE_CORNER_HZ), sr),
        };
        rack.update_controls(true);
        rack
    }

    pub fn set_settings(&mut self, settings: InsertSettings) {
        self.target = settings.sanitized();
    }

    /// Clear every memory (filters, followers, held samples). No allocation.
    pub fn reset(&mut self) {
        self.filter.svf = [Svf::default(); 2];
        self.eq.low = [Svf::default(); 2];
        self.eq.mid = [Svf::default(); 2];
        self.eq.high = [Svf::default(); 2];
        self.drive.x1 = [0.0; 2];
        self.drive.tone_lp = [0.0; 2];
        self.crush.held = [0.0; 2];
        self.crush.phase = 1.0;
        self.comp = Compressor { y1: 0.0, yl: 0.0 };
        let t = &mut self.transient;
        t.fast_atk.value = 0.0;
        t.slow_atk.value = 0.0;
        t.fast_rel.value = 0.0;
        t.slow_rel.value = 0.0;
        t.gain_db = 0.0;
    }

    /// Is anything in (or still fading out)?
    pub fn is_active(&self) -> bool {
        self.target.any_on() || self.fade.iter().any(|f| *f > 1.0e-4)
    }

    fn update_controls(&mut self, snap: bool) {
        let k = if snap { 1.0 } else { self.smooth_coef };
        let v = self.target.values;
        let sr = self.sr;
        // Filter
        let f = &mut self.filter;
        let log_fc = f.log_fc.to(v[FILTER][1].max(20.0).log2(), k);
        let res = f.res.to(v[FILTER][2], k);
        f.mix.to(v[FILTER][3], k);
        // Resonance 0..1 -> Q 0.5..20 (exponential: the useful part spread evenly).
        let q = 0.5 * 40.0f32.powf(res);
        f.coefs = SvfCoefs::filter(v[FILTER][0].round().clamp(0.0, 3.0) as usize, 2f32.powf(log_fc), q, sr);
        // EQ
        let e = &mut self.eq;
        let low = e.db[0].to(v[EQ][0], k);
        let mid = e.db[1].to(v[EQ][1], k);
        let high = e.db[2].to(v[EQ][3], k);
        let log_mid = e.log_mid.to(v[EQ][2].max(20.0).log2(), k);
        let log_low = e.log_low.to(v[EQ][4].max(20.0).log2(), k);
        e.c_low = SvfCoefs::low_shelf(2f32.powf(log_low), low, sr);
        e.c_mid = SvfCoefs::bell(2f32.powf(log_mid), mid, EQ_MID_Q, sr);
        e.c_high = SvfCoefs::high_shelf(EQ_HIGH_HZ, high, sr);
        // Drive
        let d = &mut self.drive;
        d.drive_db.to(v[DRIVE][1], k);
        d.tone.to(v[DRIVE][2], k);
        d.mix.to(v[DRIVE][3], k);
        // Bitcrush
        let c = &mut self.crush;
        c.bits.to(v[BITCRUSH][0], k);
        c.rate.to(v[BITCRUSH][1], k);
        c.mix.to(v[BITCRUSH][2], k);
    }

    /// One stereo sample through the inserts that are in.
    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if !self.is_active() {
            return (in_l, in_r);
        }
        if self.counter == 0 {
            self.update_controls(false);
        }
        self.counter = (self.counter + 1) % CONTROL_PERIOD;
        let mut x = [
            if in_l.is_finite() { in_l } else { 0.0 },
            if in_r.is_finite() { in_r } else { 0.0 },
        ];
        for pos in 0..INSERT_COUNT {
            let i = self.target.order[pos] as usize;
            let want = if self.target.on[i] { 1.0 } else { 0.0 };
            self.fade[i] += (want - self.fade[i]) * self.fade_coef;
            if (self.fade[i] - want).abs() < 1.0e-5 {
                self.fade[i] = want;
            }
            let fade = self.fade[i];
            if fade <= 0.0 {
                continue;
            }
            let wet = match i {
                TRANSIENT => self.transient(x),
                FILTER => self.filter(x),
                EQ => self.eq(x),
                DRIVE => self.drive(x),
                BITCRUSH => self.crush(x),
                _ => self.compress(x),
            };
            if fade >= 1.0 {
                x = wet;
            } else {
                for ch in 0..2 {
                    x[ch] += (wet[ch] - x[ch]) * fade;
                }
            }
        }
        (x[0], x[1])
    }

    fn transient(&mut self, x: [f32; 2]) -> [f32; 2] {
        let v = self.target.values[TRANSIENT];
        let level = x[0].abs().max(x[1].abs());
        let [fa, sa, fr, sr] = self.t_coefs;
        let t = &mut self.transient;
        // Same release, different attacks: their ratio rises at the start of a hit.
        let fast_a = t.fast_atk.follow(level, fa, sr);
        let slow_a = t.slow_atk.follow(level, sa, sr);
        // Same (instant) attack, different releases: their ratio rises in the tail.
        let fast_r = t.fast_rel.follow(level, fa, fr);
        let slow_r = t.slow_rel.follow(level, fa, sr);
        let gate = (slow_r / 1.0e-4).min(1.0); // nothing on silence
        let attack_db = (gain_to_db(fast_a) - gain_to_db(slow_a)).clamp(0.0, 30.0);
        let sustain_db = (gain_to_db(slow_r) - gain_to_db(fast_r)).clamp(0.0, 30.0);
        let target = ((v[0] * attack_db + v[1] * sustain_db) * TRANSIENT_DEPTH * gate)
            .clamp(-TRANSIENT_MAX_DB, TRANSIENT_MAX_DB);
        // A short smoothing (about 1 ms) so the gain itself does not crackle.
        t.gain_db += (target - t.gain_db) * 0.05;
        let g = db_to_gain(t.gain_db);
        [x[0] * g, x[1] * g]
    }

    fn filter(&mut self, x: [f32; 2]) -> [f32; 2] {
        let f = &mut self.filter;
        let mix = f.mix.value;
        let mut out = x;
        for ch in 0..2 {
            let y = f.svf[ch].process(&f.coefs, x[ch]);
            out[ch] = x[ch] + (y - x[ch]) * mix;
        }
        out
    }

    fn eq(&mut self, x: [f32; 2]) -> [f32; 2] {
        let e = &mut self.eq;
        let mut out = x;
        for ch in 0..2 {
            let y = e.low[ch].process(&e.c_low, x[ch]);
            let y = e.mid[ch].process(&e.c_mid, y);
            out[ch] = e.high[ch].process(&e.c_high, y);
        }
        out
    }

    fn drive(&mut self, x: [f32; 2]) -> [f32; 2] {
        let kind = self.target.values[DRIVE][0].round().clamp(0.0, 2.0) as usize;
        let d = &mut self.drive;
        let gain = db_to_gain(d.drive_db.value);
        // Compensation: a -12 dBFS signal comes out at about the level it went in.
        let comp = DRIVE_REF_LEVEL / shaper(kind, gain * DRIVE_REF_LEVEL).abs().max(1.0e-4);
        let tilt = d.tone.value * TONE_RANGE_DB;
        let (g_low, g_high) = (db_to_gain(-tilt), db_to_gain(tilt));
        let mix = d.mix.value;
        let mut out = x;
        for ch in 0..2 {
            let xin = x[ch] * gain;
            let y = adaa(kind, xin, d.x1[ch]) * comp;
            // The dry path takes the same half-sample delay as the ADAA output.
            let dry = 0.5 * (x[ch] + d.x1[ch] / gain);
            d.x1[ch] = xin;
            d.tone_lp[ch] = flush(d.tone_lp[ch] + (y - d.tone_lp[ch]) * self.tone_coef);
            let toned = d.tone_lp[ch] * g_low + (y - d.tone_lp[ch]) * g_high;
            out[ch] = dry + (toned - dry) * mix;
        }
        out
    }

    fn crush(&mut self, x: [f32; 2]) -> [f32; 2] {
        let c = &mut self.crush;
        // Rate 0..1 -> hold each sample 1..64 samples (exponential).
        let hold = 2f32.powf(c.rate.value * 6.0);
        c.phase += 1.0 / hold;
        let step = 2.0 / 2f32.powf(c.bits.value);
        if c.phase >= 1.0 {
            c.phase -= 1.0;
            for ch in 0..2 {
                c.held[ch] = (x[ch] / step).round() * step;
            }
        }
        let mix = c.mix.value;
        if mix >= 0.9999 {
            return c.held;
        }
        [x[0] + (c.held[0] - x[0]) * mix, x[1] + (c.held[1] - x[1]) * mix]
    }

    fn compress(&mut self, x: [f32; 2]) -> [f32; 2] {
        let v = self.target.values[COMPRESSOR];
        let (threshold, ratio) = (v[0], v[1].max(1.0));
        let (a_atk, a_rel) = (time_coef(v[2], self.sr), time_coef(v[3], self.sr));
        let x_db = gain_to_db(x[0].abs().max(x[1].abs()));
        // Gain computer, soft knee (Giannoulis et al., eq. 4).
        let over = x_db - threshold;
        let y_db = if 2.0 * over < -COMP_KNEE_DB {
            x_db
        } else if 2.0 * over.abs() <= COMP_KNEE_DB {
            x_db + (1.0 / ratio - 1.0) * (over + COMP_KNEE_DB / 2.0).powi(2) / (2.0 * COMP_KNEE_DB)
        } else {
            threshold + over / ratio
        };
        let x_l = x_db - y_db;
        // Smooth decoupled peak detector (eq. 17).
        let c = &mut self.comp;
        c.y1 = x_l.max(a_rel * c.y1 + (1.0 - a_rel) * x_l);
        c.yl = a_atk * c.yl + (1.0 - a_atk) * c.y1;
        // Makeup: half of the reduction a full-scale signal would get.
        let makeup = 0.5 * (threshold - threshold / ratio).abs();
        let g = db_to_gain(makeup - c.yl);
        [x[0] * g, x[1] * g]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn rack(on: usize, values: &[(usize, f32)]) -> InsertRack {
        let mut s = InsertSettings::default();
        s.on[on] = true;
        for (p, v) in values {
            s.values[on][*p] = *v;
        }
        let mut r = InsertRack::new(SR);
        r.set_settings(s);
        // Let the smoothing settle.
        r.update_controls(true);
        r.fade[on] = 1.0;
        r
    }

    /// Steady-state gain (dB) of a sine at `hz` through the rack.
    fn response_db(r: &mut InsertRack, hz: f32) -> f32 {
        let n = (SR * 0.5) as usize;
        let mut e_in = 0.0f64;
        let mut e_out = 0.0f64;
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f32 / SR).sin() * 0.1;
            let (y, _) = r.process(x, x);
            if i > n / 2 {
                e_in += (x * x) as f64;
                e_out += (y * y) as f64;
            }
        }
        10.0 * (e_out / e_in).log10() as f32
    }

    #[test]
    fn everything_off_is_a_wire() {
        let mut r = InsertRack::new(SR);
        for i in 0..1000 {
            let x = (i as f32 * 0.37).sin();
            assert_eq!(r.process(x, -x), (x, -x));
        }
    }

    #[test]
    fn the_low_pass_is_flat_below_and_falls_12_db_per_octave_above() {
        let mut r = rack(FILTER, &[(0, 0.0), (1, 1000.0), (2, 0.0), (3, 1.0)]);
        // Resonance 0 -> Q 0.5 (no peak): -6 dB at the cutoff, steady 12 dB/oct beyond.
        let pass = response_db(&mut r, 100.0);
        let far = response_db(&mut r, 8000.0);
        println!("LP 1 kHz: 100 Hz {pass:.2} dB, 8 kHz {far:.2} dB");
        assert!(pass.abs() < 0.5, "pass band {pass}");
        assert!(far < -32.0 && far > -45.0, "three octaves above: {far}");
    }

    #[test]
    fn the_high_pass_band_pass_and_notch_do_their_job() {
        let mut hp = rack(FILTER, &[(0, 1.0), (1, 1000.0), (2, 0.0), (3, 1.0)]);
        assert!(response_db(&mut hp, 8000.0).abs() < 0.5);
        assert!(response_db(&mut hp, 125.0) < -32.0);
        let mut bp = rack(FILTER, &[(0, 2.0), (1, 1000.0), (2, 0.3), (3, 1.0)]);
        let centre = response_db(&mut bp, 1000.0);
        assert!(centre.abs() < 0.5, "band-pass is unity at its centre ({centre})");
        assert!(response_db(&mut bp, 8000.0) < -12.0);
        let mut notch = rack(FILTER, &[(0, 3.0), (1, 1000.0), (2, 0.3), (3, 1.0)]);
        let hole = response_db(&mut notch, 1000.0);
        assert!(hole < -30.0, "the notch must be deep ({hole})");
        assert!(response_db(&mut notch, 100.0).abs() < 0.5);
    }

    #[test]
    fn resonance_makes_a_peak_at_the_cutoff() {
        let mut flat = rack(FILTER, &[(0, 0.0), (1, 1000.0), (2, 0.0), (3, 1.0)]);
        let mut peaky = rack(FILTER, &[(0, 0.0), (1, 1000.0), (2, 0.8), (3, 1.0)]);
        let (a, b) = (response_db(&mut flat, 1000.0), response_db(&mut peaky, 1000.0));
        println!("at the cutoff: Q 0.5 {a:.1} dB, resonance 0.8 {b:.1} dB");
        assert!(b > a + 15.0);
    }

    /// The SVF's whole point: its cutoff can sweep fast (a p-lock jump, an LFO) without
    /// blowing up or clicking.
    #[test]
    fn the_filter_is_stable_while_its_cutoff_sweeps() {
        let mut r = rack(FILTER, &[(0, 0.0), (1, 1000.0), (2, 0.9), (3, 1.0)]);
        let mut s = r.target;
        let mut peak = 0.0f32;
        for i in 0..(SR as usize * 2) {
            if i % 64 == 0 {
                s.values[FILTER][1] = if (i / 2048) % 2 == 0 { 20.0 } else { 18_000.0 };
                r.set_settings(s);
            }
            let x = ((i as f32 * 0.9).sin() + (i as f32 * 0.013).sin()) * 0.3;
            let (y, z) = r.process(x, x);
            assert!(y.is_finite() && z.is_finite());
            peak = peak.max(y.abs());
        }
        assert!(peak < 20.0, "bounded ({peak})");
    }

    #[test]
    fn the_eq_bands_land_where_they_say() {
        let mut low = rack(EQ, &[(0, 6.0), (1, 0.0), (2, 1000.0), (3, 0.0)]);
        let (l_in, l_out) = (response_db(&mut low, 30.0), response_db(&mut low, 10_000.0));
        let mut mid = rack(EQ, &[(0, 0.0), (1, 6.0), (2, 1000.0), (3, 0.0)]);
        let (m_in, m_out) = (response_db(&mut mid, 1000.0), response_db(&mut mid, 8000.0));
        let mut high = rack(EQ, &[(0, 0.0), (1, 0.0), (2, 1000.0), (3, -6.0)]);
        let (h_in, h_out) = (response_db(&mut high, 15_000.0), response_db(&mut high, 100.0));
        println!("low +6: {l_in:.2}/{l_out:.2}, mid +6: {m_in:.2}/{m_out:.2}, high -6: {h_in:.2}/{h_out:.2}");
        assert!((l_in - 6.0).abs() < 0.5 && l_out.abs() < 0.3);
        assert!((m_in - 6.0).abs() < 0.3 && m_out.abs() < 1.0);
        assert!((h_in + 6.0).abs() < 0.7 && h_out.abs() < 0.3);
        let mut flat = rack(EQ, &[(0, 0.0), (1, 0.0), (2, 1000.0), (3, 0.0)]);
        for hz in [50.0, 500.0, 5000.0, 15_000.0] {
            assert!(response_db(&mut flat, hz).abs() < 0.05, "all at 0 dB is flat");
        }
    }

    /// The anti-aliasing works: a 5 kHz sine through a hard drive leaves much less energy
    /// on non-harmonic frequencies (the folded-back aliases) than a naive clipper.
    #[test]
    fn the_drive_anti_aliasing_reduces_the_aliases() {
        const N: usize = 4096;
        let f0 = 5000.0f32;
        let gain = db_to_gain(30.0);
        let input: Vec<f32> = (0..N + 1000).map(|i| (2.0 * PI * f0 * i as f32 / SR).sin() * 0.5).collect();
        let naive: Vec<f32> = input.iter().map(|x| shaper(1, x * gain)).collect();
        let mut x1 = 0.0;
        let anti: Vec<f32> = input
            .iter()
            .map(|x| {
                let y = adaa(1, x * gain, x1);
                x1 = x * gain;
                y
            })
            .collect();
        let alias_share = |sig: &[f32]| {
            let seg = &sig[1000..1000 + N];
            let (mut alias, mut total) = (0.0f64, 0.0f64);
            for k in 1..N / 2 {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (n, s) in seg.iter().enumerate() {
                    let w = 0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N as f64).cos();
                    let a = 2.0 * std::f64::consts::PI * k as f64 * n as f64 / N as f64;
                    re += *s as f64 * w * a.cos();
                    im -= *s as f64 * w * a.sin();
                }
                let p = re * re + im * im;
                total += p;
                let hz = k as f32 * SR / N as f32;
                let harmonic = (hz / f0).round();
                if (hz - harmonic * f0).abs() > 4.0 * SR / N as f32 {
                    alias += p;
                }
            }
            10.0 * (alias / total).log10()
        };
        let (a_naive, a_anti) = (alias_share(&naive), alias_share(&anti));
        println!("alias energy: naive {a_naive:.1} dB, ADAA {a_anti:.1} dB");
        assert!(a_anti < a_naive - 6.0, "ADAA must cut the aliases by 6 dB or more");
    }

    #[test]
    fn the_drive_keeps_its_level_and_adds_harmonics() {
        let mut r = rack(DRIVE, &[(0, 0.0), (1, 24.0), (2, 0.0), (3, 1.0)]);
        let level = response_db(&mut r, 200.0);
        println!("soft drive +24 dB, -20 dBFS sine: {level:.1} dB");
        assert!(level.abs() < 9.0, "Drive changes the colour more than the level ({level})");
    }

    #[test]
    fn the_compressor_follows_its_static_curve() {
        // -6 dBFS sine (peak), threshold -18, ratio 4: 12 dB over -> 9 dB of reduction,
        // plus the makeup of half of what full scale gets (0.5 * 13.5 = 6.75 dB).
        let mut r = rack(COMPRESSOR, &[(0, -18.0), (1, 4.0), (2, 1.0), (3, 50.0)]);
        let n = SR as usize;
        let (mut peak_in, mut peak_out) = (0.0f32, 0.0f32);
        for i in 0..n {
            let x = (2.0 * PI * 100.0 * i as f32 / SR).sin() * db_to_gain(-6.0);
            let (y, _) = r.process(x, x);
            if i > n / 2 {
                peak_in = peak_in.max(x.abs());
                peak_out = peak_out.max(y.abs());
            }
        }
        let change = gain_to_db(peak_out) - gain_to_db(peak_in);
        println!("compressor: {change:.2} dB (expected -9 + 6.75 = -2.25)");
        assert!((change + 2.25).abs() < 1.0, "{change}");
    }

    #[test]
    fn under_the_threshold_the_compressor_only_adds_its_makeup() {
        let mut r = rack(COMPRESSOR, &[(0, -18.0), (1, 4.0), (2, 1.0), (3, 50.0)]);
        let level = response_db(&mut r, 100.0); // a 0.1 sine is -20 dBFS: under -18 - knee/2
        assert!((level - 6.75).abs() < 0.5, "{level}");
    }

    /// The transient designer reshapes a hit: Attack up lifts the first milliseconds,
    /// Sustain down shortens the tail.
    #[test]
    fn the_transient_shaper_moves_the_attack_and_the_tail() {
        let hit = |i: usize| {
            let t = i as f32 / SR;
            (2.0 * PI * 120.0 * t).sin() * (-t / 0.15).exp() * 0.5
        };
        let measure = |attack: f32, sustain: f32| {
            let mut r = rack(TRANSIENT, &[(0, attack), (1, sustain)]);
            let (mut head, mut tail) = (0.0f32, 0.0f32);
            for i in 0..(SR * 0.6) as usize {
                let (y, _) = r.process(hit(i), hit(i));
                if i < (0.006 * SR) as usize {
                    head = head.max(y.abs());
                } else if i > (0.2 * SR) as usize {
                    tail += y * y;
                }
            }
            (head, tail)
        };
        let (head0, tail0) = measure(0.0, 0.0);
        let (head_up, _) = measure(1.0, 0.0);
        let (_, tail_down) = measure(0.0, -1.0);
        let (_, tail_up) = measure(0.0, 1.0);
        println!("head {head0:.3} -> {head_up:.3}; tail {tail0:.4} -> down {tail_down:.4}, up {tail_up:.4}");
        assert!(head_up > head0 * 1.3, "Attack up must lift the start of the hit");
        assert!(tail_down < tail0 * 0.6, "Sustain down must shorten the tail");
        assert!(tail_up > tail0 * 1.5, "Sustain up must lengthen it");
    }

    #[test]
    fn the_bitcrush_quantises_and_holds() {
        let mut r = rack(BITCRUSH, &[(0, 2.0), (1, 1.0), (2, 1.0)]);
        let mut runs = 0;
        let mut last = f32::NAN;
        for i in 0..6400 {
            let x = (i as f32 * 0.01).sin() * 0.9;
            let (y, _) = r.process(x, x);
            // 2 bits: steps of 0.5.
            assert!(((y / 0.5).round() * 0.5 - y).abs() < 1e-6, "{y}");
            if y != last {
                runs += 1;
                last = y;
            }
        }
        assert!(runs <= 6400 / 64 + 2, "held for 64 samples at Rate 1 ({runs} changes)");
    }

    /// Switching an insert on or off crossfades: no step in the output.
    #[test]
    fn switching_an_insert_does_not_click() {
        let mut r = InsertRack::new(SR);
        let mut s = InsertSettings::default();
        s.values[FILTER] = [0.0, 200.0, 0.0, 1.0, 0.0];
        let mut prev = 0.0f32;
        let mut worst = 0.0f32;
        for i in 0..(SR as usize) {
            if i == 10_000 {
                s.on[FILTER] = true;
                r.set_settings(s);
            }
            if i == 30_000 {
                s.on[FILTER] = false;
                r.set_settings(s);
            }
            let x = (2.0 * PI * 3000.0 * i as f32 / SR).sin() * 0.5;
            let (y, _) = r.process(x, x);
            if i > 1 {
                worst = worst.max((y - prev).abs());
            }
            prev = y;
        }
        // A 3 kHz sine at 0.5 moves by at most 0.21 per sample; a click would jump more.
        assert!(worst < 0.25, "{worst}");
    }

    #[test]
    fn everything_on_stays_finite_and_does_not_allocate() {
        let mut s = InsertSettings::default();
        s.on = [true; INSERT_COUNT];
        s.values[DRIVE][1] = 36.0;
        s.values[FILTER][2] = 1.0;
        let mut r = InsertRack::new(SR);
        r.set_settings(s);
        // (The checks run outside: a failing assert would allocate its message.)
        let mut worst = 0.0f32;
        let mut finite = true;
        assert_no_alloc::assert_no_alloc(|| {
            for i in 0..20_000 {
                let x = if i % 5000 < 300 { 1.0 } else { 0.0 };
                let (a, b) = r.process(x, -x);
                finite &= a.is_finite() && b.is_finite();
                worst = worst.max(a.abs()).max(b.abs());
            }
        });
        assert!(finite && worst < 1000.0, "{worst}");
        let garbage = InsertSettings {
            on: [true; INSERT_COUNT],
            values: [[f32::NAN; INSERT_PARAMS]; INSERT_COUNT],
            order: [9; INSERT_COUNT],
        };
        r.set_settings(garbage);
        for _ in 0..1000 {
            let (a, b) = r.process(0.3, 0.3);
            assert!(a.is_finite() && b.is_finite());
        }
    }

    #[test]
    fn the_low_band_moves_with_low_freq() {
        // +6 dB shelf at 400 Hz: 100 Hz is lifted; at 150 Hz (the old fixed corner) far less so.
        let mut r = rack(EQ, &[(0, 6.0), (4, 400.0)]);
        let at_100 = response_db(&mut r, 100.0);
        let at_2k = response_db(&mut r, 2000.0);
        let mut r150 = rack(EQ, &[(0, 6.0), (4, 60.0)]);
        let at_100_low = response_db(&mut r150, 100.0);
        println!("low shelf +6: corner 400 Hz -> 100 Hz {at_100:.2}, 2 kHz {at_2k:.2}; corner 60 Hz -> 100 Hz {at_100_low:.2}");
        assert!(at_100 > 5.0 && at_2k.abs() < 0.5);
        assert!(at_100_low < 3.5, "a lower corner lifts 100 Hz less");
    }

    /// The order is the lane's: Drive then Filter is not Filter then Drive.
    #[test]
    fn the_chain_follows_the_lane_order() {
        let run = |order: [u8; INSERT_COUNT]| {
            let mut s = InsertSettings::default();
            s.on[FILTER] = true;
            s.on[DRIVE] = true;
            s.values[FILTER] = [0.0, 300.0, 0.0, 1.0, 0.0];
            s.values[DRIVE][1] = 30.0;
            s.order = order;
            let mut r = InsertRack::new(SR);
            r.set_settings(s);
            r.update_controls(true);
            r.fade = [1.0; INSERT_COUNT];
            let mut high = 0.0f32;
            let mut prev = 0.0f32;
            for i in 0..8000 {
                let x = (2.0 * PI * 100.0 * i as f32 / SR).sin() * 0.5;
                let (y, _) = r.process(x, x);
                if i > 2000 {
                    high += (y - prev) * (y - prev);
                }
                prev = y;
            }
            high
        };
        let filter_last = run([0, 2, 3, 1, 4, 5]); // Drive, then Filter: the harmonics are filtered
        let drive_last = run(DEFAULT_ORDER); // Filter, then Drive: the harmonics come after
        println!("high energy: filter last {filter_last:.4}, drive last {drive_last:.4}");
        assert!(drive_last > 3.0 * filter_last);
    }

    #[test]
    fn a_broken_order_falls_back_to_the_default() {
        let s = InsertSettings { order: [0, 0, 1, 2, 3, 4], ..InsertSettings::default() }.sanitized();
        assert_eq!(s.order, DEFAULT_ORDER);
        let s = InsertSettings { order: [5, 4, 3, 2, 1, 0], ..InsertSettings::default() }.sanitized();
        assert_eq!(s.order, [5, 4, 3, 2, 1, 0]);
    }

    /// A lane saved by the first inserts build (4 values per insert, no order) still loads.
    #[test]
    fn settings_saved_with_four_values_and_no_order_load() {
        let json = r#"{"on":[false,true,false,false,false,false],"values":[[0,0,0,0],[1,440,0.3,1],[2,1,1000,3],[0,12,0,1],[8,0.25,1,0],[-18,4,5,120]]}"#;
        let s: InsertSettings = serde_json::from_str(json).unwrap();
        assert!(s.on[FILTER]);
        assert_eq!(s.values[FILTER][1], 440.0);
        assert_eq!(s.values[EQ][0], 2.0);
        assert_eq!(s.values[EQ][4], 150.0, "the new Low Freq gets its default");
        assert_eq!(s.order, DEFAULT_ORDER);
    }

    /// Every insert's rows land in distinct value slots, inside the storage.
    #[test]
    fn every_knob_has_its_own_value_slot() {
        for (n, def) in INSERTS.iter().enumerate() {
            let mut used = [false; INSERT_PARAMS];
            for (row, p) in def.params.iter().enumerate() {
                let k = value_index(row, p);
                assert!(k < INSERT_PARAMS, "{} row {row}", def.name);
                assert!(!used[k], "{} slot {k} used twice", def.name);
                used[k] = true;
                assert_eq!(knob_at(n, k).map(|d| d.label), Some(p.label));
            }
        }
    }
}
