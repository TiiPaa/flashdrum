//! Modular track model.
//!
//! The plugin exposes a fixed internal pool of `MAX_TRACKS` slots. Only the
//! active slots are visible in the UI. Each active slot holds an instrument
//! kind, its own sound settings, routing, MIDI note and pattern data.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::Arc;

pub const MAX_TRACKS: usize = 14;

/// [286] The last slots are effect buses (Fx1, Fx2): they take effect kinds only,
/// never an instrument, and instruments send to them.
pub const FX_SLOT_COUNT: usize = 2;
/// First effect slot (12): Fx1 = slot 12, Fx2 = slot 13.
pub const FX_FIRST_SLOT: usize = MAX_TRACKS - FX_SLOT_COUNT;

/// Whether `slot` is one of the effect-bus slots.
pub const fn is_fx_slot(slot: usize) -> bool {
    slot >= FX_FIRST_SLOT && slot < MAX_TRACKS
}

/// Can `kind` be put on `slot`? Effects only on the effect-bus slots, instruments
/// only elsewhere. `existing` is the kind the slot already holds: an instrument
/// that was there before the buses existed (sessions saved with 13 or 14
/// lanes) may stay and be replaced by another instrument.
pub fn kind_allowed_on_slot(
    kind: TrackInstrumentKind,
    slot: usize,
    existing: Option<TrackInstrumentKind>,
) -> bool {
    if kind.is_effect() {
        is_fx_slot(slot)
    } else {
        !is_fx_slot(slot) || existing.is_some_and(|k| !k.is_effect())
    }
}

/// Functional instrument kind. 11 fixed types; the three legacy Tom voices
/// collapse into a single `Tom` kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrackInstrumentKind {
    Kick = 0,
    Snare = 1,
    HiHat = 2,
    OpenHiHat = 3,
    Tom = 4,
    Clap = 5,
    Ride = 6,
    Cymbal = 7,
    Snare606 = 8,
    BassDrum808 = 9,
    Perc1 = 10,
    Bd6smp = 11,
    Sd6smp = 12,
    Ch6smp = 13,
    Buzz = 14,
    Sdrex = 15,
    Bd6Ac = 16,
    Sd6Ac = 17,
    Hh6Ac = 18,
    Oh6Ac = 19,
    Cl6Ac = 20,
    Tm6Ac = 21,
    /// [208] TR-606 open hi-hat sampler - the CH6smp engine on the open bank.
    Oh6smp = 22,
    /// [221] Rift - a slice lifted out of a long embedded texture.
    Rift = 23,
    /// [243] One-Shot - the lane's own sample file, played start to finish.
    OneShot = 24,
    /// [286] Reverb - a send effect living on an effect-bus slot (12 / 13).
    Reverb = 25,
    /// [286] Delay - the second send effect (an effect-bus slot, 12 / 13).
    Delay = 26,
    /// [286] Rumble - the third send effect: the kick rumble (an effect-bus slot, 12 / 13).
    Rumble = 27,
}

/// Instrument category used to group the kind pickers/menus
/// (BD, SD, HH, PERC, FX, OTHER).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstrumentCategory {
    BassDrum,
    Snare,
    HiHat,
    Perc,
    Fx,
    Other,
    /// [286] Send effects (reverb): offered on the effect-bus slots only.
    Effect,
}

impl InstrumentCategory {
    pub const ALL: [Self; 7] = [
        Self::BassDrum,
        Self::Snare,
        Self::HiHat,
        Self::Perc,
        Self::Fx,
        Self::Other,
        Self::Effect,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::BassDrum => "BD",
            Self::Snare => "SD",
            Self::HiHat => "HH",
            Self::Perc => "PERC",
            Self::Fx => "FX",
            Self::Other => "OTHER",
            Self::Effect => "EFFECT",
        }
    }
}

impl TrackInstrumentKind {
    pub const COUNT: usize = 28;

    /// Every kind, in stable declaration order.
    pub const ALL: [Self; Self::COUNT] = [
        Self::Kick,
        Self::Snare,
        Self::HiHat,
        Self::OpenHiHat,
        Self::Tom,
        Self::Clap,
        Self::Ride,
        Self::Cymbal,
        Self::Snare606,
        Self::BassDrum808,
        Self::Perc1,
        Self::Bd6smp,
        Self::Sd6smp,
        Self::Ch6smp,
        Self::Buzz,
        Self::Sdrex,
        Self::Bd6Ac,
        Self::Sd6Ac,
        Self::Hh6Ac,
        Self::Oh6Ac,
        Self::Cl6Ac,
        Self::Tm6Ac,
        Self::Oh6smp,
        Self::Rift,
        Self::OneShot,
        Self::Reverb,
        Self::Delay,
        Self::Rumble,
    ];

    /// Musical family of this kind (grouping for pickers/menus).
    pub fn category(self) -> InstrumentCategory {
        match self {
            Self::Kick | Self::BassDrum808 | Self::Bd6smp | Self::Bd6Ac => {
                InstrumentCategory::BassDrum
            }
            Self::Snare
            | Self::Snare606
            | Self::Sd6smp
            | Self::Clap
            | Self::Sdrex
            | Self::Sd6Ac
            | Self::Cl6Ac => InstrumentCategory::Snare,
            Self::HiHat
            | Self::OpenHiHat
            | Self::Ch6smp
            | Self::Oh6smp
            | Self::Hh6Ac
            | Self::Oh6Ac => InstrumentCategory::HiHat,
            Self::Tom | Self::Perc1 | Self::Tm6Ac | Self::OneShot => InstrumentCategory::Perc,
            Self::Buzz | Self::Rift => InstrumentCategory::Fx,
            Self::Ride | Self::Cymbal => InstrumentCategory::Other,
            Self::Reverb | Self::Delay | Self::Rumble => InstrumentCategory::Effect,
        }
    }

    /// Kinds retired from the pickers, and what a saved lane becomes instead
    /// ([203] Snare606, [204] OpenHiHat).
    ///
    /// The enum variants and their DSP voices **stay**: `index()` is `self as
    /// usize` and it is persisted in sessions and in presets, so deleting a
    /// variant would renumber every kind after it and silently turn every saved
    /// lane into a different instrument. They simply can no longer be chosen,
    /// and a session that still holds one is rewritten on load.
    pub fn retired_replacement(self) -> Option<Self> {
        match self {
            // [203] Superseded by the analogcode snare.
            Self::Snare606 => Some(Self::Sd6Ac),
            // [204] The closed hat is enough; an open hat is a hat with a long
            // decay, and its lane keeps that decay through the migration.
            Self::OpenHiHat => Some(Self::HiHat),
            _ => None,
        }
    }

    /// Can this kind still be picked in the instrument menus?
    pub fn selectable(self) -> bool {
        self.retired_replacement().is_none()
    }

    /// Kinds of one category, in `ALL` order. Retired kinds are left out, so
    /// every picker drops them at once.
    pub fn kinds_in(category: InstrumentCategory) -> impl Iterator<Item = Self> {
        Self::ALL
            .into_iter()
            .filter(move |k| k.selectable() && k.category() == category)
    }

