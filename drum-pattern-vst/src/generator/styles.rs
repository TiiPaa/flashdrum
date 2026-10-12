//! Style definitions for musical pattern generation.
//! Each style provides *structured* rhythmic roles rather than raw probabilities.

use crate::sequencer::pattern::{Pattern, INSTRUMENT_COUNT, STEP_COUNT};
use nih_plug::prelude::*;

/// Available musical styles.
#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Style {
    #[id = "rock"]
    #[name = "Rock"]
    #[default]
    Rock,
    #[id = "funk"]
    #[name = "Funk"]
    Funk,
    #[id = "techno"]
    #[name = "Techno"]
    Techno,
    #[id = "hiphop"]
    #[name = "Hip-Hop"]
    HipHop,
    #[id = "jazz"]
    #[name = "Jazz"]
    Jazz,
    #[id = "metal"]
    #[name = "Metal"]
    Metal,
    #[id = "latin"]
    #[name = "Latin"]
    Latin,
    #[id = "disco"]
    #[name = "Disco"]
    Disco,
    #[id = "trap"]
    #[name = "Trap"]
    Trap,
    #[id = "reggae"]
    #[name = "Reggae"]
    Reggae,
    #[id = "bossa"]
    #[name = "Bossa Nova"]
    BossaNova,
    #[id = "house"]
    #[name = "House"]
    House,
    #[id = "dnb"]
    #[name = "Drum'n'Bass"]
    DrumAndBass,
    #[id = "afrobeat"]
    #[name = "Afrobeat"]
    Afrobeat,
    #[id = "dub"]
    #[name = "Dub"]
    Dub,
    #[id = "breakbeat"]
    #[name = "Breakbeat"]
    Breakbeat,
}

impl Style {
    pub fn label(&self) -> &'static str {
        match self {
            Style::Rock => "Rock",
            Style::Funk => "Funk",
            Style::Techno => "Techno",
            Style::HipHop => "Hip-Hop",
            Style::Jazz => "Jazz",
            Style::Metal => "Metal",
            Style::Latin => "Latin",
            Style::Disco => "Disco",
            Style::Trap => "Trap",
            Style::Reggae => "Reggae",
            Style::BossaNova => "Bossa Nova",
            Style::House => "House",
            Style::DrumAndBass => "Drum'n'Bass",
            Style::Afrobeat => "Afrobeat",
            Style::Dub => "Dub",
            Style::Breakbeat => "Breakbeat",
        }
    }
}

/// Rhythmic role for an instrument within a style.
/// Describes *where* an instrument tends to play, not just probabilities.
#[derive(Clone, Debug)]
pub struct RhythmicRole {
    /// Steps where the instrument MUST play (foundational hits).
    pub anchors: &'static [usize],
    /// Steps where the instrument MAY play (subdivisions, ghost notes).
    pub candidates: &'static [usize],
    /// Probability of playing at a candidate step (0.0..1.0).
    pub candidate_prob: f32,
    /// Steps where the instrument should NEVER play (exclusion zones).
    pub exclusions: &'static [usize],
}

/// A musical template defines the structured roles for all instruments.
pub struct MusicalTemplate {
    pub roles: [RhythmicRole; INSTRUMENT_COUNT],
    /// Typical BPM range for this style.
    pub bpm_range: (f32, f32),
}

impl MusicalTemplate {
    pub fn for_style(style: Style) -> Self {
        match style {
            Style::Rock => Self::rock(),
            Style::Funk => Self::funk(),
            Style::Techno => Self::techno(),
            Style::HipHop => Self::hiphop(),
            Style::Jazz => Self::jazz(),
            Style::Metal => Self::metal(),
            Style::Latin => Self::latin(),
            Style::Disco => Self::disco(),
            Style::Trap => Self::trap(),
            Style::Reggae => Self::reggae(),
            Style::BossaNova => Self::bossa_nova(),
            Style::House => Self::house(),
            Style::DrumAndBass => Self::drum_and_bass(),
            Style::Afrobeat => Self::afrobeat(),
            Style::Dub => Self::dub(),
            Style::Breakbeat => Self::breakbeat(),
        }
    }

