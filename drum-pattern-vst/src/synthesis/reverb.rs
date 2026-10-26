//! [286] Reverb - the send effect that lives on an effect-bus slot (Fx1 / Fx2).
//!
//! Built after Signalsmith's "Let's Write A Reverb" and the FDN literature (Jot; Schlecht):
//! pre-delay, low cut, then a **multi-channel diffuser** (8 channels, 5 steps, each a
//! per-channel delay, a shuffle with polarity flips and a Hadamard mix, every step
//! about twice as long as the one before) that turns one hit into thousands of echoes
//! a second, then eight LONG delay lines mixed by a **Householder** matrix (a gentle
//! mix: a full Hadamard in the loop clusters the eigentones and colours the tail).
//! Each line has its own feedback gain (set from the decay time, so every line rings
//! down at the same rate) and a one-pole damping filter; a slow LFO wobbles the line
//! lengths. The metallic sound of a small reverb is a lack of echo density (diffuser)
//! and of modal density (long lines), not of filtering.
//!
//! Real-time contract: every buffer is allocated in [`ReverbCore::new`], called
//! from `DrumSynthesizer::initialize_with_layout` (non-RT). `process` allocates
//! and locks nothing. Parameters are smoothed at control rate (every
//! `CONTROL_PERIOD` samples) and the delay lengths are slewed per sample, so a
//! knob or a p-lock never clicks.
//!
//! **Types** (the lane's Type menu): Hall, Room and Plate are the SAME engine with
//! three characters (`Character`): the length of the lines, how much the input is
//! diffused, how bright the tail stays, how much the lines wobble, and whether the
//! first reflections of a room are heard. **Gate** cuts the tail a fixed time after
//! each hit, the gated-snare sound, on any type.
//!
//! **Warp** ([294]) links Decay to the length of the delays (the lines and the diffuser):
//! a long Decay stretches them (up to 4 times), a short one shrinks them (down to a
//! quarter). Moving Decay then moves the delays under the ringing tail, which bends its
//! pitch, as Size does.
//!
//! The effect is NOT a `Voice`: a voice makes sound on a trigger from nothing,
//! a reverb makes it from a bus input. [`ReverbVoice`] is the do-nothing voice
//! the effect slot carries so the generic per-slot machinery (settings push,
//! p-locks, presets) keeps working; the DSP lives in the synthesizer.

use super::dsp::Biquad;
use super::{Voice, VoiceSettings};
use std::f32::consts::TAU;

const LINES: usize = 8;
/// Line lengths in ms at Size = 1: mutually "incommensurable" so the echoes of
/// the lines do not pile up on the same samples.
const BASE_MS: [f32; LINES] = [53.1, 66.7, 79.3, 91.9, 103.7, 117.1, 130.3, 143.9];
/// Diffuser: steps, and the longest delay of each at `diff_scale` 1 (each step twice the
/// one before, so the onset is smooth and the tail has no bump in the middle).
const DIFF_STEPS: usize = 5;
const DIFF_MAX_MS: [f32; DIFF_STEPS] = [5.0, 10.0, 20.0, 40.0, 80.0];
/// Where in its eighth of the range each channel's delay falls (irregular on purpose:
/// regular delays would make the diffuser itself comb-like).
const DIFF_FRAC: [[f32; LINES]; DIFF_STEPS] = [
    [0.31, 0.77, 0.12, 0.58, 0.93, 0.24, 0.66, 0.45],
    [0.83, 0.19, 0.52, 0.07, 0.71, 0.38, 0.96, 0.27],
    [0.14, 0.63, 0.88, 0.35, 0.49, 0.81, 0.22, 0.74],
    [0.57, 0.09, 0.41, 0.92, 0.18, 0.69, 0.33, 0.85],
    [0.26, 0.74, 0.05, 0.62, 0.89, 0.15, 0.47, 0.98],
];
/// A different shuffle for each step, and a polarity flip on some channels.
const DIFF_SHUFFLE: [[usize; LINES]; DIFF_STEPS] = [
    [3, 6, 0, 5, 7, 2, 4, 1],
    [5, 2, 7, 0, 3, 6, 1, 4],
    [1, 4, 6, 3, 0, 7, 5, 2],
    [6, 0, 3, 7, 2, 5, 1, 4],
    [4, 7, 1, 2, 6, 0, 5, 3],
];
const DIFF_SIGN: [[f32; LINES]; DIFF_STEPS] = [
    [1.0, -1.0, 1.0, 1.0, -1.0, -1.0, 1.0, -1.0],
    [-1.0, 1.0, 1.0, -1.0, 1.0, -1.0, -1.0, 1.0],
    [1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, -1.0],
    [-1.0, -1.0, 1.0, -1.0, 1.0, 1.0, -1.0, 1.0],
    [1.0, -1.0, -1.0, 1.0, 1.0, -1.0, 1.0, -1.0],
];
const MAX_PREDELAY_MS: f32 = 250.0;
/// Longest early-reflection tap, in ms: the pre-delay line keeps that much more.
const MAX_EARLY_MS: f32 = 45.0;
/// Gate: the longest hold, the input level that opens it, and how fast it opens and closes.
pub const MAX_GATE_MS: f32 = 1500.0;
const GATE_THRESHOLD: f32 = 0.01;
const GATE_OPEN_MS: f32 = 1.0;
/// Gate release range, in ms.
pub const MIN_GATE_RELEASE_MS: f32 = 2.0;
pub const MAX_GATE_RELEASE_MS: f32 = 500.0;
/// Warp: the Decay at which the delays keep their length, and the Decay range it maps
/// onto two octaves of delay length each way.
const WARP_REF_DECAY_S: f32 = 2.5;
const MIN_DECAY_S: f32 = 0.2;
const MAX_DECAY_S: f32 = 12.0;
/// The most Warp stretches a delay (sizes the line and diffuser buffers).
const MAX_WARP: f32 = 4.0;
/// The longest wobble any type asks for (sizes the line buffers).
const MOD_DEPTH_MS: f32 = 3.0;
/// Samples between two control-rate updates.
const CONTROL_PERIOD: usize = 32;
/// Gain on the wet signal: tuned so a bus fed at unity returns about unity.
const WET_TRIM: f32 = 0.7;

/// `special[]` indices of the reverb (index 4 and 31 are reserved by the
/// p-lock layout, see `param_id.rs`).
pub mod sp {
    pub const SIZE: usize = 0;
    pub const DECAY: usize = 1;
    pub const DAMPING: usize = 2;
    pub const PREDELAY: usize = 3;
    pub const LOW_CUT: usize = 5;
    pub const HIGH_CUT: usize = 9;
    pub const WIDTH: usize = 6;
    /// Not a DSP knob: the lane's p-lock mode (0 = step, 1 = glide), read by the
    /// plugin when it evaluates the lane's locks (`fx_locks.rs`).
    pub const GLIDE: usize = 7;
    /// 1 = the lane does NOT restart from its sliders at the loop point (the locks
    /// form a circle). Stored inverted so that every older lane, which holds 0 here,
    /// keeps restarting.
    pub const NO_RESTART: usize = 8;
    /// The lane's type: 0 = Hall (the original sound), 1 = Room, 2 = Plate.
    pub const TYPE: usize = 10;
    /// Gate hold time in ms (used when the gate is on).
    pub const GATE: usize = 11;
    /// 1 = the gate is on (a lane setting, never locked).
    pub const GATE_ON: usize = 12;
    /// How fast the gate closes, in ms.
    pub const GATE_RELEASE: usize = 13;
    /// 1 = Decay also sets the length of the lines (Warp, a lane setting, never locked).
    pub const WARP: usize = 14;
}

