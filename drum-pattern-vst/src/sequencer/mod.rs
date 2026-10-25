//! Sequencer module for 16-step drum patterns
//!
//! Architecture:
//! - One master beat position advances uniformly (0..4 beats = 1 bar = 16 steps).
//! - Groove (swing type + amount) is applied to the master grid.
//! - Each track reads  master_step % track_length  → loops cleanly on its own length.
//! - Per-track push/pull shifts the effective beat before step lookup.
//! - Per-track humanize only affects velocity (no timing jitter → no double triggers).

pub(crate) mod pattern;

use std::sync::Arc;

use crate::groove::{self, GrooveType};
use pattern::with_linked_morph;
pub use pattern::{FusedGroup, MorphTarget, Pattern, SharedPattern, MAX_FUSIONS};

const MAX_TRACKS: usize = crate::track::MAX_TRACKS;
/// Mirrors `plock::STEP_COUNT` (kept literal: `test_standalone` compiles this
/// module without the `plock` module).
const SEQ_STEP_COUNT: usize = 64;

/// Per-instrument state.
#[derive(Clone, Copy, Debug)]
pub struct TrackState {
    pub previous_step: usize,
    /// Previous master step (0-15), used for trigger detection.
    /// Detecting on shifted_master instead of current_step fixes the
    /// track_length=1 bug where current_step never changes.
    pub previous_shifted_master: usize,
    pub track_length: usize,
    pub push_pull_ms: f32,
    pub humanize_amount: f32,
    /// Independent step counter for true polyrhythm.
    /// Increments by 1 on every master-step transition.
    /// current_step = step_counter % track_length.
    pub step_counter: usize,
    /// Simple LCG RNG state for deterministic per-track randomness.
    rng_state: u32,
    /// Microtiming late-fire: the current cell's trigger, deferred by a
    /// positive nudge; emitted once the shifted beat passes `late_fire_beat`.
    pub late_trigger: Option<TriggerResult>,
    pub late_fire_beat: f64,
    /// Set when the next boundary's cell was already fired early (negative
    /// microtiming): the normal transition still advances the state but must
    /// stay silent for that cell.
    pub suppress_next: bool,
    /// Set once the next boundary's cell has been fired early, so the early
    /// check does not re-fire on every remaining sample before the boundary.
    pub early_fired: bool,
}

#[derive(Clone, Copy, Debug)]
struct FusionTrack {
    groups: [FusedGroup; MAX_FUSIONS],
    count: usize,
}

impl Default for FusionTrack {
    fn default() -> Self {
        Self {
            groups: [FusedGroup::default(); MAX_FUSIONS],
            count: 0,
        }
    }
}

impl FusionTrack {
    #[cfg(test)]
    fn set_from_slice(&mut self, fusions: &[FusedGroup]) {
        self.count = 0;
        for group in fusions
            .iter()
            .copied()
            .filter(FusedGroup::is_valid)
            .take(MAX_FUSIONS)
        {
            self.groups[self.count] = group;
            self.count += 1;
        }
    }

    fn set_from_pattern(&mut self, pattern: &SharedPattern, instrument: usize) {
        self.count = pattern.load_fusions_into(instrument, &mut self.groups);
    }

    /// [283] This track holds the instrument's linked-lane morphs instead.
    fn set_linked_morphs_from_pattern(&mut self, pattern: &SharedPattern, instrument: usize) {
        self.count = pattern.load_linked_morphs_into(instrument, &mut self.groups);
    }

    fn containing(&self, step: usize) -> Option<FusedGroup> {
        for i in 0..self.count {
            let group = self.groups[i];
            if group.contains(step) {
                return Some(group);
            }
        }
        None
    }
}

impl Default for TrackState {
    fn default() -> Self {
        Self {
            previous_step: 15,
            previous_shifted_master: 15,
            track_length: 16,
            push_pull_ms: 0.0,
            humanize_amount: 0.0,
            step_counter: 0,
            rng_state: 0xACE1_0000,
            late_trigger: None,
            late_fire_beat: 0.0,
            suppress_next: false,
            early_fired: false,
        }
    }
}

impl TrackState {
    fn next_rand(&mut self) -> f32 {
        self.rng_state = self.rng_state.wrapping_mul(1103515245).wrapping_add(12345);
        ((self.rng_state >> 16) & 0x7FFF) as f32 / 32767.0
    }

    /// Drop any pending microtiming state (seek/stop/reset): a late trigger
    /// scheduled before the jump must not fire after it.
    fn clear_microtiming_state(&mut self) {
        self.late_trigger = None;
        self.suppress_next = false;
        self.early_fired = false;
    }
}

/// What a cell does when the playhead reaches it.
enum CellFire {
    /// Covered by a fused cell starting earlier: no independent trigger.
    Covered,
    /// Emits a trigger (a plain cell or the start of a fused cell).
    Fire {
        source_step: usize,
        fusion_pulse_count: u8,
        fusion_span_cells: u8,
        morph_count: u8,
        morph_targets: [MorphTarget; 4],
    },
}

/// `own_morphs`: for a linked lane, its own morphs - it plays the fused groups
/// of the lane it follows but never that lane's morph ([283]).
fn classify_cell(fusions: &FusionTrack, own_morphs: Option<&FusionTrack>, step: usize) -> CellFire {
    let group = fusions.containing(step).map(|group| match own_morphs {
        Some(morphs) => with_linked_morph(group, &morphs.groups[..morphs.count]),
        None => group,
    });
    match group {
        Some(group) if group.is_start(step) => CellFire::Fire {
            source_step: group.start_cell as usize,
            fusion_pulse_count: group.step_count.clamp(1, 64),
            fusion_span_cells: group.cell_span().min(64) as u8,
            morph_count: group.morph_count,
            morph_targets: group.morph_targets,
        },
        Some(_) => CellFire::Covered,
        None => CellFire::Fire {
            source_step: step,
            fusion_pulse_count: 1,
            fusion_span_cells: 1,
            morph_count: 0,
            morph_targets: [MorphTarget::default(); 4],
        },
    }
}

pub struct Sequencer {
    is_playing: bool,
    /// Master beat position (0.0 .. master_length * 0.25). 1 step = 0.25 beat.
    beat_position: f64,
    tracks: [TrackState; MAX_TRACKS],
    /// Cached values for sync_to_host.
    swing: f32,
    groove_type: GrooveType,
    /// [275] A lane's own swing, instead of the global one (`None` = the global).
    /// Copied from the track layout once per buffer.
    lane_swings: [Option<(f32, GrooveType)>; MAX_TRACKS],
    pattern: Arc<SharedPattern>,
    mutes: [bool; MAX_TRACKS],
    /// Global pattern length (1-64 steps). Controls master loop point.
    master_length: usize,
    /// How many times the master pattern has looped (for step conditions).
    loop_count: usize,
    /// Per-slot mapping to the legacy `DrumVoice` index used for synthesis.
    /// `None` means the slot is inactive.
    slot_voices: [Option<usize>; MAX_TRACKS],
    /// Per-instrument fused cell groups (Step Fusion), copied from SharedPattern once per buffer.
    fusions: [FusionTrack; MAX_TRACKS],
    /// [283] Per-slot morphs of a LINKED lane for the fused groups it borrows.
    linked_morphs: [FusionTrack; MAX_TRACKS],
    /// Per-slot grid source: a lane linked to the one above plays that lane's
    /// steps + fusions (layering). Identity by default; refreshed each buffer
    /// from the track layout via `set_grid_slots`.
    grid_slots: [usize; MAX_TRACKS],
    /// Per-cell microtiming (ms, -100..+100), indexed by (slot, source step) like
    /// the sequencer plocks. Copied from the atomics once per buffer.
    microtimings: [[f32; SEQ_STEP_COUNT]; MAX_TRACKS],
    /// [212] Page loop: the master position stays inside this 16-step page
    /// (index 0-3) instead of running the whole pattern. `None` = off. A page
    /// beyond the pattern length counts as off.
    page_loop: Option<usize>,
    /// Tempo of the last processed sample, for the push/pull conversion when a
    /// page loop engages between two samples.
    bpm: f32,
}

