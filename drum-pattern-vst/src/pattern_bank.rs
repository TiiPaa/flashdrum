//! Pattern bank — stores up to 16 pattern slots.
//!
//! Kept simple: each slot stores raw serialized bytes that can be
//! directly fed back into the plock / seq-plock / grid systems.

use crate::sequencer::pattern::{
    FusedGroup, FUSION_SLOT_COUNT, INSTRUMENT_COUNT, MAX_FUSIONS, STEP_COUNT,
};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, RwLock};

pub const SLOT_COUNT: usize = 16;

/// Max serialized size for sound plock state.
/// Uses actual constants so the buffer never under-allocates when FIELD_COUNT grows.
pub const MAX_PLOCK_BYTES: usize = crate::sequencer::pattern::INSTRUMENT_COUNT
        * crate::sequencer::pattern::STEP_COUNT
        * crate::plock::FIELD_COUNT
        * 4 // values
    + crate::sequencer::pattern::INSTRUMENT_COUNT * 8 // masks
    + crate::sequencer::pattern::INSTRUMENT_COUNT
        * crate::sequencer::pattern::STEP_COUNT
        * 8; // field_masks

/// Max serialized size for sequencer plock state.
pub const MAX_SEQ_PLOCK_BYTES: usize = crate::sequencer::pattern::INSTRUMENT_COUNT
        * crate::sequencer::pattern::STEP_COUNT
        * 4
        * 4 // 4 fields per cell
    + crate::sequencer::pattern::INSTRUMENT_COUNT * 8 // active masks
    + crate::sequencer::pattern::INSTRUMENT_COUNT * 8; // solo masks (appended, backward-compatible)

/// Max serialized size for fused groups.
pub const MAX_FUSION_BYTES: usize = crate::sequencer::pattern::INSTRUMENT_COUNT
    * crate::sequencer::pattern::MAX_FUSIONS
    * FUSION_SLOT_COUNT
    * 8;

/// A single saved pattern slot.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PatternSlot {
    /// Step masks for all 64 steps.
    #[serde(with = "serde_arrays")]
    pub step_masks: [u16; STEP_COUNT],
    /// Serialized sound plock state.
    pub plock_bytes: Vec<u8>,
    /// Serialized sequencer plock state.
    pub seq_plock_bytes: Vec<u8>,
    /// Serialized fused groups.
    pub fusion_bytes: Vec<u8>,
    /// Pattern length.
    pub pattern_length: u8,
    /// Occupied flag.
    pub occupied: bool,
}

impl Default for PatternSlot {
    fn default() -> Self {
        Self {
            step_masks: [0; STEP_COUNT],
            // Pre-allocate so capture() never allocates in the audio thread.
            plock_bytes: Vec::with_capacity(MAX_PLOCK_BYTES),
            seq_plock_bytes: Vec::with_capacity(MAX_SEQ_PLOCK_BYTES),
            fusion_bytes: Vec::with_capacity(MAX_FUSION_BYTES),
            pattern_length: 16,
            occupied: false,
        }
    }
}

/// Default repeat count for a song step (one play-through).
fn default_repeats() -> [u8; 64] {
    [1u8; 64]
}

/// Number of pattern blocks in the song sequence.
pub const SONG_BLOCKS: usize = 16;

/// A song sequence — an ordered chain of pattern slots.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SongSequence {
    /// Steps reference pattern slots (0-15 = P1-P16, -1 = empty/end).
    #[serde(with = "serde_arrays")]
    pub steps: [i8; 64],
    /// Number of active steps (1-SONG_BLOCKS).
    pub length: u8,
    /// Loop the song when reaching the end.
    pub loop_enabled: bool,
    /// How many times each step plays before advancing to the next step.
    #[serde(with = "serde_arrays", default = "default_repeats")]
    pub repeats: [u8; 64],
}

impl Default for SongSequence {
    fn default() -> Self {
        Self {
            steps: [-1; 64],
            length: SONG_BLOCKS as u8,
            loop_enabled: true,
            repeats: [1u8; 64],
        }
    }
}

impl SongSequence {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the slot index for a given song step, or None if empty/end.
    pub fn slot_at(&self, step: usize) -> Option<usize> {
        if step >= self.length as usize {
            return None;
        }
        let slot = self.steps[step];
        if slot < 0 || slot as usize >= SLOT_COUNT {
            None
        } else {
            Some(slot as usize)
        }
    }

    /// Set a step to a given slot index (-1 to clear).
    pub fn set_step(&mut self, step: usize, slot: i8) {
        if step < 64 {
            self.steps[step] = slot.clamp(-1, SLOT_COUNT as i8 - 1);
        }
    }

    /// Return the repeat count for a given song step, clamped to at least 1.
    pub fn repeat_at(&self, step: usize) -> u8 {
        self.repeats.get(step).copied().unwrap_or(1).max(1)
    }

    /// Set the repeat count for a given song step (clamped to 1..255).
    pub fn set_repeat(&mut self, step: usize, count: u8) {
        if step < 64 {
            self.repeats[step] = count.max(1);
        }
    }
}

/// Bank of 16 pattern slots + one song sequence.
#[derive(Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct PatternBank {
    /// The saved pattern slots (P1-P16).
    ///
    /// Deserialized length-tolerantly: sessions saved with a smaller bank
    /// (e.g. the legacy 8-slot `pattern-bank-v1`) fill the first N slots and
    /// leave the rest empty, instead of failing the whole `from_slice` and
    /// silently reloading an empty bank. A longer array (future growth) is
    /// truncated to `SLOT_COUNT`.
    #[serde(deserialize_with = "deserialize_slots_padded")]
    pub slots: [PatternSlot; SLOT_COUNT],
    pub song: SongSequence,
}

/// Length-tolerant deserializer for the `slots` array (see field docs).
fn deserialize_slots_padded<'de, D>(deserializer: D) -> Result<[PatternSlot; SLOT_COUNT], D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    let mut slots: Vec<PatternSlot> = Vec::deserialize(deserializer)?;
    slots.truncate(SLOT_COUNT);
    while slots.len() < SLOT_COUNT {
        slots.push(PatternSlot::default());
    }
    slots
        .try_into()
        .map_err(|_| serde::de::Error::custom("pattern bank slots length normalization failed"))
}