/// The three characters of the reverb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReverbKind {
    /// The original sound: a medium-large space.
    Hall,
    /// A small room: short lines, the first reflections up front.
    Room,
    /// A plate: dense from the first instant, bright, no room.
    Plate,
    /// Not in the Type menu: what the Rumble runs on. A hall with NO direct diffuser output,
    /// so nothing of the input comes back for the first ~100 ms: the tail blooms after the
    /// hit instead of sitting on it.
    Rumble,
}

impl ReverbKind {
    pub fn from_value(v: f32) -> Self {
        match v.round() as i32 {
            1 => Self::Room,
            2 => Self::Plate,
            3 => Self::Rumble,
            _ => Self::Hall,
        }
    }

    fn character(self) -> Character {
        match self {
            Self::Hall => Character {
                len_scale: 1.0,
                diff_scale: 1.0,
                bright: 1.0,
                mod_ms: 0.7,
                early_level: 0.0,
                diffuse_out: 0.7,
            },
            Self::Room => Character {
                len_scale: 0.45,
                diff_scale: 0.35,
                bright: 0.85,
                mod_ms: 0.3,
                early_level: 0.55,
                diffuse_out: 0.5,
            },
            Self::Rumble => Character {
                len_scale: 1.0,
                diff_scale: 1.0,
                bright: 1.0,
                mod_ms: 0.7,
                early_level: 0.0,
                diffuse_out: 0.0,
            },
            Self::Plate => Character {
                len_scale: 0.8,
                diff_scale: 0.5,
                bright: 3.0,
                mod_ms: 1.0,
                early_level: 0.0,
                diffuse_out: 0.6,
            },
        }
    }
}

/// What makes a type sound like itself. Everything is smoothed, so switching the
/// type while it plays slides from one character to the other.
#[derive(Clone, Copy, Debug)]
struct Character {
    /// Scales every delay line (a room is small, a hall is big).
    len_scale: f32,
    /// Scales the diffuser's delays (how long the echoes take to thicken).
    diff_scale: f32,
    /// Multiplies the damping cut-off: above 1, the tail stays brighter.
    bright: f32,
    /// Depth of the slow wobble of the lines, in ms.
    mod_ms: f32,
    /// Level of the early reflections (0 = none).
    early_level: f32,
    /// Level of the diffuser's own output in the return: it fills the gap before the
    /// first lines come back with a dense, smooth early field.
    diffuse_out: f32,
}

/// Warp: how much Decay stretches the delays. 1 at `WARP_REF_DECAY_S`, `MAX_WARP` at the
/// longest Decay, its inverse at the shortest; even in octaves of Decay on each side.
fn warp_scale(decay_s: f32) -> f32 {
    let octaves = if decay_s >= WARP_REF_DECAY_S {
        (decay_s / WARP_REF_DECAY_S).log2() / (MAX_DECAY_S / WARP_REF_DECAY_S).log2()
    } else {
        -(WARP_REF_DECAY_S / decay_s).log2() / (WARP_REF_DECAY_S / MIN_DECAY_S).log2()
    };
    MAX_WARP.powf(octaves.clamp(-1.0, 1.0))
}

/// Early reflections of the Room: `(left ms, right ms, gain)`, read from the dry input.
const EARLY_TAPS: [(f32, f32, f32); 5] = [
    (6.3, 8.1, 0.70),
    (13.9, 16.7, 0.55),
    (21.7, 25.3, 0.42),
    (33.1, 37.9, 0.30),
    (41.0, 44.0, 0.22),
];

/// The knobs of the reverb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// Room size, 0.5..2 (scales every delay line).
    pub size: f32,
    /// Time for the tail to fall 60 dB, in seconds.
    pub decay_s: f32,
    /// High-frequency damping of the tail, 0..1.
    pub damping: f32,
    pub predelay_ms: f32,
    pub low_cut_hz: f32,
    /// High cut of the return, in Hz (20 kHz = open).
    pub high_cut_hz: f32,
    /// Stereo width of the return, 0 (mono) .. 1.
    pub width: f32,
    /// Return level (the lane's Volume).
    pub level: f32,
    /// The character of the reverb.
    pub kind: ReverbKind,
    /// The gate cuts the tail `gate_ms` after each hit, when `gate_on`.
    pub gate_on: bool,
    pub gate_ms: f32,
    /// How fast the gate closes, in ms.
    pub gate_release_ms: f32,
    /// Decay also sets the length of the lines.
    pub warp: bool,
    /// Depth of the slow wobble of the lines, in ms. Negative = the type's own (the Type
    /// menu); the Rumble sets it.
    pub mod_override: f32,
    /// Scale of the diffuser's delays (1 = the type's own at its widest, small = the input is
    /// barely smeared, so the first repeats stay distinct). Negative = the type's own.
    pub diff_override: f32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            size: 1.0,
            decay_s: 2.5,
            damping: 0.5,
            predelay_ms: 20.0,
            low_cut_hz: 120.0,
            high_cut_hz: 20_000.0,
            width: 1.0,
            level: 1.0,
            kind: ReverbKind::Hall,
            gate_on: false,
            gate_ms: 250.0,
            gate_release_ms: 30.0,
            warp: false,
            mod_override: -1.0,
            diff_override: -1.0,
        }
    }
}

impl ReverbParams {
    fn sanitized(self) -> Self {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        Self {
            size: fin(self.size, d.size).clamp(0.5, 2.0),
            decay_s: fin(self.decay_s, d.decay_s).clamp(MIN_DECAY_S, MAX_DECAY_S),
            damping: fin(self.damping, d.damping).clamp(0.0, 1.0),
            predelay_ms: fin(self.predelay_ms, d.predelay_ms).clamp(0.0, MAX_PREDELAY_MS),
            low_cut_hz: fin(self.low_cut_hz, d.low_cut_hz).clamp(20.0, 1000.0),
            high_cut_hz: fin(self.high_cut_hz, d.high_cut_hz).clamp(20.0, 20_000.0),
            width: fin(self.width, d.width).clamp(0.0, 1.0),
            level: fin(self.level, d.level).clamp(0.0, 2.0),
            kind: self.kind,
            gate_on: self.gate_on,
            gate_ms: fin(self.gate_ms, d.gate_ms).clamp(10.0, MAX_GATE_MS),
            gate_release_ms: fin(self.gate_release_ms, d.gate_release_ms)
                .clamp(MIN_GATE_RELEASE_MS, MAX_GATE_RELEASE_MS),
            warp: self.warp,
            mod_override: fin(self.mod_override, -1.0).clamp(-1.0, MOD_DEPTH_MS),
            diff_override: fin(self.diff_override, -1.0).clamp(-1.0, 1.0),
        }
    }
}

