//! Special parameter definitions per instrument.
//!
//! Each instrument declares the synthesis algorithms it supports.
//! The UI editor queries these definitions to build dynamic dropdowns.
//! The audio thread reads `VoiceSettings.special[index]` directly — no
//! UI metadata travels with it.

/// Description of a synthesis algorithm for UI display.
/// The position in the slice is the algorithm index used by the voice.
#[derive(Clone, Copy, Debug)]
pub struct AlgoDef {
    pub name: &'static str,
}

// ── Kick ────────────────────────────────────────────────────────────────────

pub const KICK_ALGOS: &[AlgoDef] = &[
    AlgoDef { name: "Sine" },
    AlgoDef { name: "Square" },
    AlgoDef { name: "FM" },
];

// ── Snare ───────────────────────────────────────────────────────────────────

pub const SNARE_ALGOS: &[AlgoDef] = &[
    AlgoDef { name: "Synth" },
    AlgoDef { name: "Noise" },
    AlgoDef { name: "Layered" },
];

// ── Snare 606 ───────────────────────────────────────────────────────────────

pub const SNARE606_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── HiHat (closed & open share the same algo set) ───────────────────────────

pub const HIHAT_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── Tom ─────────────────────────────────────────────────────────────────────

pub const TOM_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }, AlgoDef { name: "Deep" }];

// ── Clap ────────────────────────────────────────────────────────────────────

pub const CLAP_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }, AlgoDef { name: "Tight" }];

// ── Ride ────────────────────────────────────────────────────────────────────

pub const RIDE_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }, AlgoDef { name: "Bell" }];

// ── Cymbal ──────────────────────────────────────────────────────────────────

pub const CYMBAL_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── 808 Bass Drum ───────────────────────────────────────────────────────────

pub const BASSDRUM808_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── Perc1 ───────────────────────────────────────────────────────────────────

pub const PERC1_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Sine" }, AlgoDef { name: "Saw" }];

// ── BD606 (multisample) ─────────────────────────────────────────────────────

pub const BD606_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── SD606 (multisample) ─────────────────────────────────────────────────────

pub const SD606_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];
pub const CH606_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// -- Rift [221] ---------------------------------------------------------------

pub const RIFT_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];
pub const ONE_SHOT_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── Buzz (tonal perc + gate) ────────────────────────────────────────────────

/// Gate retrigger mode: Smooth = ramp from current, Razor = snap from zero.
pub const BUZZ_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Smooth" }, AlgoDef { name: "Razor" }];

pub const SDREX_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── AC606 voices (ported analogcode engines) ────────────────────────────────

pub const AC606_ALGOS: &[AlgoDef] = &[AlgoDef { name: "Standard" }];

// ── Registry helpers ────────────────────────────────────────────────────────

use super::DrumVoice;

/// Returns the algo definitions for a given drum voice.
pub fn algos_for(voice: DrumVoice) -> &'static [AlgoDef] {
    match voice {
        DrumVoice::Kick => KICK_ALGOS,
        DrumVoice::Snare => SNARE_ALGOS,
        DrumVoice::HiHat => HIHAT_ALGOS,
        DrumVoice::OpenHiHat => HIHAT_ALGOS,
        DrumVoice::Tom1 => TOM_ALGOS,
        DrumVoice::Tom2 => TOM_ALGOS,
        DrumVoice::Tom3 => TOM_ALGOS,
        DrumVoice::Clap => CLAP_ALGOS,
        DrumVoice::Ride => RIDE_ALGOS,
        DrumVoice::Cymbal => CYMBAL_ALGOS,
        DrumVoice::Snare606 => SNARE606_ALGOS,
        DrumVoice::BassDrum808 => BASSDRUM808_ALGOS,
        DrumVoice::Perc1 => PERC1_ALGOS,
        DrumVoice::Bd606 => BD606_ALGOS,
        DrumVoice::Sd606 => SD606_ALGOS,
        DrumVoice::Ch606 => CH606_ALGOS,
        // [208] Same sampler engine as the closed hat, same single algo.
        DrumVoice::Oh606 => CH606_ALGOS,
        DrumVoice::Rift => RIFT_ALGOS,
        DrumVoice::OneShot => ONE_SHOT_ALGOS,
        DrumVoice::Buzz => BUZZ_ALGOS,
        DrumVoice::Sdrex => SDREX_ALGOS,
        DrumVoice::Bd6Ac
        | DrumVoice::Sd6Ac
        | DrumVoice::Hh6Ac
        | DrumVoice::Oh6Ac
        | DrumVoice::Cl6Ac
        | DrumVoice::Tm6Ac => AC606_ALGOS,
        // [286] An effect, not a synthesis voice: a single "Standard" entry.
        DrumVoice::Reverb | DrumVoice::Delay => AC606_ALGOS,
    }
}