    fn rock() -> Self {
        Self {
            roles: [
                // Kick: on 1,3 + optional syncopation
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Snare: backbeat on 2,4
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.1,
                    exclusions: &[0, 8],
                },
                // HiHat: steady 8ths with light 16th ghost notes
                RhythmicRole {
                    anchors: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidates: &[1, 3, 5, 7, 9, 11, 13, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Open HH: offbeats occasionally
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.25,
                    exclusions: &[0, 4, 8, 12],
                },
                // Toms: fill territory only
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Snare 606: tight backbeat layer, lighter than main snare
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 6, 10],
                    candidate_prob: 0.35,
                    exclusions: &[0, 8],
                },
                // 808 Kick: sub-bass reinforcement on downbeats only
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // Perc1: crash/FX accents and downbeat emphasis
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15, 7, 11],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (90.0, 140.0),
        }
    }

    fn funk() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[3, 6, 10, 12],
                    candidate_prob: 0.5,
                    exclusions: &[1, 2, 4, 5, 7, 9, 11, 13, 14, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[7, 14],
                    candidate_prob: 0.35,
                    exclusions: &[0, 8],
                },
                // HiHat: offbeat 8ths + 16th ghost notes
                RhythmicRole {
                    anchors: &[1, 3, 5, 7, 9, 11, 13, 15],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.25,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.4,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.25,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (90.0, 130.0),
        }
    }

    fn techno() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 4, 8, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.1,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[0, 8],
                    candidate_prob: 0.05,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                // HiHat: machine 16ths
                RhythmicRole {
                    anchors: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.3,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (120.0, 140.0),
        }
    }

    fn hiphop() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[6, 10, 12, 14],
                    candidate_prob: 0.35,
                    exclusions: &[1, 2, 3, 4, 5, 7, 9, 11, 13, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[0, 8],
                },
                // HiHat: sparse swung hats
                RhythmicRole {
                    anchors: &[2, 6, 10, 14],
                    candidates: &[0, 4, 8, 12, 1, 5, 9, 13],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 7, 11, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (80.0, 100.0),
        }
    }

    fn jazz() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12],
                    candidate_prob: 0.2,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.1,
                    exclusions: &[0, 8],
                },
                // HiHat: ride 8ths with skip-beat accents
                RhythmicRole {
                    anchors: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidates: &[3, 7, 11, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.25,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (100.0, 160.0),
        }
    }

    fn metal() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidates: &[1, 3, 5, 7, 9, 11, 13, 15],
                    candidate_prob: 0.4,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[0, 8],
                    candidate_prob: 0.25,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                // HiHat: blast-beat 16ths
                RhythmicRole {
                    anchors: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.2,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.3,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.25,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (140.0, 200.0),
        }
    }

    fn latin() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[3, 6, 10, 12],
                    candidate_prob: 0.4,
                    exclusions: &[1, 2, 4, 5, 7, 9, 11, 13, 14, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[7, 15],
                    candidate_prob: 0.3,
                    exclusions: &[0, 8],
                },
                // HiHat: syncopated clave-like pattern
                RhythmicRole {
                    anchors: &[0, 3, 6, 10, 12, 15],
                    candidates: &[2, 4, 8, 11, 14],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.25,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.25,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (100.0, 140.0),
        }
    }

    fn disco() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 4, 8, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.1,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[0, 8],
                    candidate_prob: 0.05,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                // HiHat: four-on-the-floor 16ths
                RhythmicRole {
                    anchors: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[3, 7, 11, 15],
                    candidates: &[1, 5, 9, 13],
                    candidate_prob: 0.15,
                    exclusions: &[0, 2, 4, 6, 8, 10, 12, 14],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (110.0, 130.0),
        }
    }

    fn trap() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[7, 10, 12],
                    candidate_prob: 0.5,
                    exclusions: &[1, 2, 3, 4, 5, 6, 9, 11, 13, 14, 15],
                },
                RhythmicRole {
                    anchors: &[8],
                    candidates: &[4, 12],
                    candidate_prob: 0.3,
                    exclusions: &[0, 1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                // HiHat: dense 16ths / rolls
                RhythmicRole {
                    anchors: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidates: &[1, 3, 5, 7, 9, 11, 13, 15],
                    candidate_prob: 0.55,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12],
                    candidate_prob: 0.25,
                    exclusions: &[0, 8],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (130.0, 160.0),
        }
    }

    fn reggae() -> Self {
        Self {
            roles: [
                RhythmicRole {
                    anchors: &[8],
                    candidates: &[0, 4, 12],
                    candidate_prob: 0.2,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[0, 8],
                    candidate_prob: 0.1,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                // HiHat: one-drop sparse
                RhythmicRole {
                    anchors: &[2, 6, 10, 14],
                    candidates: &[0, 4, 8, 12],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.2,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // Clap
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // Ride
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Cymbal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // Snare 606 (no auto-generation — user-only)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 808 Kick: plays like a secondary kick
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[4, 12, 2, 6, 10, 14],
                    candidate_prob: 0.15,
                    exclusions: &[1, 3, 5, 7, 9, 11, 13, 15],
                },
                // Perc1: occasional fill / effect
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // Extra slot: sparse, style-agnostic filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (70.0, 90.0),
        }
    }

    fn bossa_nova() -> Self {
        Self {
            roles: [
                // 0 Kick: surdo — 1 & 3 with syncopated pickups
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[3, 6, 11, 14],
                    candidate_prob: 0.4,
                    exclusions: &[1, 5, 9, 13],
                },
                // 1 Snare: cross-stick 3-2 son clave
                RhythmicRole {
                    anchors: &[0, 3, 6, 10, 12],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 2 HiHat: brushed quarter pulse + offbeat lift
                RhythmicRole {
                    anchors: &[0, 4, 8, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.3,
                    exclusions: &[],
                },
                // 3 Open HH: rare
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14],
                    candidate_prob: 0.1,
                    exclusions: &[0, 4, 8, 12],
                },
                // 4-6 Toms: light fills only
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.12,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14, 15],
                    candidate_prob: 0.08,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // 7 Clap: none
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 8 Ride: steady comp
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.15,
                    exclusions: &[],
                },
                // 9 Cymbal: sparse accent
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // 10 Snare 606: light clave echo
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 10],
                    candidate_prob: 0.25,
                    exclusions: &[0, 8],
                },
                // 11 808 Kick: sub on downbeats
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 12 Perc1: shaker / agogô
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 4, 6, 8, 10, 12, 14],
                    candidate_prob: 0.35,
                    exclusions: &[],
                },
                // 13 Extra: sparse filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (120.0, 140.0),
        }
    }

    fn house() -> Self {
        Self {
            roles: [
                // 0 Kick: four-on-the-floor
                RhythmicRole {
                    anchors: &[0, 4, 8, 12],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                },
                // 1 Snare: backbeat
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[0, 8],
                },
                // 2 HiHat: closed 16th ghosts (offbeat open hat carries the pulse)
                RhythmicRole {
                    anchors: &[],
                    candidates: &[1, 3, 5, 7, 9, 11, 13, 15],
                    candidate_prob: 0.5,
                    exclusions: &[],
                },
                // 3 Open HH: classic offbeat open hat
                RhythmicRole {
                    anchors: &[2, 6, 10, 14],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[0, 4, 8, 12],
                },
                // 4-6 Toms: minimal
                RhythmicRole {
                    anchors: &[],
                    candidates: &[15],
                    candidate_prob: 0.08,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[15],
                    candidate_prob: 0.06,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[15],
                    candidate_prob: 0.05,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // 7 Clap: layered on backbeat
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 8 Ride: sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 8],
                    candidate_prob: 0.08,
                    exclusions: &[],
                },
                // 9 Cymbal: crash on 1
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // 10 Snare 606: none
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 11 808 Kick: sub reinforcement
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 12 Perc1: offbeat percussion
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 13 Extra: sparse filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (118.0, 128.0),
        }
    }

    fn drum_and_bass() -> Self {
        Self {
            roles: [
                // 0 Kick: two-step — 1 and the "& of 3"
                RhythmicRole {
                    anchors: &[0, 10],
                    candidates: &[6, 8],
                    candidate_prob: 0.3,
                    exclusions: &[4, 12],
                },
                // 1 Snare: 2 and 4 with ghost pickups
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[7, 10, 15],
                    candidate_prob: 0.3,
                    exclusions: &[0],
                },
                // 2 HiHat: rolling 16th ride
                RhythmicRole {
                    anchors: &[0, 4, 8, 12],
                    candidates: &[1, 2, 3, 5, 6, 7, 9, 10, 11, 13, 14, 15],
                    candidate_prob: 0.45,
                    exclusions: &[],
                },
                // 3 Open HH: offbeat lifts
                RhythmicRole {
                    anchors: &[],
                    candidates: &[6, 14],
                    candidate_prob: 0.2,
                    exclusions: &[0, 8],
                },
                // 4-6 Toms: amen-style fills
                RhythmicRole {
                    anchors: &[],
                    candidates: &[7, 11, 15],
                    candidate_prob: 0.2,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 13],
                    candidate_prob: 0.15,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[9, 15],
                    candidate_prob: 0.12,
                    exclusions: &[0, 4, 8, 12],
                },
                // 7 Clap: reinforce snare
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 8 Ride: sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 8],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // 9 Cymbal: crash on 1
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // 10 Snare 606: ghost snares
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 7, 11, 15],
                    candidate_prob: 0.3,
                    exclusions: &[0, 4, 8, 12],
                },
                // 11 808 Kick: sub-bass on kick anchors
                RhythmicRole {
                    anchors: &[0, 10],
                    candidates: &[6],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 12 Perc1: syncopated accents
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 11],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 13 Extra: sparse filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
            ],
            bpm_range: (160.0, 180.0),
        }
    }

    fn afrobeat() -> Self {
        Self {
            roles: [
                // 0 Kick: syncopated, tresillo-leaning
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[3, 6, 11, 14],
                    candidate_prob: 0.4,
                    exclusions: &[],
                },
                // 1 Snare / rim: cross accents
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[2, 7, 10, 14],
                    candidate_prob: 0.3,
                    exclusions: &[],
                },
                // 2 HiHat: busy 16th tresillo feel
                RhythmicRole {
                    anchors: &[0, 3, 6, 8, 11, 14],
                    candidates: &[1, 5, 9, 13],
                    candidate_prob: 0.3,
                    exclusions: &[],
                },
                // 3 Open HH: offbeat
                RhythmicRole {
                    anchors: &[],
                    candidates: &[6, 14],
                    candidate_prob: 0.3,
                    exclusions: &[0, 8],
                },
                // 4-6 Toms: conga-like
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 7, 11, 15],
                    candidate_prob: 0.25,
                    exclusions: &[0, 8],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[5, 13],
                    candidate_prob: 0.2,
                    exclusions: &[0, 8],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[1, 9],
                    candidate_prob: 0.15,
                    exclusions: &[0, 8],
                },
                // 7 Clap: offbeat claps
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 8 Ride: bell pattern
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 3, 6, 8, 11, 14],
                    candidate_prob: 0.3,
                    exclusions: &[],
                },
                // 9 Cymbal: sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0],
                    candidate_prob: 0.05,
                    exclusions: &[],
                },
                // 10 Snare 606: rim layer on offbeats
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.3,
                    exclusions: &[],
                },
                // 11 808 Kick: sub
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[3, 11],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 12 Perc1: percussion-forward
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 2, 3, 5, 6, 8, 10, 11, 13, 14],
                    candidate_prob: 0.4,
                    exclusions: &[],
                },
                // 13 Extra: filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
            ],
            bpm_range: (100.0, 125.0),
        }
    }

    fn dub() -> Self {
        Self {
            roles: [
                // 0 Kick: one-drop — accent on beat 3
                RhythmicRole {
                    anchors: &[8],
                    candidates: &[0, 11],
                    candidate_prob: 0.25,
                    exclusions: &[4, 12],
                },
                // 1 Snare: one-drop rim on 3, ghost near 4
                RhythmicRole {
                    anchors: &[8],
                    candidates: &[12],
                    candidate_prob: 0.2,
                    exclusions: &[0, 4],
                },
                // 2 HiHat: sparse skank on backbeat
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 3 Open HH: rare accents
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14],
                    candidate_prob: 0.15,
                    exclusions: &[0, 8],
                },
                // 4-6 Toms: dub fills, very sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[15],
                    candidate_prob: 0.12,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[15],
                    candidate_prob: 0.08,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[14],
                    candidate_prob: 0.06,
                    exclusions: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13],
                },
                // 7 Clap: none
                RhythmicRole {
                    anchors: &[],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 8 Ride: sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 8],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // 9 Cymbal: sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0],
                    candidate_prob: 0.08,
                    exclusions: &[],
                },
                // 10 Snare 606: rimshot on 3
                RhythmicRole {
                    anchors: &[],
                    candidates: &[8],
                    candidate_prob: 0.3,
                    exclusions: &[0, 4, 12],
                },
                // 11 808 Kick: heavy sub
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[3, 11],
                    candidate_prob: 0.3,
                    exclusions: &[],
                },
                // 12 Perc1: dub echoes
                RhythmicRole {
                    anchors: &[],
                    candidates: &[2, 10],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 13 Extra: very sparse filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.04,
                    exclusions: &[],
                },
            ],
            bpm_range: (60.0, 90.0),
        }
    }

    fn breakbeat() -> Self {
        Self {
            roles: [
                // 0 Kick: broken / funky, avoids the backbeat
                RhythmicRole {
                    anchors: &[0],
                    candidates: &[3, 6, 8, 10, 11],
                    candidate_prob: 0.4,
                    exclusions: &[4, 12],
                },
                // 1 Snare: 2 and 4 with syncopated hits
                RhythmicRole {
                    anchors: &[4, 12],
                    candidates: &[7, 10, 14, 15],
                    candidate_prob: 0.35,
                    exclusions: &[0],
                },
                // 2 HiHat: 8th ride with offbeat push
                RhythmicRole {
                    anchors: &[0, 4, 8, 12],
                    candidates: &[2, 6, 10, 14],
                    candidate_prob: 0.4,
                    exclusions: &[],
                },
                // 3 Open HH: offbeat accents
                RhythmicRole {
                    anchors: &[],
                    candidates: &[6, 14],
                    candidate_prob: 0.25,
                    exclusions: &[0, 8],
                },
                // 4-6 Toms: funky-drummer fills
                RhythmicRole {
                    anchors: &[],
                    candidates: &[7, 11, 15],
                    candidate_prob: 0.25,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 13],
                    candidate_prob: 0.2,
                    exclusions: &[0, 4, 8, 12],
                },
                RhythmicRole {
                    anchors: &[],
                    candidates: &[9, 15],
                    candidate_prob: 0.15,
                    exclusions: &[0, 4, 8, 12],
                },
                // 7 Clap: reinforce backbeat
                RhythmicRole {
                    anchors: &[],
                    candidates: &[4, 12],
                    candidate_prob: 0.2,
                    exclusions: &[],
                },
                // 8 Ride: sparse
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 8],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // 9 Cymbal: crash on 1
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0],
                    candidate_prob: 0.1,
                    exclusions: &[],
                },
                // 10 Snare 606: ghost snares
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 7, 11, 15],
                    candidate_prob: 0.35,
                    exclusions: &[0, 4, 12],
                },
                // 11 808 Kick: sub on downbeats
                RhythmicRole {
                    anchors: &[0, 8],
                    candidates: &[],
                    candidate_prob: 0.0,
                    exclusions: &[],
                },
                // 12 Perc1: syncopated accents
                RhythmicRole {
                    anchors: &[],
                    candidates: &[3, 7, 11],
                    candidate_prob: 0.25,
                    exclusions: &[],
                },
                // 13 Extra: filler
                RhythmicRole {
                    anchors: &[],
                    candidates: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15],
                    candidate_prob: 0.06,
                    exclusions: &[],
                },
            ],
            bpm_range: (120.0, 140.0),
        }
    }
}

