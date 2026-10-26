//! Pattern data structures and shared realtime-safe state.

use nih_plug::params::persist::PersistentField;
use std::array;
use std::sync::{
    atomic::{AtomicU16, AtomicU64, Ordering},
    Arc,
};

/// Maximum fused groups per instrument in SharedPattern.
pub const MAX_FUSIONS: usize = 16;

/// Number of AtomicU64 slots used per fused group in SharedPattern.
pub(crate) const FUSION_SLOT_COUNT: usize = 3;

/// Direction of a morph target within a fused group.
/// - `Target`: the stored `end_value` is the target; the morph goes from the
///   current lane/plock value to the stored value (default, original behavior).
/// - `Source`: the stored `end_value` is the source; the morph goes from the
///   stored value to the current lane/plock value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum MorphDirection {
    #[default]
    Target,
    Source,
}

/// A single morphing target inside a fused group.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MorphTarget {
    pub field: u8,
    pub end_value: f32,
    pub direction: MorphDirection,
}

impl Default for MorphTarget {
    fn default() -> Self {
        Self {
            field: 255,
            end_value: 0.0,
            direction: MorphDirection::Target,
        }
    }
}

/// Pack a FusedGroup into 3 u64s for atomic storage.
///
/// Layout:
/// Slot 0: bits 0-5 start_cell, 6-11 end_cell, 12-17 step_count-1, 18 valid,
///         19-21 morph_count, 22-29 target[0].field, 30-61 target[0].end_value.
/// Slot 1: bits 0-7 target[1].field, 8-39 target[1].end_value,
///         40-47 target[2].field, 48-63 target[2].end_value upper 16 bits.
/// Slot 2: bits 0-15 target[2].end_value lower 16 bits,
///         16-23 target[3].field, 24-55 target[3].end_value,
///         56-59 target[0..3].direction (0=Target, 1=Source).
pub(crate) fn pack_fusion(group: &FusedGroup) -> [u64; FUSION_SLOT_COUNT] {
    let target =
        |i: usize| -> MorphTarget { group.morph_targets.get(i).copied().unwrap_or_default() };

    let t0 = target(0);
    let step_count_minus_1 = (group.step_count.clamp(1, 64) - 1) as u64;
    let slot0 = (group.start_cell as u64)
        | ((group.end_cell as u64) << 6)
        | (step_count_minus_1 << 12)
        | (1u64 << 18)
        | ((group.morph_count.clamp(0, 4) as u64) << 19)
        | ((t0.field as u64) << 22)
        | ((t0.end_value.to_bits() as u64) << 30);

    let t1 = target(1);
    let t2 = target(2);
    let t2_bits = t2.end_value.to_bits() as u64;
    let slot1 = (t1.field as u64)
        | ((t1.end_value.to_bits() as u64) << 8)
        | ((t2.field as u64) << 40)
        | ((t2_bits & 0xFFFF_0000) << 32); // upper 16 bits at 48-63

    let t3 = target(3);
    let mut slot2 = ((t2_bits & 0xFFFF) << 0)
        | ((t3.field as u64) << 16)
        | ((t3.end_value.to_bits() as u64) << 24);
    for i in 0..4usize {
        if group
            .morph_targets
            .get(i)
            .map(|t| t.direction)
            .unwrap_or_default()
            == MorphDirection::Source
        {
            slot2 |= 1u64 << (56 + i);
        }
    }

    [slot0, slot1, slot2]
}

/// [283] Marks a record of a lane's fusion list that is not a fused group but a
/// LINKED lane's own morph: a linked lane plays the fused groups of the lane
/// it follows, while its morph stays its own (morph targets are parameters of
/// its instrument). Same layout as a group, valid bit (18) clear, so every
/// reader of fused groups - older builds included - skips the record; bit 62
/// is free in the packed layout (target[0].end_value ends at bit 61).
const LINKED_MORPH_BIT: u64 = 1 << 62;

/// Pack a linked lane's own morph for the fused group spanning
/// `start_cell..=end_cell` of the lane it follows (see [`LINKED_MORPH_BIT`]).
pub(crate) fn pack_linked_morph(morph: &FusedGroup) -> [u64; FUSION_SLOT_COUNT] {
    let mut slots = pack_fusion(morph);
    slots[0] = (slots[0] & !(1u64 << 18)) | LINKED_MORPH_BIT;
    slots
}

/// [283] `group` with the morph a linked lane keeps for its cells (`morphs`
/// from [`SharedPattern::load_linked_morphs`]); no morph when it keeps none.
pub fn with_linked_morph(group: FusedGroup, morphs: &[FusedGroup]) -> FusedGroup {
    let morph = morphs.iter().find(|morph| morph.same_cells(&group));
    FusedGroup {
        morph_count: morph.map_or(0, |morph| morph.morph_count),
        morph_targets: morph.map_or([MorphTarget::default(); MAX_MORPH_TARGETS], |morph| {
            morph.morph_targets
        }),
        ..group
    }
}

/// Unpack 3 u64s into an Option<FusedGroup>.
/// Only understands the current (fixed) layout; old pattern-v3 fusion data is
/// migrated at the state level in `lib.rs::filter_state`.
pub(crate) fn unpack_fusion(slots: [u64; FUSION_SLOT_COUNT]) -> Option<FusedGroup> {
    if (slots[0] >> 18) & 1 == 0 {
        return None;
    }
    unpack_group(slots)
}

/// Unpack a linked lane's own morph record (see [`LINKED_MORPH_BIT`]); `None`
/// for a fused group, an empty record, or a morph without targets.
pub(crate) fn unpack_linked_morph(slots: [u64; FUSION_SLOT_COUNT]) -> Option<FusedGroup> {
    if (slots[0] >> 18) & 1 == 1 || slots[0] & LINKED_MORPH_BIT == 0 {
        return None;
    }
    unpack_group(slots).filter(FusedGroup::morph_active)
}