impl PatternSlot {
    /// Capture the current state from the plugin into this slot.
    pub fn capture(
        &mut self,
        pattern: &crate::sequencer::pattern::SharedPattern,
        plock_state: &crate::plock::PlockState,
        seq_plock_state: &crate::plock::SequencerPlockState,
        pattern_length: u8,
    ) {
        use crate::plock::{FIELD_COUNT, STEP_COUNT};
        use crate::sequencer::pattern::INSTRUMENT_COUNT;
        use std::sync::atomic::Ordering;

        self.step_masks = pattern.step_masks();

        // Serialize plock state — clear pre-allocated buffer (no new allocation).
        self.plock_bytes.clear();
        for inst in 0..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                for field in 0..FIELD_COUNT {
                    self.plock_bytes.extend_from_slice(
                        &plock_state.values.get(inst, step, field).to_le_bytes(),
                    );
                }
            }
        }
        for inst in 0..INSTRUMENT_COUNT {
            self.plock_bytes.extend_from_slice(
                &plock_state.masks.masks[inst]
                    .load(Ordering::Relaxed)
                    .to_le_bytes(),
            );
        }
        for inst in 0..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                self.plock_bytes
                    .extend_from_slice(&plock_state.field_masks.get(inst, step).to_le_bytes());
            }
        }

        // Serialize seq plock state — clear pre-allocated buffer (no new allocation).
        self.seq_plock_bytes.clear();
        for inst in 0..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                self.seq_plock_bytes.extend_from_slice(
                    &f32::from_bits(
                        seq_plock_state.probabilities[inst][step].load(Ordering::Relaxed),
                    )
                    .to_le_bytes(),
                );
                self.seq_plock_bytes.extend_from_slice(
                    &f32::from_bits(seq_plock_state.stutters[inst][step].load(Ordering::Relaxed))
                        .to_le_bytes(),
                );
                self.seq_plock_bytes.extend_from_slice(
                    &seq_plock_state.conditions[inst][step]
                        .load(Ordering::Relaxed)
                        .to_le_bytes(),
                );
                self.seq_plock_bytes.extend_from_slice(
                    &f32::from_bits(
                        seq_plock_state.microtimings[inst][step].load(Ordering::Relaxed),
                    )
                    .to_le_bytes(),
                );
            }
        }
        for inst in 0..INSTRUMENT_COUNT {
            self.seq_plock_bytes.extend_from_slice(
                &seq_plock_state.masks[inst]
                    .load(Ordering::Relaxed)
                    .to_le_bytes(),
            );
        }
        // Step-solo masks, APPENDED after the active masks so older blobs (which
        // stop here) still parse; solos default to 0 when the trailing block is
        // absent. See restore_from_buffers.
        for inst in 0..INSTRUMENT_COUNT {
            self.seq_plock_bytes.extend_from_slice(
                &seq_plock_state.solo_masks[inst]
                    .load(Ordering::Relaxed)
                    .to_le_bytes(),
            );
        }

        // Serialize fused groups — clear pre-allocated buffer (no new allocation).
        // [283] A linked lane's own morphs follow its groups, as in the live list.
        self.fusion_bytes.clear();
        for inst in 0..INSTRUMENT_COUNT {
            let mut groups = [FusedGroup::default(); MAX_FUSIONS];
            let group_count = pattern.load_fusions_into(inst, &mut groups);
            let mut morphs = [FusedGroup::default(); MAX_FUSIONS];
            let morph_count = pattern
                .load_linked_morphs_into(inst, &mut morphs)
                .min(MAX_FUSIONS - 1 - group_count.min(MAX_FUSIONS - 1));
            let mut written = 0usize;
            // Count slot
            self.fusion_bytes.extend_from_slice(
                &(((group_count + morph_count) as u64) | (1u64 << 24)).to_le_bytes(),
            );
            written += 1;
            let records = groups[..group_count]
                .iter()
                .map(crate::sequencer::pattern::pack_fusion)
                .chain(
                    morphs[..morph_count]
                        .iter()
                        .map(crate::sequencer::pattern::pack_linked_morph),
                );
            for packed in records {
                for slot in &packed {
                    self.fusion_bytes.extend_from_slice(&slot.to_le_bytes());
                }
                written += 1;
            }
            for _ in written..MAX_FUSIONS {
                for _ in 0..FUSION_SLOT_COUNT {
                    self.fusion_bytes.extend_from_slice(&0u64.to_le_bytes());
                }
            }
        }

        self.pattern_length = pattern_length.clamp(1, 64);
        self.occupied = true;
    }

    /// Erase every trace of one lane from this saved pattern: its steps, its
    /// fusions, its sound p-locks and its sequencer p-locks. The other lanes
    /// are untouched.
    ///
    /// Deleting a lane deliberately leaves its musical data in place (so an
    /// accidental delete costs nothing), which means a freshly placed
    /// instrument would otherwise inherit the previous lane's steps — in the
    /// live pattern AND in every pattern saved before the deletion. This is
    /// what makes a new lane come up blank on all 16 patterns.
    pub fn clear_lane(&mut self, lane: usize) {
        if lane >= INSTRUMENT_COUNT {
            return;
        }
        for mask in self.step_masks.iter_mut() {
            *mask &= !(1u16 << lane);
        }
        if let Some(regions) = plock_lane_regions(self.plock_bytes.len()) {
            clear_lane_in_blob(&mut self.plock_bytes, regions, lane);
        }
        if let Some(regions) = seq_plock_lane_regions(self.seq_plock_bytes.len()) {
            clear_lane_in_blob(&mut self.seq_plock_bytes, regions, lane);
        }
        if let Some(regions) = fusion_lane_regions(self.fusion_bytes.len()) {
            // A zeroed block reads back as "no fusions": both formats take the
            // group count from the low byte of the leading word.
            clear_lane_in_blob(&mut self.fusion_bytes, regions, lane);
        }
    }

    /// Reorder this saved pattern's lanes, `order[new] = old` — the convention
    /// the grid's lane drag already uses.
    ///
    /// Without this, dragging a lane permuted the LIVE pattern only and every
    /// saved pattern kept the old lane assignment, so recalling one put each
    /// instrument's steps back on the lane it used to occupy.
    pub fn permute_lanes(&mut self, order: &[usize; INSTRUMENT_COUNT]) {
        for mask in self.step_masks.iter_mut() {
            let previous = *mask;
            let mut moved = 0u16;
            for (new_lane, &old_lane) in order.iter().enumerate() {
                if previous & (1u16 << old_lane) != 0 {
                    moved |= 1u16 << new_lane;
                }
            }
            *mask = moved;
        }
        if let Some(regions) = plock_lane_regions(self.plock_bytes.len()) {
            permute_lanes_in_blob(&mut self.plock_bytes, regions, order);
        }
        if let Some(regions) = seq_plock_lane_regions(self.seq_plock_bytes.len()) {
            permute_lanes_in_blob(&mut self.seq_plock_bytes, regions, order);
        }
        if let Some(regions) = fusion_lane_regions(self.fusion_bytes.len()) {
            permute_lanes_in_blob(&mut self.fusion_bytes, regions, order);
        }
    }

    /// Copy slot data into temporary buffers so the caller can release the
    /// bank lock before doing the (expensive) restore. Returns the pattern
    /// length if the slot is occupied, otherwise None.
    pub fn copy_data_for_restore(
        &self,
        step_masks_out: &mut [u16; STEP_COUNT],
        plock_bytes_out: &mut [u8],
        seq_plock_bytes_out: &mut [u8],
        fusion_bytes_out: &mut [u8],
    ) -> Option<u8> {
        if !self.occupied {
            return None;
        }
        step_masks_out.copy_from_slice(&self.step_masks);

        let plock_len = self.plock_bytes.len().min(plock_bytes_out.len());
        let seq_plock_len = self.seq_plock_bytes.len().min(seq_plock_bytes_out.len());
        let fusion_len = self.fusion_bytes.len().min(fusion_bytes_out.len());
        plock_bytes_out[..plock_len].copy_from_slice(&self.plock_bytes[..plock_len]);
        seq_plock_bytes_out[..seq_plock_len]
            .copy_from_slice(&self.seq_plock_bytes[..seq_plock_len]);
        fusion_bytes_out[..fusion_len].copy_from_slice(&self.fusion_bytes[..fusion_len]);

        Some(self.pattern_length)
    }

    /// Restore this slot's data into the plugin state.
    pub fn restore(
        &self,
        pattern: &crate::sequencer::pattern::SharedPattern,
        plock_state: &crate::plock::PlockState,
        seq_plock_state: &crate::plock::SequencerPlockState,
    ) -> Option<u8> {
        use crate::plock::{FIELD_COUNT, STEP_COUNT};
        use crate::sequencer::pattern::INSTRUMENT_COUNT;
        use std::sync::atomic::Ordering;

        if !self.occupied {
            return None;
        }

        pattern.load_step_masks(&self.step_masks);

        // Detect field count from data size to support legacy slots (FIELD_COUNT=18)
        // and current slots (FIELD_COUNT=46).
        let legacy_field_count = 18usize;
        let size_for = |fields: usize| {
            INSTRUMENT_COUNT * STEP_COUNT * fields * 4
                + INSTRUMENT_COUNT * 8
                + INSTRUMENT_COUNT * STEP_COUNT * 8
        };

        // [286] 48 fields (sends), then the 46 before them, then the 18 of the oldest.
        let field_count = if self.plock_bytes.len() >= size_for(FIELD_COUNT) {
            FIELD_COUNT
        } else if self.plock_bytes.len() >= size_for(crate::param_id::PRE_SEND_FIELD_COUNT) {
            crate::param_id::PRE_SEND_FIELD_COUNT
        } else if self.plock_bytes.len() >= size_for(legacy_field_count) {
            legacy_field_count
        } else {
            0
        };

        if field_count > 0 {
            let mut offset = 0usize;
            for inst in 0..INSTRUMENT_COUNT {
                for step in 0..STEP_COUNT {
                    // [286] A slot saved with fewer fields leaves the newer ones empty.
                    for field in field_count..FIELD_COUNT {
                        plock_state.values.set(inst, step, field, 0.0);
                    }
                    for field in 0..field_count {
                        let val = f32::from_le_bytes([
                            self.plock_bytes[offset],
                            self.plock_bytes[offset + 1],
                            self.plock_bytes[offset + 2],
                            self.plock_bytes[offset + 3],
                        ]);
                        plock_state.values.set(inst, step, field, val);
                        offset += 4;
                    }
                }
            }
            for inst in 0..INSTRUMENT_COUNT {
                let mask = u64::from_le_bytes([
                    self.plock_bytes[offset],
                    self.plock_bytes[offset + 1],
                    self.plock_bytes[offset + 2],
                    self.plock_bytes[offset + 3],
                    self.plock_bytes[offset + 4],
                    self.plock_bytes[offset + 5],
                    self.plock_bytes[offset + 6],
                    self.plock_bytes[offset + 7],
                ]);
                offset += 8;
                plock_state.masks.masks[inst].store(mask, Ordering::Release);
            }
            for inst in 0..INSTRUMENT_COUNT {
                for step in 0..STEP_COUNT {
                    let mask = u64::from_le_bytes([
                        self.plock_bytes[offset],
                        self.plock_bytes[offset + 1],
                        self.plock_bytes[offset + 2],
                        self.plock_bytes[offset + 3],
                        self.plock_bytes[offset + 4],
                        self.plock_bytes[offset + 5],
                        self.plock_bytes[offset + 6],
                        self.plock_bytes[offset + 7],
                    ]);
                    offset += 8;
                    // set_raw: `set` takes a field INDEX (see restore_from_buffers).
                    plock_state.field_masks.set_raw(inst, step, mask);
                }
            }
        }

        // Deserialize seq plock state
        let cell_count = INSTRUMENT_COUNT * STEP_COUNT;
        let expected_seq_size = cell_count * 4 * 4 + INSTRUMENT_COUNT * 8;
        if self.seq_plock_bytes.len() >= expected_seq_size {
            let mut offset = 0;
            for inst in 0..INSTRUMENT_COUNT {
                for step in 0..STEP_COUNT {
                    let prob = f32::from_le_bytes([
                        self.seq_plock_bytes[offset],
                        self.seq_plock_bytes[offset + 1],
                        self.seq_plock_bytes[offset + 2],
                        self.seq_plock_bytes[offset + 3],
                    ]);
                    offset += 4;
                    let stutter = f32::from_le_bytes([
                        self.seq_plock_bytes[offset],
                        self.seq_plock_bytes[offset + 1],
                        self.seq_plock_bytes[offset + 2],
                        self.seq_plock_bytes[offset + 3],
                    ]);
                    offset += 4;
                    let condition = u32::from_le_bytes([
                        self.seq_plock_bytes[offset],
                        self.seq_plock_bytes[offset + 1],
                        self.seq_plock_bytes[offset + 2],
                        self.seq_plock_bytes[offset + 3],
                    ]);
                    offset += 4;
                    let micro = f32::from_le_bytes([
                        self.seq_plock_bytes[offset],
                        self.seq_plock_bytes[offset + 1],
                        self.seq_plock_bytes[offset + 2],
                        self.seq_plock_bytes[offset + 3],
                    ]);
                    offset += 4;

                    seq_plock_state.probabilities[inst][step]
                        .store(prob.to_bits(), Ordering::Release);
                    seq_plock_state.stutters[inst][step]
                        .store(stutter.to_bits(), Ordering::Release);
                    seq_plock_state.conditions[inst][step].store(condition, Ordering::Release);
                    seq_plock_state.microtimings[inst][step]
                        .store(micro.to_bits(), Ordering::Release);
                }
            }
            for inst in 0..INSTRUMENT_COUNT {
                let mask = u64::from_le_bytes([
                    self.seq_plock_bytes[offset],
                    self.seq_plock_bytes[offset + 1],
                    self.seq_plock_bytes[offset + 2],
                    self.seq_plock_bytes[offset + 3],
                    self.seq_plock_bytes[offset + 4],
                    self.seq_plock_bytes[offset + 5],
                    self.seq_plock_bytes[offset + 6],
                    self.seq_plock_bytes[offset + 7],
                ]);
                offset += 8;
                seq_plock_state.masks[inst].store(mask, Ordering::Release);
            }
            // Optional trailing step-solo masks (absent in legacy blobs).
            if self.seq_plock_bytes.len() >= offset + INSTRUMENT_COUNT * 8 {
                for inst in 0..INSTRUMENT_COUNT {
                    let solo = u64::from_le_bytes([
                        self.seq_plock_bytes[offset],
                        self.seq_plock_bytes[offset + 1],
                        self.seq_plock_bytes[offset + 2],
                        self.seq_plock_bytes[offset + 3],
                        self.seq_plock_bytes[offset + 4],
                        self.seq_plock_bytes[offset + 5],
                        self.seq_plock_bytes[offset + 6],
                        self.seq_plock_bytes[offset + 7],
                    ]);
                    offset += 8;
                    seq_plock_state.solo_masks[inst].store(solo, Ordering::Release);
                }
            } else {
                for inst in 0..INSTRUMENT_COUNT {
                    seq_plock_state.solo_masks[inst].store(0, Ordering::Release);
                }
            }
        }

        // Deserialize fused groups.
        deserialize_fusions(&self.fusion_bytes, pattern);

        Some(self.pattern_length)
    }
}

