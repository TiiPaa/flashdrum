//! [184] Where a Lane Editor row's value comes from.
//!
//! One row implementation has to serve three stores:
//!
//! | Scope | Store | Fallback |
//! |---|---|---|
//! | lane global | `SoundSettingsState` atomics + the algo `IntParam` | — |
//! | per-step p-lock | `PlockState` (value + field mask) | the lane global |
//! | fusion morph endpoint | `FusedGroup.morph_targets` | the start cell's p-lock |
//!
//! This is a **trait with composition**, not an enum: `PlockSource` *owns* a
//! `GlobalSource`, and the morph source will own a `PlockSource`. That is what
//! makes [`ParamSource::inherited`] — the target of the revert affordance and of
//! double-click — correct by construction instead of re-implementing the
//! fallback chain in three `match` arms.
//!
//! The algo is reached through the [`AlgoSink`] seam rather than a `ParamSetter`
//! directly, because `ParamSetter` needs a live `GuiContext`: the seam is what
//! makes this whole layer testable headless.

use crate::param_id::ParamId;
use crate::plock::PlockState;
use crate::sound_settings::SoundSettingsState;

/// Whether a row may be edited in the current scope.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Support {
    Editable,
    /// Greyed with this reason on hover — **never hidden**. Hiding a row is what
    /// makes today's behaviour inexplicable (a p-lockable-looking parameter that
    /// simply is not there), and it breaks the "UI zones stables" rule.
    Disabled(&'static str),
}

impl Support {
    /// The reason a row is greyed, for its tooltip. `None` when editable — the
    /// panel needs the reason, not a bare boolean, precisely so a disabled row
    /// can explain itself.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            Support::Editable => None,
            Support::Disabled(reason) => Some(reason),
        }
    }
}

/// The algo's store, abstracted so this layer never needs a `ParamSetter`.
///
/// The algo is the one parameter with two homes: a nih-plug `IntParam` when
/// lane-global (so the host sees automation), and p-lock field 13 per step.
pub trait AlgoSink {
    fn get(&self) -> u8;
    fn set(&self, value: u8);
}

/// Reads and writes one slot's parameters for the active scope.
pub trait ParamSource {
    /// The value the row displays: the scope's override when it has one,
    /// otherwise whatever it inherits.
    fn get(&self, id: ParamId) -> f32;
    fn set(&mut self, id: ParamId, value: f32);
    /// True when the value comes from this scope rather than being inherited.
    /// Always false on the lane global — it inherits from nothing.
    fn is_overridden(&self, id: ParamId) -> bool;
    /// Drop this scope's override. No-op on the lane global.
    fn clear(&mut self, id: ParamId);
    /// What [`ParamSource::get`] would return with the override dropped: the
    /// revert target, and what the row shows greyed behind an override.
    fn inherited(&self, id: ParamId) -> f32;
    fn supports(&self, id: ParamId) -> Support;
    /// Discriminator for egui widget ids, so two live sources (a panel row and
    /// the popup, or two scopes) can never collide on `make_persistent_id`.
    fn salt(&self) -> (u8, usize, usize);
    /// Flush anything this source batches. Called once per frame by the panel.
    fn commit(&mut self) {}
    /// A standing explanation for the panel's notice strip, e.g. "this group
    /// cannot take another morph target".
    ///
    /// A state, not an event: with every non-target row greyed, the user needs to
    /// know WHY the panel looks inert, and needs it available as long as it is
    /// true. The strip is dismissable, and re-arms when the situation clears.
    fn notice(&self) -> Option<&'static str> {
        None
    }
}

// ── Lane global ─────────────────────────────────────────────────────────────

pub struct GlobalSource<'a> {
    pub settings: &'a SoundSettingsState,
    pub slot: usize,
    pub voice_idx: usize,
    pub algo: &'a dyn AlgoSink,
    /// Set by `set`, consumed by `commit`: one version bump per frame instead of
    /// one per edited row.
    dirty: bool,
}