/// Per-instrument trigger result.
#[derive(Clone, Copy, Debug)]
pub struct TriggerResult {
    pub should_trigger: bool,
    pub velocity: f32,
    /// Cell used for sound plocks, sequencer plocks and MIDI export of this hit.
    pub step: usize,
    /// Number of evenly spaced pulses to emit over `fusion_span_cells` cells.
    /// `1` means a normal, non-fused trigger.
    pub fusion_pulse_count: u8,
    pub fusion_span_cells: u8,
    /// Number of active morphing targets (0-4). Only meaningful for fused triggers.
    pub morph_count: u8,
    /// Morphing targets. Only the first `morph_count` entries are valid.
    pub morph_targets: [MorphTarget; 4],
    /// This trigger fired EARLY (negative microtiming) across the pattern wrap:
    /// it belongs to the NEXT loop, so step conditions must be evaluated with
    /// `loop_count + 1`.
    pub early_next_loop: bool,
    /// [305] The master step this hit belongs to (its boundary on the swung grid): the
    /// pulses of a fused cell are placed on the swing from there.
    pub master_step: usize,
}

impl Default for TriggerResult {
    fn default() -> Self {
        Self {
            should_trigger: false,
            velocity: 0.0,
            step: 0,
            fusion_pulse_count: 1,
            fusion_span_cells: 1,
            morph_count: 0,
            morph_targets: [MorphTarget::default(); 4],
            early_next_loop: false,
            master_step: 0,
        }
    }
}

impl TriggerResult {
    pub fn is_fusion(&self) -> bool {
        self.fusion_span_cells > 1
    }
}

impl Sequencer {
    pub fn new(pattern: Arc<SharedPattern>) -> Self {
        Self {
            is_playing: false,
            beat_position: 0.0,
            tracks: [TrackState::default(); MAX_TRACKS],
            swing: 0.0,
            groove_type: GrooveType::Swing16,
            lane_swings: [None; MAX_TRACKS],
            pattern,
            mutes: [false; MAX_TRACKS],
            master_length: 16,
            loop_count: 0,
            slot_voices: [None; MAX_TRACKS],
            fusions: [FusionTrack::default(); MAX_TRACKS],
            linked_morphs: [FusionTrack::default(); MAX_TRACKS],
            grid_slots: std::array::from_fn(|i| i),
            microtimings: [[0.0; SEQ_STEP_COUNT]; MAX_TRACKS],
            page_loop: None,
            bpm: 120.0,
        }
    }

    // ── [212] Page loop ─────────────────────────────────────────────────────

    /// Loop one 16-step page instead of the whole pattern. `None` = off.
    /// Engaging or releasing takes effect NOW, keeping the playhead's phase
    /// within the page: page 4 at step 52 becomes page 2 at step 36 and the
    /// groove never stumbles. Call once per buffer from the audio thread.
    pub fn set_page_loop(&mut self, page: Option<usize>) {
        let page = page.filter(|p| p * 16 < self.master_length);
        if page != self.page_loop {
            self.page_loop = page;
            let before = self.beat_position;
            self.beat_position = self.fold_into_loop(before);
            if self.beat_position != before {
                // A jump between two samples: the cell we land in already had
                // its boundary, so no lane fires it again - the first cell
                // played is the next one - and every counter follows the new
                // position (measured: without this, engaging at 52.5 re-hit
                // step 20 on landing).
                self.realign_tracks_to_position();
            }
        }
    }

    /// Put every lane exactly where the master now is: boundary behind us,
    /// counter derived from the position, microtiming state dropped.
    fn realign_tracks_to_position(&mut self) {
        let bpm = self.bpm;
        for i in 0..MAX_TRACKS {
            let (swing, groove_type) = self.groove_of(i);
            let push_pull_beats = self.tracks[i].push_pull_ms as f64 * bpm as f64 / (60.0 * 1000.0);
            let shifted_beat = self.fold_shifted(self.beat_position - push_pull_beats);
            let shifted_master = groove::beat_to_step(shifted_beat, swing, groove_type);
            let track = &mut self.tracks[i];
            track.previous_shifted_master = shifted_master;
            track.step_counter = shifted_master;
            track.previous_step = shifted_master % track.track_length.max(1);
            track.clear_microtiming_state();
        }
    }

    pub fn page_loop(&self) -> Option<usize> {
        self.page_loop.filter(|p| p * 16 < self.master_length)
    }

    /// Bounds of what loops, in beats: the page, or the whole pattern. The
    /// last page of a 40-step pattern is 8 steps long and loops on those.
    fn loop_bounds(&self) -> (f64, f64) {
        match self.page_loop() {
            Some(p) => {
                let start = p * 16;
                let end = (start + 16).min(self.master_length);
                (start as f64 * 0.25, end as f64 * 0.25)
            }
            None => (0.0, self.master_length as f64 * 0.25),
        }
    }

    /// Length of the loop in beats (the pattern's when no page loops).
    pub fn loop_span_beats(&self) -> f64 {
        let (start, end) = self.loop_bounds();
        (end - start).max(0.25)
    }

    /// Bring a master position inside the loop. Inside already: unchanged.
    /// Outside: the phase within a 16-step page is kept (what the ear follows).
    fn fold_into_loop(&self, beat: f64) -> f64 {
        let (start, end) = self.loop_bounds();
        let span = end - start;
        if span <= 0.0 {
            return start;
        }
        if beat >= start && beat < end {
            return beat;
        }
        start + beat.rem_euclid(4.0).rem_euclid(span)
    }

    /// A track's shifted (push/pull) position, wrapped INSIDE the loop rather
    /// than inside the pattern: a lane pulled late just after the page wrap is
    /// still on the page's last step, not on a step of the previous page.
    fn fold_shifted(&self, beat: f64) -> f64 {
        let (start, end) = self.loop_bounds();
        start + (beat - start).rem_euclid((end - start).max(1e-9))
    }

    /// Where the host's absolute position lands in the plugin's own timeline:
    /// modulo the pattern, or - with a page loop - modulo the page, offset to
    /// its start. Shared by `sync_to_host` and the seek detector, so a page
    /// wrap never reads as a transport jump.
    pub fn host_to_local(&self, position_beats: f64) -> f64 {
        let (start, _) = self.loop_bounds();
        start + position_beats.rem_euclid(self.loop_span_beats())
    }

    /// Set the per-slot grid source (for lane linking / layering). Call once per
    /// buffer from the audio thread. Identity = no linking.
    pub fn set_grid_slots(&mut self, grid_slots: [usize; MAX_TRACKS]) {
        self.grid_slots = grid_slots;
    }

    /// [275] Each lane's own swing (`None` = the global one). Call once per buffer.
    pub fn set_lane_swings(&mut self, lane_swings: [Option<(f32, GrooveType)>; MAX_TRACKS]) {
        self.lane_swings = lane_swings;
    }

    /// [275] The swing and groove `slot` plays with: its own, else the global ones.
    fn groove_of(&self, slot: usize) -> (f32, GrooveType) {
        self.lane_swings[slot].unwrap_or((self.swing, self.groove_type))
    }

    /// Copy the per-cell microtiming (ms) from the seq-plock atomics. Call once
    /// per audio buffer, never per sample.
    pub fn set_microtimings(&mut self, microtimings: [[f32; SEQ_STEP_COUNT]; MAX_TRACKS]) {
        self.microtimings = microtimings;
    }

    /// Copy fusion groups from SharedPattern into fixed local arrays.
    /// Call once per audio buffer, never per sample.
    pub fn sync_fusions_from_pattern(&mut self) {
        for slot in 0..MAX_TRACKS {
            self.fusions[slot].set_from_pattern(&self.pattern, slot);
            self.linked_morphs[slot].set_linked_morphs_from_pattern(&self.pattern, slot);
        }
    }