    pub fn from_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Kick),
            1 => Some(Self::Snare),
            2 => Some(Self::HiHat),
            3 => Some(Self::OpenHiHat),
            4 => Some(Self::Tom),
            5 => Some(Self::Clap),
            6 => Some(Self::Ride),
            7 => Some(Self::Cymbal),
            8 => Some(Self::Snare606),
            9 => Some(Self::BassDrum808),
            10 => Some(Self::Perc1),
            11 => Some(Self::Bd6smp),
            12 => Some(Self::Sd6smp),
            13 => Some(Self::Ch6smp),
            14 => Some(Self::Buzz),
            15 => Some(Self::Sdrex),
            16 => Some(Self::Bd6Ac),
            17 => Some(Self::Sd6Ac),
            18 => Some(Self::Hh6Ac),
            19 => Some(Self::Oh6Ac),
            20 => Some(Self::Cl6Ac),
            21 => Some(Self::Tm6Ac),
            22 => Some(Self::Oh6smp),
            23 => Some(Self::Rift),
            24 => Some(Self::OneShot),
            25 => Some(Self::Reverb),
            26 => Some(Self::Delay),
            27 => Some(Self::Rumble),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn default_label(self) -> &'static str {
        match self {
            TrackInstrumentKind::Kick => "BD",
            TrackInstrumentKind::Snare => "SD",
            TrackInstrumentKind::HiHat => "HH",
            TrackInstrumentKind::OpenHiHat => "OH",
            TrackInstrumentKind::Tom => "TM",
            TrackInstrumentKind::Clap => "CL",
            TrackInstrumentKind::Ride => "RD",
            TrackInstrumentKind::Cymbal => "CY",
            TrackInstrumentKind::Snare606 => "S6",
            TrackInstrumentKind::BassDrum808 => "B8",
            TrackInstrumentKind::Perc1 => "P1",
            TrackInstrumentKind::Bd6smp => "B6",
            TrackInstrumentKind::Sd6smp => "s6",
            TrackInstrumentKind::Ch6smp => "c6",
            TrackInstrumentKind::Buzz => "Bz",
            TrackInstrumentKind::Sdrex => "Sx",
            TrackInstrumentKind::Bd6Ac => "BA",
            TrackInstrumentKind::Sd6Ac => "SA",
            TrackInstrumentKind::Hh6Ac => "HA",
            TrackInstrumentKind::Oh6Ac => "OA",
            TrackInstrumentKind::Cl6Ac => "CA",
            TrackInstrumentKind::Tm6Ac => "TA",
            TrackInstrumentKind::Oh6smp => "o6",
            TrackInstrumentKind::Rift => "Rf",
            TrackInstrumentKind::OneShot => "OS",
            TrackInstrumentKind::Reverb => "RV",
            TrackInstrumentKind::Delay => "DL",
            TrackInstrumentKind::Rumble => "RU",
        }
    }

    pub fn default_name(self) -> &'static str {
        match self {
            TrackInstrumentKind::Kick => "Kick",
            TrackInstrumentKind::Snare => "Snare",
            TrackInstrumentKind::HiHat => "Hi-Hat",
            TrackInstrumentKind::OpenHiHat => "Open Hi-Hat",
            TrackInstrumentKind::Tom => "Tom",
            TrackInstrumentKind::Clap => "Clap",
            TrackInstrumentKind::Ride => "Ride",
            TrackInstrumentKind::Cymbal => "Cymbal",
            TrackInstrumentKind::Snare606 => "Snare 606",
            TrackInstrumentKind::BassDrum808 => "808 Kick",
            TrackInstrumentKind::Perc1 => "Perc1",
            TrackInstrumentKind::Bd6smp => "BD6smp",
            TrackInstrumentKind::Sd6smp => "SD6smp",
            TrackInstrumentKind::Ch6smp => "CH6smp",
            TrackInstrumentKind::Buzz => "Buzz",
            TrackInstrumentKind::Sdrex => "SDrex",
            TrackInstrumentKind::Bd6Ac => "BD6(AC)",
            TrackInstrumentKind::Sd6Ac => "SD6(AC)",
            TrackInstrumentKind::Hh6Ac => "HH6(AC)",
            TrackInstrumentKind::Oh6Ac => "OH6(AC)",
            TrackInstrumentKind::Cl6Ac => "CL6(AC)",
            TrackInstrumentKind::Tm6Ac => "TM6(AC)",
            TrackInstrumentKind::Oh6smp => "OH6smp",
            TrackInstrumentKind::Rift => "Rift",
            TrackInstrumentKind::OneShot => "One-Shot",
            TrackInstrumentKind::Reverb => "Reverb",
            TrackInstrumentKind::Delay => "Delay",
            TrackInstrumentKind::Rumble => "Rumble",
        }
    }

    /// Default GM drum note for this kind.
    pub fn default_midi_note(self) -> u8 {
        match self {
            TrackInstrumentKind::Kick => 36,
            TrackInstrumentKind::Snare => 38,
            TrackInstrumentKind::HiHat => 42,
            TrackInstrumentKind::OpenHiHat => 46,
            TrackInstrumentKind::Tom => 50,
            TrackInstrumentKind::Clap => 39,
            TrackInstrumentKind::Ride => 51,
            TrackInstrumentKind::Cymbal => 49,
            TrackInstrumentKind::Snare606 => 40,
            TrackInstrumentKind::BassDrum808 => 35,
            TrackInstrumentKind::Perc1 => 37,
            TrackInstrumentKind::Bd6smp => 41,
            TrackInstrumentKind::Sd6smp => 40,
            TrackInstrumentKind::Ch6smp => 42,
            TrackInstrumentKind::Buzz => 44,
            TrackInstrumentKind::Sdrex => 48,
            TrackInstrumentKind::Bd6Ac => 52,
            TrackInstrumentKind::Sd6Ac => 53,
            TrackInstrumentKind::Hh6Ac => 54,
            TrackInstrumentKind::Oh6Ac => 55,
            TrackInstrumentKind::Cl6Ac => 56,
            TrackInstrumentKind::Tm6Ac => 57,
            // GM open hi-hat, like Ch6smp reuses the closed hat's 42.
            TrackInstrumentKind::Oh6smp => 46,
            // GM vibraslap - free, and an FX slot suits an FX voice.
            TrackInstrumentKind::Rift => 58,
            // GM ride cymbal 2 - free.
            TrackInstrumentKind::OneShot => 59,
            // An effect bus plays no note and listens to none; 0 is never matched
            // (`slots_listening_to` skips effect kinds).
            TrackInstrumentKind::Reverb | TrackInstrumentKind::Delay | TrackInstrumentKind::Rumble => 0,
        }
    }

    /// Map to the legacy `DrumVoice` index used by the synthesis layer.
    /// Tom always maps to the first tom variant because `DrumVoice` keeps
    /// three tom slots for backward compatibility while the track model has
    /// a single `Tom` kind.
    pub fn drum_voice_index(self) -> usize {
        match self {
            TrackInstrumentKind::Kick => 0,
            TrackInstrumentKind::Snare => 1,
            TrackInstrumentKind::HiHat => 2,
            TrackInstrumentKind::OpenHiHat => 3,
            TrackInstrumentKind::Tom => 4,
            TrackInstrumentKind::Clap => 7,
            TrackInstrumentKind::Ride => 8,
            TrackInstrumentKind::Cymbal => 9,
            TrackInstrumentKind::Snare606 => 10,
            TrackInstrumentKind::BassDrum808 => 11,
            TrackInstrumentKind::Perc1 => 12,
            TrackInstrumentKind::Bd6smp => 13,
            TrackInstrumentKind::Sd6smp => 14,
            TrackInstrumentKind::Ch6smp => 15,
            TrackInstrumentKind::Buzz => 16,
            TrackInstrumentKind::Sdrex => 17,
            TrackInstrumentKind::Bd6Ac => 18,
            TrackInstrumentKind::Sd6Ac => 19,
            TrackInstrumentKind::Hh6Ac => 20,
            TrackInstrumentKind::Oh6Ac => 21,
            TrackInstrumentKind::Cl6Ac => 22,
            TrackInstrumentKind::Tm6Ac => 23,
            TrackInstrumentKind::Oh6smp => 24,
            TrackInstrumentKind::Rift => 25,
            TrackInstrumentKind::OneShot => 26,
            TrackInstrumentKind::Reverb => 27,
            TrackInstrumentKind::Delay => 28,
            TrackInstrumentKind::Rumble => 29,
        }
    }

    /// Build from a legacy 13-voice `DrumVoice` index.
    pub fn from_drum_voice_index(index: usize) -> Option<Self> {
        match index {
            0 => Some(Self::Kick),
            1 => Some(Self::Snare),
            2 => Some(Self::HiHat),
            3 => Some(Self::OpenHiHat),
            4 | 5 | 6 => Some(Self::Tom),
            7 => Some(Self::Clap),
            8 => Some(Self::Ride),
            9 => Some(Self::Cymbal),
            10 => Some(Self::Snare606),
            11 => Some(Self::BassDrum808),
            12 => Some(Self::Perc1),
            13 => Some(Self::Bd6smp),
            14 => Some(Self::Sd6smp),
            15 => Some(Self::Ch6smp),
            16 => Some(Self::Buzz),
            17 => Some(Self::Sdrex),
            18 => Some(Self::Bd6Ac),
            19 => Some(Self::Sd6Ac),
            20 => Some(Self::Hh6Ac),
            21 => Some(Self::Oh6Ac),
            22 => Some(Self::Cl6Ac),
            23 => Some(Self::Tm6Ac),
            24 => Some(Self::Oh6smp),
            25 => Some(Self::Rift),
            26 => Some(Self::OneShot),
            27 => Some(Self::Reverb),
            28 => Some(Self::Delay),
            29 => Some(Self::Rumble),
            _ => None,
        }
    }

    /// [286] A send effect rather than an instrument: it has no steps, no note
    /// and no trigger, and only lives on the effect-bus slots.
    pub fn is_effect(self) -> bool {
        matches!(self, Self::Reverb | Self::Delay | Self::Rumble)
    }

    pub fn algo_count(self) -> usize {
        crate::instrument_registry::INSTRUMENTS[self.drum_voice_index()].algo_count
    }

    pub fn instrument_def(self) -> &'static crate::instrument_registry::InstrumentDef {
        &crate::instrument_registry::INSTRUMENTS[self.drum_voice_index()]
    }
}

/// Selected audio destination for a track.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrackAudioOut {
    Main,
    Out(u8), // 1..=MAX_TRACKS
}

impl TrackAudioOut {
    pub fn label(self) -> String {
        match self {
            TrackAudioOut::Main => "No Aux".to_string(),
            TrackAudioOut::Out(n) => format!("Out {}", n),
        }
    }

    pub fn from_index(index: u8) -> Self {
        if index == 0 {
            TrackAudioOut::Main
        } else {
            TrackAudioOut::Out(index.min(MAX_TRACKS as u8))
        }
    }

    pub fn index(self) -> u8 {
        match self {
            TrackAudioOut::Main => 0,
            TrackAudioOut::Out(n) => n,
        }
    }
}