/// Default of the Density slider (`gen_density`).
pub const DEFAULT_DENSITY: f32 = 0.7;

/// [283] Density → multiplier of a role's candidate odds: 0 keeps a few
/// candidates (sparse, but each GENERATE still differs), the 0.7 default keeps
/// the former feel, 1 makes the candidates about three times likelier.
pub fn density_gain(density: f32) -> f32 {
    const DEFAULT: f32 = DEFAULT_DENSITY;
    const GAIN_AT_DEFAULT: f32 = 0.79; // former 0.3 + 0.7 × density, at 0.7
    const GAIN_MIN: f32 = 0.35;
    const GAIN_MAX: f32 = 3.0;
    let d = density.clamp(0.0, 1.0);
    if d <= DEFAULT {
        GAIN_MIN + (GAIN_AT_DEFAULT - GAIN_MIN) * d / DEFAULT
    } else {
        GAIN_AT_DEFAULT + (GAIN_MAX - GAIN_AT_DEFAULT) * (d - DEFAULT) / (1.0 - DEFAULT)
    }
}

/// Odds that a candidate step plays at this density (capped so a busy pattern
/// still breathes).
pub fn candidate_odds(candidate_prob: f32, density: f32) -> f32 {
    (candidate_prob * density_gain(density)).min(0.9)
}