    #[cfg(test)]
    fn set_fusions_for_test(&mut self, instrument: usize, fusions: &[FusedGroup]) {
        if instrument < MAX_TRACKS {
            self.fusions[instrument].set_from_slice(fusions);
        }
    }

    pub fn process_sample(
        &mut self,
        bpm: f32,
        sample_rate: f32,
        swing: f32,
        groove_type: GrooveType,
    ) -> [TriggerResult; MAX_TRACKS] {
        let mut triggers = [TriggerResult::default(); MAX_TRACKS];

        if !self.is_playing {
            return triggers;
        }

        self.swing = swing;
        self.groove_type = groove_type;
        self.bpm = bpm;

        // Advance master beat position uniformly. Wrap at the loop's end: the
        // pattern's, or the looped page's ([212]).
        let beat_increment = (bpm as f64 / 60.0) / sample_rate as f64;
        let (loop_start, loop_end) = self.loop_bounds();
        let loop_span = loop_end - loop_start;
        // Outside the loop (page loop engaged, pattern shortened): fold first.
        if self.beat_position < loop_start || self.beat_position >= loop_end {
            self.beat_position = self.fold_into_loop(self.beat_position);
        }
        let prev_beat = self.beat_position;
        self.beat_position += beat_increment;
        if self.beat_position >= loop_end {
            self.beat_position -= loop_span;
            if self.beat_position < loop_start {
                self.beat_position = loop_start;
            }
            // Detect loop wrap (only when actually wrapping, not on seek). A
            // page loop counts a loop per page: step conditions follow it.
            if prev_beat + beat_increment >= loop_end {
                self.loop_count = self.loop_count.wrapping_add(1);
            }
        }
        let page_loop_on = self.page_loop().is_some();
        let loop_end_step = (loop_end / 0.25).round() as usize;
        let loop_start_step = (loop_start / 0.25).round() as usize;

        // Master beat advances uniformly; each track derives its own step.

        for slot in 0..MAX_TRACKS {
            // [275] A lane with its own swing steps on its own swung grid.
            let (swing, groove_type) = self.lane_swings[slot].unwrap_or((swing, groove_type));
            let track = &mut self.tracks[slot];

            // Push/pull: convert ms to beats and subtract so positive = late.
            let push_pull_beats = track.push_pull_ms as f64 * bpm as f64 / (60.0 * 1000.0);
            let shifted_beat = {
                let raw = self.beat_position - push_pull_beats;
                loop_start + (raw - loop_start).rem_euclid(loop_span.max(1e-9))
            };

            // Re-compute master step for this track's shifted timeline.
            let shifted_master = groove::beat_to_step(shifted_beat, swing, groove_type);

            // Grid source: a linked lane plays the steps + fusions of the
            // lane above it (layering); identity otherwise.
            let grid = self.grid_slots[slot];
            let own_morphs = (grid != slot).then(|| &self.linked_morphs[slot]);

            // 1) Microtiming late-fire: a positively nudged cell whose delayed
            // fire time has arrived.
            if let Some(late) = track.late_trigger {
                if shifted_beat >= track.late_fire_beat {
                    track.late_trigger = None;
                    triggers[slot] = late;
                }
            }

            // 2) Trigger on master step transition.
            // Using shifted_master (not current_step) fixes the track_length=1 bug
            // where current_step never changes (always 0).
            if shifted_master != track.previous_shifted_master {
                // True polyrhythm: each track advances its own independent
                // counter. [212] Except when the master JUMPED (page-loop wrap
                // or engage) rather than moved to the next step: the counter
                // is then re-derived from the position, so every lane - a
                // polymetric one included - plays what the grid shows at that
                // step. The pattern's own wrap is not a jump: 63 -> 0 is the
                // next step, and polymeter keeps drifting there as before.
                let expected = (track.previous_shifted_master + 1) % self.master_length.max(1);
                if page_loop_on && shifted_master != expected {
                    track.step_counter = shifted_master;
                } else {
                    track.step_counter = track.step_counter.wrapping_add(1);
                }
                let current_step = track.step_counter % track.track_length.max(1);
                track.previous_shifted_master = shifted_master;
                track.previous_step = current_step;
                track.early_fired = false;

                if track.suppress_next {
                    // This cell already fired early (negative microtiming).
                    track.suppress_next = false;
                } else if let CellFire::Fire {
                    source_step,
                    fusion_pulse_count,
                    fusion_span_cells,
                    morph_count,
                    morph_targets,
                } = classify_cell(&self.fusions[grid], own_morphs, current_step)
                {
                    let mut trig = Self::eval_trigger(
                        &self.pattern,
                        self.mutes[slot],
                        track,
                        source_step,
                        fusion_pulse_count,
                        fusion_span_cells,
                        morph_count,
                        morph_targets,
                        grid,
                    );
                    trig.master_step = shifted_master;
                    if trig.should_trigger {
                        let micro = self.microtimings[slot][source_step].clamp(-100.0, 100.0);
                        if micro > 0.0 {
                            // Positive nudge: defer the whole trigger (its
                            // stutter/fusion pulses expand from the delayed
                            // fire time in the audio engine).
                            let micro_beats = micro as f64 * bpm as f64 / 60000.0;
                            track.late_trigger = Some(trig);
                            track.late_fire_beat = shifted_beat + micro_beats;
                        } else if triggers[slot].should_trigger {
                            // Rare collision with a late fire this very sample:
                            // defer this trigger by one sample instead of
                            // dropping a hit.
                            track.late_trigger = Some(trig);
                            track.late_fire_beat = shifted_beat;
                        } else {
                            triggers[slot] = trig;
                        }
                    }
                }
            }

            // 3) Microtiming early-fire: the NEXT boundary's cell has a
            // negative nudge — fire it up to 100 ms before its step boundary.
            if !track.early_fired && !track.suppress_next {
                let next_master = shifted_master + 1;
                let crosses_loop_end = next_master >= loop_end_step;
                // [212] Across a page-loop wrap the next cell is the page's
                // first step, not the counter's successor.
                let next_step = if page_loop_on && crosses_loop_end {
                    loop_start_step % track.track_length.max(1)
                } else {
                    track.step_counter.wrapping_add(1) % track.track_length.max(1)
                };
                if let CellFire::Fire { source_step, .. } =
                    classify_cell(&self.fusions[grid], own_morphs, next_step)
                {
                    let micro = self.microtimings[slot][source_step].clamp(-100.0, 100.0);
                    if micro < 0.0 {
                        let (delta_beats, crosses_wrap) = if !crosses_loop_end {
                            (
                                groove::step_start_beat(next_master, swing, groove_type)
                                    - shifted_beat,
                                false,
                            )
                        } else {
                            (loop_end - shifted_beat, true)
                        };
                        let delta_ms = delta_beats / bpm as f64 * 60000.0;
                        if delta_ms <= -(micro as f64) {
                            if let CellFire::Fire {
                                source_step,
                                fusion_pulse_count,
                                fusion_span_cells,
                                morph_count,
                                morph_targets,
                            } = classify_cell(&self.fusions[grid], own_morphs, next_step)
                            {
                                let mut trig = Self::eval_trigger(
                                    &self.pattern,
                                    self.mutes[slot],
                                    track,
                                    source_step,
                                    fusion_pulse_count,
                                    fusion_span_cells,
                                    morph_count,
                                    morph_targets,
                                    grid,
                                );
                                if trig.should_trigger {
                                    trig.early_next_loop = crosses_wrap;
                                    trig.master_step =
                                        if crosses_loop_end { loop_start_step } else { next_master };
                                    if triggers[slot].should_trigger {
                                        // Same-sample collision: one sample
                                        // late beats a dropped hit.
                                        track.late_trigger = Some(trig);
                                        track.late_fire_beat = shifted_beat;
                                    } else {
                                        triggers[slot] = trig;
                                    }
                                    track.early_fired = true;
                                    track.suppress_next = true;
                                }
                            }
                        }
                    }
                }
            }
        }

        triggers
    }

