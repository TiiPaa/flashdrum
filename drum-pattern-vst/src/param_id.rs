//! [184] The canonical identity of an editable per-slot sound parameter.
//!
//! This module owns the **only** conversion between a parameter and its
//! positional `plock-v1` / morph-target field index. Nothing else may hardcode
//! `0..=45`.
//!
//! # Why this exists
//!
//! The same `StandardField` mapping used to be hand-written **six** times and
//! had to stay in sync by discipline alone:
//!
//! | Where | Direction |
//! |---|---|
//! | `instrument_registry::plock_field_index` | `StandardField` → p-lock field |
//! | `ui/plock.rs` `get_global_value` | `StandardField` → the `load()` tuple |
//! | `ui/plock.rs` morph reset values | the same, again |
//! | `ui/grid.rs` `current_field_value_for_fusion` | p-lock field → the `load()` tuple |
//! | `lib.rs` `read_morph_value` / `apply_morph_value` | p-lock field → `VoiceSettings` |
//! | `ui/preset_browser.rs` `ORDER` | `StandardField` order for presets |
//!
//! # The two numbering schemes
//!
//! - The **`StandardField` discriminant** (0..12) is the order of
//!   `InstrumentSettingsState::load()`, of `sound_settings_default`, and of the
//!   `sound-settings-v2` blob. **Attack is 4.**
//! - The **p-lock field index** is the order of the `plock-v1` blob, where
//!   Attack was appended late as field **18** and specials live at
//!   `SPECIAL_FIELD_START + special_index`. **Attack is 18.**
//!
//! Confusing the two silently shifts every value, which is exactly what this
//! module makes impossible.
//!
//! # The Attack / special-4 collision, and how it was repaired
//!
//! Attack landed on field 18 == `SPECIAL_FIELD_START + 4`, so the special of
//! index 4 shared its slot and was silently ignored on every voice - the
//! Kick's Saturation Output Gain, SDrex's Wet, Buzz's Noise Type...
//!
//! [187] re-homed it on field **45**, the slot of special index
//! [`RESERVED_SPECIAL_INDEX`], which **no voice declares** (the highest index
//! across the 18 instruments is 17). The blob length is unchanged, so there is
//! no new persistence version - and field 12 stays untouched for the Clap's own
//! legacy echo fallback, which is precisely why that slot was NOT the one
//! chosen.
//!
//! The one hazard was old full snapshots: they set all 46 mask bits, field 45
//! included, with the value `0.0`. See [`sanitize_field_mask`].

use crate::instrument_registry::StandardField;

// ── The `plock-v1` field layout ─────────────────────────────────────────────
// These constants live here, in the module that owns the layout, and are
// re-exported by `plock.rs` so every existing `crate::plock::X` path keeps
// working. The dependency runs identity -> storage, never the other way, which
// is what lets the headless `test_standalone` binary include this module
// without dragging in the persistence layer.

/// 13 standard + 1 algo + 32 special + 2 sends ([286]).
pub const FIELD_COUNT: usize = 48;
/// The field count before the sends existed: `plock-v1` blobs, pattern-bank slots
/// and presets written by earlier builds use this stride. Frozen.
pub const PRE_SEND_FIELD_COUNT: usize = 46;
/// [286] The send levels of a lane to Fx1 and Fx2, appended after the specials.
pub const SEND_FIELD_START: usize = 46;
pub const SEND_FIELD_COUNT: usize = 2;
/// The per-slot synthesis algorithm.
pub const ALGO_FIELD: usize = 13;
/// Where the 32 special fields start.
pub const SPECIAL_FIELD_START: usize = 14;
pub const SPECIAL_FIELD_COUNT: usize = 32;
/// Attack was appended late, landing on `SPECIAL_FIELD_START + 4`.
pub const ATTACK_FIELD: usize = 18;
/// Dead storage: only the Clap still reads it, as a legacy fallback.
pub const LEGACY_CLAP_ECHO_FIELD: usize = 12;

