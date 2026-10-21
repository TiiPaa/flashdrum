//! Rift ([221]) — a percussion or FX carved out of a long texture.
//!
//! Every other voice in the plugin builds a sound from nothing. Rift instead
//! *lifts a slice* out of one of four embedded textures of several seconds
//! (`sample_bank::texture`), and shapes it with an amplitude envelope, a
//! resonant multi-mode filter and the shared saturation chain. The same
//! texture gives a shaker, a bell, a click or a riser depending on where the
//! slice is taken and how it is cut.
//!
//! The parameter that carries the instrument is **Offset**. Being a plain
//! continuous special parameter, it is p-lockable per step and morphable
//! across a fusion for free — sixteen cells of a page become sixteen different
//! samplings of the same matter, with no code of our own behind it.
//!
//! Retrigger follows the [179] contract: the read position, filter and DC
//! blocker all restart from a clean state, and `RetrigDeclick` (3 ms) absorbs
//! the discontinuity. There is deliberately no phase continuity here.

use super::{dsp, sample_bank, saturation, settings::rift::RiftSettings, Voice, VoiceSettings};

/// Anti-click floor for the amplitude attack (a true 0 ms attack is a step).
const MIN_AMP_ATTACK_MS: f32 = 0.3;
/// Full-scale amp attack as an ABSOLUTE fade-in time; the parameter is a
/// fraction of this ceiling, not of the slice length (same reasoning as the
/// 606 samplers: a fraction of a 5 ms slice gives no usable range).
const MAX_AMP_ATTACK_SECS: f32 = 0.08;
/// Grain length at Grain = 0 and the factor spanning it up to Grain = 1,
/// i.e. 5 ms … 4 s, exponentially.
const MIN_GRAIN_SECS: f32 = 0.005;
const GRAIN_SPAN: f32 = 800.0;
/// Amp and filter decays are ABSOLUTE times, not fractions of the slice: a
/// 1.5 s tail must stay reachable whatever the window is set to.
const MAX_DECAY_SECS: f32 = 1.5;
const MIN_DECAY_SECS: f32 = 0.005;
/// Hold is an absolute time too, capped at one second.
const MAX_HOLD_SECS: f32 = 1.0;
/// Shortest fade at each end of a grain. Cutting a texture mid-cycle is a
/// click even when the amp envelope is still open, and with Loop on that edge
/// comes round again at every repetition.
const MIN_GRAIN_FADE_SECS: f32 = 0.001;
/// Top of the filter sweep, as in `buzz.rs` — the envelope opens the cutoff
/// toward it and the cutoff falls back to its own setting afterwards.
const FILTER_OPEN_HZ: f32 = 20000.0;

/// The cutoff the Filter LFO gives: `lfo` (-1..1) swings the cutoff by +-`depth` octaves around
/// `base`. When the base is so low (or so high) that half of that swing would fall outside the
/// filter's range and be eaten by the clamp (the 20 Hz floor with a full depth: only the upward
/// half was heard, i.e. half the amplitude), the CENTRE of the swing moves just far enough that
/// the whole +-`depth` fits. Anywhere with room the base stays the centre, so nothing else changes.
fn lfo_cutoff(base: f32, lfo: f32, depth: f32) -> f32 {
    let (lo, hi) = (20.0f32.log2(), FILTER_OPEN_HZ.log2());
    let depth = depth.min(0.5 * (hi - lo));
    let centre = base.clamp(20.0, FILTER_OPEN_HZ).log2().clamp(lo + depth, hi - depth);
    2f32.powf(centre + lfo.clamp(-1.0, 1.0) * depth).clamp(20.0, FILTER_OPEN_HZ)
}
/// Anti-click floor for the filter-envelope attack, as in `buzz.rs`.
const MIN_FILTER_ATTACK_S: f32 = 0.0005;
/// Steepness of the pitch sweep, fixed as on the other voices. Public so the
/// Sound Panel's pitch-envelope graph draws the SAME law.
pub const PITCH_ENV_CURVE: f32 = 4.0;
/// The biquad is re-tuned every N samples rather than every sample: its
/// coefficients cost two transcendentals, the envelope driving them moves
/// slowly, and fourteen slots can play at once. 8 samples is 5.5 kHz of
/// update rate at 44.1 kHz — far above anything the envelope does.
const FILTER_UPDATE_SAMPLES: u32 = 8;
/// Stereo Spread at full setting reads the right channel this far AHEAD of
/// the left in the texture. The mapping is squared: the first half of the
/// slider stays in the "wide, still the same matter" zone, the top half turns
/// into two different sounds under one envelope.
const SPREAD_MAX_SECS: f32 = 0.5;

/// Grain length in seconds for a `Grain` parameter value ([225]).
///
/// Public because the Sound Panel draws the grain window on the texture, and it
/// has to use the SAME mapping as the voice or the picture would lie.
pub fn grain_seconds(grain: f32) -> f32 {
    MIN_GRAIN_SECS * GRAIN_SPAN.powf(grain.clamp(0.0, 1.0))
}

pub struct RiftVoice {
    settings: RiftSettings,
    sample_rate: f32,

    /// Read position in SOURCE samples (fractional).
    pos: f32,
    /// Where the slice starts and ends, in SOURCE samples.
    win_start: f32,
    win_end: f32,
    /// Source-sample increment per output sample, pitch modulation EXCLUDED —
    /// the envelope and LFO scale it per sample.
    base_step: f32,
    /// Fade length at each end of the slice, in SOURCE samples.
    fade_len: f32,
    /// Which texture this trigger is reading (index in the Texture menu).
    texture_idx: usize,
    /// [228] The texture itself, resolved at trigger: embedded, or a user
    /// file held alive by its `Arc` for the length of the hit.
    source: sample_bank::TextureSource,
    /// The instance's lane textures and this voice's lane; `None` = embedded
    /// only (test harness).
    pool: Option<(std::sync::Arc<sample_bank::TexturePool>, usize)>,
    /// xorshift32 state for Wander — seeded at construction, never reseeded on
    /// trigger, so a rendered pattern is reproducible from the start.
    rng: u32,

    /// Which advance step the next hit takes. [227] The plugin sets it
    /// through `set_hit_index` before every trigger (it knows the pattern,
    /// the page and the lane's options); on its own - test harness, direct
    /// triggers - the voice simply counts its hits.
    advance_count: u32,

    amp_env: dsp::DecayReleaseEnvelope,
    /// Pitch sweep, in semitones scaled by `pitch_env`.
    pitch_env: dsp::ExpDecayEnvelope,
    /// The two LFOs share a shape and a phase policy; only their rate and depth
    /// differ.
    pitch_lfo: dsp::Lfo,
    filter_lfo: dsp::Lfo,
    /// Last filter-LFO value, sampled every frame but consumed only when the
    /// biquad is re-tuned.
    filter_lfo_value: f32,
    filter: dsp::Biquad,
    /// Right-channel twin of `filter`: same tuning (copied at every re-tune),
    /// its own delay state. Only runs while Stereo Spread is on.
    filter_r: dsp::Biquad,
    /// Elapsed time in the A-H-D filter envelope, as in `buzz.rs` — a manual
    /// ramp rather than a decay object, because attack and decay each carry
    /// their own bipolar curve.
    filter_env_time: f32,
    filter_tick: u32,
    saturation: saturation::SaturationConfig,
    dc_block: dsp::DcBlocker,
    dc_block_r: dsp::DcBlocker,
    /// Lo-fi stage (build 3), one hold per channel. The crush levels and the
    /// hold factor are recomputed on setting changes, never per sample.
    decimator: [dsp::Decimator; 2],
    crush_levels: f32,
    /// Peak of the slice being read (from the texture's column summary),
    /// measured at trigger. Crush quantises RELATIVE to it: a quiet region of
    /// the texture is crushed as hard as a loud one instead of vanishing into
    /// the zero step, so sixteen p-locked offsets all keep sounding.
    crush_norm: f32,
    decimate_factor: f32,
    /// Source rate of the texture being read: the spread is a TIME.
    texture_rate: f32,
    /// One declick per channel: with Stereo Spread on the two sides end on
    /// different residues, and a shared mid ramp would step both.
    declick: [dsp::RetrigDeclick; 2],
    last_out: [f32; 2],

    active: bool,
}