fn unpack_group(slots: [u64; FUSION_SLOT_COUNT]) -> Option<FusedGroup> {
    let slot0 = slots[0];
    let s2 = slots[2];
    let mut targets = [MorphTarget::default(); 4];
    let count = ((slot0 >> 19) & 0x7) as usize;
    if count > 0 {
        targets[0] = MorphTarget {
            field: ((slot0 >> 22) & 0xFF) as u8,
            end_value: f32::from_bits(((slot0 >> 30) & 0xFFFFFFFF) as u32),
            direction: if (s2 >> 56) & 1 == 1 {
                MorphDirection::Source
            } else {
                MorphDirection::Target
            },
        };
    }
    if count > 1 {
        let s1 = slots[1];
        targets[1] = MorphTarget {
            field: (s1 & 0xFF) as u8,
            end_value: f32::from_bits(((s1 >> 8) & 0xFFFFFFFF) as u32),
            direction: if (s2 >> 57) & 1 == 1 {
                MorphDirection::Source
            } else {
                MorphDirection::Target
            },
        };
    }
    if count > 2 {
        let s1 = slots[1];
        let field = ((s1 >> 40) & 0xFF) as u8;
        let value_upper = ((s1 >> 48) & 0xFFFF) as u32;
        let value_lower = (s2 & 0xFFFF) as u32;
        targets[2] = MorphTarget {
            field,
            end_value: f32::from_bits((value_upper << 16) | value_lower),
            direction: if (s2 >> 58) & 1 == 1 {
                MorphDirection::Source
            } else {
                MorphDirection::Target
            },
        };
    }
    if count > 3 {
        targets[3] = MorphTarget {
            field: ((s2 >> 16) & 0xFF) as u8,
            end_value: f32::from_bits(((s2 >> 24) & 0xFFFFFFFF) as u32),
            direction: if (s2 >> 59) & 1 == 1 {
                MorphDirection::Source
            } else {
                MorphDirection::Target
            },
        };
    }

    let group = FusedGroup {
        start_cell: (slot0 & 0x3F) as u8,
        end_cell: ((slot0 >> 6) & 0x3F) as u8,
        step_count: (((slot0 >> 12) & 0x3F) as u8).saturating_add(1),
        morph_count: count.clamp(0, 4) as u8,
        morph_targets: targets,
    };
    group.is_valid().then_some(group)
}

/// Unpack the old broken pattern-v3 fusion layout (valid bit 24) and return
/// a geometry-only group. Used by `filter_state` to migrate saved DAW state.
pub(crate) fn unpack_fusion_v3_old(slots: [u64; FUSION_SLOT_COUNT]) -> Option<FusedGroup> {
    let meta = slots[0];
    if (meta >> 24) & 1 == 0 {
        return None;
    }
    let mut group = FusedGroup::default();
    group.start_cell = (meta & 0xFF) as u8;
    group.end_cell = ((meta >> 8) & 0xFF) as u8;
    group.step_count = ((meta >> 16) & 0xFF) as u8;
    group.morph_count = 0;
    group.morph_targets = [MorphTarget::default(); 4];
    group.is_valid().then_some(group)
}

/// Unpack a legacy single-u64 fusion (pattern-v3 before multi-target morphing).
/// Used for pattern-bank backward compatibility.
pub(crate) fn unpack_fusion_legacy(packed: u64) -> Option<FusedGroup> {
    if (packed >> 24) & 1 == 0 {
        return None;
    }
    let morph_field = ((packed >> 26) & 0x3F) as u8;
    let morph_end_value = f32::from_bits((packed >> 32) as u32);
    let mut targets = [MorphTarget::default(); 4];
    let mut count = 0u8;
    if morph_field != 255 {
        targets[0] = MorphTarget {
            field: morph_field,
            end_value: morph_end_value,
            direction: MorphDirection::Target,
        };
        count = 1;
    }
    let group = FusedGroup {
        start_cell: (packed & 0xFF) as u8,
        end_cell: ((packed >> 8) & 0xFF) as u8,
        step_count: ((packed >> 16) & 0xFF) as u8,
        morph_count: count,
        morph_targets: targets,
    };
    group.is_valid().then_some(group)
}

pub const INSTRUMENT_COUNT: usize = 14;
pub const STEP_COUNT: usize = 64;

/// Legacy 13-instrument pattern state used for migrating `pattern-v3` to `pattern-v5`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PatternStateV3 {
    #[serde(with = "serde_arrays")]
    pub masks: [u16; STEP_COUNT],
    #[serde(with = "serde_arrays")]
    pub fusions: [u64; 13 * MAX_FUSIONS * FUSION_SLOT_COUNT],
}

impl PatternStateV3 {
    // Migration helpers for pattern-v3 (old broken fusion layout) live in
    // lib.rs::filter_state. No expand() is needed here: the v3 layout is not
    // directly converted to the current format.
}

/// Legacy 13-instrument pattern state used for migrating `pattern-v4` to `pattern-v5`.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PatternStateV4 {
    #[serde(with = "serde_arrays")]
    pub masks: [u16; STEP_COUNT],
    #[serde(with = "serde_arrays")]
    pub fusions: [u64; 13 * MAX_FUSIONS * FUSION_SLOT_COUNT],
}

impl PatternStateV4 {
    /// Expand a 13-instrument pattern state into the current 14-instrument format.
    /// The 14th instrument row is initialized to empty.
    pub fn expand(self) -> PatternState {
        let mut masks = [0u16; STEP_COUNT];
        masks.copy_from_slice(&self.masks);
        let mut fusions = [0u64; INSTRUMENT_COUNT * MAX_FUSIONS * FUSION_SLOT_COUNT];
        for inst in 0..13 {
            let old_base = inst * MAX_FUSIONS * FUSION_SLOT_COUNT;
            let new_base = inst * MAX_FUSIONS * FUSION_SLOT_COUNT;
            for i in 0..(MAX_FUSIONS * FUSION_SLOT_COUNT) {
                fusions[new_base + i] = self.fusions[old_base + i];
            }
        }
        PatternState { masks, fusions }
    }
}

/// A single step in a pattern containing trigger states for all instruments.
#[derive(Clone, Debug)]
pub struct Step {
    ///  instruments: Kick, Snare, HiHat, OpenHiHat, Tom1, Tom2, Tom3, Clap, Ride, Cymbal, Snare606, BassDrum808, Zap.
    pub instruments: [bool; INSTRUMENT_COUNT],
}

impl Step {
    pub fn new() -> Self {
        Self {
            instruments: [false; INSTRUMENT_COUNT],
        }
    }