impl From<&VoiceSettings> for ReverbParams {
    fn from(v: &VoiceSettings) -> Self {
        Self {
            size: v.special[sp::SIZE],
            decay_s: v.special[sp::DECAY],
            damping: v.special[sp::DAMPING],
            predelay_ms: v.special[sp::PREDELAY],
            low_cut_hz: v.special[sp::LOW_CUT],
            high_cut_hz: v.special[sp::HIGH_CUT],
            width: v.special[sp::WIDTH],
            level: v.volume,
            kind: ReverbKind::from_value(v.special[sp::TYPE]),
            gate_on: v.special[sp::GATE_ON] >= 0.5,
            gate_ms: v.special[sp::GATE],
            gate_release_ms: v.special[sp::GATE_RELEASE],
            warp: v.special[sp::WARP] >= 0.5,
            mod_override: -1.0,
            diff_override: -1.0,
        }
        .sanitized()
    }
}

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < 1.0e-20 {
        0.0
    } else {
        x
    }
}

/// Read `buf` `delay` samples behind `write` (the slot about to be written),
/// with linear interpolation. `delay` is clamped to `1..len-2`.
#[inline]
fn read_delay(buf: &[f32], write: usize, delay: f32) -> f32 {
    let n = buf.len();
    let d = delay.clamp(1.0, (n - 2) as f32);
    let di = d as usize;
    let frac = d - di as f32;
    let i0 = (write + n - di) % n;
    let i1 = (i0 + n - 1) % n;
    buf[i0] * (1.0 - frac) + buf[i1] * frac
}

/// In-place 8-point fast Walsh-Hadamard transform, scaled to be orthonormal.
#[inline]
fn hadamard8(v: &mut [f32; LINES]) {
    let mut h = 1;
    while h < LINES {
        let mut i = 0;
        while i < LINES {
            for j in i..i + h {
                let a = v[j];
                let b = v[j + h];
                v[j] = a + b;
                v[j + h] = a - b;
            }
            i += h * 2;
        }
        h *= 2;
    }
    let scale = 1.0 / (LINES as f32).sqrt();
    for x in v.iter_mut() {
        *x *= scale;
    }
}

/// In-place Householder reflection: `v - (2/N) * sum(v)`. Orthogonal (lossless) but it
/// mixes the channels less than a Hadamard, which keeps the loop's modes spread.
#[inline]
fn householder8(v: &mut [f32; LINES]) {
    let sum: f32 = v.iter().sum();
    let k = sum * (2.0 / LINES as f32);
    for x in v.iter_mut() {
        *x -= k;
    }
}

/// The reverb engine: one stereo-in / stereo-out instance per effect bus.
pub struct ReverbCore {
    sample_rate: f32,
    target: ReverbParams,
    current: ReverbParams,

    pre: [Box<[f32]>; 2],
    pre_pos: usize,
    pre_delay: f32,

    /// Low cut: a 12 dB/oct high-pass on what enters the reverb AND another on what
    /// comes out, so the lows are gone from the input and from the tail the feedback
    /// builds (24 dB/oct overall).
    hp_in: [Biquad; 2],
    hp_out: [Biquad; 2],
    /// High cut: a 12 dB/oct low-pass on what comes out.
    lp_out: [Biquad; 2],

    /// The multi-channel diffuser: per step, 8 delay lines with a slewed length.
    diff: [[Box<[f32]>; LINES]; DIFF_STEPS],
    diff_pos: [[usize; LINES]; DIFF_STEPS],
    diff_len: [[f32; LINES]; DIFF_STEPS],
    diff_len_target: [[f32; LINES]; DIFF_STEPS],

    lines: [Box<[f32]>; LINES],
    write: [usize; LINES],
    length: [f32; LINES],
    length_target: [f32; LINES],
    lfo_phase: [f32; LINES],
    lfo_inc: [f32; LINES],
    mod_samples: f32,
    /// The character in use, slid towards the one the type asks for.
    character: Character,
    gate_timer: f32,
    gate_gain: f32,
    gate_mix: f32,
    /// 0..1: how much Warp is in (switching it slides the lines, it never jumps).
    warp_mix: f32,
    damp_state: [f32; LINES],
    damp_coef: f32,
    gain: [f32; LINES],

    dc_x1: [f32; 2],
    dc_y1: [f32; 2],
    counter: usize,
}