impl RiftVoice {
    pub fn new(sample_rate: f32, settings: RiftSettings) -> Self {
        let mut amp_env = dsp::DecayReleaseEnvelope::new(
            sample_rate,
            settings.decay_curve,
            settings.decay.clamp(0.01, 5.0),
            settings.release_curve,
            settings.release.max(0.001),
        )
        .with_attack_ms((settings.attack * 1000.0).max(MIN_AMP_ATTACK_MS));
        amp_env.set_hold(settings.hold);

        let mut voice = Self {
            settings,
            sample_rate,
            pos: 0.0,
            win_start: 0.0,
            win_end: 0.0,
            base_step: 1.0,
            advance_count: 0,
            fade_len: 1.0,
            texture_idx: 0,
            source: sample_bank::TextureSource::Embedded(sample_bank::texture(0)),
            pool: None,
            rng: 0x21F1_0001,
            amp_env,
            pitch_env: dsp::ExpDecayEnvelope::new(sample_rate, PITCH_ENV_CURVE, 0.08),
            pitch_lfo: dsp::Lfo::new(sample_rate, 0x21F1_5EED),
            filter_lfo: dsp::Lfo::new(sample_rate, 0x21F1_C0DE),
            filter_lfo_value: 0.0,
            filter: dsp::Biquad::new(),
            filter_r: dsp::Biquad::new(),
            filter_env_time: 1.0e6,
            filter_tick: 0,
            saturation: saturation::SaturationConfig {
                saturation_type: saturation::SaturationType::None,
                amount: 0.0,
                mix: 1.0,
                output_gain: 1.0,
                pre_filter: false,
                compensation_gain: 1.0,
            },
            dc_block: dsp::DcBlocker::default(),
            dc_block_r: dsp::DcBlocker::default(),
            decimator: [dsp::Decimator::default(); 2],
            crush_levels: 0.0,
            crush_norm: 1.0,
            decimate_factor: 1.0,
            texture_rate: 44100.0,
            declick: [dsp::RetrigDeclick::new(sample_rate); 2],
            last_out: [0.0; 2],
            active: false,
        };
        voice.amp_env.set_decay(voice.amp_decay_secs());
        voice.apply_filter(0.0);
        voice.refresh_lofi();
        voice.sync_saturation();
        voice
    }

    /// The saturation pack from the settings - shared by construction and
    /// `set_settings`, so a freshly built voice already routes its Distortion
    /// block where the Pre-Filter switch says (the test harness never calls
    /// `set_settings`; the plugin does right after creation).
    fn sync_saturation(&mut self) {
        self.saturation.saturation_type =
            saturation::SaturationType::from(self.settings.saturation_type);
        self.saturation.amount = self.settings.saturation_amount;
        self.saturation.mix = self.settings.saturation_mix;
        self.saturation.output_gain = self.settings.saturation_output_gain;
        self.saturation.pre_filter = self.settings.saturation_pre_filter > 0.5;
        self.saturation.update_compensation();
    }

    /// Recompute the lo-fi constants from the settings (a `powf` each, which
    /// is why they are not derived in the per-sample path).
    fn refresh_lofi(&mut self) {
        self.crush_levels = dsp::crush_levels(self.settings.crush);
        self.decimate_factor = dsp::decimate_factor(self.settings.decimate);
    }

    fn spread_on(&self) -> bool {
        self.settings.stereo_spread > 1e-3
    }

    /// Distance between the two read heads, in SOURCE samples.
    fn spread_samples(&self) -> f32 {
        let s = self.settings.stereo_spread.clamp(0.0, 1.0);
        s * s * SPREAD_MAX_SECS * self.texture_rate
    }

    /// Loudest point of the texture between two SOURCE positions, read from
    /// the decode-time column summary rather than the samples themselves.
    fn window_peak(bank: &sample_bank::TextureBank, start: f32, end: f32) -> f32 {
        let len = bank.data.len().max(1) as f32;
        let cols = bank.peaks.len();
        if cols == 0 {
            return 1.0;
        }
        let c0 = ((start / len) * cols as f32).floor().max(0.0) as usize;
        let c1 = ((end / len) * cols as f32).ceil().max(0.0) as usize;
        let (c0, c1) = (c0.min(cols - 1), c1.min(cols - 1).max(c0));
        bank.peaks[c0..=c1]
            .iter()
            .map(|(lo, hi)| lo.abs().max(hi.abs()))
            .fold(0.0f32, f32::max)
    }

    /// Linear-interpolated read; the caller guarantees `pos + 1 < data.len()`.
    #[inline]
    fn read(data: &[f32], pos: f32) -> f32 {
        let idx = pos as usize;
        let frac = pos - idx as f32;
        data[idx] + frac * (data[idx + 1] - data[idx])
    }

    /// One channel of the texture at `pos`. `channel`: `None` = the mono
    /// material (a stereo file mixed down at read time), `Some(false)` = left,
    /// `Some(true)` = right. Mono banks answer the same for all three.
    #[inline]
    fn sample(bank: &sample_bank::TextureBank, channel: Option<bool>, pos: f32) -> f32 {
        match (channel, bank.right.as_deref()) {
            (Some(true), Some(right)) => Self::read(right, pos),
            (None, Some(right)) => 0.5 * (Self::read(&bank.data, pos) + Self::read(right, pos)),
            _ => Self::read(&bank.data, pos),
        }
    }

    /// [228] The Stereo switch is on AND the texture is a stereo file: the two
    /// channels play as they are, left to left, right to right.
    fn stereo_file(&self) -> bool {
        self.settings.stereo > 0.5 && self.source.bank().right.is_some()
    }

    fn texture_index(&self) -> usize {
        // The value is the index into the registry's `options` list (embedded
        // textures, then the user slots). Clamp rather than wrap: an
        // out-of-range value must still play something.
        (self.settings.texture.round().max(0.0) as usize).min(sample_bank::TEXTURE_OPTION_COUNT - 1)
    }

    /// Resting playback ratio: coarse semitones plus fine cents.
    fn pitch_ratio(&self) -> f32 {
        let semis = self.settings.frequency.clamp(-24.0, 24.0)
            + self.settings.pitch_fine.clamp(-100.0, 100.0) / 100.0;
        2f32.powf(semis / 12.0)
    }

    fn lfo_shape(&self) -> dsp::LfoShape {
        dsp::LfoShape::from_index(self.settings.lfo_shape.round().max(0.0) as u8)
    }

    fn loop_on(&self) -> bool {
        self.settings.loop_mode > 0.5
    }

    fn reverse(&self) -> bool {
        self.settings.reverse > 0.5
    }

    /// Grain length in seconds, before pitch: 5 ms … 4 s, exponential so the
    /// short end (where the granular character lives) gets most of the slider.
    fn grain_secs(&self) -> f32 {
        grain_seconds(self.settings.grain)
    }

    /// How long the amp envelope keeps the voice audible: it is time-based
    /// and reaches zero exactly at the end of its decay. This sizes the
    /// reversed read when Loop is off.
    fn audible_secs(&self) -> f32 {
        self.amp_attack_secs() + self.hold_secs() + self.amp_decay_secs()
    }

    fn amp_attack_secs(&self) -> f32 {
        self.settings.attack.clamp(0.0, 1.0) * MAX_AMP_ATTACK_SECS
    }

    fn amp_decay_secs(&self) -> f32 {
        self.settings.decay.clamp(MIN_DECAY_SECS, MAX_DECAY_SECS)
    }

    fn filter_env_decay_secs(&self) -> f32 {
        self.settings
            .filter_env_decay
            .clamp(MIN_DECAY_SECS, MAX_DECAY_SECS)
    }

    fn hold_secs(&self) -> f32 {
        self.settings.hold.clamp(0.0, MAX_HOLD_SECS)
    }

    /// A-H-D filter envelope value at the current time, mirroring `buzz.rs`:
    /// linear attack and decay ramps, each shaped by its own bipolar curve.
    fn filter_env_value(&self) -> f32 {
        let atk = self.settings.filter_attack.max(MIN_FILTER_ATTACK_S);
        let hold = self.settings.filter_hold.max(0.0);
        let dec = self.filter_env_decay_secs();
        let t = self.filter_env_time;
        if t < atk {
            dsp::shape_curve(t / atk, self.settings.filter_atk_curve)
        } else if t < atk + hold {
            1.0
        } else {
            let p = ((t - atk - hold) / dec).clamp(0.0, 1.0);
            dsp::shape_curve(1.0 - p, self.settings.filter_dec_curve)
        }
    }