/// Restore pattern / plock / seq-plock data from raw buffers.
/// This is the same logic as `PatternSlot::restore` but decoupled from the
/// slot so the bank lock can be released before the (expensive) restore runs.
/// Field count of the legacy `plock` layout, still readable in saved sessions.
/// Kept in step with the gate in [`restore_from_buffers`].
const LEGACY_PLOCK_FIELD_COUNT: usize = 18;

/// Byte layout of one serialized blob, seen as per-lane strided regions.
///
/// Everything [`PatternSlot::capture`] writes is positional and emits all
/// `INSTRUMENT_COUNT` lanes at a fixed stride, so one lane's bytes can be
/// zeroed or moved without deserializing the rest. Each region is
/// `(byte offset of lane 0, bytes per lane)`.
#[derive(Clone, Copy)]
struct LaneRegions {
    regions: [(usize, usize); 3],
    count: usize,
}

impl LaneRegions {
    fn new(regions: &[(usize, usize)]) -> Self {
        let mut out = [(0usize, 0usize); 3];
        out[..regions.len()].copy_from_slice(regions);
        Self {
            regions: out,
            count: regions.len(),
        }
    }

    fn iter(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.regions[..self.count].iter().copied()
    }

    /// Byte range of one lane inside a region.
    fn range((offset, stride): (usize, usize), lane: usize) -> std::ops::Range<usize> {
        let start = offset + lane * stride;
        start..start + stride
    }
}

/// Regions of `plock_bytes`: the values, the per-lane masks, then the per-step
/// field masks. The current and legacy field counts are probed in the same
/// order as [`restore_from_buffers`], so a legacy blob is edited with ITS
/// stride instead of being corrupted by the current one.
fn plock_lane_regions(len: usize) -> Option<LaneRegions> {
    let masks = INSTRUMENT_COUNT * 8;
    let field_masks = INSTRUMENT_COUNT * STEP_COUNT * 8;
    for field_count in [
        crate::plock::FIELD_COUNT,
        crate::param_id::PRE_SEND_FIELD_COUNT,
        LEGACY_PLOCK_FIELD_COUNT,
    ] {
        let values = INSTRUMENT_COUNT * STEP_COUNT * field_count * 4;
        if len >= values + masks + field_masks {
            return Some(LaneRegions::new(&[
                (0, STEP_COUNT * field_count * 4),
                (values, 8),
                (values + masks, STEP_COUNT * 8),
            ]));
        }
    }
    None
}

/// Regions of `seq_plock_bytes`: the interleaved per-step cells, the active
/// masks, and the step-solo masks — that last block is optional, legacy blobs
/// stop before it.
fn seq_plock_lane_regions(len: usize) -> Option<LaneRegions> {
    // probability, stutter, condition, microtiming — 4 bytes each.
    const CELL_BYTES: usize = 4 * 4;
    let cells = INSTRUMENT_COUNT * STEP_COUNT * CELL_BYTES;
    let masks = INSTRUMENT_COUNT * 8;
    if len < cells + masks {
        return None;
    }
    let solos = cells + masks;
    let cell_region = (0, STEP_COUNT * CELL_BYTES);
    if len >= solos + INSTRUMENT_COUNT * 8 {
        Some(LaneRegions::new(&[cell_region, (cells, 8), (solos, 8)]))
    } else {
        Some(LaneRegions::new(&[cell_region, (cells, 8)]))
    }
}

/// Regions of `fusion_bytes`. Both the current format (a bare count word then
/// `MAX_FUSIONS - 1` group slots) and the legacy one-word-per-group format are
/// a single fixed-stride block per lane. Probed largest first, like
/// [`deserialize_fusions`].
fn fusion_lane_regions(len: usize) -> Option<LaneRegions> {
    let current = 8 + (MAX_FUSIONS - 1) * FUSION_SLOT_COUNT * 8;
    let legacy = MAX_FUSIONS * 8;
    for stride in [current, legacy] {
        if len >= INSTRUMENT_COUNT * stride {
            return Some(LaneRegions::new(&[(0, stride)]));
        }
    }
    None
}

/// Zero one lane's bytes in every region of a blob.
fn clear_lane_in_blob(bytes: &mut [u8], regions: LaneRegions, lane: usize) {
    for region in regions.iter() {
        let range = LaneRegions::range(region, lane);
        if range.end <= bytes.len() {
            bytes[range].fill(0);
        }
    }
}

/// Reorder the lanes of a blob, `order[new] = old`.
fn permute_lanes_in_blob(
    bytes: &mut [u8],
    regions: LaneRegions,
    order: &[usize; INSTRUMENT_COUNT],
) {
    let previous = bytes.to_vec();
    for region in regions.iter() {
        for (new_lane, &old_lane) in order.iter().enumerate() {
            let dst = LaneRegions::range(region, new_lane);
            let src = LaneRegions::range(region, old_lane);
            if dst.end <= bytes.len() && src.end <= previous.len() {
                bytes[dst].copy_from_slice(&previous[src]);
            }
        }
    }
}