    pub fn empty() -> Self {
        Self::new()
    }

    pub fn bitmask(&self) -> u16 {
        let mut bits = 0u16;
        for (instrument, active) in self.instruments.iter().copied().enumerate() {
            if active {
                bits |= 1 << instrument;
            }
        }
        bits
    }
}

/// Morph targets a single fused group can carry (the packed format has room for
/// exactly this many).
pub const MAX_MORPH_TARGETS: usize = 4;

/// A fused cell group for Step Fusion (tuplets / micro-rhythms).
///
/// A group is one UI button spanning consecutive cells on a single 16-step
/// page. When its start cell is active, the audio engine emits `step_count`
/// evenly-spaced pulses over the duration of the whole group. Sound plocks are
/// read from `start_cell`; sequencer stutter is ignored for fused cells.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct FusedGroup {
    /// First cell index (0-63) inclusive.
    pub start_cell: u8,
    /// Last cell index (0-63) inclusive.
    pub end_cell: u8,
    /// Number of pulses generated across this group (1-64).
    pub step_count: u8,
    /// Number of active morphing targets (0-4).
    pub morph_count: u8,
    /// Morphing targets. Only the first `morph_count` entries are valid.
    pub morph_targets: [MorphTarget; 4],
}

impl Default for FusedGroup {
    fn default() -> Self {
        Self {
            start_cell: 0,
            end_cell: 0,
            step_count: 0,
            morph_count: 0,
            morph_targets: [MorphTarget::default(); 4],
        }
    }
}

impl FusedGroup {
    pub fn cell_span(&self) -> usize {
        self.end_cell.saturating_sub(self.start_cell) as usize + 1
    }

    pub fn page(&self) -> usize {
        self.start_cell as usize / 16
    }

    pub fn is_page_local(&self) -> bool {
        self.start_cell / 16 == self.end_cell / 16
    }

    pub fn is_valid(&self) -> bool {
        let start = self.start_cell as usize;
        let end = self.end_cell as usize;
        start < end && end < STEP_COUNT && self.step_count >= 1 && self.is_page_local()
    }

    pub fn morph_active(&self) -> bool {
        self.morph_count > 0
    }

    /// [283] Same cells (start and end) as `other`.
    pub fn same_cells(&self, other: &FusedGroup) -> bool {
        self.start_cell == other.start_cell && self.end_cell == other.end_cell
    }

    /// Returns true if the given field index is already a morph target.
    pub fn has_morph_target(&self, field: usize) -> bool {
        self.morph_targets[..self.morph_count as usize]
            .iter()
            .any(|t| t.field == field as u8)
    }

    /// Add or update a morph target, preserving order and capping at 4.
    /// New targets default to `MorphDirection::Target`.
    ///
    /// Returns `false` when the cap refused a NEW target ([184] phase 3) — the
    /// old signature dropped it silently, so a fifth drag simply did nothing with
    /// no way for the UI to say why.
    pub fn set_morph_target(&mut self, field: usize, end_value: f32) -> bool {
        let field = field as u8;
        if let Some(existing) = self.morph_targets[..self.morph_count as usize]
            .iter_mut()
            .find(|t| t.field == field)
        {
            existing.end_value = end_value;
            return true;
        }
        if self.morph_count < MAX_MORPH_TARGETS as u8 {
            self.morph_targets[self.morph_count as usize] = MorphTarget {
                field,
                end_value,
                direction: MorphDirection::Target,
            };
            self.morph_count += 1;
            return true;
        }
        false
    }

    /// How many more fields this group can morph.
    pub fn morph_capacity_left(&self) -> usize {
        MAX_MORPH_TARGETS.saturating_sub(self.morph_count as usize)
    }

    /// Change the direction of an existing morph target without touching its value.
    pub fn set_morph_target_direction(&mut self, field: usize, direction: MorphDirection) {
        let field = field as u8;
        if let Some(existing) = self.morph_targets[..self.morph_count as usize]
            .iter_mut()
            .find(|t| t.field == field)
        {
            existing.direction = direction;
        }
    }

    /// Returns the direction of an existing morph target, or `Target` if absent.
    pub fn morph_target_direction(&self, field: usize) -> MorphDirection {
        let field = field as u8;
        self.morph_targets[..self.morph_count as usize]
            .iter()
            .find(|t| t.field == field)
            .map(|t| t.direction)
            .unwrap_or_default()
    }

    /// Remove a morph target if it exists.
    pub fn remove_morph_target(&mut self, field: usize) {
        let field = field as u8;
        let count = self.morph_count as usize;
        if let Some(pos) = self.morph_targets[..count]
            .iter()
            .position(|t| t.field == field)
        {
            for i in pos..count.saturating_sub(1) {
                self.morph_targets[i] = self.morph_targets[i + 1];
            }
            self.morph_targets[count.saturating_sub(1)] = MorphTarget::default();
            self.morph_count = self.morph_count.saturating_sub(1);
        }
    }

    /// Returns true if the given cell index is inside this group.
    pub fn contains(&self, cell: usize) -> bool {
        cell >= self.start_cell as usize && cell <= self.end_cell as usize
    }

    pub fn is_start(&self, cell: usize) -> bool {
        cell == self.start_cell as usize
    }
}

/// A 64-step pattern for drum sequencing (4 pages of 16 steps).
#[derive(Clone, Debug)]
pub struct Pattern {
    ///  steps, each with  instrument triggers.
    pub steps: [Step; STEP_COUNT],
    pub name: String,
    /// Per-instrument fused cell groups (Step Fusion feature).
    pub fusions: [Vec<FusedGroup>; INSTRUMENT_COUNT],
}

impl Pattern {
    /// Create empty pattern.
    pub fn empty() -> Self {
        Self {
            steps: array::from_fn(|_| Step::empty()),
            name: "Empty".to_string(),
            fusions: array::from_fn(|_| Vec::new()),
        }
    }

    pub fn default_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Default".to_string();

        pattern.steps[0].instruments[0] = true;
        pattern.steps[4].instruments[0] = true;
        pattern.steps[8].instruments[0] = true;
        pattern.steps[12].instruments[0] = true;