/// Where the special that Attack shadowed is re-homed ([187]).
///
/// Attack sits on field 18 == `SPECIAL_FIELD_START + 4`, so special index 4 had
/// nowhere to live and was simply dropped. It now uses the slot of special index
/// **31**, which **no voice declares** — the highest index across all 18
/// instruments is 17 — so nothing is displaced and the dead legacy clap-echo
/// field 12 stays untouched for the Clap's own fallback.
pub const SPECIAL_4_FIELD: usize = SPECIAL_FIELD_START + RESERVED_SPECIAL_INDEX;
/// Reserved: its slot is lent to special index 4. Never declare it in the registry.
pub const RESERVED_SPECIAL_INDEX: usize = 31;

/// Every field a parameter can own. The only hole is field 12, the dead legacy
/// clap-echo slot, which is not a parameter.
///
/// The SOUND fields only: a snapshot never claims the send fields ([286]), which are
/// not part of `VoiceSettings`.
pub const ADDRESSABLE_MASK: u64 =
    ((1u64 << PRE_SEND_FIELD_COUNT) - 1) & !(1u64 << LEGACY_CLAP_ECHO_FIELD);
/// What `set_all()` used to write: all 46 bits, field 12 included. Because a
/// snapshot taken since [187] writes [`ADDRESSABLE_MASK`] instead, this exact
/// value now identifies a mask written by an **older build** — which is what lets
/// [`sanitize_field_mask`] refuse to trust its field 45.
pub const LEGACY_ALL_BITS: u64 = (1u64 << PRE_SEND_FIELD_COUNT) - 1;

/// Repair a raw field mask coming from persistence, a pattern-bank slot, a preset
/// or the clipboard.
///
/// An old full snapshot has all 46 bits set, field 45 included, with the value
/// `0.0` written there by the old `set_settings` (it stored `special[31]`, which
/// no voice uses). Reading that as the re-homed special index 4 would silently
/// zero, say, the Kick's Saturation Output Gain on every snapshot step. So for
/// that one mask value — and only that one — bit 45 is cleared, which reproduces
/// the old behaviour exactly: the parameter simply follows the lane.
pub const fn sanitize_field_mask(mask: u64) -> u64 {
    if mask == LEGACY_ALL_BITS {
        ADDRESSABLE_MASK & !(1u64 << SPECIAL_4_FIELD)
    } else {
        mask
    }
}

const _: () = assert!(ATTACK_FIELD == SPECIAL_FIELD_START + 4);
const _: () = assert!(PRE_SEND_FIELD_COUNT == SPECIAL_FIELD_START + SPECIAL_FIELD_COUNT);
const _: () = assert!(FIELD_COUNT == PRE_SEND_FIELD_COUNT + SEND_FIELD_COUNT);
const _: () = assert!(SEND_FIELD_START == PRE_SEND_FIELD_COUNT);
const _: () = assert!(SPECIAL_4_FIELD == PRE_SEND_FIELD_COUNT - 1);
const _: () = assert!(SPECIAL_4_FIELD != ATTACK_FIELD);

/// Identity of one editable per-slot parameter, independent of which store
/// holds its value.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ParamId {
    /// One of the 13 named f32 fields of a slot's sound settings.
    Std(StandardField),
    /// The per-slot synthesis algorithm. A nih-plug `IntParam` when lane-global,
    /// p-lock field 13 when per-step — the only parameter with two stores.
    Algo,
    /// A registry `special_index` (0..32).
    Special(usize),
    /// [286] The send level of a lane to effect bus `0` (Fx1) or `1` (Fx2). Lane-global it
    /// lives in the track layout, per step in p-lock fields 46 / 47; not a sound value,
    /// so it is never morphed and no voice reads it.
    FxSend(usize),
    /// The per-slot Hz/Note display flag. Never p-locked and never morphed: it
    /// is a display preference, not a sound value, so every scope routes it to
    /// the lane's own state.
    FreqMode,
}