/// Per-track audio routing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackRouting {
    pub main_on: bool,
    pub out_select: TrackAudioOut,
    /// Choke group: 0 = none, 1..=4 = group index. When a slot triggers, every
    /// other active slot in the SAME group is silenced (classic HH→OH choke,
    /// generalized to all instruments).
    /// `CHOKE_LEGACY` is a serde-only sentinel for sessions saved before this
    /// field existed — never stored in the atomic layout.
    #[serde(default = "default_choke_group")]
    pub choke_group: u8,
}

pub const CHOKE_GROUP_COUNT: u8 = 4;
/// Serde-only sentinel: field absent in old `track-layout-v1` blobs.
pub const CHOKE_LEGACY: u8 = 0xFF;

fn default_choke_group() -> u8 {
    CHOKE_LEGACY
}

/// Resolve the effective choke group for a slot, migrating the legacy sentinel:
/// sessions saved before choke groups existed get the former default behavior
/// (HiHat + OpenHiHat choking each other via group 1).
pub fn effective_choke_group(kind: TrackInstrumentKind, routing: &TrackRouting) -> u8 {
    if routing.choke_group == CHOKE_LEGACY {
        if matches!(
            kind,
            TrackInstrumentKind::HiHat | TrackInstrumentKind::OpenHiHat
        ) {
            1
        } else {
            0
        }
    } else {
        routing.choke_group.min(CHOKE_GROUP_COUNT)
    }
}

impl Default for TrackRouting {
    fn default() -> Self {
        Self {
            main_on: true,
            out_select: TrackAudioOut::Main,
            choke_group: 0,
        }
    }
}

/// One internal track slot.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TrackSlot {
    pub active: bool,
    pub name: String,
    pub kind: TrackInstrumentKind,
    pub routing: TrackRouting,
    pub midi_note: u8,
    /// When true, this lane shares the GRID (step on/off + fusions) of the lane
    /// directly above it (slot index - 1) for layering — its own sound, plocks,
    /// routing, MIDI note stay independent. Only meaningful when the lane above
    /// is active. `#[serde(default)]` so older `track-layout-v1` blobs load.
    #[serde(default)]
    pub linked_up: bool,
    /// [286] Send level (0..1) to Fx1 and Fx2. `#[serde(default)]` so older
    /// `track-layout-v1` blobs load with no send.
    #[serde(default)]
    pub fx_send: [f32; FX_SLOT_COUNT],
}

impl TrackSlot {
    pub fn inactive() -> Self {
        Self {
            active: false,
            name: String::new(),
            kind: TrackInstrumentKind::Kick,
            routing: TrackRouting::default(),
            midi_note: TrackInstrumentKind::Kick.default_midi_note(),
            linked_up: false,
            fx_send: [0.0; FX_SLOT_COUNT],
        }
    }

    pub fn active_with_kind(kind: TrackInstrumentKind) -> Self {
        Self {
            active: true,
            name: kind.default_name().to_string(),
            kind,
            routing: TrackRouting::default(),
            midi_note: kind.default_midi_note(),
            linked_up: false,
            fx_send: [0.0; FX_SLOT_COUNT],
        }
    }
}

/// Full track layout persisted in the DAW state.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct TrackLayoutState {
    #[serde(with = "serde_arrays")]
    pub slots: [TrackSlot; MAX_TRACKS],
    pub global_midi_channel: u8,
    pub global_base_note: u8,
}

impl TrackLayoutState {
    /// Resolve the slot whose GRID (steps + fusions) a lane actually plays and
    /// edits. A `linked_up` lane mirrors the active lane directly above it; the
    /// chain is followed upward (C→B→A) so stacked links all share the top
    /// lane's grid. Returns the input slot when it is not linked (or its master
    /// is inactive).
    pub fn grid_slot(&self, mut slot: usize) -> usize {
        let mut guard = 0;
        while slot > 0
            && slot < MAX_TRACKS
            && self.slots[slot].linked_up
            && self.slots[slot - 1].active
            && guard < MAX_TRACKS
        {
            slot -= 1;
            guard += 1;
        }
        slot
    }

    /// Whether `slot` can be linked to the lane above (both must be active and
    /// there must be a lane above).
    pub fn can_link_up(&self, slot: usize) -> bool {
        slot > 0
            && slot < MAX_TRACKS
            && self.slots[slot].active
            && self.slots[slot - 1].active
            && !self.slots[slot].kind.is_effect()
            && !self.slots[slot - 1].kind.is_effect()
    }

    /// Whether `slot` currently mirrors the lane above (link active + master active).
    pub fn is_linked_up(&self, slot: usize) -> bool {
        slot > 0 && slot < MAX_TRACKS && self.slots[slot].linked_up && self.slots[slot - 1].active
    }

    pub fn default_layout() -> Self {
        // Product decision 2026-07-04: new instances start with the modular
        // 4-lane template (BD/SD/HH/Tom). Legacy 13-voice sessions keep their
        // saved `track-layout-v1`; sessions saved BEFORE that field existed
        // will open with 4 lanes.
        Self::modular_default_layout()
    }

    pub fn modular_default_layout() -> Self {
        let mut slots: [TrackSlot; MAX_TRACKS] = std::array::from_fn(|_| TrackSlot::inactive());
        // Default template: BD / SD / HH / Tom
        slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        slots[1] = TrackSlot::active_with_kind(TrackInstrumentKind::Snare);
        slots[2] = TrackSlot::active_with_kind(TrackInstrumentKind::HiHat);
        slots[3] = TrackSlot::active_with_kind(TrackInstrumentKind::Tom);
        Self {
            slots,
            global_midi_channel: 10,
            global_base_note: 36,
        }
    }

    pub fn empty_layout() -> Self {
        Self {
            slots: std::array::from_fn(|_| TrackSlot::inactive()),
            global_midi_channel: 10,
            global_base_note: 36,
        }
    }

    pub fn preset_12_layout() -> Self {
        let mut slots: [TrackSlot; MAX_TRACKS] = std::array::from_fn(|_| TrackSlot::inactive());
        let kinds = [
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            // [204] Was OpenHiHat: a second hat, to be lengthened for the open
            // sound. It still chokes against the first one.
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Clap,
            TrackInstrumentKind::Ride,
            TrackInstrumentKind::Cymbal,
            // [203] Was Snare606.
            TrackInstrumentKind::Sd6Ac,
            TrackInstrumentKind::BassDrum808,
        ];
        for (slot, kind) in slots.iter_mut().zip(kinds) {
            *slot = TrackSlot::active_with_kind(kind);
            // Classic closed/open hat choke out of the box.
            if matches!(kind, TrackInstrumentKind::HiHat) {
                slot.routing.choke_group = 1;
            }
        }
        Self {
            slots,
            global_midi_channel: 10,
            global_base_note: 36,
        }
    }

    /// AC kit ([195]): the analogcode voices wherever an equivalent exists.
    /// 4 lanes: BD6(AC) / SD6(AC) / HH6(AC) / TM6(AC).
    pub fn preset_ac4_layout() -> Self {
        Self::from_kinds(&[
            TrackInstrumentKind::Bd6Ac,
            TrackInstrumentKind::Sd6Ac,
            TrackInstrumentKind::Hh6Ac,
            TrackInstrumentKind::Tm6Ac,
        ])
    }

    /// AC 12-lane kit: AC voices where an equivalent exists, classic voices
    /// (Snare606, 808, Ride, Cymbal) kept for what the AC set does not cover.
    pub fn preset_ac12_layout() -> Self {
        Self::from_kinds(&[
            TrackInstrumentKind::Bd6Ac,
            TrackInstrumentKind::Sd6Ac,
            TrackInstrumentKind::Hh6Ac,
            TrackInstrumentKind::Oh6Ac,
            TrackInstrumentKind::Tm6Ac,
            TrackInstrumentKind::Tm6Ac,
            TrackInstrumentKind::Tm6Ac,
            TrackInstrumentKind::Cl6Ac,
            TrackInstrumentKind::Ride,
            TrackInstrumentKind::Cymbal,
            TrackInstrumentKind::Snare606,
            TrackInstrumentKind::BassDrum808,
        ])
    }

    /// Build a layout that activates slots `0..kinds.len()` with the given
    /// instrument kinds (remaining slots inactive). Used by the style-preset
    /// chips to install an appropriate kit. HiHat/OpenHiHat get choke group 1
    /// like the classic 12-lane preset. Extra kinds beyond `MAX_TRACKS` are
    /// ignored.
    pub fn from_kinds(kinds: &[TrackInstrumentKind]) -> Self {
        let mut slots: [TrackSlot; MAX_TRACKS] = std::array::from_fn(|_| TrackSlot::inactive());
        for (slot, &kind) in slots.iter_mut().zip(kinds.iter()) {
            *slot = TrackSlot::active_with_kind(kind);
            if matches!(
                kind,
                TrackInstrumentKind::HiHat
                    | TrackInstrumentKind::OpenHiHat
                    | TrackInstrumentKind::Hh6Ac
                    | TrackInstrumentKind::Oh6Ac
            ) {
                slot.routing.choke_group = 1;
            }
        }
        Self {
            slots,
            global_midi_channel: 10,
            global_base_note: 36,
        }
    }