        pattern
    }

    pub fn rock_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Rock".to_string();

        for bar in 0..(STEP_COUNT / 16) {
            let offset = bar * 16;
            pattern.steps[offset + 0].instruments[0] = true;
            pattern.steps[offset + 4].instruments[0] = true;
            pattern.steps[offset + 8].instruments[0] = true;
            pattern.steps[offset + 12].instruments[0] = true;

            pattern.steps[offset + 4].instruments[1] = true;
            pattern.steps[offset + 12].instruments[1] = true;
        }

        for i in (0..STEP_COUNT).step_by(2) {
            pattern.steps[i].instruments[2] = true;
        }

        pattern
    }

    pub fn funk_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Funk".to_string();

        for bar in 0..(STEP_COUNT / 16) {
            let offset = bar * 16;
            pattern.steps[offset + 0].instruments[0] = true;
            pattern.steps[offset + 3].instruments[0] = true;
            pattern.steps[offset + 6].instruments[0] = true;
            pattern.steps[offset + 10].instruments[0] = true;

            pattern.steps[offset + 4].instruments[1] = true;
            pattern.steps[offset + 7].instruments[1] = true;
            pattern.steps[offset + 12].instruments[1] = true;

            pattern.steps[offset + 2].instruments[3] = true;
            pattern.steps[offset + 6].instruments[3] = true;
            pattern.steps[offset + 10].instruments[3] = true;
            pattern.steps[offset + 14].instruments[3] = true;
        }

        for i in 0..STEP_COUNT {
            if i % 3 != 0 {
                pattern.steps[i].instruments[2] = true;
            }
        }

        pattern
    }

    pub fn disco_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Disco".to_string();

        for bar in 0..(STEP_COUNT / 16) {
            let offset = bar * 16;
            for beat in [0, 4, 8, 12] {
                pattern.steps[offset + beat].instruments[0] = true;
            }

            pattern.steps[offset + 4].instruments[1] = true;
            pattern.steps[offset + 12].instruments[1] = true;

            for step in [3, 7, 11, 15] {
                pattern.steps[offset + step].instruments[3] = true;
            }
        }

        for step in 0..STEP_COUNT {
            pattern.steps[step].instruments[2] = true;
        }

        pattern
    }

    // ── Fixed canonical grooves for the newer generator styles ──────────────
    // Deterministic (unlike GENERATE). Each is authored for the matching kit
    // installed by the style-preset chip (`TrackLayoutState::from_kinds`); the
    // lane indices below MUST match that kit's slot order.

    /// Kit: 0 Kick · 1 Snare(cross-stick) · 2 Ride · 3 HiHat · 4 Perc1.
    pub fn bossa_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Bossa".to_string();
        for bar in 0..(STEP_COUNT / 16) {
            let o = bar * 16;
            for s in [0, 6, 8, 14] {
                pattern.steps[o + s].instruments[0] = true; // surdo kick
            }
            for s in [3, 6, 10, 12] {
                pattern.steps[o + s].instruments[1] = true; // cross-stick 3-2 clave
            }
            for s in [0, 2, 4, 6, 8, 10, 12, 14] {
                pattern.steps[o + s].instruments[2] = true; // ride comping (8ths)
            }
            for s in [4, 12] {
                pattern.steps[o + s].instruments[3] = true; // pedal hat on 2 & 4
            }
            for s in [2, 6, 10, 14] {
                pattern.steps[o + s].instruments[4] = true; // shaker offbeats
            }
        }
        pattern
    }

    /// Kit: 0 Kick · 1 Clap · 2 HiHat · 3 OpenHiHat · 4 Perc1.
    pub fn house_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "House".to_string();
        for bar in 0..(STEP_COUNT / 16) {
            let o = bar * 16;
            for s in [0, 4, 8, 12] {
                pattern.steps[o + s].instruments[0] = true; // four-on-the-floor
            }
            for s in [4, 12] {
                pattern.steps[o + s].instruments[1] = true; // clap backbeat
            }
            for s in [0, 4, 8, 12] {
                pattern.steps[o + s].instruments[2] = true; // closed hat on the beat
            }
            for s in [2, 6, 10, 14] {
                pattern.steps[o + s].instruments[3] = true; // open hat on the offbeat
            }
            for s in [3, 7, 11, 15] {
                pattern.steps[o + s].instruments[4] = true; // syncopated perc
            }
        }
        pattern
    }

    /// Kit: 0 Kick · 1 Snare · 2 HiHat · 3 Snare606(ghosts) · 4 BassDrum808.
    pub fn dnb_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "DnB".to_string();
        for bar in 0..(STEP_COUNT / 16) {
            let o = bar * 16;
            for s in [0, 10] {
                pattern.steps[o + s].instruments[0] = true; // two-step kick
            }
            for s in [4, 12] {
                pattern.steps[o + s].instruments[1] = true; // snare 2 & 4
            }
            for s in [0, 2, 4, 6, 8, 10, 12, 14] {
                pattern.steps[o + s].instruments[2] = true; // 8th ride
            }
            for s in [3, 7, 11, 15] {
                pattern.steps[o + s].instruments[3] = true; // ghost snares (606)
            }
            for s in [0, 10] {
                pattern.steps[o + s].instruments[4] = true; // sub follows the kick
            }
        }
        pattern
    }

    /// Kit: 0 Kick · 1 Snare(rim) · 2 HiHat · 3 Ride(bell) · 4 Tom(conga) · 5 Perc1.
    pub fn afrobeat_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Afrobeat".to_string();
        for bar in 0..(STEP_COUNT / 16) {
            let o = bar * 16;
            for s in [0, 6, 8, 14] {
                pattern.steps[o + s].instruments[0] = true; // syncopated kick
            }
            for s in [4, 10, 12] {
                pattern.steps[o + s].instruments[1] = true; // rim/snare accents
            }
            for s in [0, 2, 4, 6, 8, 10, 12, 14] {
                pattern.steps[o + s].instruments[2] = true; // busy 8ths
            }
            for s in [0, 3, 6, 8, 11, 14] {
                pattern.steps[o + s].instruments[3] = true; // bell pattern
            }
            for s in [3, 7, 11, 15] {
                pattern.steps[o + s].instruments[4] = true; // congas
            }
            for s in [2, 6, 10, 14] {
                pattern.steps[o + s].instruments[5] = true; // shaker offbeats
            }
        }
        pattern
    }

    /// Kit: 0 Kick · 1 Snare(rim) · 2 HiHat · 3 BassDrum808 · 4 Perc1.
    pub fn dub_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Dub".to_string();
        for bar in 0..(STEP_COUNT / 16) {
            let o = bar * 16;
            pattern.steps[o + 8].instruments[0] = true; // one-drop kick on beat 3
            pattern.steps[o + 8].instruments[1] = true; // one-drop rim on beat 3
            for s in [2, 6, 10, 14] {
                pattern.steps[o + s].instruments[2] = true; // offbeat skank
            }
            for s in [0, 8] {
                pattern.steps[o + s].instruments[3] = true; // deep 808 sub
            }
            for s in [6, 14] {
                pattern.steps[o + s].instruments[4] = true; // sparse perc echoes
            }
        }
        pattern
    }

    /// Kit: 0 Kick · 1 Snare · 2 HiHat · 3 Snare606(ghosts) · 4 Ride.
    pub fn breakbeat_pattern() -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Breakbeat".to_string();
        for bar in 0..(STEP_COUNT / 16) {
            let o = bar * 16;
            for s in [0, 6, 10] {
                pattern.steps[o + s].instruments[0] = true; // broken kick
            }
            for s in [4, 12] {
                pattern.steps[o + s].instruments[1] = true; // snare 2 & 4
            }
            for s in [0, 2, 3, 4, 6, 8, 10, 11, 12, 14] {
                pattern.steps[o + s].instruments[2] = true; // busy funky hats
            }
            for s in [3, 7, 11, 15] {
                pattern.steps[o + s].instruments[3] = true; // ghost snares (606)
            }
            for s in [0, 8] {
                pattern.steps[o + s].instruments[4] = true; // ride accents
            }
        }
        pattern
    }

    pub fn get_step(&self, index: usize) -> &Step {
        &self.steps[index % STEP_COUNT]
    }

    pub fn step_masks(&self) -> [u16; STEP_COUNT] {
        array::from_fn(|step| self.get_step(step).bitmask())
    }

    pub fn toggle(&mut self, instrument: usize, step: usize) {
        if instrument < INSTRUMENT_COUNT && step < STEP_COUNT {
            self.steps[step].instruments[instrument] = !self.steps[step].instruments[instrument];
        }
    }

    pub fn set(&mut self, instrument: usize, step: usize, value: bool) {
        if instrument < INSTRUMENT_COUNT && step < STEP_COUNT {
            self.steps[step].instruments[instrument] = value;
        }
    }

    pub fn clear(&mut self) {
        for step in &mut self.steps {
            step.instruments = [false; INSTRUMENT_COUNT];
        }
    }

    /// Full random: about one step in three on every lane `fill` accepts (the
    /// Generator passes "not an effect bus": an effect lane takes no trigger).
    pub fn random_pattern(fill: impl Fn(usize) -> bool) -> Self {
        let mut pattern = Self::empty();
        pattern.name = "Random".to_string();

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);

        // Simple LCG for deterministic randomness without dependencies
        let mut rng = seed;
        let mut lcg = || {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            rng
        };

        for step in 0..STEP_COUNT {
            for instrument in 0..INSTRUMENT_COUNT {
                if (lcg() % 10) < 3 && fill(instrument) {
                    // 30% probability like the original PoC
                    pattern.steps[step].instruments[instrument] = true;
                }
            }
        }

        pattern
    }
}