/// Generate a pattern from a musical template with density control.
/// Uses a two-pass approach: anchors first, then candidates with musical rules.
pub fn generate_from_template(
    template: &MusicalTemplate,
    density: f32,
    rng: &mut impl FnMut() -> f32,
) -> Pattern {
    let mut pattern = Pattern::empty();
    pattern.name = "Generated".to_string();
    let d = density.clamp(0.0, 1.0);

    // Track which hits are anchors (foundational) so the coherence pass never
    // removes them — anchors define the style's identity. A House kick on every
    // beat must survive even where the snare/clap doubles it on 2 & 4.
    let mut is_anchor = [[false; STEP_COUNT]; INSTRUMENT_COUNT];

    for inst in 0..INSTRUMENT_COUNT {
        let role = &template.roles[inst];

        // Pass 1: anchors are always placed (foundational hits)
        for &step in role.anchors {
            pattern.steps[step].instruments[inst] = true;
            is_anchor[inst][step] = true;
        }

        // Pass 2: candidates with probability scaled by density
        for &step in role.candidates {
            let prob = candidate_odds(role.candidate_prob, d);
            if rng() < prob {
                pattern.steps[step].instruments[inst] = true;
            }
        }
    }

    // Musical coherence pass: enforce exclusion zones (candidates only — an
    // anchor never lands in its own exclusion list).
    for inst in 0..INSTRUMENT_COUNT {
        let role = &template.roles[inst];
        for &step in role.exclusions {
            if !is_anchor[inst][step] {
                pattern.steps[step].instruments[inst] = false;
            }
        }
    }

    // Rule: avoid kick and snare stacking — but only when at least one is a
    // candidate. If BOTH are anchors the overlap is intentional (four-on-the-
    // floor kick under the backbeat) and must be kept.
    if d < 0.85 {
        for step in 0..STEP_COUNT {
            if pattern.steps[step].instruments[0] && pattern.steps[step].instruments[1] {
                let kick_anchor = is_anchor[0][step];
                let snare_anchor = is_anchor[1][step];
                if kick_anchor && snare_anchor {
                    continue; // intentional overlap — leave both
                }
                if snare_anchor {
                    pattern.steps[step].instruments[0] = false; // drop the candidate kick
                } else if kick_anchor {
                    pattern.steps[step].instruments[1] = false; // drop the candidate snare
                } else if step == 4 || step == 12 {
                    pattern.steps[step].instruments[0] = false; // keep snare on the backbeat
                } else {
                    pattern.steps[step].instruments[1] = false; // keep kick elsewhere
                }
            }
        }
    }

    // Rule: closed HH and open HH should not clash. Open HH wins, unless the
    // closed hat is the anchor and the open one only a candidate.
    for step in 0..STEP_COUNT {
        if pattern.steps[step].instruments[2] && pattern.steps[step].instruments[3] {
            if is_anchor[2][step] && !is_anchor[3][step] {
                pattern.steps[step].instruments[3] = false;
            } else {
                pattern.steps[step].instruments[2] = false;
            }
        }
    }

    // Density-based ghost note suppression for kick at low density (never
    // touches anchors or the main beats).
    if d < 0.4 {
        for step in 0..STEP_COUNT {
            if pattern.steps[step].instruments[0]
                && !is_anchor[0][step]
                && !(step == 0 || step == 8 || step == 4 || step == 12)
            {
                if rng() < 0.5 {
                    pattern.steps[step].instruments[0] = false;
                }
            }
        }
    }

    pattern
}