    /// Evaluate the trigger for a firing cell: mask + mute gate, humanized
    /// velocity. Shared by the on-boundary path and the microtiming
    /// early/late paths so all three produce identical triggers.
    #[allow(clippy::too_many_arguments)]
    fn eval_trigger(
        pattern: &SharedPattern,
        muted: bool,
        track: &mut TrackState,
        source_step: usize,
        fusion_pulse_count: u8,
        fusion_span_cells: u8,
        morph_count: u8,
        morph_targets: [MorphTarget; 4],
        grid: usize,
    ) -> TriggerResult {
        let step_mask = pattern.load_step_mask(source_step);
        let active = (step_mask & (1 << grid)) != 0 && !muted;

        let velocity = if active {
            if track.humanize_amount > 0.0 {
                let r = track.next_rand();
                (0.8 + (r - 0.5) * track.humanize_amount).clamp(0.1, 1.0)
            } else {
                0.8
            }
        } else {
            0.0
        };

        TriggerResult {
            should_trigger: active,
            velocity,
            step: source_step,
            fusion_pulse_count,
            fusion_span_cells,
            morph_count,
            morph_targets,
            early_next_loop: false,
            master_step: 0,
        }
    }

    pub fn sync_to_host(&mut self, position_beats: f64, bpm: f32, _sample_rate: f32) {
        // [212] Modulo the pattern, or modulo the looped page offset to its
        // start: `host_to_local` is the one mapping, shared with the seek
        // detector in `lib.rs`.
        self.beat_position = self.host_to_local(position_beats);
        // Keep loop_count in sync with the host's absolute timeline so
        // step conditions (1st loop, 2/2, etc.) work when driven by DAW transport.
        self.loop_count = (position_beats / self.loop_span_beats()).floor() as usize;
        let page_loop_on = self.page_loop().is_some();
        for i in 0..MAX_TRACKS {
            let (swing, groove_type) = self.groove_of(i);
            let push_pull_beats = self.tracks[i].push_pull_ms as f64 * bpm as f64 / (60.0 * 1000.0);
            let shifted_beat = self.fold_shifted(self.beat_position - push_pull_beats);
            let shifted_master = groove::beat_to_step(shifted_beat, swing, groove_type);
            let track = &mut self.tracks[i];
            track.previous_shifted_master = shifted_master;

            // Reconstruct the number of shifted step boundaries crossed so far.
            // Using the shifted timeline (master position minus push/pull offset)
            // keeps each track's phase correct after a seek, instead of snapping
            // every track to the master step count. With a page loop the
            // counter follows the FOLDED position instead: the host's absolute
            // step count would put a 64-step lane on another page than the one
            // the master is looping.
            let shifted_steps = ((position_beats - push_pull_beats) / 0.25).floor() as i64;
            track.step_counter = if page_loop_on {
                shifted_master
            } else {
                shifted_steps as usize
            };
            track.previous_step = track.step_counter % track.track_length.max(1);
            track.clear_microtiming_state();
        }
    }

    pub fn reset(&mut self) {
        self.beat_position = 0.0;
        self.loop_count = 0;
        let max_step = self.master_length.saturating_sub(1);
        for track in self.tracks.iter_mut() {
            track.previous_step = max_step;
            track.previous_shifted_master = max_step;
            track.step_counter = 0;
            track.clear_microtiming_state();
        }
        self.is_playing = false;
    }

    pub fn play(&mut self) {
        self.is_playing = true;
        self.loop_count = 0;
        let max_step = self.master_length.saturating_sub(1);
        for track in self.tracks.iter_mut() {
            track.previous_step = max_step; // Force trigger on next step 0
            track.previous_shifted_master = max_step;
            track.step_counter = track.track_length.wrapping_sub(1);
            track.clear_microtiming_state();
        }
    }

    /// Force a trigger on step 0 for all tracks.
    /// Call after sync_to_host when starting near beat 0 to avoid missing the first step.
    pub fn force_step0_trigger(&mut self) {
        let max_step = self.master_length.saturating_sub(1);
        for track in self.tracks.iter_mut() {
            track.previous_step = max_step;
            track.previous_shifted_master = max_step;
            track.step_counter = track.track_length.wrapping_sub(1);
            track.clear_microtiming_state();
        }
    }

    /// Start the current pattern from step 0 on the next processed sample.
    pub fn restart_pattern_from_step0(&mut self) {
        self.beat_position = 0.0;
        self.loop_count = 0;
        self.force_step0_trigger();
    }

    pub fn stop(&mut self) {
        self.is_playing = false;
        self.reset();
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing
    }

    /// Set per-track parameters. Called once per buffer (or when params change).
    pub fn set_track_params(
        &mut self,
        lengths: [usize; MAX_TRACKS],
        push_pulls: [f32; MAX_TRACKS],
        humanizes: [f32; MAX_TRACKS],
        master_length: usize,
    ) {
        self.master_length = master_length.clamp(1, 64);
        for i in 0..MAX_TRACKS {
            self.tracks[i].track_length = lengths[i].clamp(1, 64);
            self.tracks[i].push_pull_ms = push_pulls[i];
            self.tracks[i].humanize_amount = humanizes[i].clamp(0.0, 1.0);
        }
    }

    /// Returns the master step for UI highlighting.
    pub fn current_step(&self) -> usize {
        groove::beat_to_step(self.beat_position, self.swing, self.groove_type)
    }

    /// Returns the internal beat position (0..4).
    pub fn beat_position(&self) -> f64 {
        self.beat_position
    }

    /// Returns per-track current step for UI highlighting.
    ///
    /// Push/Pull moves a lane's AUDIO only: its counter runs on the shifted
    /// timeline, so the playhead is brought back to the master's by the
    /// whole steps that separate the two timelines (0 when Push/Pull is 0).
    pub fn current_steps(&self) -> [usize; MAX_TRACKS] {
        let (start, end) = self.loop_bounds();
        let loop_steps = (((end - start) / 0.25).round() as i64).max(1);
        let mut steps = [0usize; MAX_TRACKS];
        for (i, track) in self.tracks.iter().enumerate() {
            // [275] Master and lane on the lane's own swung grid: only Push/Pull
            // separates them.
            let (swing, groove_type) = self.groove_of(i);
            let master = groove::beat_to_step(self.beat_position, swing, groove_type) as i64;
            let len = track.track_length.max(1) as i64;
            let push_pull_beats = track.push_pull_ms as f64 * self.bpm as f64 / (60.0 * 1000.0);
            let shifted_beat = self.fold_shifted(self.beat_position - push_pull_beats);
            let shifted = groove::beat_to_step(shifted_beat, swing, groove_type) as i64;
            let mut ahead = (master - shifted).rem_euclid(loop_steps);
            if ahead > loop_steps / 2 {
                ahead -= loop_steps;
            }
            steps[i] = (track.step_counter as i64 + ahead).rem_euclid(len) as usize;
        }
        steps
    }

    pub fn loop_count(&self) -> usize {
        self.loop_count
    }

    pub fn set_mutes(&mut self, mutes: [bool; MAX_TRACKS]) {
        self.mutes = mutes;
    }

    /// The lanes silenced by Mute or by another lane's Solo, one bit per slot.
    pub fn muted_mask(&self) -> u16 {
        self.mutes
            .iter()
            .enumerate()
            .fold(0, |mask, (slot, &muted)| if muted { mask | 1 << slot } else { mask })
    }