/// Lock-free pattern storage shared between the audio thread and the UI.
pub struct SharedPattern {
    steps: [AtomicU16; STEP_COUNT],
    /// Fused groups per instrument, packed as `FUSION_SLOT_COUNT` u64s per group.
    /// Index: instrument * MAX_FUSIONS * FUSION_SLOT_COUNT + group_index * FUSION_SLOT_COUNT + slot.
    /// The first slot for each instrument stores the count (packed as count | (1<<24)).
    fusions: [AtomicU64; INSTRUMENT_COUNT * MAX_FUSIONS * FUSION_SLOT_COUNT],
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PatternMasks(#[serde(with = "serde_arrays")] pub [u16; STEP_COUNT]);

/// Full pattern state persisted by the DAW: step masks + fused groups.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PatternState {
    #[serde(with = "serde_arrays")]
    pub masks: [u16; STEP_COUNT],
    #[serde(with = "serde_arrays")]
    pub fusions: [u64; INSTRUMENT_COUNT * MAX_FUSIONS * FUSION_SLOT_COUNT],
}

#[derive(Clone)]
pub struct PersistentPattern {
    shared: Arc<SharedPattern>,
}

impl PersistentPattern {
    pub fn new(pattern: &Pattern) -> Self {
        Self {
            shared: SharedPattern::new(pattern),
        }
    }

    pub fn shared(&self) -> Arc<SharedPattern> {
        self.shared.clone()
    }
}

impl<'a> PersistentField<'a, PatternState> for PersistentPattern {
    fn set(&self, new_value: PatternState) {
        self.shared.load_step_masks(&new_value.masks);
        for (i, &packed) in new_value.fusions.iter().enumerate() {
            self.shared.fusions[i].store(packed, Ordering::Relaxed);
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&PatternState) -> R,
    {
        let masks = self.shared.step_masks();
        let fusions = std::array::from_fn(|i| self.shared.fusions[i].load(Ordering::Relaxed));
        f(&PatternState { masks, fusions })
    }
}

impl SharedPattern {
    pub fn new(pattern: &Pattern) -> Arc<Self> {
        let fusions = array::from_fn(|i| {
            let inst = i / (MAX_FUSIONS * FUSION_SLOT_COUNT);
            let rem = i % (MAX_FUSIONS * FUSION_SLOT_COUNT);
            let group_idx = rem / FUSION_SLOT_COUNT;
            let slot = rem % FUSION_SLOT_COUNT;
            if group_idx == 0 && slot == 0 {
                // First slot stores count
                AtomicU64::new((pattern.fusions[inst].len() as u64) | (1u64 << 24))
            } else if group_idx > 0 && group_idx <= pattern.fusions[inst].len() {
                let packed = pack_fusion(&pattern.fusions[inst][group_idx - 1]);
                AtomicU64::new(packed[slot])
            } else {
                AtomicU64::new(0)
            }
        });
        let shared = Arc::new(Self {
            steps: array::from_fn(|step| AtomicU16::new(pattern.get_step(step).bitmask())),
            fusions,
        });
        shared
    }