    fn next_rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng >> 8) as f32 / 16_777_216.0
    }

    /// Cutoff for the current envelope value — **the plugin's own filter
    /// convention**, taken from `buzz.rs` rather than reinvented.
    ///
    /// The envelope sweeps the cutoff EXPONENTIALLY from its setting up toward
    /// 20 kHz and it falls back onto that setting; `Filter Env` (0..1) says how
    /// far toward open it goes. A low cutoff plus a high amount is the
    /// full-range percussive drop.
    fn filter_cutoff(&self, env: f32) -> f32 {
        let base = self.settings.filter_freq.clamp(20.0, FILTER_OPEN_HZ);
        let amt = (env * self.settings.filter_env_amount).clamp(0.0, 1.0);
        (base * (FILTER_OPEN_HZ / base).powf(amt)).clamp(20.0, FILTER_OPEN_HZ)
    }

    /// Re-tune the biquad for the current envelope value, LFO included.
    fn apply_filter(&mut self, env: f32) {
        let depth = self.settings.filter_lfo_depth.clamp(0.0, 3.0);
        let cutoff = if depth > 0.0 {
            // The LFO rides the swept cutoff in OCTAVES, so its depth means the
            // same thing wherever the envelope has taken the filter.
            lfo_cutoff(self.filter_cutoff(env), self.filter_lfo_value, depth)
        } else {
            self.filter_cutoff(env)
        };
        let q = self.settings.resonance.clamp(0.5, 20.0);
        match self.settings.filter_type.round() as i32 {
            1 => self.filter.set_highpass(cutoff, q, self.sample_rate),
            2 => self.filter.set_bandpass(cutoff, q, self.sample_rate),
            _ => self.filter.set_lowpass(cutoff, q, self.sample_rate),
        }
        self.filter_r.copy_coefficients_from(&self.filter);
    }

    /// Raised-cosine fade at both ends of the grain, so cutting the texture
    /// mid-cycle never clicks — and, with Loop on, so the seam does not either.
    fn grain_gain(&self) -> f32 {
        let from_start = self.pos - self.win_start;
        let to_end = self.win_end - self.pos;
        let edge = from_start.min(to_end);
        if edge >= self.fade_len {
            return 1.0;
        }
        let t = (edge / self.fade_len).clamp(0.0, 1.0);
        0.5 - 0.5 * (std::f32::consts::PI * t).cos()
    }

    /// One output frame. `stereo` = read a second head for the right channel,
    /// `Stereo Spread` ahead of the left in the texture, under the SAME
    /// envelopes, fades and filter tuning. Mono leaves every right-side state
    /// untouched and returns the left twice.
    fn render(&mut self, stereo: bool) -> (f32, f32) {
        if !self.active {
            let t0 = self.declick[0].next();
            let t1_own = self.declick[1].next();
            let t1 = if stereo { t1_own } else { t0 };
            self.last_out = [t0, t1];
            return (t0, t1);
        }

        let data_len = self.source.bank().data.len();
        // Loop on: fold the read back to the far end of the grain instead of
        // ending the voice. The fade at both ends is what makes the seam
        // inaudible, which is why it never drops to zero length. Reversed
        // playback runs the other way, so it is the START that it falls off.
        let span = self.win_end - self.win_start;
        let past_end = if self.reverse() {
            self.pos - 1.0 <= self.win_start
        } else {
            self.pos + 1.0 >= self.win_end
        };
        if past_end {
            if self.loop_on() && span > 2.0 {
                if self.reverse() {
                    self.pos += span;
                    if self.pos > self.win_end - 2.0 {
                        self.pos = self.win_end - 2.0;
                    }
                } else {
                    self.pos -= span;
                    if self.pos < self.win_start {
                        self.pos = self.win_start;
                    }
                }
            } else {
                return self.cut();
            }
        }
        let idx = self.pos as usize;
        if self.pos < 0.0 || idx + 1 >= data_len {
            return self.cut();
        }
        let gain = self.grain_gain();
        let spread = if stereo { self.spread_samples() } else { 0.0 };
        // Stereo file with the switch on: left reads left, right reads right
        // (Stereo Spread then offsets the right head on top). Otherwise both
        // heads read the mono material.
        let two_channels = self.stereo_file();
        let (raw_l, raw_r) = {
            let bank = self.source.bank();
            let (ch_l, ch_r) = if two_channels {
                (Some(false), Some(true))
            } else {
                (None, None)
            };
            let raw_l = Self::sample(bank, ch_l, self.pos) * gain;
            let raw_r = if stereo {
                // The right head wraps around the texture end rather than
                // stopping: the spread must never shorten a sound.
                let max = (data_len - 2) as f32;
                let mut p = self.pos + spread;
                if p >= max {
                    p -= max;
                }
                Self::sample(bank, ch_r, p.clamp(0.0, max)) * gain
            } else {
                raw_l
            };
            (raw_l, raw_r)
        };

        // Pitch modulation. Both sources advance every sample so their timing
        // stays right; the exponential is only paid when they actually move.
        let shape = self.lfo_shape();
        let sweep = self.pitch_env.next() * self.settings.pitch_env;
        let vibrato = self.pitch_lfo.next(shape) * self.settings.pitch_lfo_depth.max(0.0);
        self.filter_lfo_value = self.filter_lfo.next(shape);
        let semis = sweep + vibrato;
        let advance = if semis.abs() > 1e-4 {
            self.base_step * 2f32.powf(semis / 12.0)
        } else {
            self.base_step
        };
        self.pos += if self.reverse() { -advance } else { advance };

        let amp = self.amp_env.next();
        if amp <= 0.0 {
            return self.cut();
        }

        if self.filter_tick == 0 {
            let env = self.filter_env_value();
            self.apply_filter(env);
        }
        self.filter_env_time += 1.0 / self.sample_rate;
        self.filter_tick = (self.filter_tick + 1) % FILTER_UPDATE_SAMPLES;

        let l = self.render_channel(0, raw_l, amp);
        let r = if stereo {
            self.render_channel(1, raw_r, amp)
        } else {
            l
        };
        let t0 = self.declick[0].next();
        let t1_own = self.declick[1].next();
        let t1 = if stereo { t1_own } else { t0 };
        let (out_l, out_r) = (l + t0, r + t1);
        // What the next hit's (or the cut's) declick starts from, per channel.
        self.last_out = [out_l, out_r];
        (out_l, out_r)
    }

    /// End the voice NOW - envelope over, slice exhausted, read out of range -
    /// without a step. The filter and the DC blocker still hold a residue when
    /// the amp envelope reaches zero (measured: about -0.001 with the filter
    /// open, twelve times less with it closed), and dropping to exact silence
    /// in one sample was an audible tick at the end of every filter-swept hit.
    /// The declick ramps that residue out over 3 ms, as it does at trigger.
    fn cut(&mut self) -> (f32, f32) {
        self.active = false;
        for ch in 0..2 {
            self.declick[ch].arm(self.last_out[ch]);
        }
        let t0 = self.declick[0].next();
        let t1 = self.declick[1].next();
        self.last_out = [t0, t1];
        (t0, t1)
    }

    /// The Distortion block for one channel: Decimate, then Crush, then the
    /// saturation, as ONE unit that sits before or after the filter on the
    /// **Pre-Filter** switch ([232], user request: the switch must take Crush
    /// and Decimate along, not the saturation alone). Runs on the enveloped
    /// signal like the saturation always did; at two bits a decaying tail
    /// does fall into the zero step - that is the effect.
    #[inline]
    fn render_channel(&mut self, ch: usize, raw: f32, amp: f32) -> f32 {
        let x = raw * amp;
        let pre_filter = self.saturation.pre_filter;
        let x = if pre_filter { self.lofi(ch, x) } else { x };
        let pre = self.saturation.process_at(true, x);
        let filtered = if ch == 0 {
            self.filter.process(pre)
        } else {
            self.filter_r.process(pre)
        };
        let y = if pre_filter {
            filtered
        } else {
            self.lofi(ch, filtered)
        };
        let post = self.saturation.process_at(false, y);
        let dc = if ch == 0 {
            self.dc_block.process(post)
        } else {
            self.dc_block_r.process(post)
        };
        dc * self.settings.volume
    }

    /// Decimate then Crush for one channel; the crush is relative to the
    /// slice's own peak so every offset is crushed as hard.
    #[inline]
    fn lofi(&mut self, ch: usize, x: f32) -> f32 {
        let x = self.decimator[ch].process(x, self.decimate_factor);
        if self.crush_levels > 0.0 {
            dsp::crush_sample(x / self.crush_norm, self.crush_levels) * self.crush_norm
        } else {
            x
        }
    }

    /// [253] Retire a replaced USER texture to the pool's queue (drained on
    /// the UI thread) instead of dropping it on the audio thread — the voice
    /// may hold its last reference. Embedded textures are `'static`, nothing
    /// to free. No pool = test harness: dropped in place.
    fn retire_source(&self, old: sample_bank::TextureSource) {
        if let sample_bank::TextureSource::User(bank) = old {
            if let Some((pool, _)) = &self.pool {
                pool.retire(bank);
            }
        }
    }
}

