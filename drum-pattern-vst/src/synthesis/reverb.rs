//! [286] Reverb - the send effect that lives on an effect-bus slot (Fx1 / Fx2).
//!
//! A stereo feedback delay network: pre-delay, low cut, two Schroeder all-pass
//! diffusers per channel, then eight delay lines mixed by a Hadamard matrix.
//! Each line has its own feedback gain (set from the decay time, so every line
//! rings down at the same rate) and a one-pole damping filter; a slow LFO
//! wobbles the line lengths so the tail does not sound metallic.
//!
//! Real-time contract: every buffer is allocated in [`ReverbCore::new`], called
//! from `DrumSynthesizer::initialize_with_layout` (non-RT). `process` allocates
//! and locks nothing. Parameters are smoothed at control rate (every
//! `CONTROL_PERIOD` samples) and the delay lengths are slewed per sample, so a
//! knob or a p-lock never clicks.
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
const BASE_MS: [f32; LINES] = [29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.7, 73.1];
const AP_MS: [f32; 4] = [5.1, 7.7, 6.3, 9.1];
const AP_GAIN: f32 = 0.6;
const MAX_PREDELAY_MS: f32 = 250.0;
const MOD_DEPTH_MS: f32 = 0.30;
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
}

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
        }
    }
}

impl ReverbParams {
    fn sanitized(self) -> Self {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        Self {
            size: fin(self.size, d.size).clamp(0.5, 2.0),
            decay_s: fin(self.decay_s, d.decay_s).clamp(0.2, 12.0),
            damping: fin(self.damping, d.damping).clamp(0.0, 1.0),
            predelay_ms: fin(self.predelay_ms, d.predelay_ms).clamp(0.0, MAX_PREDELAY_MS),
            low_cut_hz: fin(self.low_cut_hz, d.low_cut_hz).clamp(20.0, 1000.0),
            high_cut_hz: fin(self.high_cut_hz, d.high_cut_hz).clamp(1000.0, 20_000.0),
            width: fin(self.width, d.width).clamp(0.0, 1.0),
            level: fin(self.level, d.level).clamp(0.0, 2.0),
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
        }
        .sanitized()
    }
}

/// Schroeder all-pass diffuser.
struct AllPass {
    buf: Box<[f32]>,
    pos: usize,
}

impl AllPass {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(2)].into_boxed_slice(),
            pos: 0,
        }
    }

    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let delayed = self.buf[self.pos];
        let v = x - AP_GAIN * delayed;
        self.buf[self.pos] = v;
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        delayed + AP_GAIN * v
    }

    fn reset(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
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

    allpass: [AllPass; 4],

    lines: [Box<[f32]>; LINES],
    write: [usize; LINES],
    length: [f32; LINES],
    length_target: [f32; LINES],
    lfo_phase: [f32; LINES],
    lfo_inc: [f32; LINES],
    mod_samples: f32,
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
        let line_cap = ms(BASE_MS[LINES - 1] * 2.0 + MOD_DEPTH_MS + 2.0) + 8;
        let pre_cap = ms(MAX_PREDELAY_MS) + 8;
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
            allpass: std::array::from_fn(|i| AllPass::new(ms(AP_MS[i]))),
            lines: std::array::from_fn(|_| vec![0.0; line_cap].into_boxed_slice()),
            write: [0; LINES],
            length: [0.0; LINES],
            length_target: [0.0; LINES],
            lfo_phase: std::array::from_fn(|i| i as f32 * TAU / LINES as f32),
            // Slow, distinct rates (0.11 .. 0.31 Hz).
            lfo_inc: std::array::from_fn(|i| TAU * (0.11 + 0.028 * i as f32) / sr),
            mod_samples: MOD_DEPTH_MS * 0.001 * sr,
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
        for a in self.allpass.iter_mut() {
            a.reset();
        }
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

        let sr = self.sample_rate;
        for i in 0..LINES {
            let seconds = BASE_MS[i] * 0.001 * c.size;
            self.length_target[i] = seconds * sr;
            if snap {
                self.length[i] = self.length_target[i];
            }
            // 60 dB of loss over `decay_s`: every line loses the same per second.
            self.gain[i] = 10.0f32.powf(-3.0 * seconds / c.decay_s);
        }
        // Damping: 18 kHz (none) down to ~1.3 kHz (dark).
        let cutoff = 18_000.0 * 0.07f32.powf(c.damping);
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

        // Diffusion.
        xl = self.allpass[0].process(xl);
        xl = self.allpass[1].process(xl);
        xr = self.allpass[2].process(xr);
        xr = self.allpass[3].process(xr);

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
        hadamard8(&mut mixed);

        // Feed back, and inject the input: even lines hear left, odd lines right.
        for i in 0..LINES {
            let input = if i % 2 == 0 { xl } else { xr };
            let x = input * 0.5 + mixed[i] * self.gain[i];
            let n = self.lines[i].len();
            self.lines[i][self.write[i]] = flush(x);
            self.write[i] += 1;
            if self.write[i] == n {
                self.write[i] = 0;
            }
        }

        // Output: even taps left, odd taps right, then width.
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for i in 0..LINES {
            if i % 2 == 0 {
                l += taps[i];
            } else {
                r += taps[i];
            }
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
        };
        assert_eq!(bad.sanitized(), ReverbParams::default());
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
}
