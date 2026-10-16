//! [286] Delay - the second send effect of an effect-bus slot (Fx1 / Fx2).
//!
//! A stereo feedback delay. Each channel has its own delay line; the feedback
//! path goes through a low-pass (damping) and a 12 dB/oct high-pass (low cut),
//! then a soft clipper, so the repeats get darker and thinner as they go and the
//! loop can never run away. **Ping-pong** crosses the feedback between the two
//! channels and feeds the input into the left line only, so the repeats bounce
//! left, right, left...
//!
//! The delay time is slewed, not jumped: moving it (a Glide p-lock, or the knob)
//! makes the repeats bend in pitch like a tape or a bucket-brigade delay does,
//! which is the point of gliding it.
//!
//! Real-time contract: both lines are allocated in [`DelayCore::new`], called from
//! `DrumSynthesizer::initialize_with_layout` (non-RT). `process` allocates and
//! locks nothing. [`DelayVoice`] is the do-nothing voice the effect slot carries so
//! the generic per-slot machinery (settings push, p-locks, presets) keeps working.

use super::dsp::Biquad;
use super::{Voice, VoiceSettings};
use std::f32::consts::TAU;

/// Longest delay, in milliseconds.
pub const MAX_TIME_MS: f32 = 2000.0;
/// The widest left/right offset (the Width knob at 1), in milliseconds.
pub const MAX_WIDTH_MS: f32 = 45.0;

/// Note values of the Sync menu, in beats (quarter notes); index 0 = Free (the Time
/// knob in milliseconds). The menu entries are in the registry (`delay_sync`).
pub const SYNC_BEATS: [f32; 14] = [
    0.0,
    0.125,
    1.0 / 6.0,
    0.25,
    1.0 / 3.0,
    0.375,
    0.5,
    2.0 / 3.0,
    0.75,
    1.0,
    4.0 / 3.0,
    1.5,
    2.0,
    4.0,
];

/// Delay time, in milliseconds, of Sync entry `entry` at `bpm` (None for Free).
pub fn synced_time_ms(entry: usize, bpm: f32) -> Option<f32> {
    if entry == 0 || entry >= SYNC_BEATS.len() {
        return None;
    }
    let bpm = if bpm.is_finite() { bpm.clamp(20.0, 400.0) } else { 120.0 };
    Some((SYNC_BEATS[entry] * 60_000.0 / bpm).clamp(1.0, MAX_TIME_MS))
}
/// Samples between two control-rate updates.
const CONTROL_PERIOD: usize = 32;

/// `special[]` indices of the delay (index 4 and 31 are reserved by the p-lock
/// layout, see `param_id.rs`).
pub mod sp {
    pub const TIME: usize = 0;
    pub const FEEDBACK: usize = 1;
    pub const DAMPING: usize = 2;
    pub const LOW_CUT: usize = 3;
    pub const HIGH_CUT: usize = 9;
    pub const SYNC: usize = 6;
    pub const PINGPONG: usize = 5;
    pub const WIDTH: usize = 10;
    /// The lane's p-lock mode (0 = step, 1 = glide), read by the plugin when it
    /// evaluates the lane's locks (`fx_locks.rs`); not a DSP knob.
    pub const GLIDE: usize = 7;
    /// 1 = the lane does NOT restart from its sliders at the loop point (see
    /// `reverb::sp::NO_RESTART`).
    #[allow(dead_code)] // read through `reverb::sp` (same index); kept for symmetry
    pub const NO_RESTART: usize = 8;
}

/// The knobs of the delay.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DelayParams {
    pub time_ms: f32,
    /// Share of the repeats fed back, 0..0.95.
    pub feedback: f32,
    /// High-frequency loss of the repeats, 0..1.
    pub damping: f32,
    /// Low cut of the repeats, in Hz.
    pub low_cut_hz: f32,
    /// High cut of the repeats, in Hz (20 kHz = open): each repeat loses a little
    /// more.
    pub high_cut_hz: f32,
    /// 0 = two independent channels, 1 = full ping-pong.
    pub pingpong: f32,
    /// 0..1: the right repeats trail the left ones by up to `MAX_WIDTH_MS`.
    pub width: f32,
    /// Return level (the lane's Volume).
    pub level: f32,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            time_ms: 375.0,
            feedback: 0.4,
            damping: 0.4,
            low_cut_hz: 100.0,
            high_cut_hz: 20_000.0,
            pingpong: 0.5,
            width: 0.0,
            level: 1.0,
        }
    }
}