    /// [283] Keep the global MIDI settings (channel, base note) of `current`
    /// when this layout replaces it: a lane preset changes the lanes, not the
    /// channel chosen in Settings.
    pub fn keep_globals_of(&mut self, current: &TrackLayoutState) {
        self.global_midi_channel = current.global_midi_channel;
        self.global_base_note = current.global_base_note;
        // [286] The effect buses belong to the project, not to a lane preset: a
        // preset that has none of its own leaves the current ones in place.
        for slot in FX_FIRST_SLOT..MAX_TRACKS {
            let keeps = &current.slots[slot];
            let is_effect = |s: &TrackSlot| s.active && s.kind.is_effect();
            if is_effect(keeps) && !is_effect(&self.slots[slot]) {
                self.slots[slot] = keeps.clone();
            }
        }
    }

    /// Does this lane mirror the grid of the lane above ([191])?
    ///
    /// A link is **positional** - "I play the lane directly above me" - so this
    /// is exactly what forbids moving such a lane on its own: displacing it, or
    /// slipping another lane above it, would silently re-point the link at a
    /// different instrument.
    pub fn is_grid_follower(&self, slot: usize) -> bool {
        slot < MAX_TRACKS && self.grid_slot(slot) != slot
    }

    /// How many lanes travel together when `slot` is dragged: the lane itself
    /// plus the unbroken run of followers directly below it. 1 when it leads
    /// nothing.
    pub fn chain_len(&self, slot: usize) -> usize {
        if slot >= MAX_TRACKS {
            return 0;
        }
        let mut len = 1;
        while slot + len < MAX_TRACKS && self.is_grid_follower(slot + len) {
            len += 1;
        }
        len
    }

    /// Move a drop point out of the middle of a chain, to the nearer of its two
    /// ends ([191]).
    ///
    /// `gap` means "insert before the lane currently at this index". A gap whose
    /// lane is a follower sits between that follower and what it follows, so
    /// anything dropped there would come between a master and its slaves.
    pub fn snap_gap_out_of_chains(&self, gap: usize) -> usize {
        let gap = gap.min(MAX_TRACKS);
        if gap == 0 || gap >= MAX_TRACKS || !self.is_grid_follower(gap) {
            return gap;
        }
        let mut start = gap;
        while start > 0 && self.is_grid_follower(start) {
            start -= 1;
        }
        let mut end = gap;
        while end < MAX_TRACKS && self.is_grid_follower(end) {
            end += 1;
        }
        if gap - start <= end - gap {
            start
        } else {
            end
        }
    }

    /// Rewrite every lane holding a retired kind ([203] [204]). Returns whether
    /// anything changed.
    ///
    /// The lane **keeps its MIDI note**, so a project already driving it from
    /// the DAW keeps triggering the same lane. Only a name still equal to the
    /// retired kind's default follows the new one: a lane labelled
    /// "Open Hi-Hat" that is now a Hi-Hat would just be lying, while a name the
    /// user typed is theirs to keep.
    ///
    /// The sound settings are left untouched. They were stored against the old
    /// voice's parameter shape, so a migrated lane does not sound exactly like
    /// the retired voice did - that is the point of retiring it.
    /// [298] An effect lane used to be named after its place, "Fx1" / "Fx2": it is now named
    /// after its effect. Only those two exact automatic names are replaced, so a name the user
    /// typed is kept. Returns whether anything changed.
    pub fn migrate_fx_names(&mut self) -> bool {
        let mut changed = false;
        for (i, slot) in self.slots.iter_mut().enumerate() {
            if is_fx_slot(i)
                && slot.kind.is_effect()
                && slot.name == format!("Fx{}", i - FX_FIRST_SLOT + 1)
            {
                slot.name = slot.kind.default_name().to_string();
                changed = true;
            }
        }
        changed
    }

    pub fn migrate_retired_kinds(&mut self) -> bool {
        let mut changed = false;
        for slot in self.slots.iter_mut() {
            let Some(replacement) = slot.kind.retired_replacement() else {
                continue;
            };
            if slot.name == slot.kind.default_name() {
                slot.name = replacement.default_name().to_string();
            }
            slot.kind = replacement;
            changed = true;
        }
        changed
    }

    /// Migrate a legacy 13-voice session into 14 slots.
    pub fn from_legacy_13() -> Self {
        let mut slots: [TrackSlot; MAX_TRACKS] = std::array::from_fn(|_| TrackSlot::inactive());
        let legacy_kinds = [
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::OpenHiHat,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Clap,
            TrackInstrumentKind::Ride,
            TrackInstrumentKind::Cymbal,
            TrackInstrumentKind::Snare606,
            TrackInstrumentKind::BassDrum808,
            TrackInstrumentKind::Perc1,
        ];
        for (i, kind) in legacy_kinds.iter().enumerate() {
            let mut slot = TrackSlot {
                active: true,
                name: format!("{} {}", kind.default_name(), occurrence_label(i, *kind)),
                kind: *kind,
                routing: TrackRouting::default(),
                midi_note: crate::instrument_registry::INSTRUMENTS[i].midi_note,
                linked_up: false,
                fx_send: [0.0; FX_SLOT_COUNT],
            };
            // Classic HH↔OH choke out of the box.
            if matches!(
                kind,
                TrackInstrumentKind::HiHat | TrackInstrumentKind::OpenHiHat
            ) {
                slot.routing.choke_group = 1;
            }
            slots[i] = slot;
        }
        Self {
            slots,
            global_midi_channel: 10,
            global_base_note: 36,
        }
    }

    pub fn active_slot_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.active)
            .map(|(i, _)| i)
    }

    pub fn active_count(&self) -> usize {
        self.slots.iter().filter(|s| s.active).count()
    }

    pub fn move_slot(&mut self, from: usize, to: usize) {
        if from >= MAX_TRACKS || to >= MAX_TRACKS || from == to {
            return;
        }

        let moved = self.slots[from].clone();
        if from < to {
            for idx in from..to {
                self.slots[idx] = self.slots[idx + 1].clone();
            }
        } else {
            for idx in (to + 1..=from).rev() {
                self.slots[idx] = self.slots[idx - 1].clone();
            }
        }
        self.slots[to] = moved;
    }

    pub fn first_inactive_slot(&self) -> Option<usize> {
        self.slots.iter().position(|s| !s.active)
    }

    /// How many active lanes send to this aux output ([194]).
    ///
    /// Counts **every** lane, the one being edited included, so the picker's
    /// label is a statement about the output rather than about who is reading
    /// it: "Out 3 - 2 lanes" means the bus carries two lanes, full stop. An
    /// earlier version counted only the *others*, which read as "1 lane" on an
    /// output that actually carried two.
    pub fn lanes_on_output(&self, out_number: u8) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.active && slot.routing.out_select == TrackAudioOut::Out(out_number))
            .count()
    }

    /// Route one lane to an output.
    ///
    /// [194] Several lanes may now target the **same** aux output: the audio
    /// path already sums every lane into the bus it selects, so sharing needs
    /// nothing more than letting it happen. Until then this method stole the
    /// output from whoever held it - silently, which is what [117] called
    /// exclusivity and what made a shared bus impossible.
    pub fn assign_slot_output(&mut self, slot: usize, output: TrackAudioOut) {
        if slot >= MAX_TRACKS {
            return;
        }

        // [193] Sending a lane to its own output takes it out of the main mix,
        // so it is heard ONCE - on its own channel - instead of twice.
        //
        // A default, not a rule: the Main Mix switch stays usable, so a lane can
        // still go to both for parallel treatment. Going back to `No Aux`
        // restores the main mix, or the lane would be audible nowhere.
        self.slots[slot].routing.main_on = matches!(output, TrackAudioOut::Main);

        self.slots[slot].routing.out_select = output;
    }
}

fn occurrence_label(slot: usize, kind: TrackInstrumentKind) -> String {
    // Produce "Tom 1", "Tom 2", "Tom 3" for duplicate toms, empty otherwise.
    if kind == TrackInstrumentKind::Tom {
        let tom_count_before = (0..slot)
            .filter(|&i| {
                i < MAX_TRACKS
                    && TrackInstrumentKind::from_drum_voice_index(i)
                        == Some(TrackInstrumentKind::Tom)
            })
            .count();
        return format!("{}", tom_count_before + 1);
    }
    String::new()
}

/// Atomic, lock-free view of the track layout used by the audio thread.
pub struct AtomicTrackLayout {
    pub slot_kinds: [AtomicU8; MAX_TRACKS],
    pub slot_routing: [AtomicU8; MAX_TRACKS],
    pub slot_midi_notes: [AtomicU8; MAX_TRACKS],
    /// Grid-link flag per slot (mirrors the lane above). Read by the audio
    /// thread to resolve which slot's steps/fusions a lane plays.
    pub slot_linked: [AtomicBool; MAX_TRACKS],
    /// [286] Send levels (f32 bits) to Fx1 / Fx2, per slot.
    pub slot_fx_send: [[AtomicU32; FX_SLOT_COUNT]; MAX_TRACKS],
    pub global_channel: AtomicU8,
    pub global_base_note: AtomicU8,
    pub version: std::sync::atomic::AtomicU64,
}