impl Voice for RiftVoice {
    fn trigger(&mut self) {
        // [179] The read restarts from scratch; the declick covers the jump.
        for ch in 0..2 {
            self.declick[ch].arm(self.last_out[ch]);
        }
        self.filter.reset();
        self.filter_r.reset();
        self.dc_block.reset();
        self.dc_block_r.reset();
        for d in &mut self.decimator {
            d.reset();
        }

        self.texture_idx = self.texture_index();
        // [228] Embedded texture, or the lane's own file; a lane without a
        // file plays the first embedded one rather than nothing. The Arc is
        // kept for the hit.
        let source = sample_bank::resolve_texture(
            self.texture_idx,
            self.pool
                .as_ref()
                .map(|(pool, lane)| (pool.as_ref(), *lane)),
        );
        let bank = source.bank();
        let len = bank.data.len();
        if len < 4 {
            // Unreadable texture: stay inert rather than read out of bounds.
            self.active = false;
            return;
        }
        self.active = true;
        self.texture_rate = bank.source_rate.max(1.0);

        self.base_step = (bank.source_rate / self.sample_rate) * self.pitch_ratio();

        // Wander re-rolls around Offset at every trigger; at 0 the cell always
        // lands on the same spot, which is what makes a pattern reproducible.
        let wander = self.settings.wander.clamp(0.0, 1.0);
        // Advance walks the offset forward one notch per hit, so a single
        // repeated step still travels through the texture. It wraps rather
        // than stopping at the end.
        let advance = self.settings.advance.clamp(0.0, 1.0) * self.advance_count as f32;
        let mut offset = (self.settings.offset.clamp(0.0, 1.0) + advance).rem_euclid(1.0);
        self.advance_count = self.advance_count.wrapping_add(1);
        if wander > 0.0 {
            offset += (self.next_rand() - 0.5) * wander;
            offset = offset.rem_euclid(1.0);
        }
        // Offset spans the WHOLE texture. It used to span the texture minus
        // the envelope's tail (kept in reserve so the sound would never be
        // truncated) - on a 12 s texture nobody noticed, on a real-world
        // sample shorter than that reserve Offset, Wander and Advance had no
        // room left at all and did nothing ([228], user report 2026-09-21).
        let rate = bank.source_rate.max(1.0);
        let last = len as f32 - 2.0;
        self.win_start = (offset * last).clamp(0.0, (last - 2.0).max(0.0));
        // Loop on: the read span IS the grain, and it repeats. Loop off, forward:
        // the read runs to the end of the texture and the cut there is
        // declicked - the envelope decides the length in practice. Loop off,
        // reversed: the envelope's worth of texture, read backwards from its
        // far end down to the offset, so Offset still picks the material on a
        // long texture; on a short sample a decay at least as long as the file
        // plays it backwards from its end, as a sampler does.
        self.win_end = if self.loop_on() {
            (self.win_start + self.grain_secs() * rate).min(last)
        } else if self.reverse() {
            (self.win_start + self.audible_secs() * rate).min(last)
        } else {
            last
        };
        if self.win_end - self.win_start < 2.0 {
            self.win_end = (self.win_start + 2.0).min(last);
            self.win_start = (self.win_end - 2.0).max(0.0);
        }
        let win_samples = self.win_end - self.win_start;
        self.crush_norm = Self::window_peak(bank, self.win_start, self.win_end).max(1e-3);
        self.pos = if self.reverse() {
            (self.win_end - 2.0).max(self.win_start)
        } else {
            self.win_start
        };
        self.fade_len = if self.loop_on() {
            (win_samples * 0.5 * self.settings.grain_shape.clamp(0.0, 1.0))
                .max(MIN_GRAIN_FADE_SECS * rate)
                .min(win_samples * 0.5)
        } else {
            // Just enough to take the edge off the very start and the texture
            // end; the amp envelope shapes everything in between.
            (MIN_GRAIN_FADE_SECS * rate).min(win_samples * 0.5)
        };

        self.amp_env.set_decay(self.amp_decay_secs());
        self.amp_env
            .set_attack_ms((self.amp_attack_secs() * 1000.0).max(MIN_AMP_ATTACK_MS));
        self.amp_env.set_hold(self.hold_secs());
        self.amp_env.trigger();
        self.filter_env_time = 0.0;
        self.filter_tick = 0;

        self.pitch_env
            .set_decay(self.settings.pitch_env_time.clamp(0.005, 1.5));
        self.pitch_env.set_curve(PITCH_ENV_CURVE);
        // Deterministic start: a pitch sweep that began from wherever the last
        // one left off would not repeat from hit to hit.
        self.pitch_env.trigger_reset_to(1.0);

        self.pitch_lfo
            .set_rate(self.settings.pitch_lfo_rate.clamp(0.05, 40.0));
        self.filter_lfo
            .set_rate(self.settings.filter_lfo_rate.clamp(0.05, 40.0));
        // Free phase lets the modulation drift from hit to hit; locked, every
        // hit hears the same movement.
        if self.settings.lfo_free_phase < 0.5 {
            self.pitch_lfo.retrigger();
            self.filter_lfo.retrigger();
        }
        // Keep the texture alive for the hit (last use of `bank` is above).
        // [253] The replaced texture is retired, not dropped on this thread.
        let old = std::mem::replace(&mut self.source, source);
        self.retire_source(old);
    }

    fn trigger_hard(&mut self) {
        self.trigger();
        self.amp_env.trigger_hard();
    }

    /// [253] Before the voice is swapped out on the audio thread
    /// (`reinitialize_slot`), retire the held texture instead of dropping it.
    fn release_held_texture(&mut self) {
        let old = std::mem::replace(
            &mut self.source,
            sample_bank::TextureSource::Embedded(sample_bank::texture(0)),
        );
        self.retire_source(old);
    }

    fn set_hit_index(&mut self, index: u32) {
        self.advance_count = index;
    }

    fn set_texture_pool(&mut self, pool: std::sync::Arc<sample_bank::TexturePool>, lane: usize) {
        self.pool = Some((pool, lane));
    }

    fn process_sample(&mut self) -> f32 {
        self.render(false).0
    }

    /// Two channels when Stereo Spread is on (a second read head for the
    /// right) or when a stereo file plays with the Stereo switch on (its own
    /// right channel). Otherwise the right channel is the left, byte for byte.
    fn process_sample_stereo(&mut self) -> (f32, f32) {
        let stereo = self.spread_on() || self.stereo_file();
        self.render(stereo)
    }

    fn is_active(&self) -> bool {
        self.active || self.declick.iter().any(|d| d.is_active())
    }

    fn reset(&mut self) {
        self.active = false;
        self.pos = 0.0;
        self.win_start = 0.0;
        self.win_end = 0.0;
        self.last_out = [0.0; 2];
        self.advance_count = 0;
        self.pitch_env.reset();
        self.pitch_lfo.reset();
        self.filter_lfo.reset();
        self.filter_lfo_value = 0.0;
        self.filter.reset();
        self.filter_r.reset();
        self.filter_env_time = 1.0e6;
        self.amp_env.reset();
        self.dc_block.reset();
        self.dc_block_r.reset();
        for d in &mut self.decimator {
            d.reset();
        }
        for d in &mut self.declick {
            d.reset();
        }
    }

    fn set_settings(&mut self, settings: VoiceSettings) {
        self.settings = RiftSettings::from(settings);

        // Live retune (macro/automation driving Pitch, [242]): recomputing
        // the read increment only at trigger made the knob glide while the
        // sound followed in steps (same report as One-Shot, 2026-09-23).
        // Changing the increment mid-play is phase-continuous.
        if self.active && self.texture_rate > 0.0 {
            self.base_step = (self.texture_rate / self.sample_rate) * self.pitch_ratio();
        }

        // Setters only — recreating an envelope resets its state and cuts the
        // sound mid-slider-drag.
        self.amp_env.set_decay(self.amp_decay_secs());
        self.amp_env
            .set_attack_ms((self.amp_attack_secs() * 1000.0).max(MIN_AMP_ATTACK_MS));
        self.amp_env.set_release(self.settings.release.max(0.001));
        self.amp_env.set_decay_curve(self.settings.decay_curve);
        self.amp_env.set_release_curve(self.settings.release_curve);
        self.amp_env.set_hold(self.hold_secs());

        self.apply_filter(self.filter_env_value());
        self.refresh_lofi();
        self.sync_saturation();
    }

    fn set_algo(&mut self, algo: u8) {
        self.settings.algo = algo;
    }