impl ParamId {
    /// Index in the 46-field layout shared by `plock-v1` and `MorphTarget`.
    /// `None` when the parameter has no per-step storage of its own.
    pub const fn plock_field(self) -> Option<usize> {
        match self {
            ParamId::Std(field) => Some(field.plock_field_index()),
            ParamId::Algo => Some(ALGO_FIELD),
            // Index 4 is re-homed on the reserved slot; index 31 lends it and is
            // therefore not addressable itself ([187]).
            ParamId::Special(4) => Some(SPECIAL_4_FIELD),
            ParamId::Special(RESERVED_SPECIAL_INDEX) => None,
            ParamId::Special(index) => {
                if index < SPECIAL_FIELD_COUNT {
                    Some(SPECIAL_FIELD_START + index)
                } else {
                    None
                }
            }
            ParamId::FxSend(bus) => {
                if bus < SEND_FIELD_COUNT {
                    Some(SEND_FIELD_START + bus)
                } else {
                    None
                }
            }
            ParamId::FreqMode => None,
        }
    }

    /// The inverse of [`ParamId::plock_field`].
    ///
    /// Field 18 resolves to `Std(Attack)`, not `Special(4)` — that is the
    /// documented winner of the collision, matching `PlockState::get_settings`.
    /// Field 12 (the dead legacy clap-echo slot) belongs to no parameter.
    pub fn from_plock_field(field: usize) -> Option<Self> {
        if field == StandardField::Attack.plock_field_index() {
            return Some(ParamId::Std(StandardField::Attack));
        }
        if field == ALGO_FIELD {
            return Some(ParamId::Algo);
        }
        if field == SPECIAL_4_FIELD {
            return Some(ParamId::Special(4));
        }
        if let Some(std_field) = StandardField::ALL
            .iter()
            .copied()
            .find(|f| f.plock_field_index() == field)
        {
            return Some(ParamId::Std(std_field));
        }
        if field >= SEND_FIELD_START && field < FIELD_COUNT {
            return Some(ParamId::FxSend(field - SEND_FIELD_START));
        }
        if field >= SPECIAL_FIELD_START && field < SPECIAL_FIELD_START + SPECIAL_FIELD_COUNT {
            return Some(ParamId::Special(field - SPECIAL_FIELD_START));
        }
        None
    }

    /// `false` when this parameter cannot own a per-step override.
    ///
    /// Two cases: [`ParamId::FreqMode`] (a display mode), and the special that
    /// shares its field with Attack. A UI must **grey such a row with its
    /// reason**, never hide it — hiding is what makes today's behaviour
    /// inexplicable to the user.
    pub const fn is_lockable(self) -> bool {
        self.plock_field().is_some()
    }