impl<'a> GlobalSource<'a> {
    pub fn new(
        settings: &'a SoundSettingsState,
        slot: usize,
        voice_idx: usize,
        algo: &'a dyn AlgoSink,
    ) -> Self {
        Self {
            settings,
            slot,
            voice_idx,
            algo,
            dirty: false,
        }
    }

    /// The registry's factory default for this parameter — the one definition,
    /// replacing the three the Sound panel used to carry (per-voice
    /// `sound_settings_default`, `VoiceSettings::default()`, and `def.default`
    /// for specials).
    pub fn factory_default(&self, id: ParamId) -> f32 {
        crate::instrument_registry::param_default(self.voice_idx, id)
    }
}

impl ParamSource for GlobalSource<'_> {
    fn get(&self, id: ParamId) -> f32 {
        match id {
            ParamId::Algo => self.algo.get() as f32,
            _ => self.settings.instruments[self.slot].get(id),
        }
    }

    fn set(&mut self, id: ParamId, value: f32) {
        match id {
            ParamId::Algo => self.algo.set(value.round().clamp(0.0, 255.0) as u8),
            _ => {
                self.settings.instruments[self.slot].set(id, value);
                self.dirty = true;
            }
        }
    }

    fn is_overridden(&self, _id: ParamId) -> bool {
        false
    }

    fn clear(&mut self, _id: ParamId) {}

    fn inherited(&self, id: ParamId) -> f32 {
        self.factory_default(id)
    }

    fn supports(&self, _id: ParamId) -> Support {
        Support::Editable
    }

    fn salt(&self) -> (u8, usize, usize) {
        (0, self.slot, 0)
    }

    fn commit(&mut self) {
        if self.dirty {
            // The audio thread re-polls on a version change, once per buffer.
            self.settings.bump_version();
            self.dirty = false;
        }
    }
}

// ── Per-step p-lock ─────────────────────────────────────────────────────────

pub struct PlockSource<'a> {
    pub plock: &'a PlockState,
    pub step: usize,
    pub base: GlobalSource<'a>,
}

impl<'a> PlockSource<'a> {
    pub fn new(plock: &'a PlockState, step: usize, base: GlobalSource<'a>) -> Self {
        Self { plock, step, base }
    }

    fn field(&self, id: ParamId) -> Option<usize> {
        if id.is_lockable() {
            id.plock_field()
        } else {
            None
        }
    }
}

impl ParamSource for PlockSource<'_> {
    fn get(&self, id: ParamId) -> f32 {
        match self.field(id) {
            Some(field)
                if self
                    .plock
                    .field_masks
                    .is_set(self.base.slot, self.step, field) =>
            {
                self.plock.values.get(self.base.slot, self.step, field)
            }
            _ => self.base.get(id),
        }
    }

    fn set(&mut self, id: ParamId, value: f32) {
        // A display mode belongs to the lane, in every scope.
        if id == ParamId::FreqMode {
            return self.base.set(id, value);
        }
        if let Some(field) = self.field(id) {
            // No `if overridden` gate: that gate is why the popup's checkbox rows
            // could never CREATE an override. `set_field` also raises the step's
            // active bit, so touching any row makes the p-lock exist.
            self.plock
                .set_field(self.base.slot, self.step, field, value);
        }
    }

    fn is_overridden(&self, id: ParamId) -> bool {
        self.field(id).is_some_and(|field| {
            self.plock
                .field_masks
                .is_set(self.base.slot, self.step, field)
        })
    }

    fn clear(&mut self, id: ParamId) {
        if let Some(field) = self.field(id) {
            self.plock.clear_field(self.base.slot, self.step, field);
        }
    }

    /// The **live** lane value, not the factory default: dropping a per-step
    /// override means "follow the lane again".
    fn inherited(&self, id: ParamId) -> f32 {
        self.base.get(id)
    }

    fn supports(&self, id: ParamId) -> Support {
        match id.unlockable_reason() {
            Some(reason) => Support::Disabled(reason),
            None => Support::Editable,
        }
    }

    fn salt(&self) -> (u8, usize, usize) {
        (1, self.base.slot, self.step)
    }

    fn commit(&mut self) {
        // P-lock writes are immediate (the audio thread reads them at trigger
        // time, not through the version counter), but the lane fallback may have
        // been touched by a FreqMode change.
        self.base.commit();
    }
}

