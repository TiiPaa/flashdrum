//! [286] Rumble - the third send effect of an effect-bus slot (Fx1 / Fx2): the
//! kick rumble of hard techno.
//!
//! The bus input (the kicks that lanes send to it) goes through a low-pass, an
//! optional mono **Echo** (one delay line that makes the rumble gallop), a very
//! long, dark reverb (the [`ReverbCore`] at its largest size), then a steep
//! low-pass that keeps only the sub, and is **ducked** by the input: every hit
//! pushes the rumble down at once, by **Duck**, then it swells back over the
//! **Release**, along the **Duck Curve**, so the low rumble fills the gaps between the
//! kicks instead of masking them. **Drive** adds harmonics so the
//! rumble stays audible on small speakers and lifts the quiet tail between the kicks (it
//! comes BEFORE the duck: after it, its gain would fill the dip back in); **High Pass** clears the sub; **Width**
//! goes from a mono centre to a wide return.
//!
//! Real-time contract: the reverb's and the echo's buffers are allocated in
//! [`RumbleCore::new`], called from `DrumSynthesizer::initialize_with_layout`
//! (non-RT); `process` allocates and locks nothing. [`RumbleVoice`] is the do-nothing
//! voice the effect slot carries (see `reverb.rs`).

use super::dsp::Biquad;
use super::reverb::{ReverbCore, ReverbKind, ReverbParams};
use super::{Voice, VoiceSettings};

/// Samples between two control-rate updates.
const CONTROL_PERIOD: usize = 32;
/// What goes in is only the low end of the input, so the reverb builds a sub tail.
const INPUT_LOW_PASS_RATIO: f32 = 2.0;
/// A hit is the level of the low end of the input rising to more than this many times what it
/// was in the last ~30 ms. A rise, not a level: it works at any send level, and a kick whose
/// tail is still ringing when the next one comes (a long kick) is heard all the same.
const HIT_RATIO: f32 = 1.5;
/// Below this the input is silence for the detector.
const DETECT_FLOOR: f32 = 0.0005;
/// A rise only counts when it reaches this share of the loudest recent hit (a hat leaking into
/// the low end does not duck) ...
const HIT_MIN_SHARE: f32 = 0.2;
/// ... and two hits are at least this far apart (ms): the body of a kick is one hit.
const HIT_REFRACTORY_MS: f32 = 80.0;
/// The history the detector compares the rise with: this many snapshots of the level, one
/// every `HISTORY_STEP_MS`.
const HISTORY: usize = 8;
const HISTORY_STEP_MS: f32 = 4.0;
/// How slowly the memory of the loudest hit fades (s).
const PEAK_MEMORY_S: f32 = 2.0;
/// The detector listens to the low end only (the kick, not the hats sent along with it).
const DETECT_LOW_PASS_HZ: f32 = 200.0;
const DUCK_ATTACK_MS: f32 = 3.0;
/// How long the follower of the input level takes to fall.
const FOLLOWER_FALL_MS: f32 = 30.0;
/// The longest echo, in ms (a synced value is capped there too).
const MAX_ECHO_MS: f32 = 2000.0;
/// Gain of the tail: the reverb's wet return is about -6 dB under its input.
const MAKEUP: f32 = 1.6;

/// `special[]` indices of the rumble (4 and 31 are reserved by the p-lock layout; 6 was
/// the retired Mono switch).
pub mod sp {
    pub const DECAY: usize = 0;
    pub const LOW_PASS: usize = 1;
    pub const DUCK: usize = 2;
    pub const RELEASE: usize = 3;
    pub const DRIVE: usize = 5;
    /// The lane's p-lock mode (0 = step, 1 = glide), read by the plugin; not a DSP knob.
    #[allow(dead_code)] // read through `reverb::sp` (same index); kept for symmetry
    pub const GLIDE: usize = 7;
    pub const HIGH_PASS: usize = 9;
    /// 0 = mono centre, 1 = wide.
    pub const WIDTH: usize = 10;
    pub const ECHO_MIX: usize = 11;
    pub const ECHO_TIME: usize = 12;
    /// Note value of the echo (0 = Free: the time in ms); same menu as the delay's.
    pub const ECHO_SYNC: usize = 13;
    pub const ECHO_FEEDBACK: usize = 14;
    pub const DUCK_CURVE: usize = 15;
    /// What starts the duck: 0 = every beat of the transport, 1 = every eighth, 2 = every
    /// sixteenth, 3 = a kick heard in the input (`DUCK_SOURCE_KICK`). 0 is the default so that
    /// lanes saved before the setting existed (which hold 0) pump on the beat.
    pub const DUCK_SOURCE: usize = 16;
    /// The reverb underneath: its room size (0.5..2, 2 = the biggest), the gap before it starts
    /// (ms) and how much its lines wobble (ms; 0.7 = the original sound).
    pub const REVERB_SIZE: usize = 17;
    pub const REVERB_PREDELAY: usize = 18;
    pub const REVERB_MOTION: usize = 19;
    /// How much the reverb smears its input (1 = fully, the original sound; small = the repeats of
    /// the kick stay distinct).
    pub const REVERB_BLUR: usize = 20;
}

/// `DUCK_SOURCE` value: the duck starts when the input has a kick, not on the transport.
pub const DUCK_SOURCE_KICK: u8 = 3;

/// Exponent of the duck's return for a Duck Curve value 0..1. The middle (0.5) already stays
/// down longer than a straight ramp (a straight one comes back too soon to be heard under a
/// kick); lower = most of the return comes first (a quick swell, then a slow one), higher =
/// the rumble stays down and comes back late.
pub fn duck_exponent(curve: f32) -> f32 {
    0.5 * 2.0f32.powf(4.0 * (0.5 - curve.clamp(0.0, 1.0)))
}

/// How deep a Duck setting digs (0..1: 1 = down to silence). Cubed so most of the slider
/// goes deep: 80 % leaves under 1 % of the level (-42 dB), 50 % leaves 12 % (-18 dB).
pub fn duck_depth(duck: f32) -> f32 {
    let left = 1.0 - duck.clamp(0.0, 1.0);
    1.0 - left * left * left
}