    /// Why this parameter has no per-step slot, for the greyed row's tooltip.
    pub const fn unlockable_reason(self) -> Option<&'static str> {
        match self {
            ParamId::FreqMode => Some("Display mode: applies to the whole lane"),
            ParamId::Special(RESERVED_SPECIAL_INDEX) => Some(
                "Reserved slot: no instrument declares this parameter, and its storage is lent to another one",
            ),
            ParamId::Special(_) if !self.is_lockable() => {
                Some("Cannot be locked per step: no slot in the p-lock storage format")
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table the six hand-written mappings agreed on, frozen so their removal
    /// is provably behaviour-preserving. Special index 4 is the one deliberate
    /// change ([187]): it moved from "nowhere" to the reserved slot.
    #[test]
    fn plock_field_mapping_is_pinned() {
        let expect = [
            (StandardField::Freq, 0usize),
            (StandardField::Decay, 1),
            (StandardField::Volume, 2),
            (StandardField::FilterFreq, 3),
            (StandardField::Release, 4),
            (StandardField::DecayCurve, 5),
            (StandardField::ReleaseCurve, 6),
            (StandardField::Hold, 7),
            (StandardField::FilterEnvAmount, 8),
            (StandardField::FilterEnvDecay, 9),
            (StandardField::Analog, 10),
            (StandardField::Stereo, 11),
            (StandardField::Attack, 18),
        ];
        assert_eq!(expect.len(), StandardField::ALL.len());
        for (field, index) in expect {
            assert_eq!(ParamId::Std(field).plock_field(), Some(index), "{field:?}");
        }
        assert_eq!(ParamId::Algo.plock_field(), Some(13));
        assert_eq!(ParamId::FreqMode.plock_field(), None);
        for index in 0..SPECIAL_FIELD_COUNT {
            let expected = match index {
                // Attack owns field 18, so index 4 borrows the reserved slot...
                4 => Some(SPECIAL_4_FIELD),
                // ...which is why index 31 has none of its own.
                RESERVED_SPECIAL_INDEX => None,
                _ => Some(SPECIAL_FIELD_START + index),
            };
            assert_eq!(
                ParamId::Special(index).plock_field(),
                expected,
                "special {index}"
            );
        }
        assert_eq!(ParamId::Special(SPECIAL_FIELD_COUNT).plock_field(), None);
        assert_ne!(
            ParamId::Special(4).plock_field(),
            ParamId::Std(StandardField::Attack).plock_field(),
            "special 4 must no longer collide with Attack"
        );
    }

    /// [286] The send fields come right after the specials and never alias one.
    #[test]
    fn the_send_fields_follow_the_specials() {
        assert_eq!(ParamId::FxSend(0).plock_field(), Some(46));
        assert_eq!(ParamId::FxSend(1).plock_field(), Some(47));
        assert_eq!(ParamId::FxSend(2).plock_field(), None);
        assert_eq!(ParamId::from_plock_field(46), Some(ParamId::FxSend(0)));
        assert_eq!(ParamId::from_plock_field(47), Some(ParamId::FxSend(1)));
        assert_eq!(ParamId::from_plock_field(48), None);
        // A snapshot (and the legacy all-bits mask) never claims a send.
        assert_eq!(ADDRESSABLE_MASK >> SEND_FIELD_START, 0);
        assert_eq!(LEGACY_ALL_BITS >> SEND_FIELD_START, 0);
    }

    #[test]
    fn from_plock_field_round_trips_every_id() {
        for field in StandardField::ALL {
            let id = ParamId::Std(field);
            let index = id.plock_field().expect("standard fields are addressable");
            assert_eq!(ParamId::from_plock_field(index), Some(id), "{field:?}");
        }
        assert_eq!(ParamId::from_plock_field(ALGO_FIELD), Some(ParamId::Algo));
        for index in 0..SPECIAL_FIELD_COUNT {
            let Some(field) = ParamId::Special(index).plock_field() else {
                assert_eq!(index, RESERVED_SPECIAL_INDEX);
                continue;
            };
            assert_eq!(
                ParamId::from_plock_field(field),
                Some(ParamId::Special(index)),
                "special {index} -> field {field}"
            );
        }
        // Field 18 belongs to Attack; special 4 lives on the reserved slot.
        assert_eq!(
            ParamId::from_plock_field(ATTACK_FIELD),
            Some(ParamId::Std(StandardField::Attack))
        );
        assert_eq!(
            ParamId::from_plock_field(SPECIAL_4_FIELD),
            Some(ParamId::Special(4))
        );
        // Field 12 is the dead legacy clap-echo slot: no parameter claims it.
        assert_eq!(ParamId::from_plock_field(12), None);
        assert_eq!(ParamId::from_plock_field(FIELD_COUNT), None);
    }

    /// Every addressable field is claimed by exactly one id, and every id maps
    /// to a distinct field. A bijection with exactly one hole (field 12).
    #[test]
    fn the_field_layout_is_a_bijection_with_one_documented_hole() {
        let mut owner: Vec<Option<ParamId>> = vec![None; FIELD_COUNT];
        let ids = StandardField::ALL
            .iter()
            .map(|f| ParamId::Std(*f))
            .chain(std::iter::once(ParamId::Algo))
            .chain((0..SPECIAL_FIELD_COUNT).map(ParamId::Special))
            .chain((0..SEND_FIELD_COUNT).map(ParamId::FxSend));
        for id in ids {
            if !id.is_lockable() {
                continue;
            }
            let field = id.plock_field().expect("lockable ids are addressable");
            assert!(
                owner[field].is_none(),
                "field {field} claimed twice: {:?} and {id:?}",
                owner[field]
            );
            owner[field] = Some(id);
        }
        let unclaimed: Vec<usize> = (0..FIELD_COUNT).filter(|f| owner[*f].is_none()).collect();
        assert_eq!(
            unclaimed,
            vec![LEGACY_CLAP_ECHO_FIELD],
            "only the legacy clap-echo field may be unclaimed - it belongs to the Clap's own fallback, not to a parameter"
        );
    }

    /// Since [187] the only special without a slot is the reserved one, which no
    /// instrument declares — so in practice every declared parameter is lockable.
    #[test]
    fn only_the_reserved_special_is_unlockable() {
        let unlockable: Vec<usize> = (0..SPECIAL_FIELD_COUNT)
            .filter(|i| !ParamId::Special(*i).is_lockable())
            .collect();
        assert_eq!(unlockable, vec![RESERVED_SPECIAL_INDEX]);
        assert!(
            ParamId::Special(4).is_lockable(),
            "the special Attack used to shadow is lockable again"
        );
        assert!(ParamId::Std(StandardField::Attack).is_lockable());
        assert!(ParamId::Algo.is_lockable());
        assert!(!ParamId::FreqMode.is_lockable());
        for id in [ParamId::FreqMode, ParamId::Special(RESERVED_SPECIAL_INDEX)] {
            assert!(id.unlockable_reason().is_some(), "{id:?} needs a reason");
        }
        assert!(ParamId::Std(StandardField::Freq)
            .unlockable_reason()
            .is_none());
    }

    /// An old full snapshot wrote all 46 mask bits, field 45 included, with the
    /// value 0.0 (it stored `special[31]`, which no voice uses). Trusting that
    /// would zero the re-homed special index 4 — the Kick's Saturation Output
    /// Gain, for instance — on every snapshot step.
    #[test]
    fn a_legacy_full_snapshot_mask_does_not_claim_the_rehomed_special() {
        let repaired = sanitize_field_mask(LEGACY_ALL_BITS);
        assert_eq!(
            repaired & (1u64 << SPECIAL_4_FIELD),
            0,
            "field 45 not trusted"
        );
        assert_eq!(
            repaired & (1u64 << LEGACY_CLAP_ECHO_FIELD),
            0,
            "field 12 is not a param"
        );
        // Everything else the old snapshot claimed is still claimed.
        for field in 0..PRE_SEND_FIELD_COUNT {
            if field == SPECIAL_4_FIELD || field == LEGACY_CLAP_ECHO_FIELD {
                continue;
            }
            assert_ne!(repaired & (1u64 << field), 0, "field {field} must survive");
        }
    }

    /// A snapshot taken since [187] marks the addressable fields, which is a
    /// DIFFERENT value from the legacy all-bits mask — that difference is the
    /// version marker, so it must not be sanitised away.
    #[test]
    fn a_current_snapshot_mask_is_left_alone() {
        assert_ne!(ADDRESSABLE_MASK, LEGACY_ALL_BITS);
        assert_eq!(sanitize_field_mask(ADDRESSABLE_MASK), ADDRESSABLE_MASK);
        assert_ne!(ADDRESSABLE_MASK & (1u64 << SPECIAL_4_FIELD), 0);
        // An ordinary partial mask passes through untouched.
        let partial = (1u64 << 1) | (1u64 << SPECIAL_4_FIELD);
        assert_eq!(sanitize_field_mask(partial), partial);
    }
}