/// Interpolate between two templates.
pub fn mix_templates(a: &MusicalTemplate, b: &MusicalTemplate, t: f32) -> MusicalTemplate {
    let mix = t.clamp(0.0, 1.0);
    let mut roles = a.roles.clone();
    for inst in 0..INSTRUMENT_COUNT {
        // Interpolate candidate probabilities
        roles[inst].candidate_prob =
            a.roles[inst].candidate_prob * (1.0 - mix) + b.roles[inst].candidate_prob * mix;
    }
    MusicalTemplate {
        roles,
        bpm_range: (
            a.bpm_range.0 * (1.0 - mix) + b.bpm_range.0 * mix,
            a.bpm_range.1 * (1.0 - mix) + b.bpm_range.1 * mix,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_on_the_floor_kick_survives_backbeat_overlap() {
        // House: kick anchored on every beat, snare/clap on 2 & 4. The kick must
        // survive on steps 4 & 12 despite the snare overlap — the old coherence
        // rule stripped it, killing the four-on-the-floor feel.
        let template = MusicalTemplate::for_style(Style::House);
        let mut rng = || 1.0f32; // reject every candidate → anchors only
        let pattern = generate_from_template(&template, 0.5, &mut rng);
        for beat in [0usize, 4, 8, 12] {
            assert!(
                pattern.steps[beat].instruments[0],
                "House kick missing on beat step {beat}"
            );
        }
    }

    /// Guard for every style (existing + new): all role steps stay inside the
    /// 16-step page the generator fills, probabilities are in [0, 1], and the
    /// BPM range is sane. Catches hand-authored typos (a stray step index, a
    /// probability > 1) across all `MusicalTemplate`s.
    #[test]
    fn every_style_template_is_well_formed() {
        for i in 0..Style::variants().len() {
            let style = Style::from_index(i);
            let template = MusicalTemplate::for_style(style);
            for (inst, role) in template.roles.iter().enumerate() {
                for &step in role
                    .anchors
                    .iter()
                    .chain(role.candidates.iter())
                    .chain(role.exclusions.iter())
                {
                    assert!(
                        step < 16,
                        "style '{}' inst {} references step {} outside the 16-step page",
                        style.label(),
                        inst,
                        step
                    );
                }
                assert!(
                    (0.0..=1.0).contains(&role.candidate_prob),
                    "style '{}' inst {} candidate_prob {} out of [0, 1]",
                    style.label(),
                    inst,
                    role.candidate_prob
                );
            }
            assert!(
                template.bpm_range.0 > 0.0 && template.bpm_range.0 <= template.bpm_range.1,
                "style '{}' has an invalid bpm range {:?}",
                style.label(),
                template.bpm_range
            );
        }
    }
}