impl AtomicTrackLayout {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            slot_kinds: std::array::from_fn(|_| AtomicU8::new(0xFF)),
            slot_routing: std::array::from_fn(|_| AtomicU8::new(0)),
            slot_midi_notes: std::array::from_fn(|_| AtomicU8::new(0)),
            slot_linked: std::array::from_fn(|_| AtomicBool::new(false)),
            slot_fx_send: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            global_channel: AtomicU8::new(10),
            global_base_note: AtomicU8::new(36),
            version: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// Resolve the slot whose grid (steps + fusions) `slot` actually plays,
    /// following `linked_up` chains upward to the active master. RT-safe (atomic
    /// reads only). Mirror of `TrackLayoutState::grid_slot` for the audio thread.
    pub fn grid_slot(&self, mut slot: usize) -> usize {
        let mut guard = 0;
        while slot > 0
            && slot < MAX_TRACKS
            && self.slot_linked[slot].load(Ordering::Relaxed)
            && self.slot_kinds[slot - 1].load(Ordering::Relaxed) != 0xFF
            && guard < MAX_TRACKS
        {
            slot -= 1;
            guard += 1;
        }
        slot
    }

    pub fn from_state(state: &TrackLayoutState) -> Arc<Self> {
        let atomic = Self::new();
        atomic.update_from_state(state);
        atomic
    }