/// How much of the duck is left `x` (0..1) of the way through the Release.
pub fn duck_ratio(x: f32, exponent: f32) -> f32 {
    (1.0 - x).clamp(0.0, 1.0).powf(exponent)
}

/// The knobs of the rumble.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RumbleParams {
    /// How long the tail lasts (60 dB), in seconds.
    pub decay_s: f32,
    /// The reverb's room size (0.5..2), its pre-delay (ms) and the depth of its wobble (ms).
    pub reverb_size: f32,
    pub reverb_predelay_ms: f32,
    pub reverb_motion_ms: f32,
    pub reverb_blur: f32,
    /// The rumble loses what is under this frequency, in Hz.
    pub high_pass_hz: f32,
    /// The rumble keeps what is under this frequency, in Hz.
    pub low_pass_hz: f32,
    /// 0..1: how far a hit pushes the rumble down.
    pub duck: f32,
    /// 0..1: the shape of the return (see [`duck_exponent`]).
    pub duck_curve: f32,
    /// What starts the duck (see `sp::DUCK_SOURCE`).
    pub duck_source: u8,
    /// How long the rumble takes to swell back after a hit, in ms.
    pub release_ms: f32,
    /// 0..1: harmonics added to the rumble.
    pub drive: f32,
    /// 0 = mono, 1 = wide.
    pub width: f32,
    /// 0..1: level of the echo that feeds the reverb (0 = no echo).
    pub echo_mix: f32,
    pub echo_ms: f32,
    /// 0..0.9: share of the echo fed back.
    pub echo_feedback: f32,
    /// Return level (the lane's Volume).
    pub level: f32,
}

impl Default for RumbleParams {
    fn default() -> Self {
        Self {
            decay_s: 3.0,
            reverb_size: 2.0,
            reverb_predelay_ms: 0.0,
            reverb_motion_ms: 0.7,
            reverb_blur: 1.0,
            high_pass_hz: 30.0,
            low_pass_hz: 120.0,
            duck: 0.8,
            duck_curve: 0.5,
            duck_source: 0,
            release_ms: 250.0,
            drive: 0.25,
            width: 0.0,
            echo_mix: 0.0,
            echo_ms: 250.0,
            echo_feedback: 0.3,
            level: 1.0,
        }
    }
}

impl RumbleParams {
    pub(crate) fn sanitized(self) -> Self {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let d = Self::default();
        Self {
            decay_s: fin(self.decay_s, d.decay_s).clamp(0.5, 8.0),
            reverb_size: fin(self.reverb_size, d.reverb_size).clamp(0.5, 2.0),
            reverb_predelay_ms: fin(self.reverb_predelay_ms, d.reverb_predelay_ms).clamp(0.0, 250.0),
            reverb_motion_ms: fin(self.reverb_motion_ms, d.reverb_motion_ms).clamp(0.1, 3.0),
            reverb_blur: fin(self.reverb_blur, d.reverb_blur).clamp(0.05, 1.0),
            high_pass_hz: fin(self.high_pass_hz, d.high_pass_hz).clamp(20.0, 200.0),
            low_pass_hz: fin(self.low_pass_hz, d.low_pass_hz).clamp(40.0, 300.0),
            duck: fin(self.duck, d.duck).clamp(0.0, 1.0),
            duck_curve: fin(self.duck_curve, d.duck_curve).clamp(0.0, 1.0),
            duck_source: self.duck_source.min(DUCK_SOURCE_KICK),
            release_ms: fin(self.release_ms, d.release_ms).clamp(20.0, 1000.0),
            drive: fin(self.drive, d.drive).clamp(0.0, 1.0),
            width: fin(self.width, d.width).clamp(0.0, 1.0),
            echo_mix: fin(self.echo_mix, d.echo_mix).clamp(0.0, 1.0),
            echo_ms: fin(self.echo_ms, d.echo_ms).clamp(1.0, MAX_ECHO_MS),
            echo_feedback: fin(self.echo_feedback, d.echo_feedback).clamp(0.0, 0.9),
            level: fin(self.level, d.level).clamp(0.0, 2.0),
        }
    }
}