pub fn restore_from_buffers(
    step_masks: &[u16; STEP_COUNT],
    plock_bytes: &[u8],
    seq_plock_bytes: &[u8],
    fusion_bytes: &[u8],
    pattern: &crate::sequencer::pattern::SharedPattern,
    plock_state: &crate::plock::PlockState,
    seq_plock_state: &crate::plock::SequencerPlockState,
) {
    use crate::plock::{FIELD_COUNT, STEP_COUNT};
    use crate::sequencer::pattern::INSTRUMENT_COUNT;
    use std::sync::atomic::Ordering;

    pattern.load_step_masks(step_masks);

    // Detect field count from data size to support legacy slots (FIELD_COUNT=18)
    // and current slots (FIELD_COUNT=46).
    let legacy_field_count = 18usize;
    let size_for = |fields: usize| {
        INSTRUMENT_COUNT * STEP_COUNT * fields * 4
            + INSTRUMENT_COUNT * 8
            + INSTRUMENT_COUNT * STEP_COUNT * 8
    };

    // [286] 48 fields (sends), then the 46 before them, then the 18 of the oldest.
    let (field_count, has_plock_data) = if plock_bytes.len() >= size_for(FIELD_COUNT) {
        (FIELD_COUNT, true)
    } else if plock_bytes.len() >= size_for(crate::param_id::PRE_SEND_FIELD_COUNT) {
        (crate::param_id::PRE_SEND_FIELD_COUNT, true)
    } else if plock_bytes.len() >= size_for(legacy_field_count) {
        (legacy_field_count, true)
    } else {
        (FIELD_COUNT, false)
    };

    if has_plock_data {
        let mut offset = 0usize;
        for inst in 0..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                // [286] Data saved with fewer fields leaves the newer ones empty.
                for field in field_count..FIELD_COUNT {
                    plock_state.values.set(inst, step, field, 0.0);
                }
                for field in 0..field_count {
                    let val = f32::from_le_bytes([
                        plock_bytes[offset],
                        plock_bytes[offset + 1],
                        plock_bytes[offset + 2],
                        plock_bytes[offset + 3],
                    ]);
                    plock_state.values.set(inst, step, field, val);
                    offset += 4;
                }
            }
        }
        for inst in 0..INSTRUMENT_COUNT {
            let mask = u64::from_le_bytes([
                plock_bytes[offset],
                plock_bytes[offset + 1],
                plock_bytes[offset + 2],
                plock_bytes[offset + 3],
                plock_bytes[offset + 4],
                plock_bytes[offset + 5],
                plock_bytes[offset + 6],
                plock_bytes[offset + 7],
            ]);
            offset += 8;
            plock_state.masks.masks[inst].store(mask, Ordering::Release);
        }
        for inst in 0..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                let mask = u64::from_le_bytes([
                    plock_bytes[offset],
                    plock_bytes[offset + 1],
                    plock_bytes[offset + 2],
                    plock_bytes[offset + 3],
                    plock_bytes[offset + 4],
                    plock_bytes[offset + 5],
                    plock_bytes[offset + 6],
                    plock_bytes[offset + 7],
                ]);
                offset += 8;
                // set_raw, NOT set: `set` takes a field INDEX (it ORs
                // `1 << field`), which silently corrupted the mask on every
                // pattern load (bank slots and pattern presets).
                plock_state.field_masks.set_raw(inst, step, mask);
            }
        }
    }

    let cell_count = INSTRUMENT_COUNT * STEP_COUNT;
    let expected_seq_size = cell_count * 4 * 4 + INSTRUMENT_COUNT * 8;
    if seq_plock_bytes.len() >= expected_seq_size {
        let mut offset = 0;
        for inst in 0..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                let prob = f32::from_le_bytes([
                    seq_plock_bytes[offset],
                    seq_plock_bytes[offset + 1],
                    seq_plock_bytes[offset + 2],
                    seq_plock_bytes[offset + 3],
                ]);
                offset += 4;
                let stutter = f32::from_le_bytes([
                    seq_plock_bytes[offset],
                    seq_plock_bytes[offset + 1],
                    seq_plock_bytes[offset + 2],
                    seq_plock_bytes[offset + 3],
                ]);
                offset += 4;
                let condition = u32::from_le_bytes([
                    seq_plock_bytes[offset],
                    seq_plock_bytes[offset + 1],
                    seq_plock_bytes[offset + 2],
                    seq_plock_bytes[offset + 3],
                ]);
                offset += 4;
                let micro = f32::from_le_bytes([
                    seq_plock_bytes[offset],
                    seq_plock_bytes[offset + 1],
                    seq_plock_bytes[offset + 2],
                    seq_plock_bytes[offset + 3],
                ]);
                offset += 4;

                seq_plock_state.probabilities[inst][step].store(prob.to_bits(), Ordering::Release);
                seq_plock_state.stutters[inst][step].store(stutter.to_bits(), Ordering::Release);
                seq_plock_state.conditions[inst][step].store(condition, Ordering::Release);
                seq_plock_state.microtimings[inst][step].store(micro.to_bits(), Ordering::Release);
            }
        }
        for inst in 0..INSTRUMENT_COUNT {
            let mask = u64::from_le_bytes([
                seq_plock_bytes[offset],
                seq_plock_bytes[offset + 1],
                seq_plock_bytes[offset + 2],
                seq_plock_bytes[offset + 3],
                seq_plock_bytes[offset + 4],
                seq_plock_bytes[offset + 5],
                seq_plock_bytes[offset + 6],
                seq_plock_bytes[offset + 7],
            ]);
            offset += 8;
            seq_plock_state.masks[inst].store(mask, Ordering::Release);
        }
        // Step-solo masks: an OPTIONAL trailing block. Legacy `pattern-bank-v1`
        // blobs stop after the active masks, so absence means "no solos".
        if seq_plock_bytes.len() >= offset + INSTRUMENT_COUNT * 8 {
            for inst in 0..INSTRUMENT_COUNT {
                let solo = u64::from_le_bytes([
                    seq_plock_bytes[offset],
                    seq_plock_bytes[offset + 1],
                    seq_plock_bytes[offset + 2],
                    seq_plock_bytes[offset + 3],
                    seq_plock_bytes[offset + 4],
                    seq_plock_bytes[offset + 5],
                    seq_plock_bytes[offset + 6],
                    seq_plock_bytes[offset + 7],
                ]);
                offset += 8;
                seq_plock_state.solo_masks[inst].store(solo, Ordering::Release);
            }
        } else {
            for inst in 0..INSTRUMENT_COUNT {
                seq_plock_state.solo_masks[inst].store(0, Ordering::Release);
            }
        }
    }

    // Deserialize fused groups.
    deserialize_fusions(fusion_bytes, pattern);
}

fn deserialize_fusions(fusion_bytes: &[u8], pattern: &crate::sequencer::pattern::SharedPattern) {
    decode_fusions(fusion_bytes, |inst, groups, morphs| {
        pattern.store_fusions_and_morphs(inst, groups, morphs)
    });
}

/// Decode a fusion blob (current or legacy format) lane by lane into `sink`:
/// the lane's fused groups, then its linked-lane morphs ([283]; none in the
/// legacy format). Returns false when the blob is in neither format (nothing
/// decoded).
fn decode_fusions(
    fusion_bytes: &[u8],
    mut sink: impl FnMut(usize, &[FusedGroup], &[FusedGroup]),
) -> bool {
    // Actual serialized size of the current format, per instrument: one bare
    // count u64 (8 B) followed by (MAX_FUSIONS - 1) group slots of
    // FUSION_SLOT_COUNT u64 each — matching `PatternSlot::capture`. (The count is
    // NOT padded to a full FUSION_SLOT_COUNT slot, so this is 16 B/instrument
    // less than `MAX_FUSION_BYTES`; the old gate used that larger buffer size and
    // therefore rejected every real blob, silently dropping saved fusions.)
    let expected_new = INSTRUMENT_COUNT * (8 + (MAX_FUSIONS - 1) * FUSION_SLOT_COUNT * 8);
    let expected_old = INSTRUMENT_COUNT * MAX_FUSIONS * 8;

    if fusion_bytes.len() >= expected_new {
        let mut offset = 0usize;
        for inst in 0..INSTRUMENT_COUNT {
            let count_packed = u64::from_le_bytes([
                fusion_bytes[offset],
                fusion_bytes[offset + 1],
                fusion_bytes[offset + 2],
                fusion_bytes[offset + 3],
                fusion_bytes[offset + 4],
                fusion_bytes[offset + 5],
                fusion_bytes[offset + 6],
                fusion_bytes[offset + 7],
            ]);
            offset += 8;
            let count = (count_packed & 0xFF) as usize;
            let mut groups = [FusedGroup::default(); MAX_FUSIONS - 1];
            let mut group_count = 0usize;
            let mut morphs = [FusedGroup::default(); MAX_FUSIONS - 1];
            let mut morph_count = 0usize;
            for _ in 0..count.min(MAX_FUSIONS - 1) {
                let slots = [
                    u64::from_le_bytes([
                        fusion_bytes[offset],
                        fusion_bytes[offset + 1],
                        fusion_bytes[offset + 2],
                        fusion_bytes[offset + 3],
                        fusion_bytes[offset + 4],
                        fusion_bytes[offset + 5],
                        fusion_bytes[offset + 6],
                        fusion_bytes[offset + 7],
                    ]),
                    u64::from_le_bytes([
                        fusion_bytes[offset + 8],
                        fusion_bytes[offset + 9],
                        fusion_bytes[offset + 10],
                        fusion_bytes[offset + 11],
                        fusion_bytes[offset + 12],
                        fusion_bytes[offset + 13],
                        fusion_bytes[offset + 14],
                        fusion_bytes[offset + 15],
                    ]),
                    u64::from_le_bytes([
                        fusion_bytes[offset + 16],
                        fusion_bytes[offset + 17],
                        fusion_bytes[offset + 18],
                        fusion_bytes[offset + 19],
                        fusion_bytes[offset + 20],
                        fusion_bytes[offset + 21],
                        fusion_bytes[offset + 22],
                        fusion_bytes[offset + 23],
                    ]),
                ];
                offset += FUSION_SLOT_COUNT * 8;
                if let Some(group) = crate::sequencer::pattern::unpack_fusion(slots) {
                    groups[group_count] = group;
                    group_count += 1;
                } else if let Some(morph) = crate::sequencer::pattern::unpack_linked_morph(slots) {
                    morphs[morph_count] = morph;
                    morph_count += 1;
                }
            }
            offset += (MAX_FUSIONS - 1 - count.min(MAX_FUSIONS - 1)) * FUSION_SLOT_COUNT * 8;
            sink(inst, &groups[..group_count], &morphs[..morph_count]);
        }
    } else if fusion_bytes.len() >= expected_old {
        // Legacy single-u64 format: migrate to multi-target on load.
        let mut offset = 0usize;
        for inst in 0..INSTRUMENT_COUNT {
            let count_packed = u64::from_le_bytes([
                fusion_bytes[offset],
                fusion_bytes[offset + 1],
                fusion_bytes[offset + 2],
                fusion_bytes[offset + 3],
                fusion_bytes[offset + 4],
                fusion_bytes[offset + 5],
                fusion_bytes[offset + 6],
                fusion_bytes[offset + 7],
            ]);
            offset += 8;
            let count = (count_packed & 0xFF) as usize;
            let mut groups = [FusedGroup::default(); MAX_FUSIONS - 1];
            let mut group_count = 0usize;
            for _ in 0..count.min(MAX_FUSIONS - 1) {
                let packed = u64::from_le_bytes([
                    fusion_bytes[offset],
                    fusion_bytes[offset + 1],
                    fusion_bytes[offset + 2],
                    fusion_bytes[offset + 3],
                    fusion_bytes[offset + 4],
                    fusion_bytes[offset + 5],
                    fusion_bytes[offset + 6],
                    fusion_bytes[offset + 7],
                ]);
                offset += 8;
                if let Some(group) = crate::sequencer::pattern::unpack_fusion_legacy(packed) {
                    groups[group_count] = group;
                    group_count += 1;
                }
            }
            offset += (MAX_FUSIONS - 1 - count.min(MAX_FUSIONS - 1)) * 8;
            sink(inst, &groups[..group_count], &[]);
        }
    } else {
        return false;
    }
    true
}