    /// Raw record `i` of an instrument's fusion list (after its count word).
    fn fusion_record(&self, base: usize, i: usize) -> [u64; FUSION_SLOT_COUNT] {
        let slot_base = base + (1 + i) * FUSION_SLOT_COUNT;
        [
            self.fusions[slot_base].load(Ordering::Relaxed),
            self.fusions[slot_base + 1].load(Ordering::Relaxed),
            self.fusions[slot_base + 2].load(Ordering::Relaxed),
        ]
    }

    /// Visit the records of an instrument's list that `unpack` accepts. RT-safe.
    fn for_each_record(
        &self,
        instrument: usize,
        unpack: fn([u64; FUSION_SLOT_COUNT]) -> Option<FusedGroup>,
        mut f: impl FnMut(FusedGroup),
    ) {
        if instrument >= INSTRUMENT_COUNT {
            return;
        }
        let base = instrument * MAX_FUSIONS * FUSION_SLOT_COUNT;
        let count = (self.fusions[base].load(Ordering::Relaxed) & 0xFF) as usize;
        for i in 0..count.min(MAX_FUSIONS - 1) {
            if let Some(group) = unpack(self.fusion_record(base, i)) {
                f(group);
            }
        }
    }

    /// Load fused groups for an instrument.
    pub fn load_fusions(&self, instrument: usize) -> Vec<FusedGroup> {
        let mut groups = Vec::new();
        self.for_each_record(instrument, unpack_fusion, |group| groups.push(group));
        groups
    }

    /// Load fused groups into a caller-provided fixed buffer. This is used by
    /// the audio thread to avoid allocating while syncing UI state.
    pub fn load_fusions_into(
        &self,
        instrument: usize,
        out: &mut [FusedGroup; MAX_FUSIONS],
    ) -> usize {
        let mut written = 0usize;
        self.for_each_record(instrument, unpack_fusion, |group| {
            if written < out.len() {
                out[written] = group;
                written += 1;
            }
        });
        written
    }

    /// [283] The morphs a LINKED lane keeps for the fused groups of the lane it
    /// follows, one per group it morphs (cells = that group's cells).
    pub fn load_linked_morphs(&self, instrument: usize) -> Vec<FusedGroup> {
        let mut morphs = Vec::new();
        self.for_each_record(instrument, unpack_linked_morph, |morph| morphs.push(morph));
        morphs
    }

    /// [`Self::load_linked_morphs`] into a fixed buffer (audio thread).
    pub fn load_linked_morphs_into(
        &self,
        instrument: usize,
        out: &mut [FusedGroup; MAX_FUSIONS],
    ) -> usize {
        let mut written = 0usize;
        self.for_each_record(instrument, unpack_linked_morph, |morph| {
            if written < out.len() {
                out[written] = morph;
                written += 1;
            }
        });
        written
    }

    /// Store fused groups for an instrument. Its linked-lane morphs are kept.
    pub fn store_fusions(&self, instrument: usize, groups: &[FusedGroup]) {
        let mut morphs = [FusedGroup::default(); MAX_FUSIONS];
        let n = self.load_linked_morphs_into(instrument, &mut morphs);
        self.store_fusions_and_morphs(instrument, groups, &morphs[..n]);
    }

    /// [283] Store a linked lane's own morphs. Its fused groups are kept.
    pub fn store_linked_morphs(&self, instrument: usize, morphs: &[FusedGroup]) {
        let mut groups = [FusedGroup::default(); MAX_FUSIONS];
        let n = self.load_fusions_into(instrument, &mut groups);
        self.store_fusions_and_morphs(instrument, &groups[..n], morphs);
    }

    /// Store an instrument's whole fusion list: its fused groups, then its
    /// linked-lane morphs, within the list's capacity (groups first). RT-safe.
    pub fn store_fusions_and_morphs(
        &self,
        instrument: usize,
        groups: &[FusedGroup],
        morphs: &[FusedGroup],
    ) {
        if instrument >= INSTRUMENT_COUNT {
            return;
        }
        let base = instrument * MAX_FUSIONS * FUSION_SLOT_COUNT;
        let records = groups
            .iter()
            .filter(|group| group.is_valid())
            .map(pack_fusion)
            .chain(
                morphs
                    .iter()
                    .filter(|morph| morph.is_valid() && morph.morph_active())
                    .map(pack_linked_morph),
            )
            .take(MAX_FUSIONS - 1);
        let mut count = 0usize;
        for packed in records {
            let slot_base = base + (1 + count) * FUSION_SLOT_COUNT;
            for (slot, &value) in packed.iter().enumerate() {
                self.fusions[slot_base + slot].store(value, Ordering::Relaxed);
            }
            count += 1;
        }
        // Clear remaining slots
        for i in count..(MAX_FUSIONS - 1) {
            let slot_base = base + (1 + i) * FUSION_SLOT_COUNT;
            for slot in 0..FUSION_SLOT_COUNT {
                self.fusions[slot_base + slot].store(0, Ordering::Relaxed);
            }
        }
        // Update count
        self.fusions[base].store((count as u64) | (1u64 << 24), Ordering::Relaxed);
    }

    /// [283] The fused groups lane `slot` plays: those of `grid`, the lane its
    /// link chain follows (itself when unlinked), each with `slot`'s morph -
    /// for a linked lane its own, none until it creates one.
    pub fn load_lane_fusions(&self, slot: usize, grid: usize) -> Vec<FusedGroup> {
        let mut groups = self.load_fusions(grid);
        if slot != grid {
            let morphs = self.load_linked_morphs(slot);
            for group in &mut groups {
                *group = with_linked_morph(*group, &morphs);
            }
        }
        groups
    }