impl DelayParams {
    pub(crate) fn sanitized(self) -> Self {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        Self {
            time_ms: fin(self.time_ms, d.time_ms).clamp(1.0, MAX_TIME_MS),
            feedback: fin(self.feedback, d.feedback).clamp(0.0, 0.95),
            damping: fin(self.damping, d.damping).clamp(0.0, 1.0),
            low_cut_hz: fin(self.low_cut_hz, d.low_cut_hz).clamp(20.0, 1000.0),
            high_cut_hz: fin(self.high_cut_hz, d.high_cut_hz).clamp(20.0, 20_000.0),
            pingpong: fin(self.pingpong, d.pingpong).clamp(0.0, 1.0),
            width: fin(self.width, d.width).clamp(0.0, 1.0),
            level: fin(self.level, d.level).clamp(0.0, 2.0),
        }
    }
}

impl From<&VoiceSettings> for DelayParams {
    fn from(v: &VoiceSettings) -> Self {
        Self {
            time_ms: v.special[sp::TIME],
            feedback: v.special[sp::FEEDBACK],
            damping: v.special[sp::DAMPING],
            low_cut_hz: v.special[sp::LOW_CUT],
            high_cut_hz: v.special[sp::HIGH_CUT],
            pingpong: v.special[sp::PINGPONG],
            width: v.special[sp::WIDTH],
            level: v.volume,
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

/// Read `buf` `delay` samples behind `write` (the slot about to be written), with
/// linear interpolation. `delay` is clamped to `1..len-2`.
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

/// Gentle saturation inside the loop: transparent for small signals, it holds the
/// level of a loud repeat instead of letting the feedback pile up.
#[inline]
fn soft_clip(x: f32) -> f32 {
    x.tanh()
}

/// The delay engine: one stereo-in / stereo-out instance per effect bus.
pub struct DelayCore {
    sample_rate: f32,
    target: DelayParams,
    current: DelayParams,

    buf: [Box<[f32]>; 2],
    write: usize,
    /// Current delay time in samples, slewed towards the target.
    time: f32,
    time_slew: f32,

    damp_state: [f32; 2],
    damp_coef: f32,
    hp: [Biquad; 2],
    lp: [Biquad; 2],

    dc_x1: [f32; 2],
    dc_y1: [f32; 2],
    counter: usize,
}

impl DelayCore {
    /// Allocates both delay lines. Never call from the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() {
            sample_rate.clamp(8000.0, 384_000.0)
        } else {
            44_100.0
        };
        let capacity = ((MAX_TIME_MS + MAX_WIDTH_MS) * 0.001 * sr).ceil() as usize + 16;
        let params = DelayParams::default();
        let mut core = Self {
            sample_rate: sr,
            target: params,
            current: params,
            buf: [
                vec![0.0; capacity].into_boxed_slice(),
                vec![0.0; capacity].into_boxed_slice(),
            ],
            write: 0,
            time: params.time_ms * 0.001 * sr,
            // About 60 ms to follow a change of time: slow enough to be click-free,
            // quick enough that a glide is followed closely.
            time_slew: 1.0 - (-1.0 / (0.06 * sr)).exp(),
            damp_state: [0.0; 2],
            damp_coef: 1.0,
            hp: [Biquad::new(); 2],
            lp: [Biquad::new(); 2],
            dc_x1: [0.0; 2],
            dc_y1: [0.0; 2],
            counter: 0,
        };
        core.update_controls(true);
        core
    }

    pub fn set_params(&mut self, params: DelayParams) {
        self.target = params.sanitized();
    }

    /// Silence the repeats (buffers and filter states). No allocation.
    pub fn reset(&mut self) {
        for b in self.buf.iter_mut() {
            b.fill(0.0);
        }
        self.write = 0;
        self.damp_state = [0.0; 2];
        for f in self.hp.iter_mut().chain(self.lp.iter_mut()) {
            f.reset();
        }
        self.dc_x1 = [0.0; 2];
        self.dc_y1 = [0.0; 2];
    }

    fn update_controls(&mut self, snap: bool) {
        let k = if snap {
            1.0
        } else {
            1.0 - (-(CONTROL_PERIOD as f32) / (self.sample_rate * 0.03)).exp()
        };
        let (c, t) = (&mut self.current, &self.target);
        c.time_ms = t.time_ms; // the time has its own, per-sample slew
        c.feedback += (t.feedback - c.feedback) * k;
        c.damping += (t.damping - c.damping) * k;
        c.low_cut_hz += (t.low_cut_hz - c.low_cut_hz) * k;
        c.high_cut_hz += (t.high_cut_hz - c.high_cut_hz) * k;
        c.pingpong += (t.pingpong - c.pingpong) * k;
        c.width += (t.width - c.width) * k;
        c.level += (t.level - c.level) * k;
        let sr = self.sample_rate;
        if snap {
            self.time = t.time_ms * 0.001 * sr;
        }
        // Damping: 18 kHz (none) down to ~1.3 kHz (dark).
        let cutoff = 18_000.0 * 0.07f32.powf(c.damping);
        self.damp_coef = 1.0 - (-TAU * cutoff.min(sr * 0.45) / sr).exp();
        let mut hp = Biquad::new();
        hp.set_highpass(c.low_cut_hz, std::f32::consts::FRAC_1_SQRT_2, sr);
        for f in self.hp.iter_mut() {
            f.copy_coefficients_from(&hp);
        }
        let mut lp = Biquad::new();
        lp.set_lowpass(c.high_cut_hz, std::f32::consts::FRAC_1_SQRT_2, sr);
        for f in self.lp.iter_mut() {
            f.copy_coefficients_from(&lp);
        }
    }

    /// One stereo sample in, one stereo sample of repeats out (wet only).
    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if self.counter == 0 {
            self.update_controls(false);
        }
        self.counter += 1;
        if self.counter == CONTROL_PERIOD {
            self.counter = 0;
        }

        // The time follows its target with a slew: moving it bends the repeats.
        let wanted = self.current.time_ms * 0.001 * self.sample_rate;
        self.time += (wanted - self.time) * self.time_slew;

        // Width: the right line is read a little later than the left one.
        let offset = self.current.width * MAX_WIDTH_MS * 0.001 * self.sample_rate;
        let dl = read_delay(&self.buf[0], self.write, self.time);
        let dr = read_delay(&self.buf[1], self.write, self.time + offset);

        // The loop: damp, cut the lows, soften the peaks.
        let mut fb = [dl, dr];
        for ch in 0..2 {
            self.damp_state[ch] += self.damp_coef * (fb[ch] - self.damp_state[ch]);
            let cut = self.hp[ch].process(self.damp_state[ch]);
            fb[ch] = soft_clip(cut) * self.current.feedback;
        }

        // Ping-pong: the input goes into the left line only and the feedback
        // crosses over; at 0 the two channels stay independent.
        let p = self.current.pingpong;
        let mono = 0.5 * (in_l + in_r);
        let wl = in_l * (1.0 - p) + mono * p + fb[0] * (1.0 - p) + fb[1] * p;
        let wr = in_r * (1.0 - p) + fb[1] * (1.0 - p) + fb[0] * p;
        // The high cut sits on what is written into the lines: every repeat goes
        // through it again, so each one loses a little more.
        let wl = self.lp[0].process(wl);
        let wr = self.lp[1].process(wr);
        let n = self.buf[0].len();
        self.buf[0][self.write] = flush(wl);
        self.buf[1][self.write] = flush(wr);
        self.write += 1;
        if self.write == n {
            self.write = 0;
        }

        let mut out = [dl, dr];
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
pub struct DelayVoice;

impl Voice for DelayVoice {
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

    /// Responses to a single click: `(left, right)` per sample.
    fn impulse_response(params: DelayParams, seconds: f32) -> Vec<(f32, f32)> {
        let mut core = DelayCore::new(SR);
        core.set_params(params);
        for _ in 0..(SR as usize / 2) {
            core.process(0.0, 0.0);
        }
        (0..(SR * seconds) as usize)
            .map(|i| if i == 0 { core.process(1.0, 1.0) } else { core.process(0.0, 0.0) })
            .collect()
    }

    fn peak_at(ir: &[(f32, f32)], ms: f32, window_ms: f32) -> (f32, f32) {
        let from = (ms * 0.001 * SR) as usize;
        let to = from + (window_ms * 0.001 * SR) as usize;
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for (a, b) in &ir[from..to.min(ir.len())] {
            l = l.max(a.abs());
            r = r.max(b.abs());
        }
        (l, r)
    }

    #[test]
    fn silence_in_silence_out() {
        let mut core = DelayCore::new(SR);
        for _ in 0..10_000 {
            assert_eq!(core.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn the_first_repeat_comes_after_the_delay_time_and_not_before() {
        let ir = impulse_response(
            DelayParams {
                time_ms: 300.0,
                feedback: 0.0,
                pingpong: 0.0,
                ..DelayParams::default()
            },
            1.0,
        );
        let (before_l, _) = peak_at(&ir, 0.0, 280.0);
        let (on_l, on_r) = peak_at(&ir, 290.0, 20.0);
        assert!(before_l < 1.0e-4, "nothing before the repeat: {before_l}");
        assert!(on_l > 0.3 && on_r > 0.3, "the repeat: {on_l} / {on_r}");
    }

    #[test]
    fn feedback_makes_repeats_that_fade() {
        let ir = impulse_response(
            DelayParams {
                time_ms: 200.0,
                feedback: 0.6,
                pingpong: 0.0,
                damping: 0.0,
                low_cut_hz: 20.0,
                ..DelayParams::default()
            },
            2.0,
        );
        let first = peak_at(&ir, 190.0, 20.0).0;
        let second = peak_at(&ir, 390.0, 20.0).0;
        let third = peak_at(&ir, 590.0, 20.0).0;
        assert!(second > first * 0.3 && second < first, "{first} -> {second}");
        assert!(third > second * 0.3 && third < second, "{second} -> {third}");
    }

    #[test]
    fn ping_pong_bounces_between_the_sides() {
        let ir = impulse_response(
            DelayParams {
                time_ms: 200.0,
                feedback: 0.7,
                pingpong: 1.0,
                damping: 0.0,
                low_cut_hz: 20.0,
                ..DelayParams::default()
            },
            1.5,
        );
        let (l1, r1) = peak_at(&ir, 190.0, 20.0);
        let (l2, r2) = peak_at(&ir, 390.0, 20.0);
        let (l3, r3) = peak_at(&ir, 590.0, 20.0);
        assert!(l1 > 0.3 && r1 < 0.05, "first repeat on the left: {l1} / {r1}");
        assert!(r2 > 0.2 && l2 < 0.05, "second on the right: {l2} / {r2}");
        assert!(l3 > 0.1 && r3 < 0.05, "third back on the left: {l3} / {r3}");
    }

    #[test]
    fn extreme_parameters_stay_finite_and_bounded() {
        for params in [
            DelayParams {
                time_ms: 2000.0,
                feedback: 0.95,
                damping: 0.0,
                low_cut_hz: 20.0,
                high_cut_hz: 20_000.0,
                pingpong: 1.0,
                width: 1.0,
                level: 2.0,
            },
            DelayParams {
                time_ms: 1.0,
                feedback: 0.95,
                damping: 1.0,
                low_cut_hz: 1000.0,
                high_cut_hz: 1000.0,
                pingpong: 0.0,
                width: 0.0,
                level: 0.0,
            },
        ] {
            let mut core = DelayCore::new(SR);
            core.set_params(params);
            for i in 0..(SR as usize * 6) {
                let x = if i % 7000 == 0 { 1.0 } else { 0.0 };
                let (l, r) = core.process(x, -x);
                assert!(l.is_finite() && r.is_finite() && l.abs() < 20.0, "{params:?}");
            }
        }
    }

    #[test]
    fn nan_parameters_fall_back_to_defaults() {
        let bad = DelayParams {
            time_ms: f32::NAN,
            feedback: f32::INFINITY,
            damping: f32::NAN,
            low_cut_hz: f32::NAN,
            high_cut_hz: f32::NAN,
            pingpong: f32::NAN,
            width: f32::NAN,
            level: f32::NAN,
        };
        assert_eq!(bad.sanitized(), DelayParams::default());
    }

    #[test]
    fn reset_clears_the_repeats() {
        let mut core = DelayCore::new(SR);
        for i in 0..30_000 {
            core.process(if i < 100 { 1.0 } else { 0.0 }, 0.0);
        }
        core.reset();
        for _ in 0..30_000 {
            assert_eq!(core.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn works_at_other_sample_rates() {
        for sr in [22_050.0f32, 48_000.0, 96_000.0, 192_000.0] {
            let mut core = DelayCore::new(sr);
            core.set_params(DelayParams {
                time_ms: 100.0,
                ..DelayParams::default()
            });
            let mut peak = 0.0f32;
            for i in 0..(sr as usize) {
                let (l, r) = core.process(if i == 0 { 1.0 } else { 0.0 }, 0.0);
                assert!(l.is_finite() && r.is_finite());
                if i > (sr * 0.09) as usize {
                    peak = peak.max(l.abs()).max(r.abs());
                }
            }
            assert!(peak > 1.0e-2, "no repeat at {sr} Hz");
        }
    }

    /// Moving the time bends the repeats in pitch: a tone fed in while the time
    /// lengthens comes out lower than the tone that went in.
    #[test]
    fn a_lengthening_delay_bends_the_pitch_down() {
        let mut core = DelayCore::new(SR);
        core.set_params(DelayParams {
            time_ms: 100.0,
            feedback: 0.0,
            pingpong: 0.0,
            damping: 0.0,
            low_cut_hz: 20.0,
            ..DelayParams::default()
        });
        let tone = 1000.0f32;
        let mut out = Vec::new();
        for i in 0..(SR as usize * 3) {
            // Glide the time from 100 ms to 600 ms across seconds 1-2.
            let t = i as f32 / SR;
            let ms = 100.0 + 500.0 * (t - 1.0).clamp(0.0, 1.0);
            if i % 32 == 0 {
                core.set_params(DelayParams {
                    time_ms: ms,
                    feedback: 0.0,
                    pingpong: 0.0,
                    damping: 0.0,
                    low_cut_hz: 20.0,
                    ..DelayParams::default()
                });
            }
            let x = (TAU * tone * t).sin() * 0.5;
            out.push(core.process(x, x).0);
        }
        // Zero crossings per second over the middle of the glide.
        let crossings = |from: usize, to: usize| {
            out[from..to].windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count() as f32
                / ((to - from) as f32 / SR)
        };
        let steady = crossings(SR as usize / 2, SR as usize);
        let bending = crossings((SR * 1.3) as usize, (SR * 1.7) as usize);
        println!("steady {steady:.0} Hz, while the delay lengthens {bending:.0} Hz");
        assert!((steady - tone).abs() < 30.0, "steady {steady}");
        assert!(bending < steady * 0.9, "no bend: {steady} -> {bending}");
    }

    /// The audio thread must never allocate.
    #[test]
    fn processing_does_not_allocate() {
        let mut core = DelayCore::new(SR);
        assert_no_alloc::assert_no_alloc(|| {
            core.set_params(DelayParams {
                time_ms: 777.0,
                feedback: 0.8,
                pingpong: 1.0,
                ..DelayParams::default()
            });
            for i in 0..20_000 {
                core.process((i % 7) as f32 * 0.1, 0.0);
            }
            core.reset();
        });
    }

    /// The high cut shapes the repeats: a bright click comes back duller with a low cut-off.
    #[test]
    fn the_high_cut_darkens_the_repeats() {
        let energy = |cut: f32| {
            let mut core = DelayCore::new(SR);
            core.set_params(DelayParams {
                time_ms: 100.0,
                feedback: 0.0,
                pingpong: 0.0,
                damping: 0.0,
                low_cut_hz: 20.0,
                high_cut_hz: cut,
                ..DelayParams::default()
            });
            for _ in 0..(SR as usize / 2) {
                core.process(0.0, 0.0);
            }
            // A 12 kHz tone, so that only the high cut matters.
            let mut sum = 0.0f32;
            for i in 0..(SR as usize * 2) {
                let x = (TAU * 12_000.0 * i as f32 / SR).sin() * 0.5;
                let (l, _) = core.process(x, x);
                if i > SR as usize {
                    sum += l * l;
                }
            }
            sum
        };
        let open = energy(20_000.0);
        let cut = energy(2_000.0);
        assert!(cut < open * 0.01, "12 kHz must fall at least 20 dB: {open} -> {cut}");
    }

    /// Width trails the right repeats behind the left ones.
    #[test]
    fn width_offsets_the_right_channel() {
        let ir = impulse_response(
            DelayParams {
                time_ms: 200.0,
                feedback: 0.0,
                pingpong: 0.0,
                width: 1.0,
                ..DelayParams::default()
            },
            0.6,
        );
        let first_over = |ch: usize| {
            ir.iter()
                .position(|s| (if ch == 0 { s.0 } else { s.1 }).abs() > 0.05)
                .unwrap() as f32
                / SR
                * 1000.0
        };
        let (l, r) = (first_over(0), first_over(1));
        println!("left repeat at {l:.1} ms, right at {r:.1} ms");
        assert!((r - l - MAX_WIDTH_MS).abs() < 3.0, "offset {}", r - l);
    }

    #[test]
    fn the_sync_menu_gives_note_values_at_the_tempo() {
        assert_eq!(synced_time_ms(0, 120.0), None, "Free uses the Time knob");
        // 1/4 note = 1 beat = 500 ms at 120 bpm; 1/8 = 250; dotted 1/8 = 375; 1/8 triplet = 166.7.
        assert!((synced_time_ms(9, 120.0).unwrap() - 500.0).abs() < 0.01);
        assert!((synced_time_ms(6, 120.0).unwrap() - 250.0).abs() < 0.01);
        assert!((synced_time_ms(8, 120.0).unwrap() - 375.0).abs() < 0.01);
        assert!((synced_time_ms(4, 120.0).unwrap() - 166.67).abs() < 0.1);
        // Slow tempo: a bar would be longer than the delay line: it is capped.
        assert_eq!(synced_time_ms(13, 40.0), Some(MAX_TIME_MS));
        assert_eq!(synced_time_ms(99, 120.0), None);
    }

    /// [295] The High Cut goes down to 20 Hz: the repeats stay finite and bounded, and are
    /// much quieter than with it open.
    #[test]
    fn the_high_cut_can_go_down_to_20_hz() {
        let low = impulse_response(
            DelayParams {
                high_cut_hz: 20.0,
                feedback: 0.7,
                ..DelayParams::default()
            },
            3.0,
        );
        assert!(low.iter().all(|(l, r)| l.is_finite() && r.is_finite() && l.abs() < 4.0 && r.abs() < 4.0));
        let open = impulse_response(DelayParams { feedback: 0.7, ..DelayParams::default() }, 3.0);
        let e = |v: &[(f32, f32)]| v.iter().map(|(l, r)| l * l + r * r).sum::<f32>();
        assert!(e(&low) < 0.3 * e(&open));
        assert_eq!(DelayParams { high_cut_hz: 5.0, ..DelayParams::default() }.sanitized().high_cut_hz, 20.0);
    }
}