fn le_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(b)
}

fn le_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(b)
}

impl PatternSlot {
    /// [283] Whether the live grid holds exactly what this slot would load:
    /// steps, length, fusions, sound p-locks and sequencer p-locks. Only what
    /// plays is compared (the locked fields of the active cells), so values
    /// left behind in unlocked cells never count as a change. Reads the blobs
    /// with the layouts of [`restore_from_buffers`], legacy ones included, and
    /// allocates nothing (it runs every editor frame for the `P1*` marker).
    pub fn matches_live(
        &self,
        pattern: &crate::sequencer::pattern::SharedPattern,
        plock_state: &crate::plock::PlockState,
        seq_plock_state: &crate::plock::SequencerPlockState,
        pattern_length: u8,
    ) -> bool {
        use crate::plock::{FIELD_COUNT, STEP_COUNT};
        use std::sync::atomic::Ordering;

        if self.pattern_length != pattern_length || self.step_masks != pattern.step_masks() {
            return false;
        }

        // Sound p-locks. A blob without p-lock data loads none.
        let cells = INSTRUMENT_COUNT * STEP_COUNT;
        let tail = INSTRUMENT_COUNT * 8 + cells * 8;
        let field_count = if self.plock_bytes.len() >= cells * FIELD_COUNT * 4 + tail {
            Some(FIELD_COUNT)
        } else if self.plock_bytes.len()
            >= cells * crate::param_id::PRE_SEND_FIELD_COUNT * 4 + tail
        {
            Some(crate::param_id::PRE_SEND_FIELD_COUNT)
        } else if self.plock_bytes.len() >= cells * LEGACY_PLOCK_FIELD_COUNT * 4 + tail {
            Some(LEGACY_PLOCK_FIELD_COUNT)
        } else {
            None
        };
        let bytes = &self.plock_bytes;
        for inst in 0..INSTRUMENT_COUNT {
            let live_mask = plock_state.masks.masks[inst].load(Ordering::Relaxed);
            let Some(fc) = field_count else {
                if live_mask != 0 {
                    return false;
                }
                continue;
            };
            let masks_at = cells * fc * 4;
            if le_u64(bytes, masks_at + inst * 8) != live_mask {
                return false;
            }
            for step in 0..STEP_COUNT {
                if live_mask & (1u64 << step) == 0 {
                    continue;
                }
                let cell = inst * STEP_COUNT + step;
                let field_mask = le_u64(bytes, masks_at + INSTRUMENT_COUNT * 8 + cell * 8);
                if field_mask != plock_state.field_masks.get_raw(inst, step) {
                    return false;
                }
                for field in 0..fc {
                    // An empty field mask is a full snapshot: every field plays.
                    if field_mask != 0 && field_mask & (1u64 << field) == 0 {
                        continue;
                    }
                    let saved = le_u32(bytes, (cell * fc + field) * 4);
                    if saved != plock_state.values.get(inst, step, field).to_bits() {
                        return false;
                    }
                }
            }
        }

        // Sequencer p-locks: 4 values per cell, active masks, then the
        // optional step-solo masks. A blob too short for them loads none.
        let bytes = &self.seq_plock_bytes;
        let masks_at = cells * 16;
        let has_seq = bytes.len() >= masks_at + INSTRUMENT_COUNT * 8;
        let solos_at = masks_at + INSTRUMENT_COUNT * 8;
        let has_solos = bytes.len() >= solos_at + INSTRUMENT_COUNT * 8;
        for inst in 0..INSTRUMENT_COUNT {
            let live_mask = seq_plock_state.masks[inst].load(Ordering::Relaxed);
            let live_solo = seq_plock_state.solo_masks[inst].load(Ordering::Relaxed);
            let (saved_mask, saved_solo) = if has_seq {
                let solo = if has_solos {
                    le_u64(bytes, solos_at + inst * 8)
                } else {
                    0
                };
                (le_u64(bytes, masks_at + inst * 8), solo)
            } else {
                (0, 0)
            };
            if saved_mask != live_mask || saved_solo != live_solo {
                return false;
            }
            for step in 0..STEP_COUNT {
                if live_mask & (1u64 << step) == 0 {
                    continue;
                }
                let at = (inst * STEP_COUNT + step) * 16;
                let live = [
                    seq_plock_state.probabilities[inst][step].load(Ordering::Relaxed),
                    seq_plock_state.stutters[inst][step].load(Ordering::Relaxed),
                    seq_plock_state.conditions[inst][step].load(Ordering::Relaxed),
                    seq_plock_state.microtimings[inst][step].load(Ordering::Relaxed),
                ];
                for (k, value) in live.iter().enumerate() {
                    if le_u32(bytes, at + k * 4) != *value {
                        return false;
                    }
                }
            }
        }

        // Fusions. A blob in no known format loads none.
        let mut same = true;
        let mut live = [FusedGroup::default(); MAX_FUSIONS];
        let decoded = decode_fusions(&self.fusion_bytes, |inst, saved, saved_morphs| {
            let n = pattern.load_fusions_into(inst, &mut live);
            same &= live[..n] == *saved;
            let n = pattern.load_linked_morphs_into(inst, &mut live);
            same &= live[..n] == *saved_morphs;
        });
        if !decoded {
            for inst in 0..INSTRUMENT_COUNT {
                same &= pattern.load_fusions_into(inst, &mut live) == 0;
                same &= pattern.load_linked_morphs_into(inst, &mut live) == 0;
            }
        }
        same
    }
}

impl PatternBank {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply [`PatternSlot::clear_lane`] to all 16 saved patterns, so a lane
    /// that just received an instrument is blank everywhere and not only in the
    /// pattern on screen.
    pub fn clear_lane_everywhere(&mut self, lane: usize) {
        for slot in self.slots.iter_mut() {
            slot.clear_lane(lane);
        }
    }

    /// Apply [`PatternSlot::permute_lanes`] to all 16 saved patterns, so a lane
    /// drag reorders the saved patterns the same way it reorders the live one.
    pub fn permute_lanes_everywhere(&mut self, order: &[usize; INSTRUMENT_COUNT]) {
        for slot in self.slots.iter_mut() {
            slot.permute_lanes(order);
        }
    }
}

/// Wrapper around PatternBank for nih-plug persistence.
///
/// The mutable bank stays behind a short-lived `Mutex`. The serialized snapshot
/// has its own lock so nih-plug can safely read it while the UI replaces it.
#[derive(Clone)]
pub struct PersistentPatternBank {
    pub bank: Arc<Mutex<PatternBank>>,
    snapshot: Arc<RwLock<Vec<u8>>>,
    snapshot_dirty: Arc<AtomicBool>,
}