    pub fn set_slot_voices(&mut self, slot_voices: [Option<usize>; MAX_TRACKS]) {
        self.slot_voices = slot_voices;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequencer_timing() {
        let shared_pattern = SharedPattern::new(&Pattern::rock_pattern());
        let mut seq = Sequencer::new(shared_pattern);
        let sample_rate = 44100.0;
        let bpm = 120.0;

        seq.play();

        let mut triggers_count = 0;
        for _ in 0..6000 {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Swing16);
            if triggers.iter().any(|trigger| trigger.should_trigger) {
                triggers_count += 1;
            }
        }

        assert!(triggers_count > 0);
    }

    #[test]
    fn test_sequencer_drift_over_bars() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0x3ff);
        }
        let mut seq = Sequencer::new(shared_pattern);
        let sample_rate = 44100.0;
        let bpm = 120.0;
        seq.play();

        let samples_per_bar_exact = (60.0 / bpm * 4.0 * sample_rate) as usize; // 88200
        let total_bars = 10;
        let total_samples = samples_per_bar_exact * total_bars + 10;

        let mut step_triggers = Vec::new();
        for sample_idx in 0..total_samples {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Swing16);
            if triggers.iter().any(|trigger| trigger.should_trigger) {
                step_triggers.push(sample_idx);
            }
        }

        let start_of_bar_2 = step_triggers[16];
        let bar_2_drift = (start_of_bar_2 as isize) - (samples_per_bar_exact as isize);
        assert!(
            bar_2_drift.abs() <= 1,
            "Timing drift: bar 2 started at sample {} instead of {} (drift = {})",
            start_of_bar_2,
            samples_per_bar_exact,
            bar_2_drift
        );

        let start_of_bar_11 = step_triggers[160];
        let expected_bar_11_start = 10 * samples_per_bar_exact;
        let drift = expected_bar_11_start as isize - start_of_bar_11 as isize;
        assert!(
            drift.abs() <= 1,
            "Timing drift: {} samples over 10 bars",
            drift
        );
    }

    /// Ext MIDI mode reads the muted lanes from this mask (Mute and Solo).
    #[test]
    fn muted_mask_has_one_bit_per_silenced_lane() {
        let mut seq = Sequencer::new(SharedPattern::new(&Pattern::empty()));
        assert_eq!(seq.muted_mask(), 0);
        let mut mutes = [false; MAX_TRACKS];
        mutes[0] = true;
        mutes[5] = true;
        mutes[MAX_TRACKS - 1] = true;
        seq.set_mutes(mutes);
        assert_eq!(seq.muted_mask(), 1 | 1 << 5 | 1 << (MAX_TRACKS - 1));
    }

    /// [275] A lane with its own swing delays its odd steps while the others stay
    /// straight; its even steps stay on the grid.
    #[test]
    fn a_lane_with_its_own_swing_swings_alone() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0b11);
        }
        let mut seq = Sequencer::new(shared_pattern);
        let mut lane_swings = [None; MAX_TRACKS];
        lane_swings[1] = Some((0.5, GrooveType::Swing16));
        seq.set_lane_swings(lane_swings);
        seq.play();
        let mut hits: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
        for sample_idx in 0..44100 {
            let triggers = seq.process_sample(120.0, 44100.0, 0.0, GrooveType::Swing16);
            for (lane, hits) in hits.iter_mut().enumerate() {
                if triggers[lane].should_trigger {
                    hits.push(sample_idx);
                }
            }
        }
        // 16ths at 120 BPM: 5512.5 samples; Swing16 +50 % puts the odd step at 2/3 of
        // the 8th, 7350 samples after the even one.
        assert!((hits[0][1] as i64 - 5512).abs() <= 1, "global straight: {}", hits[0][1]);
        assert!((hits[1][1] as i64 - 7350).abs() <= 1, "own swing: {}", hits[1][1]);
        assert!((hits[1][2] as i64 - hits[0][2] as i64).abs() <= 1, "even steps stay");
        assert_eq!(hits[0].len(), hits[1].len(), "same number of hits");
    }

    #[test]
    fn test_swing_delays_odd_steps() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0x3ff);
        }

        let sample_rate = 44100.0;
        let bpm = 120.0;

        let mut seq = Sequencer::new(shared_pattern.clone());
        seq.play();
        let mut straight_positions = Vec::new();
        for sample_idx in 0..(44100 * 2) {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Swing16);
            if triggers.iter().any(|trigger| trigger.should_trigger) {
                straight_positions.push(sample_idx);
            }
        }

        let mut seq = Sequencer::new(shared_pattern);
        seq.play();
        let mut swing_positions = Vec::new();
        for sample_idx in 0..(44100 * 2) {
            let triggers = seq.process_sample(bpm, sample_rate, 0.5, GrooveType::Swing16);
            if triggers.iter().any(|trigger| trigger.should_trigger) {
                swing_positions.push(sample_idx);
            }
        }

        let step1_straight = straight_positions[1];
        let step1_swing = swing_positions[1];
        assert!(
            step1_swing > step1_straight,
            "Step 1 with swing should be delayed (straight={}, swing={})",
            step1_straight,
            step1_swing
        );

        let step0_delta = (swing_positions[0] as isize - straight_positions[0] as isize).abs();
        assert!(
            step0_delta <= 1,
            "Step 0 should not drift with swing (delta={})",
            step0_delta
        );
    }

    #[test]
    fn test_swing_total_bar_duration_unchanged() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0x3ff);
        }

        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_bar = (60.0 / bpm * 4.0 * sample_rate) as usize;

        for swing in [0.0_f32, 0.25, 0.5] {
            let mut seq = Sequencer::new(shared_pattern.clone());
            seq.play();

            let mut bar_start = 0_usize;
            let mut triggers_seen = 0_usize;

            for sample_idx in 0..(samples_per_bar * 3 + 100) {
                let triggers = seq.process_sample(bpm, sample_rate, swing, GrooveType::Swing16);
                if triggers.iter().any(|trigger| trigger.should_trigger) {
                    triggers_seen += 1;
                    if triggers_seen % 16 == 1 && triggers_seen > 1 {
                        let measured_bar_duration = sample_idx - bar_start;
                        let drift = (measured_bar_duration as isize) - (samples_per_bar as isize);
                        assert!(
                            drift.abs() <= 1,
                            "Swing={}: bar duration drift = {} samples",
                            swing,
                            drift
                        );
                        bar_start = sample_idx;
                    }
                }
            }
        }
    }

    #[test]
    fn test_polyrhythm_different_lengths() {
        // Kick on every step (16-step cycle), snare on every step (12-step cycle).
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0b0000_0000_0011); // kick + snare
        }

        let mut seq = Sequencer::new(shared_pattern);
        seq.tracks[0].track_length = 16; // Kick
        seq.tracks[1].track_length = 12; // Snare
        seq.play();

        let sample_rate = 44100.0;
        let bpm = 120.0;

        let mut kick_steps = Vec::new();
        let mut snare_steps = Vec::new();

        for _ in 0..(88200 * 4) {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            if triggers[0].should_trigger {
                kick_steps.push(seq.tracks[0].previous_step);
            }
            if triggers[1].should_trigger {
                snare_steps.push(seq.tracks[1].previous_step);
            }
        }

        // Kick should cycle 0..15 repeatedly
        assert!(
            kick_steps
                .windows(16)
                .any(|w| w == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]),
            "Kick should cycle through all 16 steps"
        );

        // Snare should cycle 0..11 repeatedly
        assert!(
            snare_steps
                .windows(12)
                .any(|w| w == [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            "Snare should cycle through all 12 steps"
        );
    }

    #[test]
    fn test_push_pull_delays_track() {
        // Kick triggers on every step so we can measure interval differences.
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0b0000_0000_0001); // kick on every step
        }

        let mut seq = Sequencer::new(shared_pattern.clone());
        seq.tracks[0].push_pull_ms = 20.0; // 20 ms late
        seq.play();

        let sample_rate = 44100.0;
        let bpm = 120.0;

        let mut seq_straight = Sequencer::new(shared_pattern);
        seq_straight.play();

        let mut delayed_steps = Vec::new();
        let mut straight_steps = Vec::new();

        for sample_idx in 0..20000 {
            let t1 = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            let t2 = seq_straight.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            if t1[0].should_trigger {
                delayed_steps.push(sample_idx);
            }
            if t2[0].should_trigger {
                straight_steps.push(sample_idx);
            }
        }

        // With +20 ms push/pull (positive = late), step 1 should arrive later.
        let step1_delayed: usize = delayed_steps[1];
        let step1_straight: usize = straight_steps[1];
        let expected_delay: usize = (0.020 * sample_rate as f64) as usize;
        let actual_delay = step1_delayed.saturating_sub(step1_straight);
        assert!(
            actual_delay >= expected_delay - 50 && actual_delay <= expected_delay + 50,
            "Push/pull delay mismatch: expected ~{}, got {} (delayed={}, straight={})",
            expected_delay,
            actual_delay,
            step1_delayed,
            step1_straight
        );
    }

    #[test]
    fn restart_pattern_from_step0_forces_step0_after_length_change() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        shared_pattern.set_step_mask(0, 0b0000_0000_0001);

        let mut seq = Sequencer::new(shared_pattern);
        seq.play();
        seq.set_track_params([8; MAX_TRACKS], [0.0; MAX_TRACKS], [0.0; MAX_TRACKS], 8);
        seq.set_track_params([16; MAX_TRACKS], [0.0; MAX_TRACKS], [0.0; MAX_TRACKS], 16);
        seq.restart_pattern_from_step0();

        let triggers = seq.process_sample(120.0, 44_100.0, 0.0, GrooveType::Swing16);

        assert!(triggers[0].should_trigger);
        assert_eq!(triggers[0].step, 0);
        assert_eq!(seq.current_step(), 0);
    }

    #[test]
    fn test_push_pull_sync_to_host_preserves_phase() {
        // Kick on every step, with +30 ms push/pull.
        // Verify that after a sync_to_host the push/pull track keeps a stable
        // ~30 ms offset relative to a straight track, and that triggers stay
        // regular (no skipped or doubled steps).
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0b0000_0000_0001);
        }

        let mut seq = Sequencer::new(shared_pattern.clone());
        seq.tracks[0].push_pull_ms = 30.0;
        seq.play();

        let mut straight = Sequencer::new(shared_pattern);
        straight.play();

        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_step = (60.0 / bpm / 4.0 * sample_rate) as usize;

        // Run 2 bars, then sync to beat 2.0 in the third bar, then run 2 more bars.
        for _ in 0..(samples_per_step * 32) {
            seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            straight.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
        }
        seq.sync_to_host(2.0, bpm, sample_rate);
        straight.sync_to_host(2.0, bpm, sample_rate);

        let mut delayed_samples = Vec::new();
        let mut straight_samples = Vec::new();
        for sample_idx in 0..(samples_per_step * 32) {
            let t1 = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            let t2 = straight.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            if t1[0].should_trigger {
                delayed_samples.push(sample_idx);
            }
            if t2[0].should_trigger {
                straight_samples.push(sample_idx);
            }
        }

        // Roughly the same number of triggers after the sync (±1 because the
        // sync can land on either side of a step boundary).
        assert!(
            delayed_samples.len().abs_diff(straight_samples.len()) <= 1,
            "Push/pull and straight should produce roughly the same number of triggers after sync ({} vs {})",
            delayed_samples.len(),
            straight_samples.len()
        );
        assert!(
            straight_samples.len() >= 30 && straight_samples.len() <= 34,
            "Expected ~32 triggers after sync, got {}",
            straight_samples.len()
        );

        // Intervals between consecutive triggers must stay regular (one step
        // apart). Allow a small tolerance for the first interval after sync.
        for i in 1..delayed_samples.len() {
            let interval = delayed_samples[i] - delayed_samples[i - 1];
            assert!(
                interval >= samples_per_step - 10 && interval <= samples_per_step + 10,
                "Irregular push/pull interval at trigger {}: {} samples (expected ~{})",
                i,
                interval,
                samples_per_step
            );
        }

        // Average offset to the nearest straight trigger should be ~30 ms.
        let expected_delay = (0.030 * sample_rate as f64) as usize;
        let mut total_delay = 0_usize;
        let mut count = 0_usize;
        for &d in &delayed_samples {
            // Find the nearest straight trigger.
            let nearest = straight_samples
                .iter()
                .map(|&s| s.abs_diff(d))
                .min()
                .unwrap_or(0);
            total_delay += nearest;
            count += 1;
        }
        if count > 0 {
            let avg_delay = total_delay / count;
            assert!(
                avg_delay >= expected_delay - 100 && avg_delay <= expected_delay + 100,
                "Average delay mismatch: expected ~{}, got {} (expected_ms=30)",
                expected_delay,
                avg_delay
            );
        }
    }

    #[test]
    fn test_humanize_no_double_triggers() {
        // High humanize should never create two triggers for the same instrument
        // within the same step interval.
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        for step in 0..16 {
            shared_pattern.set_step_mask(step, 0b0000_0000_0001); // kick every step
        }

        let mut seq = Sequencer::new(shared_pattern);
        seq.tracks[0].humanize_amount = 1.0; // max humanize
        seq.play();

        let sample_rate = 44100.0;
        let bpm = 120.0;

        let mut trigger_count = 0_usize;
        let mut last_step = 99_usize;

        for _ in 0..88200 {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            if triggers[0].should_trigger {
                let current = seq.tracks[0].previous_step;
                assert!(
                    current != last_step,
                    "Double trigger detected on step {} (humanize should not duplicate)",
                    current
                );
                last_step = current;
                trigger_count += 1;
            }
        }

        // Over one bar we expect exactly 16 triggers (or very close).
        assert!(
            trigger_count >= 15 && trigger_count <= 17,
            "Expected ~16 triggers, got {} (no duplicates)",
            trigger_count
        );
    }

    #[test]
    fn test_fusion_filters_invalid_groups() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        let mut seq = Sequencer::new(shared_pattern);
        seq.set_fusions_for_test(
            0,
            &[
                FusedGroup {
                    start_cell: 0,
                    end_cell: 3,
                    step_count: 3,
                    ..Default::default()
                },
                FusedGroup {
                    start_cell: 5,
                    end_cell: 5,
                    step_count: 1,
                    ..Default::default()
                }, // single-cell
                FusedGroup {
                    start_cell: 14,
                    end_cell: 17,
                    step_count: 3,
                    ..Default::default()
                }, // crosses page
            ],
        );
        let kick_fusions = &seq.fusions[0];
        assert_eq!(
            kick_fusions.count, 1,
            "Invalid fusions should be filtered out"
        );
        assert_eq!(kick_fusions.groups[0].start_cell, 0);
        assert_eq!(kick_fusions.groups[0].end_cell, 3);
        assert_eq!(kick_fusions.groups[0].step_count, 3);
    }

    #[test]
    fn test_fusion_triggers_only_start_cell_with_pulse_metadata() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        // Start and internal cells are active to prove internals do not trigger independently.
        for step in 0..=3 {
            shared_pattern.set_step_mask(step, 0b0000_0000_0001);
        }

        let mut seq = Sequencer::new(shared_pattern);
        seq.set_fusions_for_test(
            0,
            &[FusedGroup {
                start_cell: 0,
                end_cell: 3,
                step_count: 3,
                ..Default::default()
            }],
        );
        seq.play();

        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_step = (60.0 / bpm / 4.0 * sample_rate) as usize;
        let mut hits = Vec::new();
        for _ in 0..(samples_per_step * 5) {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            if triggers[0].should_trigger {
                hits.push(triggers[0]);
            }
        }

        assert_eq!(
            hits.len(),
            1,
            "Fusion internals should be suppressed by the sequencer"
        );
        assert_eq!(hits[0].step, 0);
        assert_eq!(hits[0].fusion_pulse_count, 3);
        assert_eq!(hits[0].fusion_span_cells, 4);
        assert!(hits[0].is_fusion());
    }

    /// [283] A linked lane plays the fused groups of the lane it follows, but
    /// with its own morph (none until it creates one), never that lane's.
    #[test]
    fn linked_lane_plays_shared_fusion_with_its_own_morph() {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        shared_pattern.set_step_mask(0, 0b1);
        let mut group = FusedGroup {
            start_cell: 0,
            end_cell: 3,
            step_count: 3,
            ..Default::default()
        };
        group.set_morph_target(0, 300.0);
        shared_pattern.store_fusions(0, &[group]);

        let first_hits = |pattern: &Arc<SharedPattern>| {
            let mut seq = Sequencer::new(pattern.clone());
            let mut grid_slots: [usize; MAX_TRACKS] = std::array::from_fn(|i| i);
            grid_slots[1] = 0;
            seq.set_grid_slots(grid_slots);
            seq.sync_fusions_from_pattern();
            seq.play();
            let samples_per_step = (60.0 / 120.0 / 4.0 * 44100.0) as usize;
            let mut hits = [None, None];
            for _ in 0..samples_per_step {
                let triggers = seq.process_sample(120.0, 44100.0, 0.0, GrooveType::Straight);
                for lane in 0..2 {
                    if triggers[lane].should_trigger && hits[lane].is_none() {
                        hits[lane] = Some(triggers[lane]);
                    }
                }
            }
            hits.map(|hit| hit.expect("both lanes fire the shared fusion"))
        };

        let [lead, linked] = first_hits(&shared_pattern);
        assert_eq!((lead.fusion_pulse_count, linked.fusion_pulse_count), (3, 3));
        assert_eq!(lead.morph_count, 1);
        assert_eq!(linked.morph_count, 0, "not the followed lane's morph");

        let mut own = group;
        own.morph_targets[0].end_value = 7.0;
        shared_pattern.store_lane_fusion(1, 0, 0, own);
        let [lead, linked] = first_hits(&shared_pattern);
        assert_eq!(lead.morph_targets[0].end_value, 300.0);
        assert_eq!(linked.morph_count, 1);
        assert_eq!(linked.morph_targets[0].end_value, 7.0);
    }

    // -- Microtiming (per-cell nudge) -----------------------------------------

    /// Collect (sample_idx, trigger) for slot 0 over `total_samples`.
    fn collect_slot0_hits(
        seq: &mut Sequencer,
        bpm: f32,
        sample_rate: f32,
        total_samples: usize,
    ) -> Vec<(usize, TriggerResult)> {
        let mut hits = Vec::new();
        for sample_idx in 0..total_samples {
            let triggers = seq.process_sample(bpm, sample_rate, 0.0, GrooveType::Straight);
            if triggers[0].should_trigger {
                hits.push((sample_idx, triggers[0]));
            }
        }
        hits
    }

    fn one_cell_pattern(step: usize) -> Arc<SharedPattern> {
        let shared_pattern = SharedPattern::new(&Pattern::empty());
        shared_pattern.set_step_mask(step, 0b1);
        shared_pattern
    }

    fn microtiming_grid(step: usize, ms: f32) -> [[f32; SEQ_STEP_COUNT]; MAX_TRACKS] {
        let mut grid = [[0.0; SEQ_STEP_COUNT]; MAX_TRACKS];
        grid[0][step] = ms;
        grid
    }

    #[test]
    fn test_microtiming_positive_delays_trigger() {
        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_step = 60.0 / bpm / 4.0 * sample_rate; // 5512.5

        let baseline = {
            let mut seq = Sequencer::new(one_cell_pattern(1));
            seq.play();
            collect_slot0_hits(
                &mut seq,
                bpm,
                sample_rate,
                (samples_per_step * 3.0) as usize,
            )
        };
        let nudged = {
            let mut seq = Sequencer::new(one_cell_pattern(1));
            seq.set_microtimings(microtiming_grid(1, 25.0));
            seq.play();
            collect_slot0_hits(
                &mut seq,
                bpm,
                sample_rate,
                (samples_per_step * 3.0) as usize,
            )
        };

        assert_eq!(baseline.len(), 1);
        assert_eq!(nudged.len(), 1, "positive nudge must not double-fire");
        let expected_shift = 0.025 * sample_rate as f64; // 25 ms
        let shift = nudged[0].0 as f64 - baseline[0].0 as f64;
        assert!(
            (shift - expected_shift).abs() <= 2.0,
            "trigger should be ~25 ms late (shift = {shift} samples, expected {expected_shift})"
        );
        assert_eq!(nudged[0].1.step, 1);
        assert!(!nudged[0].1.early_next_loop);
    }

    #[test]
    fn test_microtiming_negative_fires_early() {
        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_step = 60.0 / bpm / 4.0 * sample_rate;

        let baseline = {
            let mut seq = Sequencer::new(one_cell_pattern(5));
            seq.play();
            collect_slot0_hits(
                &mut seq,
                bpm,
                sample_rate,
                (samples_per_step * 7.0) as usize,
            )
        };
        let nudged = {
            let mut seq = Sequencer::new(one_cell_pattern(5));
            seq.set_microtimings(microtiming_grid(5, -75.0));
            seq.play();
            collect_slot0_hits(
                &mut seq,
                bpm,
                sample_rate,
                (samples_per_step * 7.0) as usize,
            )
        };

        assert_eq!(baseline.len(), 1);
        assert_eq!(nudged.len(), 1, "negative nudge must not double-fire");
        let expected_shift = 0.075 * sample_rate as f64;
        let shift = baseline[0].0 as f64 - nudged[0].0 as f64;
        assert!(
            (shift - expected_shift).abs() <= 2.0,
            "trigger should be ~75 ms early (shift = {shift} samples, expected {expected_shift})"
        );
        assert_eq!(nudged[0].1.step, 5);
    }

    #[test]
    fn test_microtiming_negative_across_wrap_flags_next_loop() {
        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_bar = (60.0 / bpm * 4.0 * sample_rate) as usize; // 88200

        // Step 0 nudged early: the SECOND loop's step 0 must fire ~25 ms before
        // the wrap and report early_next_loop so conditions use loop_count + 1.
        let mut seq = Sequencer::new(one_cell_pattern(0));
        seq.set_microtimings(microtiming_grid(0, -25.0));
        seq.play();
        let hits = collect_slot0_hits(&mut seq, bpm, sample_rate, samples_per_bar + 100);

        assert_eq!(hits.len(), 2, "one hit per loop (start + early wrap)");
        let early = hits[1];
        let expected = samples_per_bar as f64 - 0.025 * sample_rate as f64;
        assert!(
            (early.0 as f64 - expected).abs() <= 3.0,
            "loop-1 step 0 should fire ~25 ms before the wrap (at {}, expected ~{expected})",
            early.0
        );
        assert!(
            early.1.early_next_loop,
            "early fire across the wrap must flag early_next_loop"
        );
    }

    // ── [212] Page loop ─────────────────────────────────────────────────────

    /// Slot 0 on every step of a 64-step pattern, slot 1 (16-step lane) too.
    fn full_pattern_64() -> Arc<SharedPattern> {
        let shared = SharedPattern::new(&Pattern::empty());
        for step in 0..64 {
            shared.set_step_mask(step, 0b11);
        }
        shared
    }

    fn master_steps_at_hits(seq: &mut Sequencer, samples: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for _ in 0..samples {
            let triggers = seq.process_sample(120.0, 44100.0, 0.0, GrooveType::Straight);
            if triggers[0].should_trigger {
                out.push((seq.tracks[0].previous_step, seq.tracks[1].previous_step));
            }
        }
        out
    }

    #[test]
    fn page_loop_keeps_the_playhead_in_the_page_and_realigns_shorter_lanes() {
        let mut seq = Sequencer::new(full_pattern_64());
        seq.set_track_params(
            [64, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16],
            [0.0; 14],
            [0.0; 14],
            64,
        );
        seq.set_page_loop(Some(2)); // steps 32..47
        seq.play();
        // Four bars = four page loops.
        let hits = master_steps_at_hits(&mut seq, 88200 * 4);
        assert!(hits.len() >= 60, "{} hits", hits.len());
        assert!(
            hits.iter().all(|(m, _)| (32..48).contains(m)),
            "master step left the page: {:?}",
            hits.iter().map(|h| h.0).collect::<Vec<_>>()
        );
        let masters: Vec<usize> = hits.iter().map(|h| h.0).collect();
        assert!(
            masters
                .windows(17)
                .any(|w| w[..16] == (32..48).collect::<Vec<_>>()[..] && w[16] == 32),
            "the page must come round: {masters:?}"
        );
        // The 16-step lane plays step k under master step 32 + k, every loop.
        for (m, lane) in &hits {
            assert_eq!(*lane, m - 32, "lane misaligned at master {m}");
        }
    }

    #[test]
    fn page_loop_engaged_mid_play_keeps_the_phase_within_the_page() {
        let mut seq = Sequencer::new(full_pattern_64());
        seq.set_track_params([64; 14], [0.0; 14], [0.0; 14], 64);
        seq.play();
        // Run to the middle of page 4 (step 52 = 13 beats).
        let samples_per_step = 5512.5f64;
        let n = (samples_per_step * 52.5) as usize;
        for _ in 0..n {
            seq.process_sample(120.0, 44100.0, 0.0, GrooveType::Straight);
        }
        assert_eq!(seq.current_step(), 52);
        seq.set_page_loop(Some(1)); // page 2 = steps 16..31
                                    // Step 52 is the fifth step of its page; so is 20.
        assert_eq!(seq.current_step(), 20, "same phase within the page");
        // The next hits follow the page, lane realigned to the master.
        let hits = master_steps_at_hits(&mut seq, (samples_per_step * 3.0) as usize);
        assert_eq!(
            hits.iter().map(|h| h.0).collect::<Vec<_>>(),
            vec![21, 22, 23],
            "{hits:?}"
        );
        // Releasing the loop continues from where the playhead is: no jump.
        seq.set_page_loop(None);
        let step = seq.current_step();
        assert!((20..24).contains(&step), "released at {step}");
    }

    #[test]
    fn page_loop_uses_the_partial_last_page_and_ignores_pages_beyond_the_pattern() {
        // 40 steps: page 3 is steps 32..39 and loops on those eight.
        let mut seq = Sequencer::new(full_pattern_64());
        seq.set_track_params([64; 14], [0.0; 14], [0.0; 14], 40);
        seq.set_page_loop(Some(2));
        seq.play();
        let masters: Vec<usize> = master_steps_at_hits(&mut seq, 88200 * 2)
            .iter()
            .map(|h| h.0)
            .collect();
        assert!(masters.iter().all(|m| (32..40).contains(m)), "{masters:?}");
        assert!(
            masters
                .windows(9)
                .any(|w| w == [32, 33, 34, 35, 36, 37, 38, 39, 32]),
            "{masters:?}"
        );

        // 32 steps: page 3 does not exist, the loop is off. (Lane lengths
        // follow the master here, as `resolve_track_length` does in the
        // plugin; `previous_step` is the LANE's step.)
        let mut seq = Sequencer::new(full_pattern_64());
        seq.set_track_params([32; 14], [0.0; 14], [0.0; 14], 32);
        seq.set_page_loop(Some(2));
        assert_eq!(seq.page_loop(), None);
        seq.play();
        let masters: Vec<usize> = master_steps_at_hits(&mut seq, 88200 * 2)
            .iter()
            .map(|h| h.0)
            .collect();
        assert!(
            masters
                .windows(33)
                .any(|w| w[0] == 0 && w[31] == 31 && w[32] == 0),
            "{masters:?}"
        );
    }

    #[test]
    fn push_pull_moves_the_audio_but_not_the_lane_playhead() {
        for push in [50.0f32, -50.0] {
            let mut seq = Sequencer::new(full_pattern_64());
            let mut pushes = [0.0f32; 14];
            pushes[0] = push;
            seq.set_track_params([16; 14], pushes, [0.0; 14], 16);
            seq.play();
            for n in 0..88200 {
                seq.process_sample(120.0, 44100.0, 0.0, GrooveType::Straight);
                let steps = seq.current_steps();
                assert_eq!(
                    steps[0], steps[1],
                    "push {push} ms: playhead of the pushed lane differs from the plain one at sample {n}"
                );
            }
        }
    }

    #[test]
    fn page_loop_host_sync_folds_the_host_position_into_the_page() {
        let mut seq = Sequencer::new(full_pattern_64());
        seq.set_track_params(
            [64, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 16],
            [0.0; 14],
            [0.0; 14],
            64,
        );
        seq.set_page_loop(Some(1)); // beats 4..8
        seq.play();
        // Host at beat 17 (= step 68 absolute): page phase 1 beat -> step 20.
        seq.sync_to_host(17.0, 120.0, 44100.0);
        assert_eq!(seq.current_step(), 20);
        assert_eq!(
            seq.loop_count(),
            4,
            "one loop per page round on the host timeline"
        );
        assert_eq!(seq.tracks[0].previous_step, 20);
        assert_eq!(
            seq.tracks[1].previous_step, 4,
            "16-step lane aligned to the page"
        );
        // The mapping the seek detector uses agrees.
        assert!((seq.host_to_local(17.0) - 5.0).abs() < 1e-9);
        assert!((seq.loop_span_beats() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn page_loop_early_fire_crosses_the_page_wrap() {
        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_bar = (60.0 / bpm * 4.0 * sample_rate) as usize;
        // Only step 16 (first of page 2) is active, nudged 25 ms early.
        let shared = SharedPattern::new(&Pattern::empty());
        shared.set_step_mask(16, 0b1);
        let mut seq = Sequencer::new(shared);
        seq.set_track_params([64; 14], [0.0; 14], [0.0; 14], 64);
        seq.set_microtimings(microtiming_grid(16, -25.0));
        seq.set_page_loop(Some(1));
        seq.play();
        let hits = collect_slot0_hits(&mut seq, bpm, sample_rate, samples_per_bar + 100);
        assert_eq!(
            hits.len(),
            2,
            "one hit per page loop (start + early wrap): {hits:?}"
        );
        let early = hits[1];
        let expected = samples_per_bar as f64 - 0.025 * sample_rate as f64;
        assert!(
            (early.0 as f64 - expected).abs() <= 3.0,
            "page 2's first step should fire ~25 ms before the page wrap (at {}, expected ~{expected})",
            early.0
        );
        assert_eq!(early.1.step, 16);
        assert!(early.1.early_next_loop);
    }

    #[test]
    fn test_microtiming_zero_keeps_grid() {
        let sample_rate = 44100.0;
        let bpm = 120.0;
        let samples_per_step = 60.0 / bpm / 4.0 * sample_rate;

        let baseline = {
            let mut seq = Sequencer::new(one_cell_pattern(3));
            seq.play();
            collect_slot0_hits(
                &mut seq,
                bpm,
                sample_rate,
                (samples_per_step * 5.0) as usize,
            )
        };
        let zeroed = {
            let mut seq = Sequencer::new(one_cell_pattern(3));
            seq.set_microtimings(microtiming_grid(3, 0.0));
            seq.play();
            collect_slot0_hits(
                &mut seq,
                bpm,
                sample_rate,
                (samples_per_step * 5.0) as usize,
            )
        };

        assert_eq!(baseline.len(), 1);
        assert_eq!(zeroed.len(), 1);
        assert_eq!(
            baseline[0].0, zeroed[0].0,
            "zero nudge must not move the hit"
        );
    }
}

// Stress tests module
#[cfg(test)]
mod stress_tests;
