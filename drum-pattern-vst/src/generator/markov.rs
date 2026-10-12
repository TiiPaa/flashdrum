//! Markov-chain pattern generator (Technique B).
//! Uses musical templates as transition priors for coherent rhythmic evolution.

use super::styles::{candidate_odds, density_gain, MusicalTemplate, Style};
use crate::sequencer::pattern::{Pattern, INSTRUMENT_COUNT, STEP_COUNT};

pub fn generate(style: Style, density: f32, rng: &mut impl FnMut() -> f32) -> Pattern {
    let template = MusicalTemplate::for_style(style);

    let mut pattern = Pattern::empty();
    pattern.name = "Markov".to_string();

    for inst in 0..INSTRUMENT_COUNT {
        let role = &template.roles[inst];
        let mut state = false;

        // Initialize from template's anchors probability
        let anchor_count = role.anchors.len();
        // ([283] scaled by density: at 0 only the anchors play).
        if anchor_count > 0 {
            state =
                rng() < (anchor_count as f32 / STEP_COUNT as f32) * density_gain(density).min(1.0);
        }

        // Transition probabilities derived from template
        let p_on = candidate_odds(role.candidate_prob, density);
        let p_stay = 0.3 + 0.5 * p_on;

        for step in 0..STEP_COUNT {
            pattern.steps[step].instruments[inst] = state;
            if state {
                state = rng() < p_stay.clamp(0.0, 1.0);
            } else {
                state = rng() < p_on.clamp(0.0, 1.0);
            }
        }
    }

    // Re-apply musical rules from template
    for inst in 0..INSTRUMENT_COUNT {
        let role = &template.roles[inst];
        for &step in role.exclusions {
            pattern.steps[step].instruments[inst] = false;
        }
        for &step in role.anchors {
            pattern.steps[step].instruments[inst] = true;
        }
    }

    // Enforce coherence: kick/snare exclusion
    for step in 0..STEP_COUNT {
        if pattern.steps[step].instruments[0] && pattern.steps[step].instruments[1] {
            if step == 4 || step == 12 {
                pattern.steps[step].instruments[0] = false;
            } else {
                pattern.steps[step].instruments[1] = false;
            }
        }
    }

    pattern
}