// ── Fusion morph endpoint ───────────────────────────────────────────────────

/// Which end of a fused group's morph the panel is editing ([184] phase 3).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum MorphEnd {
    /// The value the group starts from.
    #[default]
    Start,
    /// The value it reaches on its last pulse.
    End,
}

/// Edits a fused group's morph, either endpoint.
///
/// # Why one source handles both ends
///
/// Only **one** endpoint is stored (`MorphTarget::end_value`); the other is
/// resolved at trigger time from the start cell's merged settings, and
/// `MorphDirection` says which side the stored number sits on. So the endpoint
/// the user asks for and the store to write compose as a 2×2:
///
/// | tab | direction | the row writes |
/// |---|---|---|
/// | End | `Target` | the group's stored value |
/// | Start | `Target` | the start cell's p-lock (the live end) |
/// | Start | `Source` | the group's stored value |
/// | End | `Source` | the start cell's p-lock |
///
/// That is one boolean, not four cases — and it makes inherited `Source` targets
/// work with **no data migration**, which is why the direction flag is kept.
/// A field that is not yet a target reports `Target` by default, so a first drag
/// in the End tab creates the morph while the same drag in the Start tab writes
/// the p-lock.
pub struct MorphSource<'a> {
    pub base: PlockSource<'a>,
    pattern: &'a crate::sequencer::SharedPattern,
    fusion_index: usize,
    end: MorphEnd,
    /// Working copy: loaded once, published once by `commit`. The old morph menu
    /// re-published the lane's whole fusion array on **every slider frame**.
    group: crate::sequencer::pattern::FusedGroup,
    /// [283] The lane whose fused groups the edited lane plays (itself unless
    /// linked): a linked lane publishes only its own morph.
    grid: usize,
    dirty: bool,
}

impl<'a> MorphSource<'a> {
    pub fn new(
        pattern: &'a crate::sequencer::SharedPattern,
        fusion_index: usize,
        group: crate::sequencer::pattern::FusedGroup,
        end: MorphEnd,
        base: PlockSource<'a>,
    ) -> Self {
        let grid = base.base.slot;
        Self {
            base,
            pattern,
            fusion_index,
            end,
            group,
            grid,
            dirty: false,
        }
    }

    /// [283] The edited lane is linked: its fused groups are those of `grid`.
    pub fn following(mut self, grid: usize) -> Self {
        self.grid = grid;
        self
    }

    /// The 2×2 above, plus the creation case.
    fn writes_group(&self, id: ParamId) -> bool {
        let Some(field) = self.field(id) else {
            return false;
        };
        if !self.group.has_morph_target(field) {
            // No morph on this field yet: EITHER tab creates one, so that setting
            // an endpoint always produces an audible ramp toward the live value.
            // Writing the p-lock instead would just play that value flat on every
            // pulse — nothing would morph, which is what made "set Start, hear no
            // ramp" so confusing.
            return true;
        }
        let stored_is_end = self.group.morph_target_direction(field)
            == crate::sequencer::pattern::MorphDirection::Target;
        matches!(
            (self.end, stored_is_end),
            (MorphEnd::End, true) | (MorphEnd::Start, false)
        )
    }

    /// Which side the stored value sits on when this tab creates a target.
    ///
    /// `End` stores the arrival value (`Target`: the ramp runs live -> stored),
    /// `Start` stores the departure value (`Source`: stored -> live). The other
    /// end is the start cell's merged value, i.e. its p-lock if it has one and
    /// the lane's sound otherwise — so editing one endpoint in the panel and the
    /// other on the lane behaves the way it reads.
    fn direction_for_tab(&self) -> crate::sequencer::pattern::MorphDirection {
        match self.end {
            MorphEnd::End => crate::sequencer::pattern::MorphDirection::Target,
            MorphEnd::Start => crate::sequencer::pattern::MorphDirection::Source,
        }
    }