impl ReverbCore {
    /// Allocates every buffer. Never call from the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() {
            sample_rate.clamp(8000.0, 384_000.0)
        } else {
            44_100.0
        };
        let ms = |m: f32| (m * 0.001 * sr).ceil() as usize;
        let line_cap = ms(BASE_MS[LINES - 1] * 2.0 * MAX_WARP + MOD_DEPTH_MS + 2.0) + 8;
        let pre_cap = ms(MAX_PREDELAY_MS + MAX_EARLY_MS) + 8;
        let params = ReverbParams::default();
        let mut core = Self {
            sample_rate: sr,
            target: params,
            current: params,
            pre: [
                vec![0.0; pre_cap].into_boxed_slice(),
                vec![0.0; pre_cap].into_boxed_slice(),
            ],
            pre_pos: 0,
            pre_delay: params.predelay_ms * 0.001 * sr,
            hp_in: [Biquad::new(); 2],
            hp_out: [Biquad::new(); 2],
            lp_out: [Biquad::new(); 2],
            diff: std::array::from_fn(|st| {
                std::array::from_fn(|_| vec![0.0; ms(DIFF_MAX_MS[st] * MAX_WARP) + 8].into_boxed_slice())
            }),
            diff_pos: [[0; LINES]; DIFF_STEPS],
            diff_len: [[1.0; LINES]; DIFF_STEPS],
            diff_len_target: [[1.0; LINES]; DIFF_STEPS],
            lines: std::array::from_fn(|_| vec![0.0; line_cap].into_boxed_slice()),
            write: [0; LINES],
            length: [0.0; LINES],
            length_target: [0.0; LINES],
            lfo_phase: std::array::from_fn(|i| i as f32 * TAU / LINES as f32),
            // Slow, distinct rates (0.11 .. 0.31 Hz).
            lfo_inc: std::array::from_fn(|i| TAU * (0.11 + 0.028 * i as f32) / sr),
            mod_samples: MOD_DEPTH_MS * 0.001 * sr,
            character: ReverbKind::Hall.character(),
            gate_timer: 0.0,
            gate_gain: 1.0,
            gate_mix: 0.0,
            warp_mix: 0.0,
            damp_state: [0.0; LINES],
            damp_coef: 1.0,
            gain: [0.0; LINES],
            dc_x1: [0.0; 2],
            dc_y1: [0.0; 2],
            counter: 0,
        };
        core.update_controls(true);
        core
    }

    pub fn set_params(&mut self, params: ReverbParams) {
        self.target = params.sanitized();
    }

    /// Silence the tail (buffers and filter states). No allocation.
    pub fn reset(&mut self) {
        for b in self.pre.iter_mut() {
            b.fill(0.0);
        }
        for step in self.diff.iter_mut() {
            for b in step.iter_mut() {
                b.fill(0.0);
            }
        }
        self.diff_pos = [[0; LINES]; DIFF_STEPS];
        for l in self.lines.iter_mut() {
            l.fill(0.0);
        }
        self.pre_pos = 0;
        for f in self
            .hp_in
            .iter_mut()
            .chain(self.hp_out.iter_mut())
            .chain(self.lp_out.iter_mut())
        {
            f.reset();
        }
        self.damp_state = [0.0; LINES];
        self.dc_x1 = [0.0; 2];
        self.dc_y1 = [0.0; 2];
        self.write = [0; LINES];
        self.gate_timer = 0.0;
        self.gate_gain = 1.0;
    }

    /// Control-rate update: smooth the knobs towards their targets and derive
    /// the per-line gains and filter coefficients. `snap` jumps straight there
    /// (construction).
    fn update_controls(&mut self, snap: bool) {
        let k = if snap {
            1.0
        } else {
            1.0 - (-(CONTROL_PERIOD as f32) / (self.sample_rate * 0.05)).exp()
        };
        let (c, t) = (&mut self.current, &self.target);
        c.size += (t.size - c.size) * k;
        c.decay_s += (t.decay_s - c.decay_s) * k;
        c.damping += (t.damping - c.damping) * k;
        c.predelay_ms += (t.predelay_ms - c.predelay_ms) * k;
        c.low_cut_hz += (t.low_cut_hz - c.low_cut_hz) * k;
        c.high_cut_hz += (t.high_cut_hz - c.high_cut_hz) * k;
        c.width += (t.width - c.width) * k;
        c.level += (t.level - c.level) * k;
        c.kind = t.kind;
        c.gate_on = t.gate_on;
        c.gate_ms = t.gate_ms;
        c.gate_release_ms = t.gate_release_ms;
        c.warp = t.warp;

        // Slide the character towards the type's (a type change glides, it never jumps).
        let want = t.kind.character();
        let ch = &mut self.character;
        ch.len_scale += (want.len_scale - ch.len_scale) * k;
        let want_diff = if t.diff_override >= 0.0 { t.diff_override } else { want.diff_scale };
        ch.diff_scale += (want_diff - ch.diff_scale) * k;
        ch.bright += (want.bright - ch.bright) * k;
        let want_mod = if t.mod_override >= 0.0 { t.mod_override } else { want.mod_ms };
        ch.mod_ms += (want_mod - ch.mod_ms) * k;
        ch.early_level += (want.early_level - ch.early_level) * k;
        ch.diffuse_out += (want.diffuse_out - ch.diffuse_out) * k;
        if snap {
            *ch = want;
        }
        self.mod_samples = ch.mod_ms * 0.001 * self.sample_rate;
        // The gate is mixed in and out, never switched.
        let gate_target = if c.gate_on { 1.0 } else { 0.0 };
        self.gate_mix += (gate_target - self.gate_mix) * k.max(0.05);
        if snap {
            self.gate_mix = gate_target;
        }

        let warp_target = if c.warp { 1.0 } else { 0.0 };
        self.warp_mix += (warp_target - self.warp_mix) * k;
        if snap {
            self.warp_mix = warp_target;
        }
        let warp = warp_scale(c.decay_s).powf(self.warp_mix);

        let sr = self.sample_rate;
        for i in 0..LINES {
            let seconds = BASE_MS[i] * 0.001 * c.size * ch.len_scale * warp;
            self.length_target[i] = seconds * sr;
            if snap {
                self.length[i] = self.length_target[i];
            }
            // 60 dB of loss over `decay_s`: every line loses the same per second.
            self.gain[i] = 10.0f32.powf(-3.0 * seconds / c.decay_s);
        }
        // Diffuser delays: each channel in its own eighth of the step's range.
        for st in 0..DIFF_STEPS {
            for i in 0..LINES {
                let ms = DIFF_MAX_MS[st] * (i as f32 + DIFF_FRAC[st][i]) / LINES as f32 * ch.diff_scale * warp;
                self.diff_len_target[st][i] = (ms * 0.001 * sr).max(2.0);
                if snap {
                    self.diff_len[st][i] = self.diff_len_target[st][i];
                }
            }
        }
        // Damping: 18 kHz (none) down to ~1.3 kHz (dark).
        let cutoff = 18_000.0 * ch.bright * 0.07f32.powf(c.damping);
        self.damp_coef = 1.0 - (-TAU * cutoff.min(sr * 0.45) / sr).exp();
        let mut hp = Biquad::new();
        hp.set_highpass(c.low_cut_hz, std::f32::consts::FRAC_1_SQRT_2, sr);
        for f in self.hp_in.iter_mut().chain(self.hp_out.iter_mut()) {
            f.copy_coefficients_from(&hp);
        }
        let mut lp = Biquad::new();
        lp.set_lowpass(c.high_cut_hz, std::f32::consts::FRAC_1_SQRT_2, sr);
        for f in self.lp_out.iter_mut() {
            f.copy_coefficients_from(&lp);
        }
        if snap {
            self.pre_delay = c.predelay_ms * 0.001 * sr;
        }
    }

    /// One stereo sample in, one stereo sample of tail out (wet only).
    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if self.counter == 0 {
            self.update_controls(false);
        }
        self.counter += 1;
        if self.counter == CONTROL_PERIOD {
            self.counter = 0;
        }

        // Pre-delay (slewed so changing it does not click).
        let pre_target = self.current.predelay_ms * 0.001 * self.sample_rate;
        self.pre_delay += (pre_target - self.pre_delay) * 0.002;
        let pre_len = self.pre[0].len();
        self.pre[0][self.pre_pos] = in_l;
        self.pre[1][self.pre_pos] = in_r;
        self.pre_pos += 1;
        if self.pre_pos == pre_len {
            self.pre_pos = 0;
        }
        let mut xl = read_delay(&self.pre[0], self.pre_pos, self.pre_delay.max(1.0));
        let mut xr = read_delay(&self.pre[1], self.pre_pos, self.pre_delay.max(1.0));

        // Low cut, on the way in.
        xl = self.hp_in[0].process(xl);
        xr = self.hp_in[1].process(xr);

        // Diffusion: 8 channels (even ones hear left, odd ones right) through the 4 steps.
        let mut v = [0.0f32; LINES];
        for (i, x) in v.iter_mut().enumerate() {
            *x = if i % 2 == 0 { xl } else { xr };
        }
        for st in 0..DIFF_STEPS {
            let mut w = [0.0f32; LINES];
            for i in 0..LINES {
                let buf = &mut self.diff[st][i];
                let pos = self.diff_pos[st][i];
                buf[pos] = v[i];
                let next = if pos + 1 == buf.len() { 0 } else { pos + 1 };
                self.diff_pos[st][i] = next;
                self.diff_len[st][i] += (self.diff_len_target[st][i] - self.diff_len[st][i]) * 0.002;
                w[i] = read_delay(buf, next, self.diff_len[st][i]);
            }
            let mut shuffled = [0.0f32; LINES];
            for i in 0..LINES {
                shuffled[i] = DIFF_SIGN[st][i] * w[DIFF_SHUFFLE[st][i]];
            }
            hadamard8(&mut shuffled);
            v = shuffled;
        }

        // Read the lines (slewed length + slow modulation) and damp them.
        let mut taps = [0.0f32; LINES];
        let mut mixed = [0.0f32; LINES];
        for i in 0..LINES {
            self.length[i] += (self.length_target[i] - self.length[i]) * 0.002;
            let wobble = self.mod_samples * self.lfo_phase[i].sin();
            self.lfo_phase[i] += self.lfo_inc[i];
            if self.lfo_phase[i] > TAU {
                self.lfo_phase[i] -= TAU;
            }
            let tap = read_delay(&self.lines[i], self.write[i], self.length[i] + wobble);
            taps[i] = tap;
            self.damp_state[i] += self.damp_coef * (tap - self.damp_state[i]);
            mixed[i] = self.damp_state[i];
        }
        householder8(&mut mixed);

        // Feed back, and inject the input: even lines hear left, odd lines right.
        for i in 0..LINES {
            let x = v[i] * 0.5 + mixed[i] * self.gain[i];
            let n = self.lines[i].len();
            self.lines[i][self.write[i]] = flush(x);
            self.write[i] += 1;
            if self.write[i] == n {
                self.write[i] = 0;
            }
        }

        // Output: even taps left, odd taps right, then width.
        let (mut l, mut r) = (0.0f32, 0.0f32);
        // (Alternating signs: the two channels are different mixes of the lines.)
        for i in 0..LINES {
            let sign = if (i / 2) % 2 == 0 { 1.0 } else { -1.0 };
            let diffuse = 0.5 * self.character.diffuse_out * v[i];
            if i % 2 == 0 {
                l += sign * (taps[i] + diffuse);
            } else {
                r += sign * (taps[i] + diffuse);
            }
        }
        // The first reflections of a room, taken from the dry input after the pre-delay.
        let early_level = self.character.early_level;
        if early_level > 1.0e-4 {
            let sr = self.sample_rate;
            let (mut el, mut er) = (0.0f32, 0.0f32);
            for (tl, tr, gain) in EARLY_TAPS {
                el += gain * read_delay(&self.pre[0], self.pre_pos, self.pre_delay + tl * 0.001 * sr);
                er += gain * read_delay(&self.pre[1], self.pre_pos, self.pre_delay + tr * 0.001 * sr);
            }
            l += el * early_level;
            r += er * early_level;
        }
        // Low cut again, on the way out: the tail has no lows either.
        l = self.lp_out[0].process(self.hp_out[0].process(l * WET_TRIM));
        r = self.lp_out[1].process(self.hp_out[1].process(r * WET_TRIM));
        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r) * self.current.width;
        let mut out = [mid + side, mid - side];
        for (ch, o) in out.iter_mut().enumerate() {
            let y = *o - self.dc_x1[ch] + 0.995 * self.dc_y1[ch];
            self.dc_x1[ch] = *o;
            self.dc_y1[ch] = flush(y);
            *o = y * self.current.level;
        }
        // Gate: after each hit the return is held for `gate_ms`, then closed over `gate_release_ms`.
        if self.gate_mix > 1.0e-3 {
            let sr = self.sample_rate;
            if in_l.abs().max(in_r.abs()) > GATE_THRESHOLD {
                self.gate_timer = self.current.gate_ms * 0.001 * sr;
            }
            let open = self.gate_timer > 0.0;
            if open {
                self.gate_timer -= 1.0;
            }
            let time_ms = if open { GATE_OPEN_MS } else { self.current.gate_release_ms };
            let coef = 1.0 - (-1.0 / (time_ms * 0.001 * sr)).exp();
            self.gate_gain += (if open { 1.0 } else { 0.0 } - self.gate_gain) * coef;
            let g = (1.0 - self.gate_mix) + self.gate_mix * self.gate_gain;
            out[0] *= g;
            out[1] *= g;
        }
        (out[0], out[1])
    }
}