    fn set_special_param(&mut self, index: usize, value: f32) {
        match index {
            0 => self.settings.texture = value,
            1 => self.settings.offset = value,
            2 => self.settings.wander = value,
            3 => self.settings.grain = value,
            4 => self.settings.grain_shape = value,
            12 => self.settings.loop_mode = value,
            27 => self.settings.reverse = value,
            5 => self.settings.filter_type = value,
            6 => self.settings.resonance = value,
            13 => self.settings.filter_attack = value,
            14 => self.settings.filter_hold = value,
            15 => self.settings.filter_atk_curve = value,
            16 => self.settings.filter_dec_curve = value,
            17 => self.settings.pitch_fine = value,
            18 => self.settings.pitch_env = value,
            19 => self.settings.pitch_env_time = value,
            20 => self.settings.advance = value,
            21 => self.settings.lfo_shape = value,
            22 => self.settings.lfo_free_phase = value,
            23 => self.settings.pitch_lfo_rate = value,
            24 => self.settings.pitch_lfo_depth = value,
            25 => self.settings.filter_lfo_rate = value,
            26 => self.settings.filter_lfo_depth = value,
            28 => {
                self.settings.crush = value;
                self.crush_levels = dsp::crush_levels(value);
            }
            29 => {
                self.settings.decimate = value;
                self.decimate_factor = dsp::decimate_factor(value);
            }
            30 => self.settings.stereo_spread = value,
            7 => {
                self.settings.saturation_type = value as u8;
                self.saturation.saturation_type =
                    saturation::SaturationType::from(self.settings.saturation_type);
                self.saturation.update_compensation();
            }
            8 => {
                self.settings.saturation_amount = value;
                self.saturation.amount = value;
                self.saturation.update_compensation();
            }
            9 => {
                self.settings.saturation_mix = value;
                self.saturation.mix = value;
                self.saturation.update_compensation();
            }
            10 => {
                self.settings.saturation_output_gain = value;
                self.saturation.output_gain = value;
                self.saturation.update_compensation();
            }
            11 => {
                self.settings.saturation_pre_filter = value;
                self.saturation.pre_filter = value > 0.5;
                self.saturation.update_compensation();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice_with(settings: VoiceSettings) -> RiftVoice {
        RiftVoice::new(44100.0, RiftSettings::from(settings))
    }

    /// Render one hit and return its samples.
    fn render(voice: &mut RiftVoice, n: usize) -> Vec<f32> {
        voice.trigger();
        (0..n).map(|_| voice.process_sample()).collect()
    }

    fn peak(buf: &[f32]) -> f32 {
        buf.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
    }

    /// The filter must behave like the rest of the plugin: `Filter Env` opens
    /// the cutoff toward 20 kHz and it falls back onto its setting. Same
    /// formula as `buzz.rs` — this test is what keeps the two aligned.
    ///
    /// Regression for the first two versions: one added a fixed number of
    /// hertz (measured 0 octaves of travel from a wide-open filter), the other
    /// invented a bipolar sweep no other voice had.
    #[test]
    fn filter_env_opens_the_cutoff_like_the_other_voices() {
        let mut st = VoiceSettings::rift();
        st.filter_freq = 500.0;
        st.filter_env_amount = 1.0;
        let v = voice_with(st);
        let open = v.filter_cutoff(1.0);
        let rest = v.filter_cutoff(0.0);
        assert!(
            (open - 20000.0).abs() < 1.0,
            "at full amount the envelope must reach 20 kHz, got {open}"
        );
        assert!(
            (rest - 500.0).abs() < 1.0,
            "the cutoff must fall back onto its setting, got {rest}"
        );
        assert!(
            (open / rest).log2() > 5.0,
            "only {} octaves",
            (open / rest).log2()
        );
    }

    /// `Advance` walks the offset forward one notch per hit, so a single
    /// repeated step still travels through the texture.
    #[test]
    fn advance_moves_through_the_texture() {
        let mut s = VoiceSettings::rift();
        s.special[20] = 0.05; // Advance
        let mut voice = voice_with(s);
        let first = render(&mut voice, 22050);
        let second = render(&mut voice, 22050);
        assert_ne!(first, second, "Advance did not move the read");

        // At 0 the same step must repeat identically.
        let mut s = VoiceSettings::rift();
        s.special[20] = 0.0;
        let mut voice = voice_with(s);
        let first = render(&mut voice, 22050);
        let second = render(&mut voice, 22050);
        assert_eq!(first, second, "Advance = 0 must repeat the same hit");
    }

    /// The pitch envelope must actually change the playback rate: measured on
    /// how far into the texture the read has travelled.
    #[test]
    fn pitch_env_sweeps_the_playback_rate() {
        let travelled = |semitones: f32| {
            let mut s = VoiceSettings::rift();
            s.special[18] = semitones; // Pitch Env
            s.special[19] = 0.5; // over half a second
            s.special[3] = 0.9; // a long grain, so the window is not the limit
            s.decay = 1.0;
            let mut v = voice_with(s);
            v.trigger();
            let start = v.pos;
            for _ in 0..4410 {
                v.process_sample();
            }
            v.pos - start
        };
        let up = travelled(24.0);
        let flat = travelled(0.0);
        let down = travelled(-24.0);
        assert!(up > flat * 1.5, "sweeping up read {up} against {flat}");
        assert!(
            down < flat * 0.7,
            "sweeping down read {down} against {flat}"
        );
    }

    /// The pitch LFO must modulate, and its depth must matter.
    #[test]
    fn pitch_lfo_modulates_the_read() {
        let mut s = VoiceSettings::rift();
        s.special[23] = 8.0; // rate
        s.special[24] = 0.0; // depth 0
        let flat = render(&mut voice_with(s), 8000);

        let mut s = VoiceSettings::rift();
        s.special[23] = 8.0;
        s.special[24] = 12.0; // an octave
        let moved = render(&mut voice_with(s), 8000);
        assert_ne!(flat, moved, "the pitch LFO changed nothing");
    }

    /// The filter LFO must reach the cutoff.
    #[test]
    fn filter_lfo_moves_the_cutoff() {
        let mut s = VoiceSettings::rift();
        s.special[25] = 6.0; // rate
        s.special[26] = 0.0; // depth 0
        let flat = render(&mut voice_with(s), 8000);

        let mut s = VoiceSettings::rift();
        s.special[25] = 6.0;
        s.special[26] = 2.0; // two octaves
        let moved = render(&mut voice_with(s), 8000);
        assert_ne!(flat, moved, "the filter LFO changed nothing");
    }

    /// Locked phase means every hit hears the same modulation; free phase means
    /// it drifts.
    #[test]
    fn lfo_free_phase_drifts_and_locked_phase_repeats() {
        let mut s = VoiceSettings::rift();
        s.special[23] = 7.0;
        s.special[24] = 6.0;
        s.special[22] = 0.0; // locked
        let mut voice = voice_with(s);
        let first = render(&mut voice, 22050);
        let second = render(&mut voice, 22050);
        assert_eq!(first, second, "a locked LFO must repeat exactly");

        let mut s = VoiceSettings::rift();
        s.special[23] = 7.0;
        s.special[24] = 6.0;
        s.special[22] = 1.0; // free
        let mut voice = voice_with(s);
        let first = render(&mut voice, 22050);
        let second = render(&mut voice, 22050);
        assert_ne!(first, second, "a free-running LFO must drift between hits");
    }

    /// Reverse must read the slice backwards — measured on the read position,
    /// which has to travel from the far end toward the start.
    #[test]
    fn reverse_reads_the_slice_backwards() {
        let mut s = VoiceSettings::rift();
        s.special[27] = 1.0; // Reverse
        s.special[3] = 0.6; // a long enough grain to watch
        s.decay = 0.5;
        let mut v = voice_with(s);
        v.trigger();
        let start = v.pos;
        for _ in 0..2000 {
            v.process_sample();
        }
        assert!(
            v.pos < start,
            "the read went forward: {} -> {}",
            start,
            v.pos
        );
        assert!(
            v.pos >= v.win_start - 2.0,
            "the read ran past the slice start"
        );
    }

    /// Reversed and forward renders of the same slice must differ, and reversed
    /// playback must still produce sound rather than falling silent at once.
    #[test]
    fn reverse_sounds_and_differs_from_forward() {
        let mut s = VoiceSettings::rift();
        s.special[3] = 0.5;
        s.decay = 0.3;
        let forward = render(&mut voice_with(s), 16000);

        let mut s = VoiceSettings::rift();
        s.special[3] = 0.5;
        s.special[27] = 1.0;
        s.decay = 0.3;
        let backward = render(&mut voice_with(s), 16000);

        assert!(peak(&backward) > 1e-3, "reversed playback is silent");
        assert_ne!(forward, backward, "Reverse changed nothing");
        assert!(backward.iter().all(|v| v.is_finite()));
    }

    /// A reversed grain must loop too, folding back to the far end.
    #[test]
    fn reverse_loops_without_dying() {
        let mut s = VoiceSettings::rift();
        s.special[27] = 1.0; // Reverse
        s.special[12] = 1.0; // Loop
        s.special[3] = 0.2; // ~14 ms
        s.decay = 0.5;
        let out = render(&mut voice_with(s), 22050);
        let late = peak(&out[13230..17640]);
        assert!(late > 1e-3, "the reversed loop died: {late}");
    }

    /// The filter envelope must have a real attack and real curves, like Buzz
    /// and SDrex — a decay alone left the panel's graph with nothing to draw.
    #[test]
    fn filter_env_has_an_attack_a_hold_and_curves() {
        let mut st = VoiceSettings::rift();
        st.filter_freq = 500.0;
        st.filter_env_amount = 1.0;
        st.filter_env_decay = 0.2;
        st.special[13] = 0.1; // Filter Attack
        st.special[14] = 0.05; // Filter Hold
        let mut v = voice_with(st);
        v.trigger();

        // At t = 0 the envelope is at the START of its attack, so the cutoff
        // sits on its setting and has not opened yet.
        assert!((v.filter_cutoff(v.filter_env_value()) - 500.0).abs() < 5.0);

        // Halfway up the attack it is on its way, neither closed nor open.
        v.filter_env_time = 0.05;
        let mid = v.filter_env_value();
        assert!(mid > 0.05 && mid < 0.95, "attack ramp stuck at {mid}");

        // During the hold it is fully open.
        v.filter_env_time = 0.13;
        assert!(
            (v.filter_env_value() - 1.0).abs() < 1e-3,
            "hold is not flat"
        );

        // Well into the decay it has come back down.
        v.filter_env_time = 0.15 + 0.2;
        assert!(v.filter_env_value() < 0.05, "decay never finishes");
    }

    /// The two curves must actually bend their ramp, in opposite directions.
    #[test]
    fn filter_env_curves_bend_their_ramps() {
        let sample = |atk_curve: f32| {
            let mut st = VoiceSettings::rift();
            st.special[13] = 0.1; // Filter Attack
            st.special[15] = atk_curve;
            let mut v = voice_with(st);
            v.trigger();
            v.filter_env_time = 0.05; // halfway up
            v.filter_env_value()
        };
        let convex = sample(1.0);
        let linear = sample(0.0);
        let concave = sample(-1.0);
        assert!(
            convex < linear && linear < concave,
            "curves do not bend: convex {convex}, linear {linear}, concave {concave}"
        );
    }

    /// Half the amount covers half the distance, in octaves.
    #[test]
    fn filter_env_amount_is_proportional_in_octaves() {
        let mut st = VoiceSettings::rift();
        st.filter_freq = 500.0;
        st.filter_env_amount = 0.5;
        let v = voice_with(st);
        let half = (v.filter_cutoff(1.0) / 500.0).log2();
        let full = (20000.0f32 / 500.0).log2();
        assert!(
            (half - full * 0.5).abs() < 0.01,
            "half the amount gave {half} octaves, expected {}",
            full * 0.5
        );
    }

    /// Env at 0 must leave the cutoff exactly where the Filter says.
    #[test]
    fn filter_env_at_zero_is_the_plain_cutoff() {
        let mut st = VoiceSettings::rift();
        st.filter_freq = 3500.0;
        st.filter_env_amount = 0.0;
        let v = voice_with(st);
        assert!((v.filter_cutoff(1.0) - 3500.0).abs() < 1.0);
        assert!((v.filter_cutoff(0.0) - 3500.0).abs() < 1.0);
    }

    #[test]
    fn produces_finite_sound_on_trigger() {
        let mut voice = voice_with(VoiceSettings::rift());
        let out = render(&mut voice, 44100);
        assert!(peak(&out) > 0.001, "silent, peak {}", peak(&out));
        assert!(out.iter().all(|s| s.is_finite()), "non-finite sample");
    }

    #[test]
    fn every_texture_sounds() {
        for t in 0..sample_bank::TEXTURE_COUNT {
            let mut s = VoiceSettings::rift();
            s.special[0] = sample_bank::menu_index_of_bank(t) as f32;
            s.special[3] = 0.6; // a slice long enough to judge
            let mut voice = voice_with(s);
            let out = render(&mut voice, 44100);
            assert!(peak(&out) > 0.001, "texture {t} is silent");
        }
    }

    /// The point of the instrument: moving Offset moves the read head, so the
    /// same settings at two offsets must not render the same sound.
    #[test]
    fn offset_changes_what_is_played() {
        let mut a = VoiceSettings::rift();
        a.special[1] = 0.1;
        let mut b = VoiceSettings::rift();
        b.special[1] = 0.8;
        let out_a = render(&mut voice_with(a), 8000);
        let out_b = render(&mut voice_with(b), 8000);
        let diff: f32 = out_a
            .iter()
            .zip(&out_b)
            .map(|(x, y)| (x - y).abs())
            .sum::<f32>()
            / out_a.len() as f32;
        assert!(
            diff > 1e-4,
            "both offsets render the same audio (diff {diff})"
        );
    }

    /// Wander = 0 must be reproducible — otherwise a pattern would never
    /// render twice the same, and no mix would hold.
    #[test]
    fn wander_zero_is_reproducible_and_wander_one_is_not() {
        // Two fresh voices, so the comparison is about the offset and not
        // about the tail the previous hit left in the envelopes.
        let mut s = VoiceSettings::rift();
        s.special[2] = 0.0;
        let first = render(&mut voice_with(s), 4000);
        let second = render(&mut voice_with(s), 4000);
        assert_eq!(first, second, "Wander = 0 must replay identically");

        // Same voice, reset between hits: the envelopes start clean but the
        // RNG has moved on, so a re-rolled offset is the only difference left.
        let mut s = VoiceSettings::rift();
        s.special[2] = 1.0;
        let mut voice = voice_with(s);
        let first = render(&mut voice, 4000);
        voice.reset();
        let second = render(&mut voice, 4000);
        assert_ne!(first, second, "Wander = 1 must land somewhere else");
    }

    /// With Loop OFF the envelope alone decides the length: a short Grain
    /// must NOT cut the sound short. Regression for the first version, where
    /// the two controls fought over the same quantity and the shorter won.
    #[test]
    fn grain_does_not_truncate_when_loop_is_off() {
        let mut s = VoiceSettings::rift();
        s.special[12] = 0.0; // Loop off
        s.special[3] = 0.0; // shortest grain (5 ms)
        s.decay = 0.4; // but a long envelope
        let out = render(&mut voice_with(s), 22050);
        // 5 ms is 220 samples; the sound must still be there well past that.
        let late = peak(&out[8820..13230]); // 200 ms .. 300 ms
        assert!(late > 1e-3, "the envelope was cut short by Grain: {late}");
    }

    /// With Loop ON the grain repeats, so a 20 ms slice sustains a one-second
    /// envelope instead of stopping after 20 ms.
    #[test]
    fn loop_sustains_a_short_grain() {
        let mut s = VoiceSettings::rift();
        s.special[12] = 1.0; // Loop on
        s.special[3] = 0.2; // ~14 ms
        s.decay = 0.5;
        let out = render(&mut voice_with(s), 22050);
        let late = peak(&out[13230..17640]); // 300 ms .. 400 ms
        assert!(late > 1e-3, "the loop died at the end of the grain: {late}");

        // Same settings with Loop off must stop far earlier.
        let mut s = VoiceSettings::rift();
        s.special[12] = 0.0;
        s.special[3] = 0.2;
        s.decay = 0.5;
        let single = render(&mut voice_with(s), 22050);
        assert!(
            peak(&single[13230..17640]) > 1e-3,
            "with Loop off the envelope should still be running here"
        );
    }

    /// A looped grain must not click at the seam.
    ///
    /// Measured AT the seams and compared to the ordinary jump between two
    /// neighbouring samples — on noise those are large by nature, so a global
    /// "biggest jump" tells you nothing about clicking. With the fade in place
    /// the seam measures roughly twenty times SMALLER than the median jump,
    /// because the signal is near zero when it folds back.
    #[test]
    fn looped_grain_has_no_seam_click() {
        for texture in 0..sample_bank::TEXTURE_COUNT {
            let mut s = VoiceSettings::rift();
            s.special[0] = sample_bank::menu_index_of_bank(texture) as f32;
            s.special[12] = 1.0; // Loop on
            s.special[3] = 0.25;
            s.special[4] = 0.1; // a deliberately short fade
            s.decay = 0.6;
            s.filter_freq = 20000.0;
            let mut voice = voice_with(s);
            let grain = (voice.grain_secs() * 44100.0) as usize;
            let out = render(&mut voice, 22050);
            let jumps: Vec<f32> = out.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
            let mut sorted = jumps.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median = sorted[sorted.len() / 2];

            let mut seam = 0.0f32;
            let mut k = grain;
            while k + 1 < jumps.len() {
                for j in k.saturating_sub(2)..(k + 3).min(jumps.len()) {
                    seam = seam.max(jumps[j]);
                }
                k += grain;
            }
            assert!(
                seam <= median,
                "texture {texture}: seam jump {seam} exceeds the ordinary jump {median}"
            );
        }
    }

    /// An out-of-range texture index must clamp, never panic or read out of
    /// bounds — plock and preset data can hold anything.
    #[test]
    fn out_of_range_texture_index_is_clamped() {
        for t in [-5.0, 3.0, 99.0] {
            let mut s = VoiceSettings::rift();
            s.special[0] = t;
            let mut voice = voice_with(s);
            let out = render(&mut voice, 4000);
            assert!(out.iter().all(|s| s.is_finite()), "texture {t} misbehaved");
        }
    }

    /// Each of the three filter modes must pass audio.
    #[test]
    fn every_filter_mode_passes_audio() {
        for mode in 0..3 {
            let mut s = VoiceSettings::rift();
            s.special[5] = mode as f32;
            s.special[3] = 0.5;
            s.filter_freq = 2000.0;
            let mut voice = voice_with(s);
            let out = render(&mut voice, 22050);
            assert!(
                out.iter().all(|v| v.is_finite()),
                "filter mode {mode} blew up"
            );
            assert!(peak(&out) > 1e-4, "filter mode {mode} is silent");
        }
    }

    // ── Build 3: lo-fi and stereo spread ────────────────────────────────────

    fn render_stereo(voice: &mut RiftVoice, n: usize) -> (Vec<f32>, Vec<f32>) {
        voice.trigger();
        let mut l = Vec::with_capacity(n);
        let mut r = Vec::with_capacity(n);
        for _ in 0..n {
            let (a, b) = voice.process_sample_stereo();
            l.push(a);
            r.push(b);
        }
        (l, r)
    }

    fn mean_abs_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / a.len() as f32
    }

    fn zero_crossings(buf: &[f32]) -> usize {
        buf.windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count()
    }

    /// Index of the last sample above the noise floor: how long the hit lasts.
    fn last_audible(buf: &[f32]) -> usize {
        buf.iter().rposition(|s| s.abs() > 1e-5).unwrap_or(0)
    }

    /// Crush must change the sound, keep it finite and NOT gate the tail: it
    /// sits before the amp envelope, so a crushed hit still fades out.
    #[test]
    fn crush_changes_the_sound_without_gating_the_tail() {
        let mut st = VoiceSettings::rift();
        st.decay = 0.2;
        let clean = render(&mut voice_with(st), 20_000);
        let mut crushed_st = st;
        crushed_st.special[28] = 1.0;
        let crushed = render(&mut voice_with(crushed_st), 20_000);
        assert!(crushed.iter().all(|s| s.is_finite()));
        assert!(peak(&crushed) > 0.01);
        let diff = mean_abs_diff(&clean, &crushed);
        assert!(diff > 1e-4, "crush changed nothing (mean diff {diff})");
        // The amp envelope is time-based, so the clean hit ends well inside
        // the render. At two bits the quietest stretches of the texture DO
        // fall into the zero step (that is the effect); at nine bits nothing
        // does, and the hit must then last exactly as long - the quantiser
        // sits before the envelope, never gating the tail.
        let mut mild_st = st;
        mild_st.special[28] = 0.5;
        let mild = render(&mut voice_with(mild_st), 20_000);
        assert!(mean_abs_diff(&clean, &mild) > 1e-6);
        let (end_clean, end_crushed) = (last_audible(&clean), last_audible(&mild));
        assert!(
            end_clean > 5_000 && end_clean < 19_000,
            "clean ends at {end_clean}"
        );
        assert!(
            end_crushed + 100 >= end_clean,
            "crush gated the tail: ends at {end_crushed} vs {end_clean}"
        );
    }

    /// [232] Pre-Filter moves the whole Distortion block: with Decimate on and
    /// a low-pass at 1 kHz, decimating before the filter (aliases smoothed)
    /// and after it (aliases left in) are two different sounds; with nothing
    /// in the block the switch changes nothing.
    #[test]
    fn pre_filter_switch_moves_crush_and_decimate_with_the_saturation() {
        let mut st = VoiceSettings::rift();
        st.filter_freq = 1000.0;
        st.filter_env_amount = 0.0;
        st.special[29] = 1.0; // Decimate
        let mut post = st;
        post.special[11] = 0.0; // Pre-Filter off = after the filter
        let mut pre = st;
        pre.special[11] = 1.0;
        let a = render(&mut voice_with(post), 6_000);
        let b = render(&mut voice_with(pre), 6_000);
        assert!(peak(&a) > 0.01 && peak(&b) > 0.01);
        assert!(
            mean_abs_diff(&a, &b) > 1e-4,
            "Pre-Filter must move Decimate across the filter"
        );

        // Nothing in the block: the switch is inert.
        let mut clean_post = st;
        clean_post.special[29] = 0.0;
        clean_post.special[11] = 0.0;
        let mut clean_pre = clean_post;
        clean_pre.special[11] = 1.0;
        assert_eq!(
            render(&mut voice_with(clean_post), 6_000),
            render(&mut voice_with(clean_pre), 6_000)
        );
    }

    /// Decimate holds each sample for up to 64 output samples: on the noise
    /// texture the output crosses zero far less often than the clean read.
    #[test]
    fn decimate_lowers_the_effective_sample_rate() {
        let mut st = VoiceSettings::rift();
        st.filter_freq = 20000.0;
        st.filter_env_amount = 0.0;
        let clean = render(&mut voice_with(st), 8_000);
        let mut dec_st = st;
        dec_st.special[29] = 1.0;
        let decimated = render(&mut voice_with(dec_st), 8_000);
        assert!(decimated.iter().all(|s| s.is_finite()));
        assert!(peak(&decimated) > 0.01);
        let (zc_clean, zc_dec) = (zero_crossings(&clean), zero_crossings(&decimated));
        assert!(
            zc_dec * 4 < zc_clean,
            "decimation barely changed the rate: {zc_dec} vs {zc_clean} zero crossings"
        );
    }

    /// Stereo Spread at 0 is mono byte for byte; on, the two channels differ
    /// and both keep the full envelope length.
    #[test]
    fn stereo_spread_is_mono_at_zero_and_two_heads_above() {
        let mut st = VoiceSettings::rift();
        st.decay = 0.2;
        let (l0, r0) = render_stereo(&mut voice_with(st), 12_000);
        assert_eq!(l0, r0, "spread 0 must be exactly mono");
        // The mono path renders the same thing as the left channel.
        let mono = render(&mut voice_with(st), 12_000);
        assert_eq!(mono, l0);

        let mut wide = st;
        wide.special[30] = 0.6;
        let (l, r) = render_stereo(&mut voice_with(wide), 12_000);
        assert!(l.iter().chain(&r).all(|s| s.is_finite()));
        assert!(peak(&l) > 0.01 && peak(&r) > 0.01);
        assert!(
            mean_abs_diff(&l, &r) > 1e-4,
            "spread left the channels identical"
        );
        // Same envelope on both sides: the right channel lasts as long as
        // the left, the wrap-around never shortens it.
        let (end_l, end_r) = (last_audible(&l), last_audible(&r));
        assert!(end_l > 5_000, "left ends at {end_l}");
        assert!(
            end_r + 100 >= end_l,
            "right channel cut short: {end_r} vs {end_l}"
        );
        // The left channel is the plain mono read: spread adds a head, it
        // does not move the existing one.
        assert_eq!(l, l0);
    }

    /// [228] "Custom" plays the lane's file when one is loaded; a lane
    /// without a file falls back to the first embedded texture, not silence.
    #[test]
    fn user_texture_slot_plays_the_loaded_file_or_falls_back() {
        use std::sync::Arc;
        let pool = Arc::new(sample_bank::TexturePool::new());
        let mut st = VoiceSettings::rift();
        st.special[0] = sample_bank::CUSTOM_TEXTURE_INDEX as f32;
        st.decay = 0.3;
        st.filter_freq = 20000.0;
        st.filter_env_amount = 0.0;

        // Lane 5 has no file: identical to the first embedded texture.
        let mut fallback = voice_with(st);
        fallback.set_texture_pool(pool.clone(), 5);
        let fb = render(&mut fallback, 8_000);
        let mut noise_st = st;
        noise_st.special[0] = 0.0;
        let noise = render(&mut voice_with(noise_st), 8_000);
        assert_eq!(
            fb, noise,
            "an empty slot must play the first embedded texture"
        );

        // A 1 kHz sine in the slot: the output is that tone.
        let rate = 44100.0f32;
        let data: Vec<f32> = (0..44100)
            .map(|i| (i as f32 * 1000.0 * std::f32::consts::TAU / rate).sin() * 0.5)
            .collect();
        let bank = sample_bank::TextureBank {
            source_rate: rate,
            peaks: vec![(-0.5, 0.5); sample_bank::TEXTURE_PEAK_COLUMNS],
            data,
            right: None,
        };
        pool.publish(5, Some(Arc::new(bank)));
        let mut user = voice_with(st);
        user.set_texture_pool(pool.clone(), 5);
        let out = render(&mut user, 8_000);
        assert!(out.iter().all(|s| s.is_finite()));
        // Zero crossings of a 1 kHz tone over the first 100 ms: about 200.
        let zc = out[..4410]
            .windows(2)
            .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
            .count();
        assert!(
            (150..=260).contains(&zc),
            "expected a 1 kHz tone, got {zc} crossings"
        );
        assert_ne!(out, noise);
    }

    /// [228] On a SHORT custom sample (a real-world one-shot, 0.5 s, with its
    /// transient at the start and silence at the end) Offset, Wander and
    /// Reverse must bite. Before: the read window reserved the envelope's
    /// tail (four decays) and a file shorter than that left them no room.
    #[test]
    fn offset_wander_and_reverse_work_on_a_short_custom_sample() {
        use std::sync::Arc;
        let pool = Arc::new(sample_bank::TexturePool::new());
        let rate = 44100.0f32;
        let n = 22050usize; // 0.5 s
                            // A decaying 300 Hz tone: loud at the start, silent by the end.
        let data: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f32 / rate;
                (t * 300.0 * std::f32::consts::TAU).sin() * 0.8 * (-t * 12.0).exp()
            })
            .collect();
        pool.publish(
            2,
            Some(Arc::new(sample_bank::TextureBank {
                source_rate: rate,
                peaks: vec![(-0.8, 0.8); sample_bank::TEXTURE_PEAK_COLUMNS],
                data,
                right: None,
            })),
        );
        let mut st = VoiceSettings::rift();
        st.special[0] = sample_bank::CUSTOM_TEXTURE_INDEX as f32;
        st.decay = 0.3;
        st.filter_freq = 20000.0;
        st.filter_env_amount = 0.0;
        let render_on = |st: VoiceSettings| {
            let mut v = voice_with(st);
            v.set_texture_pool(pool.clone(), 2);
            render(&mut v, 8_000)
        };

        // Offset: the start of the sample is loud, its middle is quiet.
        let mut s0 = st;
        s0.special[1] = 0.0;
        let at_start = render_on(s0);
        let mut s6 = st;
        s6.special[1] = 0.6;
        let at_middle = render_on(s6);
        assert!(
            peak(&at_start) > 0.2,
            "offset 0 must hit the transient: {}",
            peak(&at_start)
        );
        assert!(
            peak(&at_middle) < peak(&at_start) * 0.25,
            "offset 0.6 must land in the quiet part: {} vs {}",
            peak(&at_middle),
            peak(&at_start)
        );

        // Wander: two hits of the same cell differ.
        let mut sw = st;
        sw.special[1] = 0.3;
        sw.special[2] = 1.0;
        let mut wandering = voice_with(sw);
        wandering.set_texture_pool(pool.clone(), 2);
        let first = render(&mut wandering, 8_000);
        let second = render(&mut wandering, 8_000);
        assert_ne!(first, second, "Wander must move the read between hits");

        // Reverse at offset 0 with a decay covering the file: the sound swells
        // toward the transient instead of starting on it - the first 50 ms are
        // quieter than the 50 ms just before the sound ends.
        let mut sr = st;
        sr.special[1] = 0.0;
        sr.special[27] = 1.0;
        sr.decay = 0.8;
        // The reversed read needs the file's 0.5 s to reach the transient.
        let mut rv = voice_with(sr);
        rv.set_texture_pool(pool.clone(), 2);
        let reversed = render(&mut rv, 30_000);
        assert!(
            peak(&reversed) > 0.1,
            "reverse must sound: {}",
            peak(&reversed)
        );
        let head = peak(&reversed[..2205]);
        let end = reversed.iter().rposition(|x| x.abs() > 1e-3).unwrap_or(0);
        let tail = peak(&reversed[end.saturating_sub(2205)..end.max(1)]);
        assert!(
            tail > head * 2.0,
            "reversed sample must swell: head {head} tail {tail}"
        );
    }

