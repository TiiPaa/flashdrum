//! Classic pattern generator (Technique D).
//! Starts from iconic hardcoded patterns, then applies musically-aware variations.

use super::styles::{density_gain, MusicalTemplate, Style, DEFAULT_DENSITY};
use crate::sequencer::pattern::{Pattern, INSTRUMENT_COUNT, STEP_COUNT};

fn pattern_amen() -> Pattern {
    let mut p = Pattern::empty();
    p.name = "Amen-ish".to_string();
    for &s in &[0, 3, 6, 10] {
        p.steps[s].instruments[0] = true;
    }
    for &s in &[4, 7, 12, 14] {
        p.steps[s].instruments[1] = true;
    }
    for &s in &[0, 2, 4, 6, 8, 10, 12, 14] {
        p.steps[s].instruments[2] = true;
    }
    p
}

fn pattern_funky_drummer() -> Pattern {
    let mut p = Pattern::empty();
    p.name = "Funky Drummer-ish".to_string();
    for &s in &[0, 5, 8, 11] {
        p.steps[s].instruments[0] = true;
    }
    for &s in &[4, 7, 12, 15] {
        p.steps[s].instruments[1] = true;
    }
    for s in 0..STEP_COUNT {
        p.steps[s].instruments[2] = true;
    }
    p
}

fn pattern_bossa_nova() -> Pattern {
    let mut p = Pattern::empty();
    p.name = "Bossa-ish".to_string();
    for &s in &[0, 9] {
        p.steps[s].instruments[0] = true;
    }
    for &s in &[4, 12] {
        p.steps[s].instruments[1] = true;
    }
    for &s in &[0, 2, 4, 6, 8, 10, 12, 14] {
        p.steps[s].instruments[2] = true;
    }
    p
}

fn pattern_motorik() -> Pattern {
    let mut p = Pattern::empty();
    p.name = "Motorik-ish".to_string();
    for &s in &[0, 4, 8, 12] {
        p.steps[s].instruments[0] = true;
    }
    for &s in &[4, 12] {
        p.steps[s].instruments[1] = true;
    }
    for s in 0..STEP_COUNT {
        p.steps[s].instruments[2] = true;
    }
    p
}

fn pattern_shuffle() -> Pattern {
    let mut p = Pattern::empty();
    p.name = "Shuffle-ish".to_string();
    for &s in &[0, 6, 8] {
        p.steps[s].instruments[0] = true;
    }
    for &s in &[4, 12] {
        p.steps[s].instruments[1] = true;
    }
    for &s in &[0, 2, 4, 6, 8, 10, 12, 14] {
        p.steps[s].instruments[2] = true;
    }
    p
}

fn all_classics() -> Vec<Pattern> {
    vec![
        pattern_amen(),
        pattern_funky_drummer(),
        pattern_bossa_nova(),
        pattern_motorik(),
        pattern_shuffle(),
    ]
}

pub fn generate(
    style: Style,
    density: f32,
    variation: f32,
    rng: &mut impl FnMut() -> f32,
) -> Pattern {
    let classics = all_classics();
    let idx = (rng() * classics.len() as f32) as usize % classics.len();
    let mut p = classics[idx].clone();
    let var = variation.clamp(0.0, 1.0);
    let template = MusicalTemplate::for_style(style);

    // [283] Density, whatever the Variation: below the default the groove
    // thins out to its hits on the beats; above it, the style's candidate
    // hits come on top.
    let gain = density_gain(density);
    let default_gain = density_gain(DEFAULT_DENSITY);
    let keep = (gain / default_gain).min(1.0);
    for step in 0..STEP_COUNT {
        for inst in 0..INSTRUMENT_COUNT {
            if p.steps[step].instruments[inst] && step % 4 != 0 && rng() >= keep {
                p.steps[step].instruments[inst] = false;
            }
        }
    }
    let extra = (gain - default_gain).max(0.0);
    if extra > 0.0 {
        for (inst, role) in template.roles.iter().enumerate() {
            for &step in role.candidates {
                if rng() < (role.candidate_prob * extra).min(0.9) {
                    p.steps[step].instruments[inst] = true;
                }
            }
        }
    }

    // Musically-aware variation
    for step in 0..STEP_COUNT {
        for inst in 0..INSTRUMENT_COUNT {
            let was_active = p.steps[step].instruments[inst];
            if rng() < var {
                let turn_on_prob = if was_active {
                    0.3 * (1.0 - density)
                } else {
                    0.3 * density
                };
                p.steps[step].instruments[inst] = rng() < turn_on_prob;
            }
        }
    }

    // Re-apply musical rules from template
    for inst in 0..INSTRUMENT_COUNT {
        let role = &template.roles[inst];
        for &step in role.exclusions {
            p.steps[step].instruments[inst] = false;
        }
        for &step in role.anchors {
            p.steps[step].instruments[inst] = true;
        }
    }

    // Kick/snare exclusion
    for step in 0..STEP_COUNT {
        if p.steps[step].instruments[0] && p.steps[step].instruments[1] {
            if step == 4 || step == 12 {
                p.steps[step].instruments[0] = false;
            } else {
                p.steps[step].instruments[1] = false;
            }
        }
    }

    p
}