/// The do-nothing voice carried by an effect slot (see the module docs).
pub struct ReverbVoice;

impl Voice for ReverbVoice {
    fn trigger(&mut self) {}

    fn process_sample(&mut self) -> f32 {
        0.0
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

    const SR: f32 = 44_100.0;

    fn energy(v: &[(f32, f32)]) -> f32 {
        v.iter().map(|(l, r)| l * l + r * r).sum()
    }

    fn impulse_response(params: ReverbParams, seconds: f32) -> Vec<(f32, f32)> {
        let mut core = ReverbCore::new(SR);
        core.set_params(params);
        // Let the smoothed controls settle on the target first.
        for _ in 0..(SR as usize) {
            core.process(0.0, 0.0);
        }
        (0..(SR * seconds) as usize)
            .map(|i| if i == 0 { core.process(1.0, 1.0) } else { core.process(0.0, 0.0) })
            .collect()
    }

    #[test]
    fn silence_in_silence_out() {
        let mut core = ReverbCore::new(SR);
        for _ in 0..10_000 {
            assert_eq!(core.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn the_tail_rings_then_decays_by_its_decay_time() {
        let params = ReverbParams {
            decay_s: 1.0,
            ..ReverbParams::default()
        };
        let ir = impulse_response(params, 2.5);
        assert!(ir.iter().all(|(l, r)| l.is_finite() && r.is_finite()));
        let at = |from: f32, to: f32| &ir[(from * SR) as usize..(to * SR) as usize];
        let early = energy(at(0.1, 0.3));
        let later = energy(at(1.0, 1.2));
        let late = energy(at(2.0, 2.2));
        assert!(early > 1.0e-4, "the tail must be audible: {early}");
        // 60 dB in 1 s = 30 dB (energy) every half-second window -> a lot lower.
        assert!(later < early * 0.05, "tail not falling: {early} -> {later}");
        assert!(late < later * 0.2, "tail keeps falling: {later} -> {late}");
    }

    #[test]
    fn a_longer_decay_rings_longer() {
        let short = impulse_response(
            ReverbParams {
                decay_s: 0.5,
                ..ReverbParams::default()
            },
            2.0,
        );
        let long = impulse_response(
            ReverbParams {
                decay_s: 6.0,
                ..ReverbParams::default()
            },
            2.0,
        );
        let window = |v: &[(f32, f32)]| energy(&v[(1.0 * SR) as usize..(1.5 * SR) as usize]);
        assert!(window(&long) > window(&short) * 20.0);
    }

    #[test]
    fn extreme_parameters_stay_finite() {
        for params in [
            ReverbParams {
                size: 2.0,
                decay_s: 12.0,
                damping: 0.0,
                predelay_ms: 250.0,
                low_cut_hz: 20.0,
                high_cut_hz: 20_000.0,
                width: 1.0,
                level: 2.0,
                kind: ReverbKind::Plate,
                gate_on: true,
                gate_ms: 1500.0,
                gate_release_ms: 500.0,
                warp: true,
                mod_override: -1.0,
                diff_override: -1.0,
            },
            ReverbParams {
                size: 0.5,
                decay_s: 0.2,
                damping: 1.0,
                predelay_ms: 0.0,
                low_cut_hz: 600.0,
                high_cut_hz: 1000.0,
                width: 0.0,
                level: 0.0,
                kind: ReverbKind::Room,
                gate_on: false,
                gate_ms: 10.0,
                gate_release_ms: 2.0,
                warp: true,
                mod_override: 3.0,
                diff_override: 0.0,
            },
        ] {
            let mut core = ReverbCore::new(SR);
            core.set_params(params);
            for i in 0..(SR as usize * 3) {
                let x = if i % 5000 == 0 { 1.0 } else { 0.0 };
                let (l, r) = core.process(x, -x);
                assert!(l.is_finite() && r.is_finite() && l.abs() < 50.0, "{params:?}");
            }
        }
    }

    #[test]
    fn nan_parameters_fall_back_to_defaults() {
        let bad = ReverbParams {
            size: f32::NAN,
            decay_s: f32::INFINITY,
            damping: f32::NAN,
            predelay_ms: f32::NAN,
            low_cut_hz: f32::NAN,
            high_cut_hz: f32::NAN,
            width: f32::NAN,
            level: f32::NAN,
            kind: ReverbKind::Hall,
            gate_on: false,
            gate_ms: f32::NAN,
            gate_release_ms: f32::NAN,
            warp: false,
            mod_override: f32::NAN,
            diff_override: f32::NAN,
        };
        assert_eq!(bad.sanitized(), ReverbParams::default());
    }

    /// The lines' settled length with these params.
    fn line_lengths(params: ReverbParams) -> [f32; LINES] {
        let mut core = ReverbCore::new(SR);
        core.set_params(params);
        for _ in 0..(SR as usize) {
            core.process(0.0, 0.0);
        }
        core.length
    }

    /// Without Warp, Decay never touches the lines (the tail keeps its pitch).
    #[test]
    fn without_warp_decay_leaves_the_lines_alone() {
        let short = line_lengths(ReverbParams { decay_s: 0.3, ..ReverbParams::default() });
        let long = line_lengths(ReverbParams { decay_s: 10.0, ..ReverbParams::default() });
        for i in 0..LINES {
            assert!((short[i] - long[i]).abs() < 0.01, "line {i}: {} vs {}", short[i], long[i]);
        }
    }

    /// With Warp, the lines follow Decay: unchanged at 2.5 s, 4 times as long at the longest
    /// Decay, a quarter as long at the shortest; and they always fit their buffers.
    #[test]
    fn warp_makes_the_lines_follow_the_decay() {
        let warped = |decay_s: f32, size: f32| {
            line_lengths(ReverbParams { decay_s, size, warp: true, ..ReverbParams::default() })
        };
        let plain = line_lengths(ReverbParams::default());
        let at_ref = warped(WARP_REF_DECAY_S, 1.0);
        let longest = warped(MAX_DECAY_S, 1.0);
        let shortest = warped(MIN_DECAY_S, 1.0);
        for i in 0..LINES {
            assert!((at_ref[i] / plain[i] - 1.0).abs() < 0.01, "line {i} at the reference");
            assert!((longest[i] / plain[i] - MAX_WARP).abs() < 0.04, "line {i} at the longest decay");
            assert!((shortest[i] / plain[i] - 1.0 / MAX_WARP).abs() < 0.01, "line {i} at the shortest decay");
        }
        let core = ReverbCore::new(SR);
        let biggest = warped(MAX_DECAY_S, 2.0);
        let wobble = MOD_DEPTH_MS * 0.001 * SR;
        assert!(biggest.iter().all(|l| l + wobble + 2.0 < core.lines[0].len() as f32));
        let mut widest = ReverbCore::new(SR);
        widest.set_params(ReverbParams { decay_s: MAX_DECAY_S, warp: true, ..ReverbParams::default() });
        for _ in 0..(SR as usize) {
            widest.process(0.0, 0.0);
        }
        for st in 0..DIFF_STEPS {
            for i in 0..LINES {
                assert!(widest.diff_len[st][i] + 2.0 < widest.diff[st][i].len() as f32, "diffuser {st}/{i}");
            }
        }
        assert!(warp_scale(0.25) < warp_scale(1.0) && warp_scale(1.0) < warp_scale(4.0));
    }

    /// With Warp the tail still falls 60 dB over Decay: the feedback follows the longer lines.
    #[test]
    fn a_warped_tail_still_decays_by_its_decay_time() {
        let ir = impulse_response(ReverbParams { decay_s: 1.0, warp: true, ..ReverbParams::default() }, 2.0);
        let at = |from: f32, to: f32| energy(&ir[(from * SR) as usize..(to * SR) as usize]);
        // 60 dB per second of energy = a factor 1e6 per second, 1e3 per half second.
        let drop_db = 10.0 * (at(0.5, 0.75) / at(1.0, 1.25)).log10();
        println!("warped tail drop over 0.5 s: {drop_db:.1} dB");
        assert!((drop_db - 30.0).abs() < 6.0, "{drop_db}");
    }

    #[test]
    fn reset_clears_the_tail() {
        let mut core = ReverbCore::new(SR);
        for i in 0..20_000 {
            core.process(if i < 100 { 1.0 } else { 0.0 }, 0.0);
        }
        core.reset();
        for _ in 0..5_000 {
            assert_eq!(core.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn width_zero_returns_the_same_signal_on_both_sides() {
        let ir = impulse_response(
            ReverbParams {
                width: 0.0,
                ..ReverbParams::default()
            },
            0.5,
        );
        assert!(ir.iter().all(|(l, r)| (l - r).abs() < 1.0e-6));
    }

    #[test]
    fn works_at_other_sample_rates() {
        for sr in [22_050.0f32, 48_000.0, 96_000.0, 192_000.0] {
            let mut core = ReverbCore::new(sr);
            let mut peak = 0.0f32;
            for i in 0..(sr as usize) {
                let (l, r) = core.process(if i == 0 { 1.0 } else { 0.0 }, 0.0);
                assert!(l.is_finite() && r.is_finite());
                peak = peak.max(l.abs()).max(r.abs());
            }
            assert!(peak > 1.0e-3, "silent at {sr} Hz");
        }
    }

    /// The audio thread must never allocate: processing and parameter changes
    /// stay allocation-free (same technique as the voice tests).
    #[test]
    fn processing_does_not_allocate() {
        let mut core = ReverbCore::new(SR);
        assert_no_alloc::assert_no_alloc(|| {
            core.set_params(ReverbParams {
                size: 1.7,
                decay_s: 4.0,
                ..ReverbParams::default()
            });
            for i in 0..20_000 {
                core.process((i % 7) as f32 * 0.1, 0.0);
            }
            core.reset();
        });
    }

    /// Fed at unity, the return sits within a few dB of the input: neither a
    /// whisper nor a wall of sound at the defaults.
    #[test]
    fn the_return_level_is_comparable_to_the_input() {
        let mut core = ReverbCore::new(SR);
        let mut seed = 0x1234_5678u32;
        let mut noise = move || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 8) as f32 / (1u32 << 23) as f32 - 1.0
        };
        let (mut sum_in, mut sum_out, mut n) = (0.0f32, 0.0f32, 0usize);
        for i in 0..(SR as usize * 6) {
            let (x, y) = (noise() * 0.5, noise() * 0.5);
            let (l, r) = core.process(x, y);
            if i > SR as usize * 3 {
                sum_in += x * x + y * y;
                sum_out += l * l + r * r;
                n += 1;
            }
        }
        let ratio_db = 10.0 * (sum_out / sum_in.max(1e-12)).log10();
        println!("reverb steady-state return vs input: {ratio_db:.1} dB over {n} samples");
        assert!((-8.0..=6.0).contains(&ratio_db), "{ratio_db} dB");
    }

    /// Switching to a short decay while a long tail rings must cut that tail at the
    /// new rate: a lock that ends (the pattern loops) hands the tail back to the
    /// lane's own decay.
    #[test]
    fn a_shorter_decay_takes_over_a_ringing_tail() {
        let mut core = ReverbCore::new(SR);
        core.set_params(ReverbParams {
            decay_s: 8.0,
            ..ReverbParams::default()
        });
        for i in 0..(SR as usize * 2) {
            core.process(if i < 4_000 { 0.5 } else { 0.0 }, 0.0);
        }
        let level = |core: &mut ReverbCore, secs: f32| {
            let n = (SR * secs) as usize;
            let mut e = 0.0f32;
            for _ in 0..n {
                let (l, r) = core.process(0.0, 0.0);
                e += l * l + r * r;
            }
            (e / n as f32).sqrt()
        };
        let ringing = level(&mut core, 0.1);
        core.set_params(ReverbParams {
            decay_s: 0.5,
            ..ReverbParams::default()
        });
        let _ = level(&mut core, 0.5);
        let after = level(&mut core, 0.1);
        println!("tail level before {ringing:.5}, 0.5 s after the switch {after:.5}");
        assert!(after < ringing * 0.05, "the old tail must die at the new rate: {ringing} -> {after}");
    }

    /// The low cut must really remove the lows: a 60 Hz tone loses at least 20 dB
    /// when the cut is at 300 Hz, while a 2 kHz tone is not touched.
    #[test]
    fn the_low_cut_removes_the_lows() {
        let level = |hz: f32, cut: f32| {
            let mut core = ReverbCore::new(SR);
            core.set_params(ReverbParams {
                low_cut_hz: cut,
                ..ReverbParams::default()
            });
            let n = (SR * 4.0) as usize;
            let mut sum = 0.0f32;
            for i in 0..n {
                let x = (TAU * hz * i as f32 / SR).sin() * 0.5;
                let (l, r) = core.process(x, x);
                if i > n / 2 {
                    sum += l * l + r * r;
                }
            }
            (sum / (n / 2) as f32).sqrt()
        };
        let open_low = level(60.0, 20.0);
        let cut_low = level(60.0, 300.0);
        let open_high = level(2000.0, 20.0);
        let cut_high = level(2000.0, 300.0);
        let low_drop_db = 20.0 * (open_low / cut_low.max(1e-9)).log10();
        let high_drop_db = 20.0 * (open_high / cut_high.max(1e-9)).log10();
        println!("low cut 300 Hz: 60 Hz -{low_drop_db:.1} dB, 2 kHz {high_drop_db:.1} dB");
        assert!(low_drop_db > 20.0, "60 Hz only dropped {low_drop_db} dB");
        assert!(high_drop_db.abs() < 1.5, "2 kHz moved by {high_drop_db} dB");
    }

    /// The high cut must really remove the highs: a 10 kHz tone loses at least 20 dB
    /// when the cut is at 2 kHz, while a 500 Hz tone is not touched.
    #[test]
    fn the_high_cut_removes_the_highs() {
        let level = |hz: f32, cut: f32| {
            let mut core = ReverbCore::new(SR);
            core.set_params(ReverbParams {
                high_cut_hz: cut,
                low_cut_hz: 20.0,
                ..ReverbParams::default()
            });
            let n = (SR * 4.0) as usize;
            let mut sum = 0.0f32;
            for i in 0..n {
                let x = (TAU * hz * i as f32 / SR).sin() * 0.5;
                let (l, r) = core.process(x, x);
                if i > n / 2 {
                    sum += l * l + r * r;
                }
            }
            (sum / (n / 2) as f32).sqrt()
        };
        let high_drop = 20.0 * (level(10_000.0, 20_000.0) / level(10_000.0, 2_000.0).max(1e-9)).log10();
        let low_drop = 20.0 * (level(500.0, 20_000.0) / level(500.0, 2_000.0).max(1e-9)).log10();
        println!("high cut 2 kHz: 10 kHz -{high_drop:.1} dB, 500 Hz {low_drop:.1} dB");
        assert!(high_drop > 20.0, "10 kHz only dropped {high_drop} dB");
        assert!(low_drop.abs() < 1.5, "500 Hz moved by {low_drop} dB");
    }

    fn params_of(kind: ReverbKind) -> ReverbParams {
        ReverbParams {
            kind,
            ..ReverbParams::default()
        }
    }

    /// The three types are three different sounds: the room has early reflections the
    /// others lack, and the plate is brighter than the hall at the same settings.
    #[test]
    fn the_types_sound_different() {
        let early = |kind: ReverbKind| {
            let ir = impulse_response(
                ReverbParams {
                    predelay_ms: 0.0,
                    ..params_of(kind)
                },
                0.2,
            );
            // Energy between 4 and 50 ms: where the early reflections of a room sit.
            energy(&ir[(0.004 * SR) as usize..(0.050 * SR) as usize])
        };
        let (hall, room, plate) = (early(ReverbKind::Hall), early(ReverbKind::Room), early(ReverbKind::Plate));
        println!("early energy: hall {hall:.4} room {room:.4} plate {plate:.4}");
        assert!(room > hall * 2.0 && room > plate * 2.0, "the room must have its reflections");

        // Brightness: share of the late tail's energy above the one-pole's corner.
        let brightness = |kind: ReverbKind| {
            let ir = impulse_response(
                ReverbParams {
                    low_cut_hz: 20.0,
                    ..params_of(kind)
                },
                2.5,
            );
            let (mut total, mut high, mut lp) = (0.0f32, 0.0f32, 0.0f32);
            for (i, (l, _)) in ir.iter().enumerate() {
                lp += 0.45 * (l - lp);
                if i > (0.5 * SR) as usize {
                    total += l * l;
                    high += (l - lp) * (l - lp);
                }
            }
            high / total.max(1e-12)
        };
        let (b_hall, b_plate) = (brightness(ReverbKind::Hall), brightness(ReverbKind::Plate));
        println!("brightness: hall {b_hall:.4} plate {b_plate:.4}");
        assert!(b_plate > b_hall * 1.3, "the plate must be brighter than the hall");
    }

    #[test]
    fn every_type_is_stable_and_audible() {
        for kind in [ReverbKind::Hall, ReverbKind::Room, ReverbKind::Plate] {
            let ir = impulse_response(params_of(kind), 2.0);
            assert!(ir.iter().all(|(l, r)| l.is_finite() && r.is_finite()), "{kind:?}");
            assert!(energy(&ir) > 1.0e-3, "{kind:?} is silent");
        }
    }

    /// Hall is the plain character: full-size lines, no early reflections, the damping
    /// cut-off as set; the others are defined against it.
    #[test]
    fn hall_is_the_plain_character() {
        let c = ReverbKind::Hall.character();
        assert_eq!((c.len_scale, c.bright, c.early_level), (1.0, 1.0, 0.0));
        assert_eq!(ReverbKind::from_value(0.0), ReverbKind::Hall);
        assert_eq!(ReverbKind::from_value(1.0), ReverbKind::Room);
        assert_eq!(ReverbKind::from_value(2.0), ReverbKind::Plate);
    }

    /// The gate cuts the tail a fixed time after a hit, however long the decay.
    #[test]
    fn the_gate_cuts_the_tail() {
        let tail_after = |gate_ms: f32| {
            let mut core = ReverbCore::new(SR);
            core.set_params(ReverbParams {
                decay_s: 6.0,
                gate_on: gate_ms > 0.0,
                gate_ms,
                ..ReverbParams::default()
            });
            let mut energy_late = 0.0f32;
            for i in 0..(SR as usize * 2) {
                let x = if i < 400 { 0.8 } else { 0.0 };
                let (l, r) = core.process(x, x);
                // 600 ms to 1 s after the hit
                if i > (SR * 0.6) as usize && i < SR as usize {
                    energy_late += l * l + r * r;
                }
            }
            energy_late
        };
        let open = tail_after(0.0);
        let gated = tail_after(250.0);
        println!("tail energy 0.6-1 s after the hit: open {open:.4}, gated at 250 ms {gated:.6}");
        assert!(open > 1.0e-3, "the open reverb rings on");
        assert!(gated < open * 0.001, "the gated tail must be gone");
    }

    /// The gate leaves the start of the tail alone.
    #[test]
    fn the_gate_keeps_the_start_of_the_tail() {
        let early = |gate_ms: f32| {
            let mut core = ReverbCore::new(SR);
            core.set_params(ReverbParams {
                gate_on: gate_ms > 0.0,
                gate_ms,
                ..ReverbParams::default()
            });
            let mut e = 0.0f32;
            for i in 0..(SR as usize / 4) {
                let x = if i < 400 { 0.8 } else { 0.0 };
                let (l, r) = core.process(x, x);
                if i > 200 {
                    e += l * l + r * r;
                }
            }
            e
        };
        let open = early(0.0);
        let gated = early(250.0);
        assert!(gated > open * 0.5, "the first 250 ms must survive: {open} -> {gated}");
    }

    /// The metallic sound is a tail whose spectrum has peaks (modes standing out): the
    /// magnitude of a diffuse tail is Rayleigh-like (spread / mean = 0.52); a ringing
    /// one is far above. Measured on 4096 samples of the late tail, 200 Hz - 8 kHz.
    #[test]
    fn the_tail_spectrum_has_no_ringing_peaks() {
        const N: usize = 4096;
        for kind in [ReverbKind::Hall, ReverbKind::Room, ReverbKind::Plate] {
            let ir = impulse_response(
                ReverbParams {
                    low_cut_hz: 20.0,
                    ..params_of(kind)
                },
                1.6,
            );
            let start = (0.4 * SR) as usize;
            let seg: Vec<f32> = (0..N)
                .map(|i| {
                    let w = 0.5 - 0.5 * (TAU * i as f32 / N as f32).cos();
                    ir[start + i].0 * w
                })
                .collect();
            let (lo, hi) = ((200.0 * N as f32 / SR) as usize, (8000.0 * N as f32 / SR) as usize);
            let mags: Vec<f32> = (lo..hi)
                .map(|k| {
                    let (mut re, mut im) = (0.0f64, 0.0f64);
                    for (n, x) in seg.iter().enumerate() {
                        let a = std::f64::consts::TAU * (k * n % N) as f64 / N as f64;
                        re += *x as f64 * a.cos();
                        im -= *x as f64 * a.sin();
                    }
                    (re * re + im * im).sqrt() as f32
                })
                .collect();
            // Divide each bin by the local mean (+-16 bins) so the damping's slope is not
            // counted as a peak: what is left is how much the bins stand out of their neighbours.
            let rel: Vec<f32> = (0..mags.len())
                .map(|i| {
                    let (a, b) = (i.saturating_sub(16), (i + 17).min(mags.len()));
                    mags[i] / (mags[a..b].iter().sum::<f32>() / (b - a) as f32)
                })
                .collect();
            let mean = rel.iter().sum::<f32>() / rel.len() as f32;
            let var = rel.iter().map(|m| (m - mean) * (m - mean)).sum::<f32>() / rel.len() as f32;
            let spread = var.sqrt() / mean;
            println!("{kind:?}: spectrum spread/mean = {spread:.3} (~0.5 = ideal noise)");
            assert!(spread < 0.7, "{kind:?} rings: {spread}");
        }
    }

    /// Normalised echo density (Abel & Huang): 1.0 = as dense as Gaussian noise. A sparse early
    /// field (a few distinct echoes) reads far below 1 and sounds like flutter / metal.
    fn ned(ir: &[(f32, f32)], center_ms: f32) -> f32 {
        let half = (0.010 * SR) as usize;
        let c = (center_ms * 0.001 * SR) as usize;
        let seg: Vec<f32> = ir[c - half..c + half].iter().map(|(l, _)| *l).collect();
        let sd = (seg.iter().map(|x| x * x).sum::<f32>() / seg.len() as f32).sqrt();
        let over = seg.iter().filter(|x| x.abs() > sd).count() as f32 / seg.len() as f32;
        over / 0.3173
    }

    #[test]
    fn the_echo_density_is_high_from_the_first_100_ms() {
        for kind in [ReverbKind::Hall, ReverbKind::Room, ReverbKind::Plate] {
            let ir = impulse_response(params_of(kind), 0.5);
            let dens: Vec<String> = [30.0, 60.0, 100.0, 200.0, 400.0]
                .iter()
                .map(|t| format!("{t} ms: {:.2}", ned(&ir, *t)))
                .collect();
            println!("NED {kind:?}: {}", dens.join("  "));
            // The previous engine read 0.43 at 100 ms and 0.96 at 200 ms: a sparse early
            // field, the flutter that sounds metallic.
            assert!(ned(&ir, 100.0) > 0.8 && ned(&ir, 200.0) > 0.9, "{kind:?} is sparse: {}", dens.join(" "));
        }
    }

    /// [295] The High Cut goes down to 20 Hz: the return is then a slow, soft swell with almost
    /// nothing above it, and stays finite and bounded.
    #[test]
    fn the_high_cut_can_go_down_to_20_hz() {
        let ir = impulse_response(
            ReverbParams {
                high_cut_hz: 20.0,
                ..ReverbParams::default()
            },
            2.0,
        );
        assert!(ir.iter().all(|(l, r)| l.is_finite() && r.is_finite() && l.abs() < 4.0 && r.abs() < 4.0));
        let open = impulse_response(ReverbParams::default(), 2.0);
        assert!(energy(&ir) < 0.2 * energy(&open), "a 20 Hz high cut must take nearly everything out");
        assert_eq!(ReverbParams { high_cut_hz: 5.0, ..ReverbParams::default() }.sanitized().high_cut_hz, 20.0);
    }
}