    /// [228] A stereo file: Stereo off mixes the two channels down (both
    /// outputs equal); Stereo on plays left to left and right to right.
    #[test]
    fn stereo_switch_plays_a_stereo_file_as_two_channels() {
        use std::sync::Arc;
        let pool = Arc::new(sample_bank::TexturePool::new());
        let rate = 44100.0f32;
        // Left: a 500 Hz tone; right: silence.
        let left: Vec<f32> = (0..44100)
            .map(|i| (i as f32 * 500.0 * std::f32::consts::TAU / rate).sin() * 0.5)
            .collect();
        let right = vec![0.0f32; 44100];
        pool.publish(
            0,
            Some(Arc::new(sample_bank::TextureBank {
                source_rate: rate,
                peaks: vec![(-0.25, 0.25); sample_bank::TEXTURE_PEAK_COLUMNS],
                data: left,
                right: Some(right),
            })),
        );
        let mut st = VoiceSettings::rift();
        st.special[0] = sample_bank::CUSTOM_TEXTURE_INDEX as f32;
        st.decay = 0.3;
        st.filter_freq = 20000.0;
        st.filter_env_amount = 0.0;

        // Switch off: the downmix on both sides.
        st.stereo = 0.0;
        let mut mono = voice_with(st);
        mono.set_texture_pool(pool.clone(), 0);
        let (l0, r0) = render_stereo(&mut mono, 6_000);
        assert_eq!(l0, r0, "Stereo off must be mono");
        assert!(peak(&l0) > 0.05, "the downmix still carries the left tone");

        // Switch on: the tone on the left, silence on the right.
        st.stereo = 1.0;
        let mut wide = voice_with(st);
        wide.set_texture_pool(pool.clone(), 0);
        let (l, r) = render_stereo(&mut wide, 6_000);
        assert!(
            peak(&l) > 0.1,
            "left channel carries the tone: {}",
            peak(&l)
        );
        assert!(
            peak(&r) < 1e-3,
            "right channel is the file's silence: {}",
            peak(&r)
        );
        // The downmix is half the left signal.
        assert!(
            (peak(&l0) - 0.5 * peak(&l)).abs() < 0.02,
            "{} vs {}",
            peak(&l0),
            peak(&l)
        );
    }