impl From<&VoiceSettings> for RumbleParams {
    fn from(v: &VoiceSettings) -> Self {
        Self {
            decay_s: v.special[sp::DECAY],
            reverb_size: v.special[sp::REVERB_SIZE],
            reverb_predelay_ms: v.special[sp::REVERB_PREDELAY],
            reverb_motion_ms: v.special[sp::REVERB_MOTION],
            reverb_blur: v.special[sp::REVERB_BLUR],
            high_pass_hz: v.special[sp::HIGH_PASS],
            low_pass_hz: v.special[sp::LOW_PASS],
            duck: v.special[sp::DUCK],
            duck_curve: v.special[sp::DUCK_CURVE],
            duck_source: v.special[sp::DUCK_SOURCE].round().clamp(0.0, DUCK_SOURCE_KICK as f32) as u8,
            release_ms: v.special[sp::RELEASE],
            drive: v.special[sp::DRIVE],
            width: v.special[sp::WIDTH],
            echo_mix: v.special[sp::ECHO_MIX],
            echo_ms: v.special[sp::ECHO_TIME],
            echo_feedback: v.special[sp::ECHO_FEEDBACK],
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
/// linear interpolation.
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

/// The rumble engine: one stereo-in / stereo-out instance per effect bus.
pub struct RumbleCore {
    sample_rate: f32,
    target: RumbleParams,
    current: RumbleParams,
    reverb: ReverbCore,
    /// 12 dB/oct low-pass in front of the reverb.
    pre_lp: [Biquad; 2],
    /// 24 dB/oct low-pass on the way out, then the 24 dB/oct high-pass after the drive.
    post_lp: [[Biquad; 2]; 2],
    post_hp: [[Biquad; 2]; 2],
    /// Drive's own low-pass: tames the harmonics it makes.
    drive_lp: [f32; 2],
    drive_lp_coef: f32,
    /// The mono echo in front of the reverb.
    echo: Box<[f32]>,
    echo_pos: usize,
    echo_len: f32,
    /// Follower of the input level and the hit detector's memory, and the duck: the samples
    /// since the last hit started it, the smoothed gain.
    env: f32,
    env_fall: f32,
    env_history: [f32; HISTORY],
    history_pos: usize,
    history_count: usize,
    history_period: usize,
    peak_memory: f32,
    peak_fall: f32,
    refractory: usize,
    detect_lp: [f32; 2],
    detect_lp_coef: f32,
    since_hit: f32,
    duck_exp: f32,
    duck_gain: f32,
    duck_coef: f32,
    counter: usize,
}

impl RumbleCore {
    /// Allocates the reverb's and the echo's buffers. Never call from the audio thread.
    pub fn new(sample_rate: f32) -> Self {
        let sr = if sample_rate.is_finite() {
            sample_rate.clamp(8000.0, 384_000.0)
        } else {
            44_100.0
        };
        let params = RumbleParams::default();
        let mut core = Self {
            sample_rate: sr,
            target: params,
            current: params,
            reverb: ReverbCore::new(sr),
            pre_lp: [Biquad::new(); 2],
            post_lp: [[Biquad::new(); 2]; 2],
            post_hp: [[Biquad::new(); 2]; 2],
            drive_lp: [0.0; 2],
            drive_lp_coef: 1.0,
            echo: vec![0.0; (MAX_ECHO_MS * 0.001 * sr).ceil() as usize + 8].into_boxed_slice(),
            echo_pos: 0,
            echo_len: params.echo_ms * 0.001 * sr,
            env: 0.0,
            env_fall: (-1.0 / (FOLLOWER_FALL_MS * 0.001 * sr)).exp(),
            env_history: [0.0; HISTORY],
            history_pos: 0,
            history_count: 0,
            history_period: ((HISTORY_STEP_MS * 0.001 * sr) as usize).max(1),
            peak_memory: 0.0,
            peak_fall: (-1.0 / (PEAK_MEMORY_S * sr)).exp(),
            refractory: 0,
            detect_lp: [0.0; 2],
            detect_lp_coef: 1.0 - (-std::f32::consts::TAU * DETECT_LOW_PASS_HZ / sr).exp(),
            since_hit: f32::MAX,
            duck_exp: 1.0,
            duck_gain: 1.0,
            duck_coef: 1.0 - (-1.0 / (DUCK_ATTACK_MS * 0.001 * sr)).exp(),
            counter: 0,
        };
        core.update_controls(true);
        core
    }

    pub fn set_params(&mut self, params: RumbleParams) {
        self.target = params.sanitized();
    }

    /// Start the duck now: the plugin calls this on the beats of the transport when the lane's
    /// Duck Trigger is Beat / 1/8 / 1/16.
    pub fn trigger_duck(&mut self) {
        self.since_hit = 0.0;
    }

    /// What starts the duck, as the engine has it now.
    pub fn duck_source(&self) -> u8 {
        self.current.duck_source
    }

    /// The duck's gain right now (1 = the rumble untouched), for the Lane Editor.
    pub fn duck_gain(&self) -> f32 {
        self.duck_gain
    }

    /// Silence the tail. No allocation.
    pub fn reset(&mut self) {
        self.reverb.reset();
        for f in self
            .pre_lp
            .iter_mut()
            .chain(self.post_lp.iter_mut().flatten())
            .chain(self.post_hp.iter_mut().flatten())
        {
            f.reset();
        }
        self.drive_lp = [0.0; 2];
        self.echo.fill(0.0);
        self.echo_pos = 0;
        self.env = 0.0;
        self.env_history = [0.0; HISTORY];
        self.history_count = 0;
        self.peak_memory = 0.0;
        self.refractory = 0;
        self.detect_lp = [0.0; 2];
        self.since_hit = f32::MAX;
        self.duck_gain = 1.0;
    }

    fn update_controls(&mut self, snap: bool) {
        let sr = self.sample_rate;
        let k = if snap {
            1.0
        } else {
            1.0 - (-(CONTROL_PERIOD as f32) / (sr * 0.05)).exp()
        };
        let (c, t) = (&mut self.current, &self.target);
        c.decay_s += (t.decay_s - c.decay_s) * k;
        c.reverb_size += (t.reverb_size - c.reverb_size) * k;
        c.reverb_predelay_ms += (t.reverb_predelay_ms - c.reverb_predelay_ms) * k;
        c.reverb_motion_ms += (t.reverb_motion_ms - c.reverb_motion_ms) * k;
        c.reverb_blur += (t.reverb_blur - c.reverb_blur) * k;
        c.high_pass_hz += (t.high_pass_hz - c.high_pass_hz) * k;
        c.low_pass_hz += (t.low_pass_hz - c.low_pass_hz) * k;
        c.duck += (t.duck - c.duck) * k;
        c.duck_curve += (t.duck_curve - c.duck_curve) * k;
        c.release_ms += (t.release_ms - c.release_ms) * k;
        c.drive += (t.drive - c.drive) * k;
        c.width += (t.width - c.width) * k;
        c.echo_mix += (t.echo_mix - c.echo_mix) * k;
        c.echo_feedback += (t.echo_feedback - c.echo_feedback) * k;
        c.level += (t.level - c.level) * k;
        // The echo time is slewed per sample in `process` (a change bends the repeats).
        c.echo_ms = t.echo_ms;
        c.duck_source = t.duck_source;

        self.duck_exp = duck_exponent(c.duck_curve);

        let mut pre = Biquad::new();
        pre.set_lowpass(
            (c.low_pass_hz * INPUT_LOW_PASS_RATIO).min(sr * 0.45),
            std::f32::consts::FRAC_1_SQRT_2,
            sr,
        );
        for f in self.pre_lp.iter_mut() {
            f.copy_coefficients_from(&pre);
        }
        let mut post = Biquad::new();
        post.set_lowpass(c.low_pass_hz.min(sr * 0.45), std::f32::consts::FRAC_1_SQRT_2, sr);
        for f in self.post_lp.iter_mut().flatten() {
            f.copy_coefficients_from(&post);
        }
        let mut hp = Biquad::new();
        hp.set_highpass(c.high_pass_hz, std::f32::consts::FRAC_1_SQRT_2, sr);
        for f in self.post_hp.iter_mut().flatten() {
            f.copy_coefficients_from(&hp);
        }
        let drive_cut = (c.low_pass_hz * 6.0).clamp(300.0, 2500.0).min(sr * 0.45);
        self.drive_lp_coef = 1.0 - (-std::f32::consts::TAU * drive_cut / sr).exp();

        // The reverb underneath: its biggest room, very dark, nothing else.
        self.reverb.set_params(ReverbParams {
            size: c.reverb_size,
            decay_s: c.decay_s,
            damping: 1.0,
            predelay_ms: c.reverb_predelay_ms,
            low_cut_hz: c.high_pass_hz,
            high_cut_hz: 1000.0,
            width: 1.0,
            level: 1.0,
            kind: ReverbKind::Rumble,
            gate_on: false,
            mod_override: c.reverb_motion_ms,
            diff_override: c.reverb_blur,
            ..ReverbParams::default()
        });
    }

    /// One stereo sample in, one stereo sample of rumble out (wet only).
    #[inline]
    pub fn process(&mut self, in_l: f32, in_r: f32) -> (f32, f32) {
        if self.counter == 0 {
            self.update_controls(false);
        }
        self.counter += 1;
        if self.counter == CONTROL_PERIOD {
            self.counter = 0;
        }
        let (in_l, in_r) = (
            if in_l.is_finite() { in_l } else { 0.0 },
            if in_r.is_finite() { in_r } else { 0.0 },
        );
        let sr = self.sample_rate;

        // The follower sees the raw input: it is the kick that ducks the rumble. A hit (the
        // level rising over the threshold) drops the rumble to its floor AT ONCE; it then
        // swells back over Release along the curve. The hit's own length does not matter.
        // Each side on its own (a kick with its two sides in opposite phase would cancel in a sum).
        for (lp, x) in self.detect_lp.iter_mut().zip([in_l, in_r]) {
            *lp += self.detect_lp_coef * (x - *lp);
            if lp.abs() < 1.0e-20 {
                *lp = 0.0;
            }
        }
        let level = self.detect_lp[0].abs().max(self.detect_lp[1].abs());
        self.env = if level > self.env { level } else { self.env * self.env_fall };
        // Snapshots of the level, one every few ms: what it was a moment ago.
        self.history_count += 1;
        if self.history_count >= self.history_period {
            self.history_count = 0;
            self.env_history[self.history_pos] = self.env;
            self.history_pos = (self.history_pos + 1) % HISTORY;
        }
        let before = self.env_history.iter().fold(f32::MAX, |m, x| m.min(*x));
        self.peak_memory = self.peak_memory.max(self.env) * if self.env > self.peak_memory { 1.0 } else { self.peak_fall };
        self.refractory = self.refractory.saturating_sub(1);
        if self.current.duck_source == DUCK_SOURCE_KICK
            && self.refractory == 0
            && self.env > DETECT_FLOOR
            && self.env > HIT_RATIO * before
            && self.env > HIT_MIN_SHARE * self.peak_memory
        {
            self.since_hit = 0.0;
            self.refractory = (HIT_REFRACTORY_MS * 0.001 * sr) as usize;
        }
        if self.since_hit < f32::MAX {
            self.since_hit += 1.0;
        }
        let release = (self.current.release_ms * 0.001 * sr).max(1.0);
        let wanted = 1.0 - duck_depth(self.current.duck) * duck_ratio(self.since_hit / release, self.duck_exp);
        self.duck_gain += (wanted - self.duck_gain) * self.duck_coef;

        // In front of the reverb: the low end, plus the mono echo that makes it gallop.
        let pl = self.pre_lp[0].process(in_l);
        let pr = self.pre_lp[1].process(in_r);
        self.echo_len += (self.current.echo_ms * 0.001 * sr - self.echo_len) * 0.002;
        let n = self.echo.len();
        let written = self.echo_pos;
        self.echo[written] = flush((0.5 * (pl + pr)).clamp(-4.0, 4.0));
        self.echo_pos += 1;
        if self.echo_pos == n {
            self.echo_pos = 0;
        }
        let delayed = read_delay(&self.echo, self.echo_pos, self.echo_len);
        // The repeats are fed back into the line, softly clipped so they can never run away.
        let fed = self.echo[written] + self.current.echo_feedback * delayed;
        self.echo[written] = flush(fed.tanh());
        let echo_out = delayed * self.current.echo_mix;

        let (mut l, mut r) = self.reverb.process(pl + echo_out, pr + echo_out);
        let l1 = self.post_lp[0][0].process(l);
        l = self.post_lp[0][1].process(l1);
        let r1 = self.post_lp[1][0].process(r);
        r = self.post_lp[1][1].process(r1);
        // Width: from the mono sum (0) to the full stereo return (1).
        let mid = 0.5 * (l + r);
        let side = 0.5 * (l - r) * self.current.width;
        let mut out = [mid + side, mid - side];

        // Drive: an asymmetric soft clip (the even harmonics make a sub audible on small
        // speakers), then its own low-pass so they stay under ~2 kHz; the high-pass after it
        // removes the DC an asymmetric clip makes. The squared law keeps the low settings
        // gentle (0.25 is about the earlier sound) and opens up to a hard growl at 1.
        let drive = self.current.drive;
        let d2 = drive * drive;
        let gain = 1.0 + 28.0 * d2;
        let bias = 1.5 * d2;
        // The clip's larger swing (the negative one) is the loudest: scale by it, and bring the
        // level down as the clip gets harder so Drive adds bite, not volume.
        let norm = 0.7 * (1.0 + bias.tanh()).recip() / (1.0 + d2);
        for (ch, o) in out.iter_mut().enumerate() {
            let driven = if drive > 1.0e-3 {
                ((*o * gain + bias).tanh() - bias.tanh()) * norm
            } else {
                *o
            };
            self.drive_lp[ch] += self.drive_lp_coef * (driven - self.drive_lp[ch]);
            if self.drive_lp[ch].abs() < 1.0e-20 {
                self.drive_lp[ch] = 0.0;
            }
            let shaped = driven * (1.0 - 0.7 * drive) + self.drive_lp[ch] * 0.7 * drive;
            // The duck is the LAST gain: a saturator after it would lift the ducked signal back
            // up (a gain of up to 29 on what the duck left) and the dip would not be heard.
            let h1 = self.post_hp[ch][0].process(shaped * self.duck_gain);
            let h2 = self.post_hp[ch][1].process(h1);
            *o = h2 * self.current.level * MAKEUP;
        }
        (out[0], out[1])
    }
}

/// The do-nothing voice carried by an effect slot (see the module docs).
pub struct RumbleVoice;

impl Voice for RumbleVoice {
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

    /// A 60 Hz kick-like thump of `ms` milliseconds.
    fn kick(i: usize, ms: f32) -> f32 {
        let t = i as f32 / SR;
        if t < ms * 0.001 {
            (std::f32::consts::TAU * 60.0 * t).sin() * (1.0 - t / (ms * 0.001)) * 0.8
        } else {
            0.0
        }
    }

    fn energy(v: &[(f32, f32)]) -> f32 {
        v.iter().map(|(l, r)| l * l + r * r).sum()
    }

    /// Four-to-the-floor at 130 bpm: returns the input and the rumble, left channel. The duck
    /// listens to the kick (the transport does not run in a unit test).
    fn run(params: RumbleParams, seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut core = RumbleCore::new(SR);
        core.set_params(RumbleParams {
            duck_source: DUCK_SOURCE_KICK,
            ..params
        });
        let beat = (SR * 60.0 / 130.0) as usize;
        let (mut input, mut out) = (Vec::new(), Vec::new());
        for i in 0..(SR * seconds) as usize {
            let x = kick(i % beat, 90.0);
            let (l, _) = core.process(x, x);
            input.push(x);
            out.push(l);
        }
        (input, out)
    }

    #[test]
    fn silence_in_silence_out() {
        let mut core = RumbleCore::new(SR);
        for _ in 0..20_000 {
            assert_eq!(core.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    #[test]
    fn the_rumble_rings_on_after_the_kick_and_stays_in_the_lows() {
        let mut core = RumbleCore::new(SR);
        core.set_params(RumbleParams {
            duck: 0.0,
            drive: 0.0,
            ..RumbleParams::default()
        });
        let tail: Vec<(f32, f32)> = (0..(SR * 2.0) as usize)
            .map(|i| core.process(kick(i, 90.0), kick(i, 90.0)))
            .collect();
        let late = &tail[(SR * 0.5) as usize..];
        assert!(energy(late) > 1.0e-3, "the rumble must outlast the kick");
        // Almost nothing above 400 Hz: a first-difference high-pass vs the signal.
        let (mut total, mut high) = (0.0f32, 0.0f32);
        for w in late.windows(2) {
            total += w[1].0 * w[1].0;
            let d = w[1].0 - w[0].0;
            high += d * d;
        }
        // A first difference of a sine at f has energy (2 pi f / sr)^2 of it: 100 Hz -> 0.0002.
        assert!(high / total < 0.002, "the rumble is not low enough: {}", high / total);
    }

    /// The point of the effect: with Duck up the rumble is quieter right after a kick than
    /// just before the next one.
    #[test]
    fn the_kick_ducks_the_rumble_and_it_swells_back() {
        let params = RumbleParams {
            duck: 1.0,
            release_ms: 300.0,
            decay_s: 4.0,
            ..RumbleParams::default()
        };
        let (_, out) = run(params, 6.0);
        let beat = (SR * 60.0 / 130.0) as usize;
        // Average level in a short window just after, and just before, a kick (late in the run).
        let level = |v: &[f32], from: usize, len: usize| {
            v[from..from + len].iter().map(|x| x.abs()).sum::<f32>() / len as f32
        };
        let k = (SR * 4.0) as usize / beat * beat; // a kick boundary
        let after = level(&out, k + (0.04 * SR) as usize, (0.05 * SR) as usize);
        let before = level(&out, k - (0.09 * SR) as usize, (0.05 * SR) as usize);
        println!("rumble level just after a kick {after:.4}, just before the next {before:.4}");
        let (_, flat) = run(RumbleParams { duck: 0.0, ..params }, 6.0);
        let after_flat = level(&flat, k + (0.04 * SR) as usize, (0.05 * SR) as usize);
        let before_flat = level(&flat, k - (0.09 * SR) as usize, (0.05 * SR) as usize);
        println!("no duck: after {after_flat:.4}, before {before_flat:.4}");
        // Just after the kick the reverb is at its loudest (the kick has just fed it): the duck
        // is what takes that peak down. Just before the next kick it has swelled back to full.
        assert!(after < 0.5 * after_flat, "ducking must lower the level right after a kick");
        assert!((before - before_flat).abs() < 0.2 * before_flat, "the rumble must be back to full before the next kick");
    }

    /// The Duck Curve shapes the return: a high curve keeps the rumble down longer.
    #[test]
    fn the_duck_curve_changes_how_the_rumble_swells_back() {
        let at_mid_release = |curve: f32| {
            let (_, out) = run(
                RumbleParams {
                    duck: 1.0,
                    release_ms: 400.0,
                    duck_curve: curve,
                    decay_s: 4.0,
                    ..RumbleParams::default()
                },
                6.0,
            );
            let beat = (SR * 60.0 / 130.0) as usize;
            let k = (SR * 4.0) as usize / beat * beat;
            // The duck starts at the kick: the middle of the 400 ms release is ~200 ms later.
            let from = k + (0.20 * SR) as usize;
            out[from..from + (0.04 * SR) as usize].iter().map(|x| x.abs()).sum::<f32>()
        };
        let (late, early) = (at_mid_release(1.0), at_mid_release(0.0));
        println!("level mid-release: late curve {late:.4}, early curve {early:.4}");
        assert!(early > late * 1.3, "a low curve must swell back sooner than a high one");
        assert!(duck_exponent(0.0) > duck_exponent(0.5) && duck_exponent(1.0) < duck_exponent(0.5));
        assert!((duck_exponent(0.5) - 0.5).abs() < 1.0e-6);
        assert!(duck_ratio(0.0, 1.0) == 1.0 && duck_ratio(1.0, 1.0) == 0.0);
    }

    #[test]
    fn the_high_pass_removes_the_sub() {
        let sub = |hp: f32| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                high_pass_hz: hp,
                low_pass_hz: 300.0,
                duck: 0.0,
                drive: 0.0,
                ..RumbleParams::default()
            });
            let mut e = 0.0f32;
            for i in 0..(SR * 3.0) as usize {
                let x = (std::f32::consts::TAU * 40.0 * i as f32 / SR).sin() * 0.5;
                let (l, _) = core.process(x, x);
                if i > (SR * 1.5) as usize {
                    e += l * l;
                }
            }
            e
        };
        assert!(sub(20.0) > sub(200.0) * 20.0, "a 200 Hz high-pass must take a 40 Hz sub out");
    }

    /// Drive adds harmonics: a pure 50 Hz tone comes out with a 150 Hz component (the third
    /// harmonic of a clipped wave) and a 100 Hz one (the second, from the asymmetry).
    #[test]
    fn drive_adds_harmonics() {
        let ratios = |drive: f32| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                drive,
                duck: 0.0,
                low_pass_hz: 300.0,
                high_pass_hz: 20.0,
                ..RumbleParams::default()
            });
            let n = (SR * 2.0) as usize;
            let out: Vec<f32> = (0..n)
                .map(|i| {
                    let x = (std::f32::consts::TAU * 50.0 * i as f32 / SR).sin() * 0.3;
                    core.process(x, x).0
                })
                .collect();
            let tail = &out[n / 2..];
            let power_at = |hz: f32| {
                let (mut re, mut im) = (0.0f64, 0.0f64);
                for (i, x) in tail.iter().enumerate() {
                    let a = std::f64::consts::TAU * hz as f64 * i as f64 / SR as f64;
                    re += *x as f64 * a.cos();
                    im += *x as f64 * a.sin();
                }
                re * re + im * im
            };
            let f = power_at(50.0).max(1e-12);
            ((power_at(100.0) / f) as f32, (power_at(150.0) / f) as f32)
        };
        let (clean2, clean3) = ratios(0.0);
        let (driven2, driven3) = ratios(1.0);
        println!("2nd / 3rd harmonic vs 50 Hz: drive 0 {clean2:.4} {clean3:.4}, drive 1 {driven2:.4} {driven3:.4}");
        assert!(driven3 > 0.02 && driven3 > clean3 * 10.0, "Drive must add odd harmonics");
        assert!(driven2 > 0.01 && driven2 > clean2 * 10.0, "Drive must add even harmonics");
    }