    /// [283] Publish group `index` of [`Self::load_lane_fusions`]. The lane
    /// owning the groups stores it whole; a linked lane only its morph (cells
    /// and pulses belong to the lane it follows).
    pub fn store_lane_fusion(&self, slot: usize, grid: usize, index: usize, group: FusedGroup) {
        let mut groups = self.load_fusions(grid);
        let Some(shared) = groups.get_mut(index) else {
            return;
        };
        if slot == grid {
            *shared = group;
            self.store_fusions(grid, &groups);
            return;
        }
        let mut morphs = self.load_linked_morphs(slot);
        morphs.retain(|morph| !morph.same_cells(shared));
        if group.morph_active() {
            morphs.push(with_linked_morph(*shared, &[group]));
        }
        self.store_linked_morphs(slot, &morphs);
    }

    /// [283] Drop the linked-lane morphs that no longer match a fused group of
    /// the lane they follow (group deleted or moved, lane unlinked), so a group
    /// later made on the same cells starts without a morph. `grid_of(slot)` is
    /// the lane `slot` follows, itself when unlinked.
    pub fn retain_linked_morphs(&self, grid_of: impl Fn(usize) -> usize) {
        for slot in 0..INSTRUMENT_COUNT {
            let mut morphs = [FusedGroup::default(); MAX_FUSIONS];
            let n = self.load_linked_morphs_into(slot, &mut morphs);
            if n == 0 {
                continue;
            }
            let grid = grid_of(slot);
            let mut groups = [FusedGroup::default(); MAX_FUSIONS];
            let g = if grid == slot {
                0
            } else {
                self.load_fusions_into(grid, &mut groups)
            };
            let mut kept = [FusedGroup::default(); MAX_FUSIONS];
            let mut k = 0usize;
            for morph in &morphs[..n] {
                if groups[..g].iter().any(|group| group.same_cells(morph)) {
                    kept[k] = *morph;
                    k += 1;
                }
            }
            if k != n {
                self.store_linked_morphs(slot, &kept[..k]);
            }
        }
    }

    pub fn load_step_mask(&self, step: usize) -> u16 {
        self.steps[step % STEP_COUNT].load(Ordering::Relaxed)
    }

    pub fn set_step_mask(&self, step: usize, mask: u16) {
        // Mask bits to the number of instruments. Each bit position
        // corresponds to a voice index in `DrumVoice`. Wider masks lose any
        // active bits for voices beyond INSTRUMENT_COUNT, which is why
        // adding an 11th voice required widening this from 0x3ff (10 bits) to
        // accommodate the new bit 10 (Snare 606).
        let valid_bits = (1u16 << INSTRUMENT_COUNT).wrapping_sub(1);
        self.steps[step % STEP_COUNT].store(mask & valid_bits, Ordering::Relaxed);
    }

    pub fn is_active(&self, step: usize, instrument: usize) -> bool {
        if instrument >= INSTRUMENT_COUNT {
            return false;
        }

        let mask = self.load_step_mask(step);
        (mask & (1 << instrument)) != 0
    }

    pub fn load_step_masks(&self, masks: &[u16; STEP_COUNT]) {
        for (step, mask) in masks.iter().copied().enumerate() {
            self.set_step_mask(step, mask);
        }
    }

    pub fn step_masks(&self) -> [u16; STEP_COUNT] {
        array::from_fn(|step| self.load_step_mask(step))
    }
}

#[cfg(test)]
mod tests {
    /// Full random never writes a step on a lane it is told to skip.
    #[test]
    fn random_pattern_leaves_skipped_lanes_empty() {
        let pattern = Pattern::random_pattern(|lane| lane < 12);
        for step in &pattern.steps {
            assert!(!step.instruments[12] && !step.instruments[13]);
        }
        assert!(pattern.steps.iter().any(|s| s.instruments[..12].iter().any(|&on| on)));
    }

    use super::*;

    #[test]
    fn new_style_presets_are_well_formed() {
        // (groove, kit size) — the groove must only touch lanes inside its kit.
        let cases = [
            (Pattern::bossa_pattern(), 5),
            (Pattern::house_pattern(), 5),
            (Pattern::dnb_pattern(), 5),
            (Pattern::afrobeat_pattern(), 6),
            (Pattern::dub_pattern(), 5),
            (Pattern::breakbeat_pattern(), 5),
        ];
        for (pat, kit_size) in cases {
            let kick = (0..16).any(|s| pat.steps[s].instruments[0]);
            let hits: usize = pat
                .steps
                .iter()
                .map(|s| s.instruments.iter().filter(|&&on| on).count())
                .sum();
            assert!(kick, "{} preset has no kick on lane 0", pat.name);
            assert!(hits >= 8, "{} preset is too sparse ({hits} hits)", pat.name);
            // Every hit must fall inside the kit's active lanes.
            for step in &pat.steps {
                for (lane, &on) in step.instruments.iter().enumerate() {
                    assert!(
                        !on || lane < kit_size,
                        "{} preset uses lane {} beyond its {}-lane kit",
                        pat.name,
                        lane,
                        kit_size
                    );
                }
            }
        }
    }

    fn make_group(morph_targets: &[(usize, f32)], step_count: u8) -> FusedGroup {
        let mut group = FusedGroup {
            start_cell: 0,
            end_cell: step_count,
            step_count,
            morph_count: morph_targets.len().min(4) as u8,
            morph_targets: [MorphTarget::default(); 4],
        };
        for (i, &(field, value)) in morph_targets.iter().enumerate().take(4) {
            group.morph_targets[i] = MorphTarget {
                field: field as u8,
                end_value: value,
                direction: MorphDirection::Target,
            };
        }
        group
    }

    #[test]
    fn pack_unpack_roundtrip_single_target() {
        let group = make_group(&[(0, 300.0)], 4);
        let packed = pack_fusion(&group);
        let unpacked = unpack_fusion(packed).unwrap();
        assert_eq!(group, unpacked);
    }

    #[test]
    fn pack_unpack_roundtrip_four_targets() {
        let group = make_group(&[(0, 300.0), (1, 2.5), (3, 8000.0), (18, 0.05)], 8);
        let packed = pack_fusion(&group);
        let unpacked = unpack_fusion(packed).unwrap();
        assert_eq!(group, unpacked);
    }