    pub fn update_from_state(&self, state: &TrackLayoutState) {
        for (i, slot) in state.slots.iter().enumerate() {
            let kind_byte = if slot.active {
                slot.kind.index() as u8
            } else {
                0xFF
            };
            self.slot_kinds[i].store(kind_byte, Ordering::Relaxed);
            // Normalize the legacy sentinel here so the atomic layout only ever
            // carries real group values (0..=4).
            let choke = effective_choke_group(slot.kind, &slot.routing);
            let routing_byte =
                routing_byte(slot.routing.main_on, slot.routing.out_select.index(), choke);
            self.slot_routing[i].store(routing_byte, Ordering::Relaxed);
            self.slot_midi_notes[i].store(slot.midi_note, Ordering::Relaxed);
            self.slot_linked[i].store(slot.linked_up, Ordering::Relaxed);
            for (bus, send) in slot.fx_send.iter().enumerate() {
                self.slot_fx_send[i][bus]
                    .store(send.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
            }
        }
        self.global_channel
            .store(state.global_midi_channel.clamp(1, 16), Ordering::Relaxed);
        self.global_base_note
            .store(state.global_base_note, Ordering::Relaxed);
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn kind_for_slot(&self, slot: usize) -> Option<TrackInstrumentKind> {
        if slot >= MAX_TRACKS {
            return None;
        }
        let byte = self.slot_kinds[slot].load(Ordering::Relaxed);
        if byte == 0xFF {
            None
        } else {
            TrackInstrumentKind::from_index(byte as usize)
        }
    }

    pub fn is_active(&self, slot: usize) -> bool {
        slot < MAX_TRACKS && self.slot_kinds[slot].load(Ordering::Relaxed) != 0xFF
    }

    /// [286] Send level of `slot` to effect bus `bus` (0 = Fx1, 1 = Fx2).
    pub fn fx_send(&self, slot: usize, bus: usize) -> f32 {
        if slot >= MAX_TRACKS || bus >= FX_SLOT_COUNT {
            return 0.0;
        }
        f32::from_bits(self.slot_fx_send[slot][bus].load(Ordering::Relaxed))
    }

    /// [286] Is `slot` an active effect bus (an effect kind on an effect slot)?
    pub fn is_effect_bus(&self, slot: usize) -> bool {
        is_fx_slot(slot) && self.kind_for_slot(slot).is_some_and(|k| k.is_effect())
    }

    pub fn routing_for_slot(&self, slot: usize) -> TrackRouting {
        if slot >= MAX_TRACKS {
            return TrackRouting::default();
        }
        decode_routing(self.slot_routing[slot].load(Ordering::Relaxed))
    }

    pub fn midi_note_for_slot(&self, slot: usize) -> u8 {
        if slot >= MAX_TRACKS {
            return 0;
        }
        self.slot_midi_notes[slot].load(Ordering::Relaxed)
    }

    /// Active slots listening on `note`, as a bitmask of slot indices ([221]).
    ///
    /// External MIDI must reach a lane by **its own** note — the same one the
    /// plugin sends on (`midi_note_for_slot`). Matching the registry's factory
    /// note instead meant a retuned lane never sounded, and that a shared
    /// factory note resolved to the first voice declaring it, often a retired
    /// one ([203] Snare606 on 40, [204] OpenHiHat on 46) that no lane can
    /// carry — so nothing played at all.
    ///
    /// Several lanes may share a note on purpose; they all trigger.
    /// Lock-free, audio-thread safe.
    pub fn slots_listening_to(&self, note: u8) -> u16 {
        let mut mask = 0u16;
        for slot in 0..MAX_TRACKS {
            if self.is_active(slot)
                && !self.kind_for_slot(slot).is_some_and(|k| k.is_effect())
                && self.midi_note_for_slot(slot) == note
            {
                mask |= 1 << slot;
            }
        }
        mask
    }

    /// Choke group of a slot (0 = none, 1..=4). Lock-free, audio-thread safe.
    pub fn choke_group_for_slot(&self, slot: usize) -> u8 {
        if slot >= MAX_TRACKS {
            return 0;
        }
        (self.slot_routing[slot].load(Ordering::Relaxed) >> 4) & 0x07
    }

    pub fn global_midi_channel(&self) -> u8 {
        self.global_channel.load(Ordering::Relaxed)
    }

    pub fn bump_version(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }
}

impl Default for AtomicTrackLayout {
    fn default() -> Self {
        // Only used when we need a standalone value; the plugin always uses Arc.
        Self {
            slot_kinds: std::array::from_fn(|_| AtomicU8::new(0xFF)),
            slot_routing: std::array::from_fn(|_| AtomicU8::new(0)),
            slot_midi_notes: std::array::from_fn(|_| AtomicU8::new(0)),
            slot_linked: std::array::from_fn(|_| AtomicBool::new(false)),
            slot_fx_send: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU32::new(0))),
            global_channel: AtomicU8::new(10),
            global_base_note: AtomicU8::new(36),
            version: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

/// Routing byte layout: bit 7 = main_on, bits 4-6 = choke group (0..=4),
/// bits 0-3 = aux output index (0..=14).
fn routing_byte(main_on: bool, out_index: u8, choke_group: u8) -> u8 {
    let main_bit = if main_on { 0x80 } else { 0 };
    main_bit | ((choke_group & 0x07) << 4) | (out_index & 0x0F)
}

fn decode_routing(byte: u8) -> TrackRouting {
    TrackRouting {
        main_on: (byte & 0x80) != 0,
        out_select: TrackAudioOut::from_index(byte & 0x0F),
        choke_group: (byte >> 4) & 0x07,
    }
}

#[derive(Clone)]
pub struct PersistentTrackLayout {
    pub state: Arc<AtomicTrackLayout>,
    /// Custom names for each slot, persisted alongside the atomic layout.
    /// Stored outside the atomic state because the audio thread does not use names.
    slot_names: Arc<std::sync::Mutex<[String; MAX_TRACKS]>>,
}

impl PersistentTrackLayout {
    pub fn new() -> Self {
        let layout = TrackLayoutState::default_layout();
        Self {
            state: AtomicTrackLayout::from_state(&layout),
            slot_names: Arc::new(std::sync::Mutex::new(std::array::from_fn(|i| {
                layout.slots[i].name.clone()
            }))),
        }
    }
}

impl Default for PersistentTrackLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> nih_plug::params::persist::PersistentField<'a, TrackLayoutState>
    for PersistentTrackLayout
{
    fn set(&self, new_value: TrackLayoutState) {
        // Single funnel for every layout write - session restore, UI edits,
        // layout presets, pattern-preset kits - so a retired kind cannot reach
        // the atomic layout from anywhere ([203] [204]).
        let mut new_value = new_value;
        new_value.migrate_retired_kinds();
        new_value.migrate_fx_names();
        self.state.update_from_state(&new_value);
        if let Ok(mut names) = self.slot_names.lock() {
            for (i, slot) in new_value.slots.iter().enumerate() {
                names[i] = slot.name.clone();
            }
        }
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&TrackLayoutState) -> R,
    {
        let slots = std::array::from_fn(|i| {
            let byte = self.state.slot_kinds[i].load(Ordering::Relaxed);
            let active = byte != 0xFF;
            let kind = if active {
                TrackInstrumentKind::from_index(byte as usize).unwrap_or(TrackInstrumentKind::Kick)
            } else {
                TrackInstrumentKind::Kick
            };
            let name = self
                .slot_names
                .lock()
                .map(|names| names[i].clone())
                .unwrap_or_else(|_| {
                    if active {
                        kind.default_name().to_string()
                    } else {
                        String::new()
                    }
                });
            TrackSlot {
                active,
                name,
                kind,
                routing: self.state.routing_for_slot(i),
                midi_note: self.state.slot_midi_notes[i].load(Ordering::Relaxed),
                linked_up: self.state.slot_linked[i].load(Ordering::Relaxed),
                fx_send: std::array::from_fn(|bus| self.state.fx_send(i, bus)),
            }
        });
        let state = TrackLayoutState {
            slots,
            global_midi_channel: self.state.global_midi_channel(),
            global_base_note: self.state.global_base_note.load(Ordering::Relaxed),
        };
        f(&state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// External MIDI must reach a lane by ITS OWN note ([221]).
    ///
    /// Regression: the audio thread used to resolve an incoming note through
    /// the registry's factory notes. A retuned lane never sounded, and a note
    /// shared by several kinds resolved to the first one declaring it — note 40
    /// to Snare606 and note 46 to OpenHiHat, both retired, so on no lane at
    /// all: nothing played.
    #[test]
    fn external_midi_matches_the_lane_note_not_the_factory_note() {
        let mut layout = TrackLayoutState::default_layout();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::Sd6smp);
        layout.slots[0].midi_note = 61; // retuned by the user
        layout.slots[1] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        layout.slots[1].midi_note = 36;
        // Two lanes deliberately sharing one note: both must trigger.
        layout.slots[2] = TrackSlot::active_with_kind(TrackInstrumentKind::HiHat);
        layout.slots[2].midi_note = 36;
        // Inactive lanes never listen.
        layout.slots[3] = TrackSlot::active_with_kind(TrackInstrumentKind::Clap);
        layout.slots[3].midi_note = 36;
        layout.slots[3].active = false;

        let atomic = AtomicTrackLayout::from_state(&layout);

        assert_eq!(atomic.slots_listening_to(61), 1 << 0, "retuned lane missed");
        assert_eq!(
            atomic.slots_listening_to(36),
            (1 << 1) | (1 << 2),
            "shared note must trigger every lane carrying it, and no inactive one"
        );
        // SD6smp's factory note is 40; nothing listens there any more.
        assert_eq!(
            atomic.slots_listening_to(40),
            0,
            "factory note still answers"
        );
    }
    use nih_plug::params::persist::PersistentField;

    #[test]
    fn categories_partition_all_kinds() {
        // Every SELECTABLE kind appears in exactly one picker category, and no
        // category is empty. Retired kinds ([203] [204]) are deliberately
        // absent from the pickers - the assertion below pins exactly which.
        let mut seen = [false; TrackInstrumentKind::COUNT];
        for cat in InstrumentCategory::ALL {
            let kinds: Vec<_> = TrackInstrumentKind::kinds_in(cat).collect();
            assert!(!kinds.is_empty(), "category {} is empty", cat.label());
            for kind in kinds {
                assert!(!seen[kind.index()], "{:?} appears in two categories", kind);
                seen[kind.index()] = true;
            }
        }
        let missing: Vec<_> = TrackInstrumentKind::ALL
            .into_iter()
            .filter(|k| !seen[k.index()])
            .collect();
        assert_eq!(
            missing,
            vec![
                TrackInstrumentKind::OpenHiHat,
                TrackInstrumentKind::Snare606
            ],
            "only the retired kinds may be missing from the pickers"
        );
        // ...and `category()` itself still answers for every kind, retired
        // included, because saved lanes are still read through it.
        for kind in TrackInstrumentKind::ALL {
            let cat = kind.category();
            assert!(InstrumentCategory::ALL.contains(&cat), "{kind:?}");
        }
    }

    /// [203] [204] A retired kind cannot be picked, but it must still decode
    /// from a saved session and say what it becomes.
    #[test]
    fn retired_kinds_are_unpickable_but_still_decodable() {
        for (retired, replacement) in [
            (TrackInstrumentKind::Snare606, TrackInstrumentKind::Sd6Ac),
            (TrackInstrumentKind::OpenHiHat, TrackInstrumentKind::HiHat),
        ] {
            assert!(!retired.selectable(), "{retired:?} must leave the pickers");
            assert_eq!(retired.retired_replacement(), Some(replacement));
            // The persisted index still resolves, or every saved lane after it
            // would decode as another instrument.
            assert_eq!(
                TrackInstrumentKind::from_index(retired.index()),
                Some(retired)
            );
            assert!(replacement.selectable(), "{replacement:?} must stay");
            assert_eq!(replacement.retired_replacement(), None);
        }
    }

    /// A saved layout is rewritten on load: the kind changes, the MIDI note is
    /// preserved, and only a default name follows the new kind.
    #[test]
    fn loading_a_layout_migrates_retired_lanes() {
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::OpenHiHat);
        layout.slots[0].midi_note = 61;
        layout.slots[1] = TrackSlot::active_with_kind(TrackInstrumentKind::Snare606);
        layout.slots[1].name = "Ma caisse".to_string();
        layout.slots[2] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);

        assert!(layout.migrate_retired_kinds());

        assert_eq!(layout.slots[0].kind, TrackInstrumentKind::HiHat);
        assert_eq!(
            layout.slots[0].name,
            TrackInstrumentKind::HiHat.default_name(),
            "a default name follows the new kind"
        );
        assert_eq!(
            layout.slots[0].midi_note, 61,
            "the MIDI note is preserved so the DAW keeps hitting this lane"
        );
        assert_eq!(layout.slots[1].kind, TrackInstrumentKind::Sd6Ac);
        assert_eq!(
            layout.slots[1].name, "Ma caisse",
            "a name the user typed is theirs"
        );
        assert_eq!(layout.slots[2].kind, TrackInstrumentKind::Kick);
        assert!(
            !layout.migrate_retired_kinds(),
            "a migrated layout is stable"
        );
    }

    #[test]
    fn category_spot_checks() {
        assert_eq!(
            TrackInstrumentKind::Kick.category(),
            InstrumentCategory::BassDrum
        );
        assert_eq!(
            TrackInstrumentKind::BassDrum808.category(),
            InstrumentCategory::BassDrum
        );
        assert_eq!(
            TrackInstrumentKind::Bd6smp.category(),
            InstrumentCategory::BassDrum
        );
        assert_eq!(
            TrackInstrumentKind::Clap.category(),
            InstrumentCategory::Snare
        );
        assert_eq!(
            TrackInstrumentKind::Ch6smp.category(),
            InstrumentCategory::HiHat
        );
        assert_eq!(
            TrackInstrumentKind::Tom.category(),
            InstrumentCategory::Perc
        );
        assert_eq!(TrackInstrumentKind::Buzz.category(), InstrumentCategory::Fx);
        assert_eq!(
            TrackInstrumentKind::Ride.category(),
            InstrumentCategory::Other
        );
    }

    /// [262] The audio thread resolves links via `AtomicTrackLayout::grid_slot`
    /// while the UI uses `TrackLayoutState::grid_slot`/`is_grid_follower` —
    /// the two must agree on every mix of links and inactive lanes.
    #[test]
    fn grid_slot_atomic_matches_state() {
        let mut seed = 0x2f6e_2b1a_9c3d_5e7fu64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as u32
        };
        for _ in 0..200 {
            let mut slots = std::array::from_fn(|_| TrackSlot::inactive());
            for (i, s) in slots.iter_mut().enumerate() {
                if next() & 1 == 0 {
                    *s = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
                }
                s.linked_up = i > 0 && next() & 1 == 0;
            }
            let state = TrackLayoutState {
                slots,
                global_midi_channel: 10,
                global_base_note: 36,
            };
            let atomic = AtomicTrackLayout::from_state(&state);
            for slot in 0..MAX_TRACKS {
                assert_eq!(
                    state.grid_slot(slot),
                    atomic.grid_slot(slot),
                    "slot {slot} disagrees between state and atomic layouts"
                );
            }
        }
    }

    #[test]
    fn grid_slot_resolves_link_chain_to_active_master() {
        let mut layout = TrackLayoutState::default_layout(); // 4 active lanes 0..=3
                                                             // No links → identity.
        assert_eq!(layout.grid_slot(2), 2);
        assert!(!layout.is_linked_up(2));
        assert!(layout.can_link_up(2)); // slot 1 & 2 both active

        // Link 2→1.
        layout.slots[2].linked_up = true;
        assert_eq!(layout.grid_slot(2), 1);
        assert!(layout.is_linked_up(2));

        // Chain 3→2→1: slot 3 links to 2 which links to 1 → resolves to 1.
        layout.slots[3].linked_up = true;
        assert_eq!(layout.grid_slot(3), 1);

        // Break the chain if the master goes inactive: deactivate slot 1 →
        // slot 2's link is void → grid_slot(2) == 2, and the chain from 3 stops.
        layout.slots[1].active = false;
        assert_eq!(layout.grid_slot(2), 2);
        assert!(!layout.is_linked_up(2));

        // Slot 0 can never link up (no lane above).
        assert!(!layout.can_link_up(0));
        assert_eq!(layout.grid_slot(0), 0);
    }

    #[test]
    fn linked_up_defaults_false_and_survives_layout_roundtrip() {
        let mut layout = TrackLayoutState::default_layout();
        layout.slots[2].linked_up = true;
        let atomic = AtomicTrackLayout::from_state(&layout);
        assert!(atomic.slot_linked[2].load(Ordering::Relaxed));
        assert_eq!(atomic.grid_slot(2), 1);
        // Inactive slots default to not-linked.
        assert!(!atomic.slot_linked[9].load(Ordering::Relaxed));
    }

    #[test]
    fn default_layout_is_modular_four_lanes() {
        let layout = TrackLayoutState::default_layout();
        assert_eq!(layout.active_count(), 4);
        assert_eq!(layout.slots[0].kind, TrackInstrumentKind::Kick);
        assert_eq!(layout.slots[1].kind, TrackInstrumentKind::Snare);
        assert_eq!(layout.slots[2].kind, TrackInstrumentKind::HiHat);
        assert_eq!(layout.slots[3].kind, TrackInstrumentKind::Tom);
        assert!(layout.slots[4..].iter().all(|slot| !slot.active));
    }

    #[test]
    fn empty_layout_has_no_active_lanes() {
        let layout = TrackLayoutState::empty_layout();
        assert_eq!(layout.active_count(), 0);
        assert!(layout.slots.iter().all(|slot| !slot.active));
    }

    #[test]
    fn preset_12_layout_uses_core_legacy_kit_without_perc1() {
        let layout = TrackLayoutState::preset_12_layout();
        assert_eq!(layout.active_count(), 12);
        assert_eq!(layout.slots[0].kind, TrackInstrumentKind::Kick);
        assert_eq!(layout.slots[1].kind, TrackInstrumentKind::Snare);
        assert_eq!(layout.slots[2].kind, TrackInstrumentKind::HiHat);
        // [204] Was OpenHiHat: a second hat, choking against the first.
        assert_eq!(layout.slots[3].kind, TrackInstrumentKind::HiHat);
        assert_eq!(layout.slots[2].routing.choke_group, 1);
        assert_eq!(layout.slots[3].routing.choke_group, 1);
        // [203] Was Snare606.
        assert_eq!(layout.slots[10].kind, TrackInstrumentKind::Sd6Ac);
        assert_eq!(layout.slots[4].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[5].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[6].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[11].kind, TrackInstrumentKind::BassDrum808);
        assert!(!layout.slots[12].active);
        assert!(!layout.slots[13].active);
    }

    #[test]
    fn preset_ac_layouts_use_ac_kinds_and_choke_the_ac_hats() {
        let ac4 = TrackLayoutState::preset_ac4_layout();
        assert_eq!(ac4.active_count(), 4);
        assert_eq!(ac4.slots[0].kind, TrackInstrumentKind::Bd6Ac);
        assert_eq!(ac4.slots[1].kind, TrackInstrumentKind::Sd6Ac);
        assert_eq!(ac4.slots[2].kind, TrackInstrumentKind::Hh6Ac);
        assert_eq!(ac4.slots[3].kind, TrackInstrumentKind::Tm6Ac);
        assert_eq!(ac4.slots[2].routing.choke_group, 1);

        let ac12 = TrackLayoutState::preset_ac12_layout();
        assert_eq!(ac12.active_count(), 12);
        assert_eq!(ac12.slots[0].kind, TrackInstrumentKind::Bd6Ac);
        assert_eq!(ac12.slots[3].kind, TrackInstrumentKind::Oh6Ac);
        assert_eq!(ac12.slots[7].kind, TrackInstrumentKind::Cl6Ac);
        assert_eq!(ac12.slots[10].kind, TrackInstrumentKind::Snare606);
        assert_eq!(ac12.slots[11].kind, TrackInstrumentKind::BassDrum808);
        assert_eq!(ac12.slots[2].routing.choke_group, 1);
        assert_eq!(ac12.slots[3].routing.choke_group, 1);
        assert!(!ac12.slots[12].active);
        assert!(!ac12.slots[13].active);
    }

    #[test]
    fn move_slot_preserves_slot_data_and_shifts_intermediate_slots() {
        let mut layout = TrackLayoutState::modular_default_layout();
        layout.slots[0].name = "Custom BD".to_string();
        layout.slots[0].routing.out_select = TrackAudioOut::Out(3);
        layout.slots[0].midi_note = 45;
        layout.move_slot(0, 3);

        assert_eq!(layout.slots[0].kind, TrackInstrumentKind::Snare);
        assert_eq!(layout.slots[1].kind, TrackInstrumentKind::HiHat);
        assert_eq!(layout.slots[2].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[3].kind, TrackInstrumentKind::Kick);
        assert_eq!(layout.slots[3].name, "Custom BD");
        assert_eq!(layout.slots[3].routing.out_select, TrackAudioOut::Out(3));
        assert_eq!(layout.slots[3].midi_note, 45);

        layout.move_slot(3, 1);
        assert_eq!(layout.slots[0].kind, TrackInstrumentKind::Snare);
        assert_eq!(layout.slots[1].kind, TrackInstrumentKind::Kick);
        assert_eq!(layout.slots[2].kind, TrackInstrumentKind::HiHat);
        assert_eq!(layout.slots[3].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[1].name, "Custom BD");
    }

    #[test]
    fn legacy_migration_maps_13_voices() {
        let layout = TrackLayoutState::from_legacy_13();
        assert_eq!(layout.active_count(), 13);
        for i in 0..13 {
            assert!(layout.slots[i].active);
        }
        assert!(!layout.slots[13].active);
        assert_eq!(layout.slots[4].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[5].kind, TrackInstrumentKind::Tom);
        assert_eq!(layout.slots[6].kind, TrackInstrumentKind::Tom);
    }

    #[test]
    fn atomic_roundtrip() {
        let layout = TrackLayoutState::from_legacy_13();
        let atomic = AtomicTrackLayout::from_state(&layout);
        assert_eq!(atomic.kind_for_slot(0), Some(TrackInstrumentKind::Kick));
        assert_eq!(atomic.kind_for_slot(1), Some(TrackInstrumentKind::Snare));
        assert_eq!(atomic.kind_for_slot(12), Some(TrackInstrumentKind::Perc1));
        assert_eq!(atomic.kind_for_slot(13), None);

        let modular = TrackLayoutState::default_layout();
        let atomic = AtomicTrackLayout::from_state(&modular);
        assert_eq!(atomic.kind_for_slot(3), Some(TrackInstrumentKind::Tom));
        assert_eq!(atomic.kind_for_slot(4), None);
    }

    #[test]
    fn persistent_track_layout_preserves_custom_names() {
        let mut layout = TrackLayoutState::default_layout();
        layout.slots[0].name = "Custom BD".to_string();
        layout.slots[1].name = "Layered Snare".to_string();

        let persistent = PersistentTrackLayout::new();
        persistent.set(layout.clone());

        persistent.map(|state| {
            assert_eq!(state.slots[0].name, "Custom BD");
            assert_eq!(state.slots[1].name, "Layered Snare");
            assert_eq!(state.slots[0].kind, TrackInstrumentKind::Kick);
            assert_eq!(state.slots[1].kind, TrackInstrumentKind::Snare);
        });
    }

    #[test]
    fn legacy_routing_without_choke_group_migrates_hats_to_group_1() {
        // Old track-layout-v1 blobs have no choke_group field.
        let legacy: TrackRouting =
            serde_json::from_str(r#"{"main_on":true,"out_select":"Main"}"#).unwrap();
        assert_eq!(legacy.choke_group, CHOKE_LEGACY);
        assert_eq!(
            effective_choke_group(TrackInstrumentKind::HiHat, &legacy),
            1
        );
        assert_eq!(
            effective_choke_group(TrackInstrumentKind::OpenHiHat, &legacy),
            1
        );
        assert_eq!(effective_choke_group(TrackInstrumentKind::Kick, &legacy), 0);
        // Explicit values are preserved (and clamped).
        let explicit = TrackRouting {
            main_on: true,
            out_select: TrackAudioOut::Main,
            choke_group: 3,
        };
        assert_eq!(
            effective_choke_group(TrackInstrumentKind::Kick, &explicit),
            3
        );
        let out_of_range = TrackRouting {
            choke_group: 9,
            ..explicit
        };
        assert_eq!(
            effective_choke_group(TrackInstrumentKind::Kick, &out_of_range),
            CHOKE_GROUP_COUNT
        );
    }

    #[test]
    fn routing_byte_packs_choke_group_and_output() {
        let decoded = decode_routing(routing_byte(true, 14, 4));
        assert!(decoded.main_on);
        assert_eq!(decoded.out_select, TrackAudioOut::Out(14));
        assert_eq!(decoded.choke_group, 4);

        let decoded = decode_routing(routing_byte(false, 0, 0));
        assert!(!decoded.main_on);
        assert_eq!(decoded.out_select, TrackAudioOut::Main);
        assert_eq!(decoded.choke_group, 0);
    }

    #[test]
    fn atomic_layout_exposes_normalized_choke_groups() {
        let mut layout = TrackLayoutState::modular_default_layout();
        layout.slots[2].routing.choke_group = CHOKE_LEGACY; // force legacy path
        let atomic = AtomicTrackLayout::from_state(&layout);
        // HiHat slot migrates to group 1, the others to none.
        assert_eq!(atomic.choke_group_for_slot(2), 1);
        assert_eq!(atomic.choke_group_for_slot(0), 0);
        assert_eq!(atomic.choke_group_for_slot(1), 0);
    }

    /// [193] Choosing an aux output drops the lane from the main mix, and going
    /// back to "No Aux" puts it back. The switch itself stays free: this is the
    /// default the assignment sets, not a lock.
    #[test]
    fn assigning_an_aux_output_takes_the_lane_out_of_the_main_mix() {
        let mut layout = TrackLayoutState::default_layout();
        assert!(
            layout.slots[0].routing.main_on,
            "a fresh lane is in the mix"
        );

        layout.assign_slot_output(0, TrackAudioOut::Out(3));
        assert!(
            !layout.slots[0].routing.main_on,
            "an aux-routed lane leaves the main mix"
        );

        // The user can still put it back by hand - nothing locks the switch.
        layout.slots[0].routing.main_on = true;
        assert_eq!(layout.slots[0].routing.out_select, TrackAudioOut::Out(3));

        layout.assign_slot_output(0, TrackAudioOut::Main);
        assert!(
            layout.slots[0].routing.main_on,
            "back to No Aux, the lane must be audible somewhere again"
        );
    }

    /// [194] Two lanes may share one aux output - the audio path sums them -
    /// where assigning used to steal it from whoever held it. The picker counts
    /// the sharers so it can say so.
    #[test]
    fn several_lanes_can_share_one_aux_output() {
        let mut layout = TrackLayoutState::default_layout();

        layout.assign_slot_output(2, TrackAudioOut::Out(2));
        layout.assign_slot_output(3, TrackAudioOut::Out(2));

        assert_eq!(layout.slots[2].routing.out_select, TrackAudioOut::Out(2));
        assert_eq!(layout.slots[3].routing.out_select, TrackAudioOut::Out(2));
        assert_eq!(
            layout.lanes_on_output(2),
            2,
            "the bus carries both lanes, whoever is asking"
        );
        assert_eq!(layout.lanes_on_output(5), 0, "nothing is on Out 5");

        // Freeing one lane leaves the other in place.
        layout.assign_slot_output(2, TrackAudioOut::Main);
        assert_eq!(layout.slots[3].routing.out_select, TrackAudioOut::Out(2));
        assert_eq!(layout.lanes_on_output(2), 1);
    }

    #[test]
    fn assigning_main_does_not_clear_other_outputs() {
        let mut layout = TrackLayoutState::default_layout();

        layout.assign_slot_output(2, TrackAudioOut::Out(2));
        layout.assign_slot_output(3, TrackAudioOut::Main);

        assert_eq!(layout.slots[2].routing.out_select, TrackAudioOut::Out(2));
        assert_eq!(layout.slots[3].routing.out_select, TrackAudioOut::Main);
    }

    /// [286] Effects live on the last two slots only, instruments elsewhere; an
    /// instrument that was already there (older sessions) may stay.
    #[test]
    fn effects_and_instruments_keep_to_their_own_slots() {
        let rv = TrackInstrumentKind::Reverb;
        let kick = TrackInstrumentKind::Kick;
        assert!(kind_allowed_on_slot(rv, 12, None));
        assert!(kind_allowed_on_slot(rv, 13, None));
        assert!(!kind_allowed_on_slot(rv, 11, None));
        assert!(!kind_allowed_on_slot(kick, 12, None));
        assert!(kind_allowed_on_slot(kick, 12, Some(TrackInstrumentKind::Snare)));
        assert!(!kind_allowed_on_slot(kick, 13, Some(rv)));
        assert!(kind_allowed_on_slot(kick, 0, None));
        assert!(is_fx_slot(12) && is_fx_slot(13) && !is_fx_slot(11));
    }

    /// [286] A saved layout from before the buses has no `fx_send`: it loads with
    /// no send, and the kind index of the effect is stable.
    #[test]
    fn old_layouts_load_with_no_send_and_the_reverb_index_is_stable() {
        let json = r#"{"active":true,"name":"Kick","kind":"Kick",
            "routing":{"main_on":true,"out_select":"Main"},"midi_note":36}"#;
        let slot: TrackSlot = serde_json::from_str(json).expect("old slot loads");
        assert_eq!(slot.fx_send, [0.0; FX_SLOT_COUNT]);
        assert_eq!(TrackInstrumentKind::Reverb.index(), 25);
        assert_eq!(TrackInstrumentKind::from_index(25), Some(TrackInstrumentKind::Reverb));
        assert_eq!(TrackInstrumentKind::Reverb.drum_voice_index(), 27);
    }

    /// [286] The delay is the second effect: its indices are frozen and it, like the
    /// reverb, lives on the effect slots only.
    #[test]
    fn the_delay_is_an_effect_with_a_stable_index() {
        let dl = TrackInstrumentKind::Delay;
        assert_eq!(dl.index(), 26);
        assert_eq!(TrackInstrumentKind::from_index(26), Some(dl));
        assert_eq!(dl.drum_voice_index(), 28);
        assert_eq!(TrackInstrumentKind::from_drum_voice_index(28), Some(dl));
        assert!(dl.is_effect() && TrackInstrumentKind::Reverb.is_effect());
        assert!(kind_allowed_on_slot(dl, 12, None) && !kind_allowed_on_slot(dl, 3, None));
        let in_menu: Vec<_> = TrackInstrumentKind::kinds_in(InstrumentCategory::Effect).collect();
        assert_eq!(in_menu, vec![TrackInstrumentKind::Reverb, dl, TrackInstrumentKind::Rumble]);
    }

    /// [286] The rumble is the third effect: frozen indices, effect slots only.
    #[test]
    fn the_rumble_is_an_effect_with_a_stable_index() {
        let ru = TrackInstrumentKind::Rumble;
        assert_eq!(ru.index(), 27);
        assert_eq!(TrackInstrumentKind::from_index(27), Some(ru));
        assert_eq!(ru.drum_voice_index(), 29);
        assert_eq!(TrackInstrumentKind::from_drum_voice_index(29), Some(ru));
        assert!(ru.is_effect());
        assert!(kind_allowed_on_slot(ru, 13, None) && !kind_allowed_on_slot(ru, 0, None));
    }

    /// [286] Sends survive the atomic round trip.
    #[test]
    fn sends_roundtrip_through_the_atomic_layout() {
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[2] = TrackSlot::active_with_kind(TrackInstrumentKind::HiHat);
        layout.slots[2].fx_send = [0.25, 0.75];
        layout.slots[12] = TrackSlot::active_with_kind(TrackInstrumentKind::Reverb);
        let atomic = AtomicTrackLayout::from_state(&layout);
        assert_eq!(atomic.fx_send(2, 0), 0.25);
        assert_eq!(atomic.fx_send(2, 1), 0.75);
        assert!(atomic.is_effect_bus(12));
        assert!(!atomic.is_effect_bus(13));
        assert!(!atomic.is_effect_bus(2));
    }

    /// [286] A bus listens to no MIDI note and never links to the lane above.
    #[test]
    fn an_effect_bus_neither_listens_nor_links() {
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[11] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        layout.slots[12] = TrackSlot::active_with_kind(TrackInstrumentKind::Reverb);
        layout.slots[12].midi_note = 36; // same note as the kick, on purpose
        assert!(!layout.can_link_up(12));
        let atomic = AtomicTrackLayout::from_state(&layout);
        assert_eq!(atomic.slots_listening_to(36), 1 << 11);
    }

    /// [286] A lane preset that has no bus of its own leaves the current buses.
    #[test]
    fn a_lane_preset_keeps_the_current_effect_buses() {
        let mut current = TrackLayoutState::modular_default_layout();
        current.slots[12] = TrackSlot::active_with_kind(TrackInstrumentKind::Reverb);
        current.slots[12].name = "Ma reverb".to_string();
        let mut incoming = TrackLayoutState::preset_12_layout();
        incoming.keep_globals_of(&current);
        assert_eq!(incoming.slots[12].kind, TrackInstrumentKind::Reverb);
        assert_eq!(incoming.slots[12].name, "Ma reverb");
        assert!(!incoming.slots[13].active);
    }

    /// [298] A new effect lane is named after its effect, and a lane saved as "Fx1" / "Fx2"
    /// takes the effect's name too, while a name the user typed stays.
    #[test]
    fn an_effect_lane_is_named_after_its_effect() {
        for kind in [
            TrackInstrumentKind::Reverb,
            TrackInstrumentKind::Delay,
            TrackInstrumentKind::Rumble,
        ] {
            assert_eq!(TrackSlot::active_with_kind(kind).name, kind.default_name());
        }
        let mut layout = TrackLayoutState::empty_layout();
        layout.slots[12] = TrackSlot::active_with_kind(TrackInstrumentKind::Reverb);
        layout.slots[12].name = "Fx1".to_string();
        layout.slots[13] = TrackSlot::active_with_kind(TrackInstrumentKind::Rumble);
        layout.slots[13].name = "Fx2".to_string();
        layout.slots[0] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        layout.slots[0].name = "Fx1".to_string(); // an instrument lane keeps whatever it was called
        assert!(layout.migrate_fx_names());
        assert_eq!(layout.slots[12].name, "Reverb");
        assert_eq!(layout.slots[13].name, "Rumble");
        assert_eq!(layout.slots[0].name, "Fx1");
        layout.slots[12].name = "Ma reverb".to_string();
        assert!(!layout.migrate_fx_names(), "a typed name is kept");
        assert_eq!(layout.slots[12].name, "Ma reverb");
    }
}