    fn field(&self, id: ParamId) -> Option<usize> {
        if id.is_lockable() {
            id.plock_field()
        } else {
            None
        }
    }

    fn stored_value(&self, field: usize) -> Option<f32> {
        self.group.morph_targets[..self.group.morph_count as usize]
            .iter()
            .find(|target| target.field == field as u8)
            .map(|target| target.end_value)
    }

    /// How many more fields this group can morph. Reads better than
    /// `group.morph_capacity_left()` at the call site; currently only the tests
    /// use it, `supports` going straight to the group.
    pub fn capacity_left(&self) -> usize {
        self.group.morph_capacity_left()
    }
}

impl ParamSource for MorphSource<'_> {
    fn get(&self, id: ParamId) -> f32 {
        match self.field(id) {
            Some(field) if self.writes_group(id) => self
                .stored_value(field)
                // Not a target yet: show the live value as the starting point, so
                // the first drag begins where the sound already is.
                .unwrap_or_else(|| self.base.get(id)),
            _ => self.base.get(id),
        }
    }

    fn set(&mut self, id: ParamId, value: f32) {
        if !self.writes_group(id) {
            return self.base.set(id, value);
        }
        let Some(field) = self.field(id) else { return };
        let creating = !self.group.has_morph_target(field);
        if self.group.set_morph_target(field, value) {
            if creating {
                // `set_morph_target` defaults to `Target`; the Start tab needs the
                // opposite side.
                self.group
                    .set_morph_target_direction(field, self.direction_for_tab());
            }
            self.dirty = true;
        }
        // A refusal (the cap) leaves the group untouched. It should not be
        // reachable any more: `supports` greys such a row up front, and `notice`
        // explains the situation.
    }

    fn is_overridden(&self, id: ParamId) -> bool {
        if self.writes_group(id) {
            self.field(id)
                .is_some_and(|field| self.group.has_morph_target(field))
        } else {
            self.base.is_overridden(id)
        }
    }

    fn clear(&mut self, id: ParamId) {
        if !self.writes_group(id) {
            return self.base.clear(id);
        }
        if let Some(field) = self.field(id) {
            self.group.remove_morph_target(field);
            self.dirty = true;
        }
    }

    fn inherited(&self, id: ParamId) -> f32 {
        self.base.inherited(id)
    }

    fn supports(&self, id: ParamId) -> Support {
        if !self.writes_group(id) {
            return self.base.supports(id);
        }
        let Some(field) = self.field(id) else {
            return self.base.supports(id);
        };
        if !crate::instrument_registry::param_is_morphable(self.base.base.voice_idx, id) {
            return Support::Disabled("Morph interpolates: continuous parameters only");
        }
        if !self.group.has_morph_target(field) && self.group.morph_capacity_left() == 0 {
            return Support::Disabled("Morph is full (4 targets) - remove one first");
        }
        Support::Editable
    }

    fn salt(&self) -> (u8, usize, usize) {
        // Distinct from the p-lock scope AND between the two tabs, so a dropdown
        // left open in one never reopens in the other.
        (2 + self.end as u8, self.base.base.slot, self.base.step)
    }

    fn notice(&self) -> Option<&'static str> {
        (self.capacity_left() == 0).then_some(
            "Morph limit: 4 targets per fused group. Click a target's accent bar to free a slot.",
        )
    }

    fn commit(&mut self) {
        self.base.commit();
        if !self.dirty {
            return;
        }
        // `store_fusions` silently DROPS any group failing `is_valid()`, which is
        // how a fusion could vanish mid-drag. Refuse to publish rather than lose
        // the group.
        if !self.group.is_valid() {
            debug_assert!(false, "refusing to publish an invalid fused group");
            return;
        }
        self.pattern.store_lane_fusion(
            self.base.base.slot,
            self.grid,
            self.fusion_index,
            self.group,
        );
        self.dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instrument_registry::StandardField;
    use crate::track::TrackLayoutState;
    use std::cell::Cell;

    /// Headless stand-in for the nih-plug `IntParam` + `ParamSetter` pair.
    struct CellAlgo(Cell<u8>);
    impl AlgoSink for CellAlgo {
        fn get(&self) -> u8 {
            self.0.get()
        }
        fn set(&self, value: u8) {
            self.0.set(value);
        }
    }

    const SLOT: usize = 2;
    const VOICE: usize = 0; // Kick
    const STEP: usize = 5;

    fn global<'a>(settings: &'a SoundSettingsState, algo: &'a CellAlgo) -> GlobalSource<'a> {
        GlobalSource::new(settings, SLOT, VOICE, algo)
    }

    #[test]
    fn global_source_round_trips_every_kind_of_parameter() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let mut src = global(&settings, &algo);

        src.set(ParamId::Std(StandardField::Decay), 0.42);
        assert_eq!(src.get(ParamId::Std(StandardField::Decay)), 0.42);

        src.set(ParamId::Special(3), 0.75);
        assert_eq!(src.get(ParamId::Special(3)), 0.75);

        // The algo goes to its own store, not the atomics.
        src.set(ParamId::Algo, 2.0);
        assert_eq!(algo.get(), 2);
        assert_eq!(src.get(ParamId::Algo), 2.0);

        src.set(ParamId::FreqMode, 1.0);
        assert_eq!(src.get(ParamId::FreqMode), 1.0);

        // Nothing is ever "overridden" on the lane itself.
        assert!(!src.is_overridden(ParamId::Std(StandardField::Decay)));
        assert_eq!(
            src.supports(ParamId::Std(StandardField::Decay)),
            Support::Editable
        );
    }

    #[test]
    fn global_source_bumps_the_version_once_per_commit() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let before = settings.version.load(std::sync::atomic::Ordering::Acquire);
        let mut src = global(&settings, &algo);

        src.set(ParamId::Std(StandardField::Decay), 0.1);
        src.set(ParamId::Std(StandardField::Volume), 0.2);
        src.set(ParamId::Special(1), 0.3);
        assert_eq!(
            settings.version.load(std::sync::atomic::Ordering::Acquire),
            before,
            "writes must not bump the version per row"
        );

        src.commit();
        assert_eq!(
            settings.version.load(std::sync::atomic::Ordering::Acquire),
            before + 1,
            "one bump per frame"
        );
        src.commit();
        assert_eq!(
            settings.version.load(std::sync::atomic::Ordering::Acquire),
            before + 1,
            "a clean commit is a no-op"
        );
    }

    #[test]
    fn plock_source_falls_back_to_the_lane_then_overrides_then_reverts() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        {
            let mut lane = global(&settings, &algo);
            lane.set(ParamId::Std(StandardField::Decay), 0.30);
            lane.commit();
        }

        let mut src = PlockSource::new(&plock, STEP, global(&settings, &algo));
        let decay = ParamId::Std(StandardField::Decay);

        // No override yet: the row shows the lane value and is not accented.
        assert_eq!(src.get(decay), 0.30);
        assert!(!src.is_overridden(decay));
        assert_eq!(src.inherited(decay), 0.30);

        // Touching the row creates the p-lock AND the field override.
        src.set(decay, 0.80);
        assert!(src.is_overridden(decay));
        assert_eq!(src.get(decay), 0.80);
        assert!(
            plock.masks.is_active(SLOT, STEP),
            "the step's plock now exists"
        );

        // Only that field is overridden.
        assert!(!src.is_overridden(ParamId::Std(StandardField::Volume)));

        // The lane keeps moving underneath; the override does not follow it.
        {
            let mut lane = global(&settings, &algo);
            lane.set(decay, 0.35);
            lane.commit();
        }
        assert_eq!(src.get(decay), 0.80);
        assert_eq!(src.inherited(decay), 0.35, "revert target follows the lane");

        // Reverting drops the override but keeps the p-lock alive.
        src.clear(decay);
        assert!(!src.is_overridden(decay));
        assert_eq!(src.get(decay), 0.35);
        assert!(plock.masks.is_active(SLOT, STEP));
    }

    /// The bug this design removes by construction: the popup gated its write on
    /// `overridden`, so a switch row could never create its own override.
    #[test]
    fn plock_source_can_create_an_override_for_a_switch_parameter() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let mut src = PlockSource::new(&plock, STEP, global(&settings, &algo));
        let stereo = ParamId::Std(StandardField::Stereo);

        assert!(!src.is_overridden(stereo));
        src.set(stereo, 1.0);
        assert!(
            src.is_overridden(stereo),
            "a switch must be able to create its override"
        );
        assert_eq!(src.get(stereo), 1.0);
    }

    #[test]
    fn freq_mode_is_lane_wide_and_the_reserved_special_has_no_slot() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let mut src = PlockSource::new(&plock, STEP, global(&settings, &algo));

        // FreqMode: writing it in p-lock scope must move the LANE, not the step.
        src.set(ParamId::FreqMode, 1.0);
        assert_eq!(settings.instruments[SLOT].get(ParamId::FreqMode), 1.0);
        assert!(!src.is_overridden(ParamId::FreqMode));
        assert_eq!(
            src.supports(ParamId::FreqMode).reason(),
            ParamId::FreqMode.unlockable_reason()
        );

        // [187] Special index 4 is p-lockable again (re-homed off Attack's field),
        // and writing it must not touch Attack.
        let rehomed = ParamId::Special(4);
        assert!(rehomed.is_lockable());
        src.set(rehomed, 0.9);
        assert!(src.is_overridden(rehomed));
        assert_eq!(src.get(rehomed), 0.9);
        assert!(
            !src.is_overridden(ParamId::Std(StandardField::Attack)),
            "writing the re-homed special must not land on Attack"
        );

        // The reserved index lends its slot, so it has none: greyed, with a reason,
        // and a write is a no-op.
        let reserved = ParamId::Special(crate::param_id::RESERVED_SPECIAL_INDEX);
        assert!(!reserved.is_lockable());
        assert!(src.supports(reserved).reason().is_some());
        src.set(reserved, 0.5);
        assert!(!src.is_overridden(reserved));
    }

    // ── Morph ───────────────────────────────────────────────────────────────

    use crate::sequencer::pattern::{FusedGroup, MorphDirection};
    use crate::sequencer::{Pattern, SharedPattern};

    /// A 4-cell group emitting 4 pulses, so morphing is active.
    fn fused_group() -> FusedGroup {
        FusedGroup {
            start_cell: 0,
            end_cell: 3,
            step_count: 4,
            ..Default::default()
        }
    }

    fn morph<'a>(
        pattern: &'a SharedPattern,
        group: FusedGroup,
        end: MorphEnd,
        base: PlockSource<'a>,
    ) -> MorphSource<'a> {
        MorphSource::new(pattern, 0, group, end, base)
    }

    /// The 2x2 that makes one source serve both endpoints: which store a row
    /// writes depends on the tab AND on where the single stored value sits.
    #[test]
    fn the_endpoint_and_the_direction_decide_which_store_a_row_writes() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let pattern = SharedPattern::new(&Pattern::empty());
        let decay = ParamId::Std(StandardField::Decay);
        let field = decay.plock_field().unwrap();

        // Direction `Target` (the default): the stored value is the END.
        let group = fused_group();
        let mut end_tab = morph(
            &pattern,
            group,
            MorphEnd::End,
            PlockSource::new(&plock, 0, global(&settings, &algo)),
        );
        end_tab.set(decay, 0.9);
        assert!(
            end_tab.is_overridden(decay),
            "End tab writes the morph target"
        );
        assert!(
            !plock.field_masks.is_set(SLOT, 0, field),
            "...not the p-lock"
        );

        // On a field that ALREADY has a `Target` target, the start is the live
        // endpoint, so the Start tab writes the start cell's p-lock.
        let mut existing = fused_group();
        assert!(existing.set_morph_target(field, 0.9));
        let mut start_tab = morph(
            &pattern,
            existing,
            MorphEnd::Start,
            PlockSource::new(&plock, 0, global(&settings, &algo)),
        );
        start_tab.set(decay, 0.2);
        assert!(
            plock.field_masks.is_set(SLOT, 0, field),
            "with a Target target, Start edits the live endpoint"
        );

        // Direction `Source`: the stored value is the START, so the tabs swap.
        let mut inverted = fused_group();
        assert!(inverted.set_morph_target(field, 0.5));
        inverted.set_morph_target_direction(field, MorphDirection::Source);
        let plock2 = PlockState::new();
        let mut start_tab = morph(
            &pattern,
            inverted,
            MorphEnd::Start,
            PlockSource::new(&plock2, 0, global(&settings, &algo)),
        );
        start_tab.set(decay, 0.7);
        assert!(
            !plock2.field_masks.is_set(SLOT, 0, field),
            "with an inverted target, Start edits the stored value"
        );
        let mut end_tab = morph(
            &pattern,
            inverted,
            MorphEnd::End,
            PlockSource::new(&plock2, 0, global(&settings, &algo)),
        );
        end_tab.set(decay, 0.7);
        assert!(
            plock2.field_masks.is_set(SLOT, 0, field),
            "...and End edits the live endpoint, i.e. the p-lock"
        );
    }

    /// The first drag in the End tab is what CREATES the morph, and the row starts
    /// from the value the sound already has rather than from zero.
    #[test]
    fn a_first_drag_in_the_end_tab_creates_the_target_from_the_live_value() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let pattern = SharedPattern::new(&Pattern::empty());
        {
            let mut lane = global(&settings, &algo);
            lane.set(ParamId::Std(StandardField::Decay), 0.33);
            lane.commit();
        }
        let mut src = morph(
            &pattern,
            fused_group(),
            MorphEnd::End,
            PlockSource::new(&plock, 0, global(&settings, &algo)),
        );
        let decay = ParamId::Std(StandardField::Decay);
        assert!(!src.is_overridden(decay));
        assert_eq!(src.get(decay), 0.33, "shows the live value before any drag");
        src.set(decay, 0.8);
        assert!(src.is_overridden(decay));
        assert_eq!(src.get(decay), 0.8);
        src.clear(decay);
        assert!(!src.is_overridden(decay), "the target can be removed again");
    }

    /// Setting an endpoint must always produce an audible ramp, whichever tab is
    /// used. Before this, the Start tab wrote the start cell's p-lock, so with no
    /// End value the step simply played that value flat on every pulse and nothing
    /// morphed — which is exactly how it was reported.
    #[test]
    fn either_tab_creates_a_morph_that_ramps_toward_the_live_value() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let pattern = SharedPattern::new(&Pattern::empty());
        let decay = ParamId::Std(StandardField::Decay);
        let field = decay.plock_field().unwrap();
        use crate::sequencer::pattern::MorphDirection;

        for (tab, expected_direction) in [
            (MorphEnd::End, MorphDirection::Target),
            (MorphEnd::Start, MorphDirection::Source),
        ] {
            let plock = PlockState::new();
            let mut src = morph(
                &pattern,
                fused_group(),
                tab,
                PlockSource::new(&plock, 0, global(&settings, &algo)),
            );
            src.set(decay, 0.66);

            assert!(src.is_overridden(decay), "{tab:?} must create the morph");
            assert_eq!(src.get(decay), 0.66, "{tab:?} shows what was just set");
            assert_eq!(
                src.group.morph_target_direction(field),
                expected_direction,
                "{tab:?} stores its value on the right side of the ramp"
            );
            assert!(
                !plock.field_masks.is_set(SLOT, 0, field),
                "{tab:?} must not pin the live endpoint, or there is nothing to ramp to"
            );
        }
    }

    /// The packed format holds 4 targets. The old code dropped the fifth silently;
    /// now the row is greyed with a reason and the panel carries a notice.
    #[test]
    fn the_fifth_morph_target_is_refused_visibly() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let pattern = SharedPattern::new(&Pattern::empty());
        let mut src = morph(
            &pattern,
            fused_group(),
            MorphEnd::End,
            PlockSource::new(&plock, 0, global(&settings, &algo)),
        );
        let four = [
            ParamId::Std(StandardField::Decay),
            ParamId::Std(StandardField::Volume),
            ParamId::Std(StandardField::Hold),
            ParamId::Std(StandardField::Analog),
        ];
        for id in four {
            src.set(id, 0.5);
            assert!(src.is_overridden(id));
        }
        assert_eq!(src.capacity_left(), 0);
        assert!(
            src.notice().is_some(),
            "a full group explains itself as soon as it is full"
        );

        let fifth = ParamId::Std(StandardField::FilterFreq);
        assert!(
            src.supports(fifth).reason().is_some(),
            "a fifth field must be greyed, not silently ignored"
        );
        src.set(fifth, 900.0);
        assert!(!src.is_overridden(fifth), "and the write is refused");
        for id in four {
            assert!(src.is_overridden(id), "the four that fit are untouched");
        }
    }

    /// Morph interpolates, so a discrete parameter has no business being a target.
    #[test]
    fn a_discrete_parameter_cannot_be_morphed() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let pattern = SharedPattern::new(&Pattern::empty());
        let src = morph(
            &pattern,
            fused_group(),
            MorphEnd::End,
            PlockSource::new(&plock, 0, global(&settings, &algo)),
        );
        // Kick special 1 is the saturation TYPE: `sp_discrete`, so not continuous.
        let discrete = ParamId::Special(1);
        assert!(!crate::instrument_registry::param_is_morphable(
            VOICE, discrete
        ));
        assert!(src.supports(discrete).reason().is_some());
        assert_eq!(
            src.supports(ParamId::Std(StandardField::Decay)),
            Support::Editable
        );
    }

    /// The old morph menu re-published the lane's whole fusion array on every
    /// slider frame. One publish per frame, and only when something changed.
    #[test]
    fn commit_publishes_once_and_preserves_the_other_groups() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let pattern = SharedPattern::new(&Pattern::empty());

        let second = FusedGroup {
            start_cell: 8,
            end_cell: 11,
            step_count: 2,
            ..Default::default()
        };
        pattern.store_fusions(SLOT, &[fused_group(), second]);

        let mut src = MorphSource::new(
            &pattern,
            0,
            fused_group(),
            MorphEnd::End,
            PlockSource::new(&plock, 0, global(&settings, &algo)),
        );
        src.commit();
        assert_eq!(
            pattern.load_fusions(SLOT).len(),
            2,
            "a clean commit must not touch the pattern"
        );

        src.set(ParamId::Std(StandardField::Decay), 0.75);
        src.commit();
        let groups = pattern.load_fusions(SLOT);
        assert_eq!(groups.len(), 2, "the other group survives the publish");
        assert_eq!(groups[1].start_cell, 8);
        assert!(
            groups[0].morph_active(),
            "the edited group carries its target"
        );
        assert_eq!(groups[0].morph_count, 1);
    }

    #[test]
    fn salts_differ_between_scopes_and_steps() {
        let layout = TrackLayoutState::default_layout();
        let settings = SoundSettingsState::new(&layout);
        let algo = CellAlgo(Cell::new(0));
        let plock = PlockState::new();
        let lane = global(&settings, &algo);
        let lane_salt = lane.salt();
        let step_a = PlockSource::new(&plock, 1, global(&settings, &algo)).salt();
        let step_b = PlockSource::new(&plock, 2, global(&settings, &algo)).salt();
        assert_ne!(lane_salt, step_a);
        assert_ne!(step_a, step_b);
    }
}