    /// The echo makes the rumble gallop: with Echo up, the input is heard again one echo
    /// time later, so the reverb is fed twice.
    #[test]
    fn the_echo_repeats_the_input_into_the_reverb() {
        let energy_between = |mix: f32| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                echo_mix: mix,
                echo_ms: 400.0,
                echo_feedback: 0.0,
                duck: 0.0,
                decay_s: 0.5,
                drive: 0.0,
                ..RumbleParams::default()
            });
            let mut e = 0.0f32;
            for i in 0..(SR * 1.2) as usize {
                let (l, r) = core.process(kick(i, 90.0), kick(i, 90.0));
                // 450-700 ms: the first kick's tail (decay 0.5 s) is nearly gone; the echo's is not.
                if i > (SR * 0.5) as usize && i < (SR * 0.8) as usize {
                    e += l * l + r * r;
                }
            }
            e
        };
        let (dry, echoed) = (energy_between(0.0), energy_between(1.0));
        println!("energy 0.5-0.8 s: no echo {dry:.5}, echo {echoed:.5}");
        assert!(echoed > dry * 1.5, "the echoed kick must feed the reverb again");
    }

    #[test]
    fn a_longer_decay_rings_longer() {
        let tail = |decay_s: f32| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                decay_s,
                duck: 0.0,
                ..RumbleParams::default()
            });
            let mut e = 0.0f32;
            for i in 0..(SR * 4.0) as usize {
                let (l, r) = core.process(kick(i, 90.0), kick(i, 90.0));
                if i > (SR * 2.0) as usize {
                    e += l * l + r * r;
                }
            }
            e
        };
        assert!(tail(6.0) > tail(1.0) * 3.0);
    }

    #[test]
    fn width_zero_sums_the_two_sides() {
        let mut core = RumbleCore::new(SR);
        core.set_params(RumbleParams {
            width: 0.0,
            ..RumbleParams::default()
        });
        for i in 0..(SR as usize) {
            let (l, r) = core.process(kick(i, 90.0), 0.5 * kick(i, 90.0));
            assert!((l - r).abs() < 1.0e-6, "width 0 lost the mono at {i}");
        }
    }

    #[test]
    fn extreme_and_garbage_parameters_stay_finite() {
        for params in [
            RumbleParams {
                decay_s: 8.0,
                reverb_size: 0.5,
                reverb_predelay_ms: 250.0,
                reverb_motion_ms: 1.0,
                reverb_blur: 0.05,
                low_pass_hz: 300.0,
                duck: 1.0,
                release_ms: 1000.0,
                drive: 1.0,
                width: 1.0,
                echo_mix: 1.0,
                echo_ms: 2000.0,
                echo_feedback: 0.9,
                high_pass_hz: 200.0,
                duck_curve: 1.0,
                duck_source: 3,
                level: 2.0,
            },
            RumbleParams {
                decay_s: f32::NAN,
                reverb_size: f32::NAN,
                reverb_predelay_ms: f32::INFINITY,
                reverb_motion_ms: f32::NAN,
                reverb_blur: f32::NAN,
                low_pass_hz: f32::INFINITY,
                duck: f32::NAN,
                release_ms: -5.0,
                drive: f32::NAN,
                width: f32::NAN,
                echo_mix: f32::NAN,
                echo_ms: f32::NAN,
                echo_feedback: f32::INFINITY,
                high_pass_hz: f32::NAN,
                duck_curve: f32::NAN,
                duck_source: 200,
                level: f32::NAN,
            },
        ] {
            let mut core = RumbleCore::new(SR);
            core.set_params(params);
            for i in 0..(SR as usize * 3) {
                let x = if i % 20_000 < 4000 { 1.0 } else { 0.0 };
                let (l, r) = core.process(x, -x);
                assert!(l.is_finite() && r.is_finite() && l.abs() < 20.0 && r.abs() < 20.0, "{params:?}");
            }
        }
    }

    #[test]
    fn reset_silences_the_tail() {
        let mut core = RumbleCore::new(SR);
        core.set_params(RumbleParams {
            echo_mix: 1.0,
            ..RumbleParams::default()
        });
        for i in 0..(SR as usize) {
            core.process(kick(i, 90.0), kick(i, 90.0));
        }
        core.reset();
        for _ in 0..4000 {
            assert_eq!(core.process(0.0, 0.0), (0.0, 0.0));
        }
    }

    /// `process` must not allocate (the reverb and the echo inside included).
    #[test]
    fn process_does_not_allocate() {
        let mut core = RumbleCore::new(SR);
        core.set_params(RumbleParams {
            echo_mix: 0.5,
            ..RumbleParams::default()
        });
        assert_no_alloc::assert_no_alloc(|| {
            for i in 0..20_000 {
                core.process(kick(i, 90.0), kick(i, 90.0));
            }
        });
    }

    /// With the defaults the rumble is a real part of the mix: its peaks between kicks reach a
    /// good share of the kick's own level (a rumble 30 dB under the kick is not heard).
    #[test]
    fn the_default_rumble_is_audible_next_to_the_kick() {
        let (input, out) = run(RumbleParams::default(), 8.0);
        let from = (SR * 4.0) as usize;
        let kick_peak = input[from..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let rumble_peak = out[from..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        println!("kick peak {kick_peak:.3}, rumble peak {rumble_peak:.3}");
        assert!(rumble_peak > 0.25 * kick_peak && rumble_peak < 1.5 * kick_peak, "{rumble_peak}");
    }

    /// The duck must leave the rumble audible most of the beat: with the defaults its average
    /// level over a bar stays above half of what it would be with no duck at all.
    #[test]
    fn the_default_duck_leaves_the_rumble_audible_most_of_the_time() {
        let mean = |params: RumbleParams| {
            let (_, out) = run(params, 8.0);
            let from = (SR * 4.0) as usize;
            out[from..].iter().map(|x| x.abs()).sum::<f32>() / (out.len() - from) as f32
        };
        let ducked = mean(RumbleParams::default());
        let free = mean(RumbleParams {
            duck: 0.0,
            ..RumbleParams::default()
        });
        println!("mean level: default duck {ducked:.4}, no duck {free:.4}");
        assert!(ducked > 0.5 * free, "the duck swallows the rumble: {ducked} vs {free}");
    }

    /// A sustained rumble: its average level can exceed an impulsive kick's (the kick is
    /// loud for 90 ms of a 460 ms beat) but its PEAK stays well under the kick's, so it
    /// fills the mix without burying the kick.
    #[test]
    fn the_rumble_fills_the_gaps_without_burying_the_kick() {
        let (input, out) = run(RumbleParams::default(), 8.0);
        let from = (SR * 4.0) as usize;
        let mean = |v: &[f32]| v[from..].iter().map(|x| x.abs()).sum::<f32>() / (v.len() - from) as f32;
        let peak = |v: &[f32]| v[from..].iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let (k, r) = (mean(&input), mean(&out));
        let db = 20.0 * (r / k).log10();
        println!("mean level: kick {k:.4}, rumble {r:.4} ({db:.1} dB re the kick)");
        assert!(db > -6.0 && db < 12.0, "{db} dB");
        assert!(peak(&out) < 0.8 * peak(&input), "the rumble's peaks must stay under the kick's");
    }

    /// The duck must work at any send level and next to other sounds: a kick at -34 dB, and a
    /// kick with a steady hat-like tone beside it, both still duck the rumble.
    #[test]
    fn the_duck_works_at_low_send_levels_and_beside_other_sounds() {
        let after_a_kick = |level: f32, hiss: f32, duck: f32| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                duck,
                release_ms: 300.0,
                decay_s: 4.0,
                drive: 0.0,
                duck_source: DUCK_SOURCE_KICK,
                ..RumbleParams::default()
            });
            let beat = (SR * 60.0 / 130.0) as usize;
            let mut window = 0.0f32;
            for i in 0..(SR * 6.0) as usize {
                let hat = (std::f32::consts::TAU * 6000.0 * i as f32 / SR).sin() * hiss;
                let x = kick(i % beat, 90.0) * level + hat;
                let (l, _) = core.process(x, x);
                let k = (SR * 4.0) as usize / beat * beat;
                if i >= k + (0.04 * SR) as usize && i < k + (0.09 * SR) as usize {
                    window += l.abs();
                }
            }
            window
        };
        for (level, hiss) in [(0.02, 0.0), (1.0, 0.0), (1.0, 0.05), (0.1, 0.01)] {
            let (ducked, free) = (after_a_kick(level, hiss, 1.0), after_a_kick(level, hiss, 0.0));
            println!("kick {level} + tone {hiss}: ducked {ducked:.5} vs free {free:.5}");
            assert!(ducked < 0.5 * free, "no duck with kick {level} and tone {hiss}");
        }
    }

    /// What the user hears must be a dip: right after a kick the rumble is far under its
    /// average level, whatever the Drive.
    #[test]
    fn the_duck_makes_a_clear_dip_at_every_kick() {
        for drive in [0.0, 0.25, 0.6, 1.0] {
            let params = RumbleParams {
                duck: 1.0,
                release_ms: 400.0,
                decay_s: 4.0,
                drive,
                ..RumbleParams::default()
            };
            let beat = (SR * 60.0 / 130.0) as usize;
            let k = (SR * 4.0) as usize / beat * beat;
            let mean = |v: &[f32], from: usize, len: usize| v[from..from + len].iter().map(|x| x.abs()).sum::<f32>() / len as f32;
            let (_, ducked) = run(params, 6.0);
            let (_, free) = run(RumbleParams { duck: 0.0, ..params }, 6.0);
            let dip = mean(&ducked, k + (0.01 * SR) as usize, (0.03 * SR) as usize);
            let average = mean(&free, k, beat);
            println!("drive {drive}: just after the kick {dip:.4}, average level {average:.4}");
            assert!(dip < 0.3 * average, "no dip at drive {drive}: {dip} vs {average}");
        }
    }

    /// Triggered from the transport the duck needs no kick in the input: the beats start it.
    #[test]
    fn the_transport_can_start_the_duck_with_no_kick_in_the_input() {
        let mut core = RumbleCore::new(SR);
        core.set_params(RumbleParams {
            duck_source: 0,
            duck: 1.0,
            release_ms: 300.0,
            ..RumbleParams::default()
        });
        // A steady tone in: nothing to hear a kick in.
        let tone = |i: usize| (std::f32::consts::TAU * 60.0 * i as f32 / SR).sin() * 0.3;
        for i in 0..(SR as usize) {
            core.process(tone(i), tone(i));
        }
        assert!(core.duck_gain() > 0.99, "the input alone must not duck in Beat mode");
        core.trigger_duck();
        let mut lowest = 1.0f32;
        for i in 0..(0.1 * SR) as usize {
            core.process(tone(i), tone(i));
            lowest = lowest.min(core.duck_gain());
        }
        assert!(lowest < 0.1, "the beat must duck the rumble ({lowest})");
        for i in 0..(0.5 * SR) as usize {
            core.process(tone(i), tone(i));
        }
        assert!(core.duck_gain() > 0.99, "and it must swell back");
    }

    /// The reverb knobs change the rumble: a smaller room, a pre-delay and a deeper wobble each
    /// give a different tail than the default, and the defaults are the original sound.
    #[test]
    fn the_reverb_knobs_change_the_rumble() {
        let tail = |params: RumbleParams| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                duck: 0.0,
                drive: 0.0,
                low_pass_hz: 300.0,
                ..params
            });
            let mut out = Vec::new();
            for i in 0..(SR * 2.0) as usize {
                out.push(core.process(kick(i, 90.0), kick(i, 90.0)).0);
            }
            out
        };
        let base = tail(RumbleParams::default());
        let diff = |other: &[f32]| {
            base.iter().zip(other).map(|(a, b)| (a - b) * (a - b)).sum::<f32>()
                / base.iter().map(|a| a * a).sum::<f32>().max(1e-12)
        };
        for (what, params) in [
            ("size", RumbleParams { reverb_size: 0.6, ..RumbleParams::default() }),
            ("pre-delay", RumbleParams { reverb_predelay_ms: 150.0, ..RumbleParams::default() }),
            ("motion", RumbleParams { reverb_motion_ms: 3.0, ..RumbleParams::default() }),
            ("blur", RumbleParams { reverb_blur: 0.05, ..RumbleParams::default() }),
        ] {
            let d = diff(&tail(params));
            println!("reverb {what}: tail differs by {d:.3}");
            assert!(d > 0.05, "{what} does nothing ({d})");
        }
        assert!(diff(&tail(RumbleParams::default())) < 1.0e-9, "deterministic");
        let d = RumbleParams::default();
        assert_eq!((d.reverb_size, d.reverb_predelay_ms, d.reverb_motion_ms, d.reverb_blur), (2.0, 0.0, 0.7, 1.0), "the original sound");
    }

    /// A long kick (still ringing when the next one comes) and a quiet kick are both heard by the
    /// Audio trigger: the duck dips at every beat.
    #[test]
    fn the_audio_trigger_hears_long_and_quiet_kicks() {
        let dips = |tau: f32, level: f32, hiss: f32| {
            let mut core = RumbleCore::new(SR);
            core.set_params(RumbleParams {
                duck_source: DUCK_SOURCE_KICK,
                duck: 1.0,
                release_ms: 250.0,
                ..RumbleParams::default()
            });
            let beat = (SR * 60.0 / 130.0) as usize;
            let mut seen = 0;
            let mut lowest = 1.0f32;
            for i in 0..(SR * 8.0) as usize {
                let t = (i % beat) as f32 / SR;
                let hat = (std::f32::consts::TAU * 6000.0 * i as f32 / SR).sin() * hiss;
                let x = (std::f32::consts::TAU * 55.0 * t).sin() * (-t / tau).exp() * level + hat;
                core.process(x, x);
                if i >= beat * 3 {
                    lowest = lowest.min(core.duck_gain());
                    // The first 40 ms after each kick: the duck must have come down.
                    if i % beat == (0.04 * SR) as usize {
                        if lowest < 0.3 {
                            seen += 1;
                        }
                        lowest = 1.0;
                    }
                }
            }
            seen
        };
        let beats = 8.0 * 130.0 / 60.0 - 3.0;
        for (tau, level, hiss) in [(0.1, 0.5, 0.0), (0.8, 0.5, 0.0), (1.5, 0.5, 0.0), (0.4, 0.01, 0.0), (0.4, 0.5, 0.1)] {
            let seen = dips(tau, level, hiss);
            println!("audio trigger: kick tail {tau} s, level {level}, hats {hiss}: {seen} dips of {beats:.0}");
            assert!(seen as f32 >= beats - 2.0, "the audio trigger misses beats (tail {tau}, level {level}): {seen}");
        }
    }
}