    #[test]
    fn pack_unpack_preserves_frequency_tom1() {
        let group = make_group(&[(0, 300.0)], 4);
        let packed = pack_fusion(&group);
        let unpacked = unpack_fusion(packed).unwrap();
        assert_eq!(unpacked.morph_targets[0].end_value, 300.0);
        assert!((unpacked.morph_targets[0].end_value - 300.0).abs() < f32::EPSILON);
    }

    #[test]
    fn pack_unpack_preserves_morph_direction() {
        let mut group = FusedGroup {
            start_cell: 2,
            end_cell: 6,
            step_count: 4,
            morph_count: 2,
            morph_targets: [
                MorphTarget {
                    field: 0,
                    end_value: 300.0,
                    direction: MorphDirection::Source,
                },
                MorphTarget {
                    field: 1,
                    end_value: 1.2,
                    direction: MorphDirection::Target,
                },
                MorphTarget::default(),
                MorphTarget::default(),
            ],
        };
        let packed = pack_fusion(&group);
        let unpacked = unpack_fusion(packed).unwrap();
        assert_eq!(unpacked.morph_targets[0].direction, MorphDirection::Source);
        assert_eq!(unpacked.morph_targets[1].direction, MorphDirection::Target);

        // Toggle and round-trip again.
        group.morph_targets[0].direction = MorphDirection::Target;
        group.morph_targets[1].direction = MorphDirection::Source;
        let packed2 = pack_fusion(&group);
        let unpacked2 = unpack_fusion(packed2).unwrap();
        assert_eq!(unpacked2.morph_targets[0].direction, MorphDirection::Target);
        assert_eq!(unpacked2.morph_targets[1].direction, MorphDirection::Source);
    }

    #[test]
    fn old_broken_format_returns_none_at_fusion_level() {
        // Old layout is no longer decoded here; migration happens in filter_state.
        let old_meta: u64 = (0u64)
            | (2u64 << 0)   // start_cell
            | (6u64 << 8)   // end_cell
            | (4u64 << 16)  // step_count
            | (1u64 << 24); // valid
        let slots = [old_meta, 0, 0];
        assert!(unpack_fusion(slots).is_none());
    }

    #[test]
    fn shared_pattern_store_load_fusion_no_morph_roundtrip() {
        let pattern = SharedPattern::new(&Pattern::empty());
        let group = FusedGroup {
            start_cell: 2,
            end_cell: 6,
            step_count: 4,
            morph_count: 0,
            morph_targets: [MorphTarget::default(); 4],
        };
        pattern.store_fusions(4, &[group]);
        let loaded = pattern.load_fusions(4);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], group);
    }

    #[test]
    fn shared_pattern_store_load_fusion_roundtrip() {
        let pattern = SharedPattern::new(&Pattern::empty());
        let group = FusedGroup {
            start_cell: 2,
            end_cell: 6,
            step_count: 4,
            morph_count: 1,
            morph_targets: [
                MorphTarget {
                    field: 0,
                    end_value: 300.0,
                    direction: MorphDirection::Target,
                },
                MorphTarget::default(),
                MorphTarget::default(),
                MorphTarget::default(),
            ],
        };
        pattern.store_fusions(4, &[group]);
        let loaded = pattern.load_fusions(4);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], group);
    }

    fn morph(field: u8, end_value: f32) -> [MorphTarget; MAX_MORPH_TARGETS] {
        let mut targets = [MorphTarget::default(); MAX_MORPH_TARGETS];
        targets[0] = MorphTarget {
            field,
            end_value,
            direction: MorphDirection::Target,
        };
        targets
    }

    /// [283] A linked lane's own morphs live in its fusion list next to its
    /// own (hidden) groups without being read as groups, survive a store of
    /// those groups, and ride on the groups of the lane it follows.
    #[test]
    fn linked_morphs_are_kept_apart_from_fused_groups() {
        let pattern = SharedPattern::new(&Pattern::empty());
        let shared = FusedGroup {
            start_cell: 4,
            end_cell: 7,
            step_count: 4,
            morph_count: 1,
            morph_targets: morph(0, 300.0),
        };
        let hidden = FusedGroup {
            start_cell: 8,
            end_cell: 9,
            step_count: 2,
            ..Default::default()
        };
        pattern.store_fusions(0, &[shared]);
        pattern.store_fusions(1, &[hidden]);
        assert!(
            pattern.load_lane_fusions(1, 0)[0].morph_count == 0,
            "no morph until created"
        );

        let own = FusedGroup {
            morph_count: 1,
            morph_targets: morph(2, 0.5),
            ..shared
        };
        pattern.store_lane_fusion(1, 0, 0, own);
        assert_eq!(
            pattern.load_fusions(0),
            vec![shared],
            "the followed lane keeps its morph"
        );
        assert_eq!(
            pattern.load_fusions(1),
            vec![hidden],
            "a morph is not a group"
        );
        assert_eq!(pattern.load_lane_fusions(1, 0), vec![own]);

        pattern.store_fusions(1, &[]);
        assert_eq!(
            pattern.load_linked_morphs(1).len(),
            1,
            "storing groups keeps morphs"
        );

        pattern.retain_linked_morphs(|slot| if slot == 1 { 0 } else { slot });
        assert_eq!(
            pattern.load_linked_morphs(1).len(),
            1,
            "its group still exists"
        );
        pattern.store_fusions(0, &[]);
        pattern.retain_linked_morphs(|slot| if slot == 1 { 0 } else { slot });
        assert!(
            pattern.load_linked_morphs(1).is_empty(),
            "its group is gone"
        );
    }

    /// [283] Older readers skip a linked-lane morph record: valid bit clear.
    #[test]
    fn linked_morph_record_is_not_a_fused_group() {
        let own = FusedGroup {
            start_cell: 0,
            end_cell: 3,
            step_count: 2,
            morph_count: 1,
            morph_targets: morph(1, 0.25),
        };
        let packed = pack_linked_morph(&own);
        assert_eq!(unpack_fusion(packed), None);
        assert_eq!(unpack_linked_morph(packed), Some(own));
        assert_eq!(unpack_linked_morph(pack_fusion(&own)), None);
    }

    #[test]
    fn invalid_fusion_returns_none() {
        let slots = [0u64; 3];
        assert!(unpack_fusion(slots).is_none());
    }
}