impl PersistentPatternBank {
    pub fn new() -> Self {
        Self {
            bank: Arc::new(Mutex::new(PatternBank::new())),
            snapshot: Arc::new(RwLock::new(Vec::new())),
            snapshot_dirty: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Wipe one lane from all 16 saved patterns and persist the change.
    ///
    /// Called from the UI when a lane becomes a new instrument, so the lane is
    /// blank on every pattern and not only on the one displayed. The lock is
    /// taken blocking: only the audio thread must stay non-blocking, and it
    /// uses `try_lock` and retries on the next buffer.
    pub fn clear_lane_in_saved_patterns(&self, lane: usize) {
        {
            let Ok(mut bank) = self.bank.lock() else {
                return;
            };
            bank.clear_lane_everywhere(lane);
        }
        self.refresh_snapshot();
    }

    /// Reorder the lanes of all 16 saved patterns and persist the change,
    /// `order[new] = old`. Keeps a lane drag from stranding the saved patterns
    /// on the previous lane assignment.
    pub fn permute_lanes_in_saved_patterns(
        &self,
        order: &[usize; crate::sequencer::pattern::INSTRUMENT_COUNT],
    ) {
        {
            let Ok(mut bank) = self.bank.lock() else {
                return;
            };
            bank.permute_lanes_everywhere(order);
        }
        self.refresh_snapshot();
    }

    /// Mark the persisted snapshot stale without allocating or blocking. This
    /// is safe to call from the audio thread after a pattern-bank mutation.
    pub fn mark_snapshot_dirty(&self) {
        self.snapshot_dirty.store(true, AtomicOrdering::Release);
    }

    /// Rebuild the persisted JSON snapshot outside the bank lock.
    /// This may allocate and is therefore only called from UI/persistence code
    /// paths, never from the audio callback.
    pub fn refresh_snapshot(&self) {
        // Clearing before cloning avoids losing a concurrent dirty mark: a
        // mutation that happens during this rebuild will set the flag again.
        self.snapshot_dirty.store(false, AtomicOrdering::Release);
        let cloned = match self.bank.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => {
                self.mark_snapshot_dirty();
                return;
            }
        };
        let bytes = serde_json::to_vec(&cloned).unwrap_or_default();
        if let Ok(mut snapshot) = self.snapshot.write() {
            *snapshot = bytes;
        } else {
            self.mark_snapshot_dirty();
        }
    }
}

impl Default for PersistentPatternBank {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> nih_plug::params::persist::PersistentField<'a, Vec<u8>> for PersistentPatternBank {
    fn set(&self, new_value: Vec<u8>) {
        if let Ok(bank) = serde_json::from_slice::<PatternBank>(&new_value) {
            if let Ok(mut guard) = self.bank.lock() {
                *guard = bank;
                // [245] Deserialized Vecs lost the capacity preallocated by
                // PatternSlot::default() — restore it here (main thread) so
                // capture() never reallocates in the audio callback.
                for slot in guard.slots.iter_mut() {
                    slot.plock_bytes
                        .reserve(MAX_PLOCK_BYTES.saturating_sub(slot.plock_bytes.len()));
                    slot.seq_plock_bytes
                        .reserve(MAX_SEQ_PLOCK_BYTES.saturating_sub(slot.seq_plock_bytes.len()));
                    slot.fusion_bytes
                        .reserve(MAX_FUSION_BYTES.saturating_sub(slot.fusion_bytes.len()));
                }
            }
            self.refresh_snapshot();
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Vec<u8>) -> R,
    {
        if self.snapshot_dirty.load(AtomicOrdering::Acquire) {
            self.refresh_snapshot();
        }
        match self.snapshot.read() {
            Ok(snapshot) => f(&snapshot),
            Err(_) => {
                let empty = Vec::new();
                f(&empty)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plock::{PlockState, SequencerPlockState};
    use crate::sequencer::pattern::{Pattern, SharedPattern};
    use nih_plug::params::persist::PersistentField;

    #[test]
    fn pattern_slot_capture_restore_roundtrip() {
        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let seq_plock = SequencerPlockState::new();

        // Set some plock data
        plock.values.set(0, 5, 0, 0.75);
        plock.masks.masks[0].store(1 << 5, std::sync::atomic::Ordering::Relaxed);
        plock.field_masks.set(0, 5, 1);

        // Set some seq plock data
        seq_plock.probabilities[0][5].store(0.5f32.to_bits(), std::sync::atomic::Ordering::Relaxed);
        seq_plock.masks[0].store(1 << 5, std::sync::atomic::Ordering::Relaxed);

        // Capture
        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq_plock, 32);
        assert!(slot.occupied);
        assert_eq!(slot.pattern_length, 32);

        // Modify original state
        pattern.set_step_mask(0, 0);
        plock.values.set(0, 5, 0, 0.0);
        seq_plock.probabilities[0][5].store(1.0f32.to_bits(), std::sync::atomic::Ordering::Relaxed);

        // Restore
        let restored_len = slot.restore(&pattern, &plock, &seq_plock);
        assert_eq!(restored_len, Some(32));

        // Verify restored state
        assert_eq!(
            pattern.load_step_mask(0),
            Pattern::rock_pattern().step_masks()[0]
        );
        assert_eq!(plock.values.get(0, 5, 0), 0.75);
        assert_eq!(
            f32::from_bits(
                seq_plock.probabilities[0][5].load(std::sync::atomic::Ordering::Relaxed)
            ),
            0.5
        );
    }

    #[test]
    fn pattern_slot_capture_restore_preserves_fusions() {
        use crate::sequencer::pattern::FusedGroup;
        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let seq_plock = SequencerPlockState::new();

        // Two fused groups on different lanes.
        let g0 = FusedGroup {
            start_cell: 2,
            end_cell: 5,
            step_count: 4,
            ..FusedGroup::default()
        };
        let g3 = FusedGroup {
            start_cell: 8,
            end_cell: 11,
            step_count: 3,
            ..FusedGroup::default()
        };
        pattern.store_fusions(0, &[g0]);
        pattern.store_fusions(3, &[g3]);

        // Save to a slot, wipe the working fusions, then restore.
        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq_plock, 16);
        pattern.store_fusions(0, &[]);
        pattern.store_fusions(3, &[]);
        assert!(pattern.load_fusions(0).is_empty());
        slot.restore(&pattern, &plock, &seq_plock);

        // Regression: the deserialize gate used to reject the real blob as legacy
        // and drop every fusion, so saving a pattern lost its fused cells.
        assert_eq!(pattern.load_fusions(0), vec![g0]);
        assert_eq!(pattern.load_fusions(3), vec![g3]);
        assert!(pattern.load_fusions(1).is_empty());
    }

    /// [283] A linked lane's own morph is saved and restored with the pattern,
    /// and editing it marks the pattern unsaved.
    #[test]
    fn pattern_slot_keeps_linked_lane_morphs() {
        use crate::sequencer::pattern::{FusedGroup, MorphTarget};
        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let seq_plock = SequencerPlockState::new();
        let shared = FusedGroup {
            start_cell: 2,
            end_cell: 5,
            step_count: 4,
            ..FusedGroup::default()
        };
        let mut own = shared;
        own.set_morph_target(0, 120.0);
        pattern.store_fusions(0, &[shared]);
        pattern.store_lane_fusion(1, 0, 0, own);

        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq_plock, 16);
        assert!(slot.matches_live(&pattern, &plock, &seq_plock, 16));

        let mut edited = own;
        edited.morph_targets[0] = MorphTarget {
            end_value: 60.0,
            ..edited.morph_targets[0]
        };
        pattern.store_lane_fusion(1, 0, 0, edited);
        assert!(
            !slot.matches_live(&pattern, &plock, &seq_plock, 16),
            "morph edit"
        );

        slot.restore(&pattern, &plock, &seq_plock);
        assert_eq!(pattern.load_lane_fusions(1, 0), vec![own]);
        assert_eq!(pattern.load_fusions(0), vec![shared]);
    }

    /// [283] The unsaved-changes check sees every kind of edit (it used to
    /// see only steps and length), and ignores leftovers that do not play.
    #[test]
    fn matches_live_sees_plock_seq_plock_and_fusion_edits() {
        use crate::sequencer::pattern::FusedGroup;
        use std::sync::atomic::Ordering::Relaxed;
        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let seq = SequencerPlockState::new();
        plock.masks.set_active(0, 5, true);
        plock.field_masks.set_raw(0, 5, 1 << 3);
        plock.values.set(0, 5, 3, 0.75);
        seq.set_probability(1, 2, 0.5);
        pattern.store_fusions(
            2,
            &[FusedGroup {
                start_cell: 8,
                end_cell: 11,
                step_count: 3,
                ..FusedGroup::default()
            }],
        );
        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq, 16);
        assert!(slot.matches_live(&pattern, &plock, &seq, 16));
        assert!(!slot.matches_live(&pattern, &plock, &seq, 32), "length");

        // A value in a field the p-lock does not lock plays nothing.
        plock.values.set(0, 5, 4, 0.1);
        assert!(
            slot.matches_live(&pattern, &plock, &seq, 16),
            "unlocked leftover"
        );

        plock.values.set(0, 5, 3, 0.2);
        assert!(
            !slot.matches_live(&pattern, &plock, &seq, 16),
            "locked value"
        );
        plock.values.set(0, 5, 3, 0.75);

        seq.set_probability(1, 2, 0.25);
        assert!(!slot.matches_live(&pattern, &plock, &seq, 16), "seq p-lock");
        seq.set_probability(1, 2, 0.5);

        seq.solo_masks[3].store(1 << 7, Relaxed);
        assert!(!slot.matches_live(&pattern, &plock, &seq, 16), "step solo");
        seq.solo_masks[3].store(0, Relaxed);

        pattern.store_fusions(2, &[]);
        assert!(!slot.matches_live(&pattern, &plock, &seq, 16), "fusion");

        // Loading the slot back (the production path) always matches it.
        restore_from_buffers(
            &slot.step_masks,
            &slot.plock_bytes,
            &slot.seq_plock_bytes,
            &slot.fusion_bytes,
            &pattern,
            &plock,
            &seq,
        );
        assert!(
            slot.matches_live(&pattern, &plock, &seq, 16),
            "after restore"
        );
    }

    #[test]
    fn seq_plock_solo_survives_capture_restore() {
        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let seq_plock = SequencerPlockState::new();

        seq_plock.set_solo(0, 5, true);
        seq_plock.set_solo(3, 12, true);

        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq_plock, 16);

        // Wipe the live state, then restore from the slot.
        seq_plock.set_solo(0, 5, false);
        seq_plock.set_solo(3, 12, false);
        slot.restore(&pattern, &plock, &seq_plock);

        assert!(seq_plock.is_solo(0, 5));
        assert!(seq_plock.is_solo(3, 12));
        assert!(!seq_plock.is_solo(1, 5));
    }

    #[test]
    fn seq_plock_legacy_blob_without_solo_defaults_false() {
        // A seq-plock buffer written before step-solo existed: exactly the old
        // size (4 fields per cell + active masks, no trailing solo block).
        use crate::plock::{FIELD_COUNT, STEP_COUNT};
        let _ = FIELD_COUNT;
        let cell_count = INSTRUMENT_COUNT * STEP_COUNT;
        let legacy_len = cell_count * 4 * 4 + INSTRUMENT_COUNT * 8;
        let legacy_bytes = vec![0u8; legacy_len];

        let seq_plock = SequencerPlockState::new();
        // Pre-set a solo to prove the legacy restore clears it back to false.
        seq_plock.set_solo(2, 8, true);

        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let step_masks = [0u16; STEP_COUNT];
        restore_from_buffers(
            &step_masks,
            &[],
            &legacy_bytes,
            &[],
            &pattern,
            &plock,
            &seq_plock,
        );

        assert!(!seq_plock.is_solo(2, 8)); // legacy blob → no solos
    }

    #[test]
    fn pattern_slot_preallocated_capacity_never_reallocates() {
        let slot = PatternSlot::default();
        assert!(slot.plock_bytes.capacity() >= MAX_PLOCK_BYTES);
        assert!(slot.seq_plock_bytes.capacity() >= MAX_SEQ_PLOCK_BYTES);
    }