    /// [227] The plugin, not the voice, decides which advance step a hit is:
    /// the same index twice gives the same hit, a different index another.
    #[test]
    fn hit_index_from_the_plugin_drives_advance() {
        let mut s = VoiceSettings::rift();
        s.special[20] = 0.05; // Advance Step
        let mut voice = voice_with(s);
        voice.set_hit_index(0);
        let a = render(&mut voice, 8_000);
        voice.set_hit_index(0);
        let b = render(&mut voice, 8_000);
        assert_eq!(a, b, "same hit index must give the same hit");
        voice.set_hit_index(3);
        let c = render(&mut voice, 8_000);
        assert_ne!(a, c, "another hit index must move the read");
    }

    /// Regression: the hit must END without a step. With the filter still open
    /// (filter decay longer than the amp decay) the filter and DC-blocker
    /// residue at the amp envelope's end was cut to zero in one sample -
    /// measured -0.00097 -> 0, an audible tick.
    #[test]
    fn no_click_when_the_amp_envelope_ends_under_an_open_filter() {
        let mut st = VoiceSettings::rift();
        st.decay = 0.12;
        st.filter_env_decay = 0.5;
        st.filter_env_amount = 1.0;
        let out = render(&mut voice_with(st), 12_000);
        let end = out
            .iter()
            .rposition(|s| s.abs() > 1e-6)
            .expect("the hit sounds");
        assert!(end > 5_000 && end < 11_000, "unexpected end {end}");
        // The last 140 samples cover the 3 ms declick ramp (132 samples)
        // plus the cut itself. The yardstick is the signal just before: the
        // end must not be a bigger discontinuity than the fading noise
        // already is (before the fix: 0.00097 against ~0.0004).
        let max_jump = |range: std::ops::Range<usize>| {
            out[range]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0f32, f32::max)
        };
        let before = max_jump(end.saturating_sub(600)..end.saturating_sub(140));
        let at_end = max_jump(end.saturating_sub(140)..(end + 4).min(out.len()));
        assert!(
            at_end <= 1.5 * before.max(1e-5),
            "step at the end of the hit: {at_end} against {before} just before (was 0.00097 before the declicked cut)"
        );
    }

    /// [297] The Filter LFO keeps its whole amplitude at the bottom (and the top) of the Filter
    /// range, where half of the old swing fell outside it.
    #[test]
    fn the_filter_lfo_keeps_its_full_swing_at_the_edges() {
        // At the 20 Hz floor with two octaves: from 20 Hz to 20 * 2^4 = 320 Hz (it was 20 -> 80).
        assert!((lfo_cutoff(20.0, -1.0, 2.0) - 20.0).abs() < 0.01);
        assert!((lfo_cutoff(20.0, 1.0, 2.0) - 320.0).abs() < 0.5);
        // With room, nothing changes: two octaves around 1 kHz.
        assert!((lfo_cutoff(1000.0, -1.0, 2.0) - 250.0).abs() < 0.1);
        assert!((lfo_cutoff(1000.0, 1.0, 2.0) - 4000.0).abs() < 0.5);
        // At the top the swing goes DOWN as far as it needs.
        assert!((lfo_cutoff(FILTER_OPEN_HZ, 1.0, 2.0) - FILTER_OPEN_HZ).abs() < 1.0);
        assert!((lfo_cutoff(FILTER_OPEN_HZ, -1.0, 2.0) - FILTER_OPEN_HZ / 16.0).abs() < 1.0);
        // The widest swing at the floor: 20 Hz up to 1280 Hz.
        assert!((lfo_cutoff(20.0, 1.0, 3.0) - 1280.0).abs() < 2.0);
        // No LFO movement, no change of the centre's reach: the value stays inside the range.
        for base in [20.0, 200.0, 2000.0, FILTER_OPEN_HZ] {
            for lfo in [-1.0, -0.3, 0.0, 0.7, 1.0] {
                let c = lfo_cutoff(base, lfo, 3.0);
                assert!((20.0..=FILTER_OPEN_HZ).contains(&c));
            }
        }
    }
}
