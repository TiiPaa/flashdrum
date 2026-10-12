//! Euclidean rhythm generator (Technique C).
//! Uses the Bjorklund algorithm, then applies musical coherence rules.

use super::styles::{MusicalTemplate, Style};
use crate::sequencer::pattern::{Pattern, INSTRUMENT_COUNT, STEP_COUNT};

fn bjorklund(hits: usize, steps: usize) -> Vec<bool> {
    if hits == 0 || steps == 0 {
        return vec![false; steps];
    }
    if hits >= steps {
        return vec![true; steps];
    }

    let mut groups: Vec<Vec<bool>> = (0..hits).map(|_| vec![true]).collect();
    let mut remainders: Vec<Vec<bool>> = (0..(steps - hits)).map(|_| vec![false]).collect();

    while remainders.len() > 1 {
        let mut new_groups = Vec::new();
        let min_len = groups.len().min(remainders.len());
        for i in 0..min_len {
            let mut combined = groups[i].clone();
            combined.extend_from_slice(&remainders[i]);
            new_groups.push(combined);
        }
        if groups.len() > remainders.len() {
            new_groups.extend(groups.into_iter().skip(min_len));
            remainders = remainders.into_iter().skip(min_len).collect();
        } else {
            new_groups.extend(remainders.into_iter().skip(min_len));
            remainders = groups.into_iter().skip(min_len).collect();
        }
        groups = new_groups;
    }

    let mut result = Vec::new();
    for g in &groups {
        result.extend_from_slice(g);
    }
    for g in &remainders {
        result.extend_from_slice(g);
    }
    result.resize(steps, false);
    result
}

/// Hits per role over the 64 steps at full density; [283] density scales them
/// down to a fifth (sparse, never empty).
const MAX_HITS: [usize; INSTRUMENT_COUNT] = [
    12, // Kick
    6,  // Snare
    32, // HiHat
    8,  // Open HiHat
    4,  // Tom 1
    3,  // Tom 2
    3,  // Tom 3
    4,  // Clap
    24, // Ride
    2,  // Cymbal
    0,  // Snare 606 — user-only, no auto-generation
    12, // 808 Kick
    6,  // Perc1
    4,  // Extra slot
];

fn euclidean_params(density: f32) -> [(usize, usize); INSTRUMENT_COUNT] {
    let d = density.clamp(0.0, 1.0);
    let fill = 0.2 + 0.8 * d;
    MAX_HITS.map(|max| ((max as f32 * fill).round() as usize, STEP_COUNT))
}

pub fn generate(
    style: Style,
    density: f32,
    rotation: &[usize; INSTRUMENT_COUNT],
    rng: &mut impl FnMut() -> f32,
) -> Pattern {
    let mut pattern = Pattern::empty();
    pattern.name = "Euclidean".to_string();
    let template = MusicalTemplate::for_style(style);

    let params = euclidean_params(density);

    for inst in 0..INSTRUMENT_COUNT {
        let (hits, steps) = params[inst];
        let mut row = bjorklund(hits, steps);
        let rot = rotation.get(inst).copied().unwrap_or(0) % steps.max(1);
        if rot > 0 {
            row.rotate_left(rot);
        }

        // Style variation: drop a few hits and add a few, more as the density
        // rises ([283]: added hits used to outweigh the density).
        let drop = 0.08;
        let add = 0.02 + 0.04 * density.clamp(0.0, 1.0);
        for step in 0..STEP_COUNT {
            let active = row[step % steps];
            pattern.steps[step].instruments[inst] = if active { rng() > drop } else { rng() < add };
        }
    }

    // Post-process: apply musical coherence rules from template, on every bar
    // (the template describes one 16-step bar; [283] only the first got them).
    for bar in (0..STEP_COUNT).step_by(16) {
        for inst in 0..INSTRUMENT_COUNT {
            let role = &template.roles[inst];
            for &step in role.exclusions {
                pattern.steps[bar + step].instruments[inst] = false;
            }
            for &step in role.anchors {
                pattern.steps[bar + step].instruments[inst] = true;
            }
        }
    }

    // Kick/snare exclusion
    for step in 0..STEP_COUNT {
        if pattern.steps[step].instruments[0] && pattern.steps[step].instruments[1] {
            if step % 16 == 4 || step % 16 == 12 {
                pattern.steps[step].instruments[0] = false;
            } else {
                pattern.steps[step].instruments[1] = false;
            }
        }
    }

    pattern
}