    #[test]
    fn song_sequence_slot_at_and_set() {
        let mut song = SongSequence::new();
        song.length = 4;
        song.set_step(0, 2); // P3
        song.set_step(1, -1); // empty
        song.set_step(2, 7); // P8
        song.set_step(3, 0); // P1

        assert_eq!(song.slot_at(0), Some(2));
        assert_eq!(song.slot_at(1), None);
        assert_eq!(song.slot_at(2), Some(7));
        assert_eq!(song.slot_at(3), Some(0));
        assert_eq!(song.slot_at(4), None); // beyond length
    }

    #[test]
    fn pattern_bank_persistence_roundtrips_song() {
        let persistent = PersistentPatternBank::new();
        {
            let mut guard = persistent.bank.lock().unwrap();
            guard.song.length = 4;
            guard.song.set_step(0, 1);
            guard.song.set_step(1, 3);
            guard.song.loop_enabled = true;
            guard.song.set_repeat(0, 4);
            guard.song.set_repeat(1, 2);
        }

        let bytes = persistent.map(|b| b.clone());

        let restored = PersistentPatternBank::new();
        restored.set(bytes);

        let guard = restored.bank.lock().unwrap();
        assert_eq!(guard.song.length, 4);
        assert_eq!(guard.song.slot_at(0), Some(1));
        assert_eq!(guard.song.slot_at(1), Some(3));
        assert!(guard.song.loop_enabled);
        assert_eq!(guard.song.repeat_at(0), 4);
        assert_eq!(guard.song.repeat_at(1), 2);
        assert_eq!(guard.song.repeat_at(2), 1); // default
    }

    #[test]
    fn song_sequence_repeat_clamps_and_defaults() {
        let mut song = SongSequence::new();
        song.length = 4;
        song.set_repeat(0, 0);
        song.set_repeat(1, 255);
        song.set_repeat(2, 5);
        // step 3 untouched -> default 1

        assert_eq!(song.repeat_at(0), 1);
        assert_eq!(song.repeat_at(1), 255);
        assert_eq!(song.repeat_at(2), 5);
        assert_eq!(song.repeat_at(3), 1);
        assert_eq!(song.repeat_at(100), 1); // out of bounds
    }

    #[test]
    fn pattern_bank_legacy_load_without_repeats_defaults_to_one() {
        // Simulate a pattern-bank-v1 JSON blob without the `repeats` field.
        let legacy = br#"{"slots":[{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false},{"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_bytes":[],"seq_plock_bytes":[],"fusion_bytes":[],"pattern_length":16,"occupied":false}],"song":{"steps":[-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],"length":0,"loop_enabled":true}}"#;
        let restored = PersistentPatternBank::new();
        restored.set(legacy.to_vec());
        let guard = restored.bank.lock().unwrap();
        assert_eq!(guard.song.repeat_at(0), 1);
        assert_eq!(guard.song.repeat_at(63), 1);
    }

    #[test]
    fn pattern_bank_migrates_8_slot_blob_to_16() {
        // A legacy 8-slot bank must load into the new 16-slot bank without
        // failing `from_slice` (which would silently reload an empty bank and
        // lose every saved pattern). The first 8 slots are preserved; P9-P16
        // are padded empty.
        let mut slot0 = PatternSlot::default();
        slot0.occupied = true;
        slot0.pattern_length = 32;
        let mut slots: Vec<PatternSlot> = vec![PatternSlot::default(); 8];
        slots[0] = slot0;
        let blob = serde_json::json!({ "slots": slots, "song": SongSequence::new() });
        let bytes = serde_json::to_vec(&blob).unwrap();

        let restored = PersistentPatternBank::new();
        restored.set(bytes);
        let guard = restored.bank.lock().unwrap();
        assert_eq!(guard.slots.len(), SLOT_COUNT); // padded to 16
        assert!(guard.slots[0].occupied); // P1 preserved
        assert_eq!(guard.slots[0].pattern_length, 32);
        assert!(!guard.slots[7].occupied); // P8 preserved (empty)
        assert!(!guard.slots[8].occupied); // P9 padded empty
        assert!(!guard.slots[15].occupied); // P16 padded empty
    }

    /// [244][245] The audio-thread save path (`try_lock` + `capture` +
    /// `mark_snapshot_dirty`) must not allocate — including right after a
    /// restore, when the deserialized slots would otherwise have lost their
    /// preallocated buffers. Fails before the [244]/[245] fixes.
    #[test]
    fn save_pattern_to_slot_is_realtime_safe() {
        let pattern = SharedPattern::new(&Pattern::rock_pattern());
        let plock = PlockState::new();
        let seq_plock = SequencerPlockState::new();
        plock.set_field(0, 5, 2, 0.75);
        seq_plock.set_probability(1, 10, 0.5);

        // Round-trip through a serialized blob, as a DAW project reopen does:
        // deserialized slots had zero-capacity Vecs before [245].
        let source_bank = PersistentPatternBank::new();
        source_bank.refresh_snapshot();
        let bytes = source_bank.map(|b| b.clone());
        let restored = PersistentPatternBank::new();
        restored.set(bytes);

        assert_no_alloc::assert_no_alloc(|| {
            {
                let mut guard = restored.bank.lock().unwrap();
                guard.slots[3].capture(&pattern, &plock, &seq_plock, 16);
            }
            // [244] The audio thread marks dirty; it does NOT serialize.
            restored.mark_snapshot_dirty();
        });

        // The snapshot is rebuilt lazily on the host's state request.
        let snapshot = restored.map(|b| b.clone());
        let back: PatternBank = serde_json::from_slice(&snapshot).unwrap();
        assert!(back.slots[3].occupied, "P4 should hold the saved pattern");
    }

    #[test]
    fn pattern_bank_roundtrips_16_slots() {
        // A full 16-slot bank must serialize and deserialize identically.
        let bank = PersistentPatternBank::new();
        {
            let mut guard = bank.bank.lock().unwrap();
            guard.slots[15].occupied = true;
            guard.slots[15].pattern_length = 48;
        }
        bank.refresh_snapshot();
        let bytes = bank.map(|b| b.clone());
        let restored = PersistentPatternBank::new();
        restored.set(bytes);
        let guard = restored.bank.lock().unwrap();
        assert_eq!(guard.slots.len(), SLOT_COUNT);
        assert!(guard.slots[15].occupied);
        assert_eq!(guard.slots[15].pattern_length, 48);
    }

    #[test]
    fn pattern_bank_switch_clears_plocks() {
        // Reproduce the user bug: save pattern A with plocks to P1,
        // save pattern B with different plocks to P2, load P1 then P2
        // and verify plocks don't leak.
        let pattern_a = SharedPattern::new(&Pattern::rock_pattern());
        let pattern_b = SharedPattern::new(&Pattern::funk_pattern());
        let plock = PlockState::new();
        let seq_plock = SequencerPlockState::new();

        // Set plock on pattern A (instrument 0, step 5, volume = 0.75)
        plock.set_field(0, 5, 2, 0.75);
        // Set seq plock on pattern A (instrument 0, step 5, probability = 0.5)
        seq_plock.set_probability(0, 5, 0.5);

        // Save pattern A to slot 0
        let mut slot0 = PatternSlot::default();
        slot0.capture(&pattern_a, &plock, &seq_plock, 16);

        // Clear and set DIFFERENT plocks for pattern B
        // (instrument 1, step 10, volume = 0.9)
        plock.clear_all();
        seq_plock.clear_all();
        plock.set_field(1, 10, 2, 0.9);
        seq_plock.set_probability(1, 10, 0.25);

        // Save pattern B to slot 1
        let mut slot1 = PatternSlot::default();
        slot1.capture(&pattern_b, &plock, &seq_plock, 16);

        // Now simulate loading slot 0 (pattern A)
        // Use fresh shared pattern and plock state
        let target_pattern = SharedPattern::new(&Pattern::empty());
        let target_plock = PlockState::new();
        let target_seq_plock = SequencerPlockState::new();

        // Load slot 0
        slot0.restore(&target_pattern, &target_plock, &target_seq_plock);

        // Verify pattern A plocks are present
        assert!(target_plock.masks.is_active(0, 5));
        assert_eq!(target_plock.values.get(0, 5, 2), 0.75);
        assert!(target_seq_plock.is_active(0, 5));
        assert_eq!(
            f32::from_bits(
                target_seq_plock.probabilities[0][5].load(std::sync::atomic::Ordering::Relaxed)
            ),
            0.5
        );
        // Pattern B plocks should NOT be present
        assert!(!target_plock.masks.is_active(1, 10));

        // Now load slot 1 (pattern B) - this is where the bug happens
        // First clear, then restore (same as load_pattern_from_slot does)
        target_plock.clear_all();
        target_seq_plock.clear_all();
        slot1.restore(&target_pattern, &target_plock, &target_seq_plock);

        // Verify pattern B plocks are present
        assert!(target_plock.masks.is_active(1, 10));
        assert_eq!(target_plock.values.get(1, 10, 2), 0.9);
        assert!(target_seq_plock.is_active(1, 10));
        assert_eq!(
            f32::from_bits(
                target_seq_plock.probabilities[1][10].load(std::sync::atomic::Ordering::Relaxed)
            ),
            0.25
        );
        // Pattern A plocks should NOT be present anymore
        assert!(
            !target_plock.masks.is_active(0, 5),
            "Plock from pattern A leaked into pattern B!"
        );
        assert!(
            !target_seq_plock.is_active(0, 5),
            "Seq plock from pattern A leaked into pattern B!"
        );
    }

    #[test]
    fn persistent_bank_snapshot_tracks_explicit_refresh() {
        let persistent = PersistentPatternBank::new();
        {
            let mut guard = persistent.bank.lock().unwrap();
            guard.song.length = 3;
            guard.song.set_step(0, 2);
        }
        persistent.refresh_snapshot();

        let bytes = persistent.map(|b| b.clone());
        let restored = PersistentPatternBank::new();
        restored.set(bytes);

        let guard = restored.bank.lock().unwrap();
        assert_eq!(guard.song.length, 3);
        assert_eq!(guard.song.slot_at(0), Some(2));
    }

    #[test]
    fn persistent_bank_map_uses_snapshot_even_when_bank_is_locked() {
        let persistent = PersistentPatternBank::new();
        {
            let mut guard = persistent.bank.lock().unwrap();
            guard.song.length = 2;
            guard.song.set_step(0, 4);
        }
        persistent.refresh_snapshot();

        let _guard = persistent.bank.lock().unwrap();
        let bytes = persistent.map(|b| b.clone());
        let restored = PersistentPatternBank::new();
        restored.set(bytes);

        let guard = restored.bank.lock().unwrap();
        assert_eq!(guard.song.length, 2);
        assert_eq!(guard.song.slot_at(0), Some(4));
    }

    #[test]
    fn persistent_bank_snapshot_is_safe_during_concurrent_refresh() {
        let persistent = Arc::new(PersistentPatternBank::new());
        persistent.refresh_snapshot();

        let writer = Arc::clone(&persistent);
        let writer_thread = std::thread::spawn(move || {
            for length in 1..=SONG_BLOCKS as u8 {
                if let Ok(mut bank) = writer.bank.lock() {
                    bank.song.length = length;
                }
                writer.refresh_snapshot();
            }
        });

        for _ in 0..256 {
            let valid =
                persistent.map(|bytes| serde_json::from_slice::<PatternBank>(bytes).is_ok());
            assert!(valid, "snapshot must always remain valid JSON");
        }
        writer_thread.join().unwrap();
    }

    // ── Per-lane editing of a saved pattern ─────────────────────────────────

    /// The one step every lane is marked on, so a lane's mark keeps its
    /// position when the lanes get permuted and only its VALUE says where it
    /// came from.
    const MARK_STEP: usize = 5;

    /// Put a recognizable mark on one lane: a step, a fusion, a sound p-lock
    /// and a sequencer p-lock.
    fn mark_lane(
        lane: usize,
        pattern: &SharedPattern,
        plock: &PlockState,
        seq: &SequencerPlockState,
    ) {
        use std::sync::atomic::Ordering;
        let step = MARK_STEP;
        pattern.set_step_mask(step, pattern.load_step_mask(step) | (1u16 << lane));
        pattern.store_fusions(
            lane,
            &[FusedGroup {
                start_cell: 16,
                end_cell: 19,
                step_count: 4,
                ..Default::default()
            }],
        );
        plock.values.set(lane, step, 0, 0.25 + lane as f32);
        plock.masks.masks[lane].store(1u64 << step, Ordering::Relaxed);
        plock.field_masks.set_raw(lane, step, 0b11);
        seq.probabilities[lane][step].store(0.5f32.to_bits(), Ordering::Relaxed);
        seq.masks[lane].store(1u64 << step, Ordering::Relaxed);
        seq.solo_masks[lane].store(1u64 << step, Ordering::Relaxed);
    }

    /// Read that mark back. `None` means the lane carries nothing at all.
    fn lane_mark(
        lane: usize,
        pattern: &SharedPattern,
        plock: &PlockState,
        seq: &SequencerPlockState,
    ) -> Option<(f32, u64, u64, usize)> {
        use std::sync::atomic::Ordering;
        let step = MARK_STEP;
        let has_step = pattern.load_step_mask(step) & (1u16 << lane) != 0;
        let value = plock.values.get(lane, step, 0);
        let plock_mask = plock.masks.masks[lane].load(Ordering::Relaxed);
        let seq_mask = seq.masks[lane].load(Ordering::Relaxed);
        let fusions = pattern.load_fusions(lane).len();
        if !has_step && value == 0.0 && plock_mask == 0 && seq_mask == 0 && fusions == 0 {
            return None;
        }
        Some((value, plock_mask, seq_mask, fusions))
    }

    type FreshState = (
        std::sync::Arc<SharedPattern>,
        std::sync::Arc<PlockState>,
        SequencerPlockState,
    );

    fn fresh_state() -> FreshState {
        (
            SharedPattern::new(&Pattern::empty()),
            PlockState::new(),
            SequencerPlockState::new(),
        )
    }

    /// The guarantee: an instrument dropped on a reused lane must find that
    /// lane blank in EVERY saved pattern, and the neighbours untouched.
    #[test]
    fn clearing_a_lane_wipes_it_from_a_saved_pattern_and_spares_the_others() {
        let (pattern, plock, seq) = fresh_state();
        for lane in [2usize, 3, 4] {
            mark_lane(lane, &pattern, &plock, &seq);
        }
        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq, 64);

        slot.clear_lane(3);

        let (after, after_plock, after_seq) = fresh_state();
        slot.restore(&after, &after_plock, &after_seq);
        assert_eq!(
            lane_mark(3, &after, &after_plock, &after_seq),
            None,
            "lane 3 must come back empty from the saved pattern"
        );
        for lane in [2usize, 4] {
            assert_eq!(
                lane_mark(lane, &after, &after_plock, &after_seq),
                lane_mark(lane, &pattern, &plock, &seq),
                "lane {lane} must survive its neighbour being cleared"
            );
        }
    }

    /// A lane drag permutes the live pattern; the saved ones have to follow, or
    /// recalling a pattern puts each instrument's steps back on its old lane.
    #[test]
    fn permuting_lanes_moves_a_saved_pattern_the_same_way() {
        let (pattern, plock, seq) = fresh_state();
        for lane in [0usize, 1, 2] {
            mark_lane(lane, &pattern, &plock, &seq);
        }
        let mut slot = PatternSlot::default();
        slot.capture(&pattern, &plock, &seq, 64);

        // Lane 0 dragged down to index 2: order[new] = old.
        let mut order: [usize; INSTRUMENT_COUNT] = std::array::from_fn(|i| i);
        order[0] = 1;
        order[1] = 2;
        order[2] = 0;
        slot.permute_lanes(&order);

        let (after, after_plock, after_seq) = fresh_state();
        slot.restore(&after, &after_plock, &after_seq);
        for (new_lane, &old_lane) in order.iter().enumerate().take(3) {
            let moved = lane_mark(new_lane, &after, &after_plock, &after_seq);
            assert!(moved.is_some(), "lane {new_lane} should carry data");
            // The p-lock value is lane-specific, so it identifies the origin.
            assert_eq!(
                moved.unwrap().0,
                0.25 + old_lane as f32,
                "lane {new_lane} must now hold what lane {old_lane} held"
            );
            assert_eq!(moved.unwrap().3, 1, "its fusion travelled too");
        }
    }

    /// A blob saved by an older build is shorter, so its per-lane stride is
    /// smaller. Editing it with the CURRENT stride would shred the neighbours.
    #[test]
    fn clearing_a_lane_uses_the_stride_of_a_legacy_plock_blob() {
        let stride = STEP_COUNT * LEGACY_PLOCK_FIELD_COUNT * 4;
        let values = INSTRUMENT_COUNT * stride;
        let masks = INSTRUMENT_COUNT * 8;
        let field_masks = INSTRUMENT_COUNT * STEP_COUNT * 8;

        // Every byte of a lane's block carries that lane's index + 1.
        let mut bytes = vec![0u8; values + masks + field_masks];
        for lane in 0..INSTRUMENT_COUNT {
            let tag = (lane + 1) as u8;
            bytes[lane * stride..(lane + 1) * stride].fill(tag);
            bytes[values + lane * 8..values + (lane + 1) * 8].fill(tag);
            let fm = values + masks;
            bytes[fm + lane * STEP_COUNT * 8..fm + (lane + 1) * STEP_COUNT * 8].fill(tag);
        }

        let mut slot = PatternSlot::default();
        slot.plock_bytes = bytes;
        slot.occupied = true;
        slot.clear_lane(3);

        assert!(
            slot.plock_bytes[3 * stride..4 * stride]
                .iter()
                .all(|b| *b == 0),
            "lane 3's values must be zeroed"
        );
        for lane in [2usize, 4] {
            let tag = (lane + 1) as u8;
            assert!(
                slot.plock_bytes[lane * stride..(lane + 1) * stride]
                    .iter()
                    .all(|b| *b == tag),
                "lane {lane} must be untouched — the legacy stride was misread"
            );
            assert!(
                slot.plock_bytes[values + lane * 8..values + (lane + 1) * 8]
                    .iter()
                    .all(|b| *b == tag),
                "lane {lane}'s mask must be untouched"
            );
        }
    }

    /// An empty bank slot has empty blobs: the per-lane edits must be no-ops
    /// rather than slicing out of bounds.
    #[test]
    fn per_lane_edits_are_harmless_on_an_empty_slot() {
        let mut slot = PatternSlot::default();
        slot.clear_lane(3);
        let order: [usize; INSTRUMENT_COUNT] = std::array::from_fn(|i| i);
        slot.permute_lanes(&order);
        assert!(slot.step_masks.iter().all(|m| *m == 0));
        assert!(slot.plock_bytes.is_empty());
        assert!(!slot.occupied);
        // Out-of-range lanes are ignored, not panics.
        slot.clear_lane(INSTRUMENT_COUNT);
        slot.clear_lane(usize::MAX);
    }
}
