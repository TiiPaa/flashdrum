//! Pattern generation module with 4 techniques:
//! A) Probabilistic style-based generation
//! B) Markov-chain generation
//! C) Euclidean rhythm generation
//! D) Classic pattern + variation generation

use nih_plug::prelude::*;

pub mod classic;
pub mod euclidean;
pub mod markov;
pub mod probabilistic;
pub mod styles;

pub use styles::Style;

use crate::sequencer::pattern::{Pattern, INSTRUMENT_COUNT, STEP_COUNT};
use crate::track::{AtomicTrackLayout, TrackInstrumentKind, MAX_TRACKS};

#[derive(Enum, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GeneratorType {
    #[id = "prob"]
    #[name = "Probabilistic"]
    #[default]
    Probabilistic,
    #[id = "markov"]
    #[name = "Markov"]
    Markov,
    #[id = "euclid"]
    #[name = "Euclidean"]
    Euclidean,
    #[id = "classic"]
    #[name = "Classic"]
    Classic,
}

pub struct GeneratorParams {
    pub generator_type: GeneratorType,
    pub style_primary: Style,
    pub style_secondary: Style,
    pub style_mix: f32,
    pub density: f32,
    pub variation: f32,
    pub seed: u64,
}

/// Create a deterministic pseudo-RNG from a seed.
fn make_rng(seed: u64) -> impl FnMut() -> f32 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (state as f32) / (u64::MAX as f32)
    }
}

/// Generate a single 16-step bar.
fn generate_bar(params: &GeneratorParams, rng: &mut impl FnMut() -> f32) -> Pattern {
    match params.generator_type {
        GeneratorType::Probabilistic => probabilistic::generate(
            params.style_primary,
            params.style_secondary,
            params.style_mix,
            params.density,
            rng,
        ),
        GeneratorType::Markov => markov::generate(params.style_primary, params.density, rng),
        GeneratorType::Classic => {
            classic::generate(params.style_primary, params.density, params.variation, rng)
        }
        GeneratorType::Euclidean => {
            // Euclidean generates the full 64-step pattern in one pass, so it is
            // handled directly in `generate()`. Return an empty bar as a safe
            // fallback to keep this function total.
            Pattern::empty()
        }
    }
}

/// Generate a pattern using the selected technique and remap the legacy voice
/// roles onto the current track layout.
///
/// Non-Euclidean generators produce 4 distinct bars (variations).
/// Euclidean already generates varied 64 steps in one pass.
pub fn generate(params: &GeneratorParams, layout: &AtomicTrackLayout) -> Pattern {
    let mut rng = make_rng(params.seed);
    let role_pattern = match params.generator_type {
        GeneratorType::Euclidean => {
            // Default rotations: slightly offset per instrument for groove
            let rotations = [0, 0, 0, 2, 4, 6, 8, 0, 0, 0, 0, 0, 0, 0];
            euclidean::generate(params.style_primary, params.density, &rotations, &mut rng)
        }
        _ => {
            let mut pattern = Pattern::empty();
            for bar in 0..4 {
                let bar_pattern = generate_bar(params, &mut rng);
                for step in 0..16 {
                    for inst in 0..INSTRUMENT_COUNT {
                        pattern.steps[bar * 16 + step].instruments[inst] =
                            bar_pattern.steps[step].instruments[inst];
                    }
                }
            }
            pattern
        }
    };

    remap_roles_to_slots(&role_pattern, layout, params.variation, &mut rng)
}

/// Remap a pattern generated for legacy voice roles into the current slot
/// layout. For duplicate slots of the same kind, apply variations so each
/// duplicate is not a carbon copy.
///
/// Mapping rules:
/// - Each active slot receives the generator role that matches its
///   `TrackInstrumentKind::drum_voice_index()`.
/// - The three legacy Tom roles (Tom1/Tom2/Tom3) are used for up to three Tom
///   slots before falling back to variations.
/// - Empty slots stay empty.
fn remap_roles_to_slots(
    role_pattern: &Pattern,
    layout: &AtomicTrackLayout,
    variation: f32,
    rng: &mut impl FnMut() -> f32,
) -> Pattern {
    let mut output = Pattern::empty();
    let mut assigned_per_voice: [usize; INSTRUMENT_COUNT] = [0; INSTRUMENT_COUNT];

    for slot_idx in 0..MAX_TRACKS {
        if !layout.is_active(slot_idx) {
            continue;
        }
        if slot_idx >= INSTRUMENT_COUNT {
            break; // Pattern cannot store more rows than INSTRUMENT_COUNT
        }
        let Some(kind) = layout.kind_for_slot(slot_idx) else {
            continue;
        };

        // The 606 multisample voices have no generator roles of their own:
        // they borrow the Kick / Snare roles so GENERATE still writes a line
        // on their lanes. Same for the AC606 voices (ported analogcode
        // engines): each borrows the role of its acoustic sibling.
        let base_voice = match kind {
            TrackInstrumentKind::Bd6smp => 0,
            TrackInstrumentKind::Sd6smp => 1,
            TrackInstrumentKind::Ch6smp => 2, // borrow the HiHat role
            TrackInstrumentKind::Oh6smp => 3, // [208] borrow the OpenHiHat role
            TrackInstrumentKind::Buzz => 12,  // borrow the Perc1 role
            TrackInstrumentKind::Rift => 12,  // [221] borrow the Perc1 role too
            TrackInstrumentKind::OneShot => 12, // [243] same: borrow the Perc1 role
            TrackInstrumentKind::Sdrex => 1,  // borrow the Snare role
            TrackInstrumentKind::Bd6Ac => 0,
            TrackInstrumentKind::Sd6Ac => 1,
            TrackInstrumentKind::Hh6Ac => 2,
            TrackInstrumentKind::Oh6Ac => 3,
            TrackInstrumentKind::Cl6Ac => 7,
            TrackInstrumentKind::Tm6Ac => 4,
            // [248] No `_` fallback: every kind gets an explicit role, so
            // adding a kind without touching this match is a compile error.
            // The old `_ => kind.drum_voice_index()` indexed
            // `assigned_per_voice` (14 cells) out of bounds for any kind
            // whose voice index is ≥ 14 — a panic, and panic = "abort"
            // takes the host down.
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
        };
        let duplicate_index = assigned_per_voice[base_voice];
        assigned_per_voice[base_voice] += 1;

        // Tom kind can use the three existing tom roles (4, 5, 6) before
        // falling back to variations.
        let source_voice = if kind == TrackInstrumentKind::Tom {
            (base_voice + duplicate_index.min(2)).min(INSTRUMENT_COUNT - 1)
        } else {
            base_voice
        };

        // Copy the chosen role into the slot row.
        for step in 0..STEP_COUNT {
            output.steps[step].instruments[slot_idx] =
                role_pattern.steps[step].instruments[source_voice];
        }

        // If this is a duplicate and we didn't get a distinct tom role, vary it.
        let needs_variation =
            duplicate_index > 0 && (kind != TrackInstrumentKind::Tom || duplicate_index >= 3);
        if needs_variation {
            let source_row = std::array::from_fn(|step| output.steps[step].instruments[slot_idx]);
            let varied = vary_pattern_row(&source_row, duplicate_index, variation, rng);
            for (step, active) in varied.iter().enumerate() {
                output.steps[step].instruments[slot_idx] = *active;
            }
        }
    }

    output
}

/// Apply a deterministic-but-random variation to a pattern row for duplicate
/// slots of the same kind.
fn vary_pattern_row(
    source: &[bool; STEP_COUNT],
    duplicate_index: usize,
    variation: f32,
    rng: &mut impl FnMut() -> f32,
) -> [bool; STEP_COUNT] {
    let var = variation.clamp(0.0, 1.0);
    // Shift the duplicate pattern so it doesn't perfectly overlap the original.
    let shift = (duplicate_index % 8).max(1);
    let thin_prob = var * 0.35;
    let add_prob = var * 0.08;
    let mut result = [false; STEP_COUNT];

    for step in 0..STEP_COUNT {
        let src_step = (step + STEP_COUNT - shift) % STEP_COUNT;
        if source[src_step] && rng() > thin_prob {
            result[step] = true;
        } else if !source[src_step] && rng() < add_prob {
            result[step] = true;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::{AtomicTrackLayout, TrackInstrumentKind, TrackLayoutState, TrackSlot};
    use std::sync::Arc;

    fn layout_from_kinds(kinds: &[TrackInstrumentKind]) -> Arc<AtomicTrackLayout> {
        let mut slots = std::array::from_fn(|_| TrackSlot::inactive());
        for (i, kind) in kinds.iter().enumerate() {
            if i < MAX_TRACKS {
                slots[i] = TrackSlot::active_with_kind(*kind);
            }
        }
        AtomicTrackLayout::from_state(&TrackLayoutState {
            slots,
            global_midi_channel: 10,
            global_base_note: 36,
        })
    }

    #[test]
    fn hihat_roles_are_style_specific() {
        let all_styles = [
            Style::Rock,
            Style::Funk,
            Style::Techno,
            Style::HipHop,
            Style::Jazz,
            Style::Metal,
            Style::Latin,
            Style::Disco,
            Style::Trap,
            Style::Reggae,
            Style::BossaNova,
            Style::House,
            Style::DrumAndBass,
            Style::Afrobeat,
            Style::Dub,
            Style::Breakbeat,
        ];
        let mut signatures = std::collections::HashSet::new();
        for style in all_styles {
            let template = styles::MusicalTemplate::for_style(style);
            let hihat = &template.roles[2];
            signatures.insert((
                hihat.anchors.to_vec(),
                hihat.candidates.to_vec(),
                (hihat.candidate_prob * 1000.0).round() as i32,
            ));
        }

        assert!(
            signatures.len() >= 8,
            "HiHat roles should be meaningfully different between styles"
        );

        let funk = styles::MusicalTemplate::for_style(Style::Funk).roles[2].anchors;
        let latin = styles::MusicalTemplate::for_style(Style::Latin).roles[2].anchors;
        let reggae = styles::MusicalTemplate::for_style(Style::Reggae).roles[2].anchors;
        assert_eq!(funk, &[1, 3, 5, 7, 9, 11, 13, 15]);
        assert_eq!(latin, &[0, 3, 6, 10, 12, 15]);
        assert_eq!(reggae, &[2, 6, 10, 14]);
    }

    #[test]
    fn every_kind_survives_generate() {
        // [248] A kind forgotten in `remap_roles_to_slots` used to fall into a
        // `_ => drum_voice_index()` fallback that indexes a 14-cell table out
        // of bounds → panic → panic = "abort" kills the host. The match is
        // now exhaustive (compile error on a missing kind); this test is the
        // runtime net: every kind through every generator type must return.
        let kinds = TrackInstrumentKind::ALL;
        for generator_type in [
            GeneratorType::Probabilistic,
            GeneratorType::Markov,
            GeneratorType::Euclidean,
            GeneratorType::Classic,
        ] {
            for kind in kinds {
                let layout = layout_from_kinds(&[kind]);
                let params = GeneratorParams {
                    generator_type,
                    style_primary: Style::Rock,
                    style_secondary: Style::Rock,
                    style_mix: 0.0,
                    density: 0.8,
                    variation: 0.5,
                    seed: 42,
                };
                let _ = generate(&params, layout.as_ref());
            }
        }
    }

    #[test]
    fn generate_maps_kick_to_kick_slot_not_opens_hh() {
        // Default 4-lane layout: Kick, Snare, HiHat, Tom
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Tom,
        ]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 0.8,
            variation: 0.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        // Slot 0 (Kick) should have kick activity (anchors on 1/3)
        assert!(pattern.steps[0].instruments[0]);
        assert!(pattern.steps[8].instruments[0]);

        // Slot 3 (Tom) should NOT receive the OpenHH role (which plays offbeats 2/6/10/14)
        let tom_activity: Vec<usize> = (0..STEP_COUNT)
            .filter(|&s| pattern.steps[s].instruments[3])
            .collect();
        // Tom role only plays in fill territory (steps 14, 15) with low probability.
        assert!(
            tom_activity.iter().all(|&s| s >= 14),
            "Tom slot received non-tom pattern: {:?}",
            tom_activity
        );

        // Slot 2 (HiHat) should have steady 8th activity on the non-clashing anchors.
        // Step 14 is an anchor for both HiHat and OpenHH; the clash rule may deactivate HiHat there.
        for &s in &[0, 2, 4, 6, 8, 10, 12] {
            assert!(
                pattern.steps[s].instruments[2],
                "HiHat missing anchor at {}",
                s
            );
        }
    }

    #[test]
    fn generate_uses_distinct_tom_roles_for_multiple_toms() {
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Tom,
        ]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 1.0,
            variation: 0.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        // With density 1.0, all tom candidates become active, so each tom role
        // should differ (Tom1: 14/15, Tom2: 14/15, Tom3: 14/15 but the roles
        // have different probabilities so at least one should differ).
        let tom1 = &pattern.steps[..]
            .iter()
            .map(|s| s.instruments[3])
            .collect::<Vec<_>>();
        let tom2 = &pattern.steps[..]
            .iter()
            .map(|s| s.instruments[4])
            .collect::<Vec<_>>();
        let tom3 = &pattern.steps[..]
            .iter()
            .map(|s| s.instruments[5])
            .collect::<Vec<_>>();

        assert!(
            tom1 != tom2 || tom2 != tom3,
            "All three Tom slots produced identical patterns"
        );
    }

    #[test]
    fn generate_varies_duplicate_kick_slots() {
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
        ]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 0.8,
            variation: 1.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        let kick0: Vec<bool> = (0..STEP_COUNT)
            .map(|s| pattern.steps[s].instruments[0])
            .collect();
        let kick1: Vec<bool> = (0..STEP_COUNT)
            .map(|s| pattern.steps[s].instruments[1])
            .collect();

        assert_ne!(
            kick0, kick1,
            "Duplicate Kick slots produced identical patterns despite variation=1.0"
        );
    }

    #[test]
    fn generate_leaves_empty_slots_silent() {
        let layout = layout_from_kinds(&[TrackInstrumentKind::Kick, TrackInstrumentKind::Snare]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 1.0,
            variation: 0.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        for slot in 2..INSTRUMENT_COUNT {
            for step in 0..STEP_COUNT {
                assert!(
                    !pattern.steps[step].instruments[slot],
                    "Empty slot {} has activity at step {}",
                    slot, step
                );
            }
        }
    }

    fn steps_equal(
        a: &[crate::sequencer::pattern::Step; STEP_COUNT],
        b: &[crate::sequencer::pattern::Step; STEP_COUNT],
    ) -> bool {
        a.iter()
            .zip(b.iter())
            .all(|(sa, sb)| sa.instruments == sb.instruments)
    }

    #[test]
    fn generate_is_deterministic_for_same_seed() {
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Tom,
        ]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 0.8,
            variation: 0.5,
            seed: 12345,
        };
        let pattern1 = generate(&params, layout.as_ref());
        let pattern2 = generate(&params, layout.as_ref());
        assert!(
            steps_equal(&pattern1.steps, &pattern2.steps),
            "Same seed should produce identical patterns"
        );
    }

    #[test]
    fn generate_differs_for_different_seed() {
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Tom,
        ]);
        let params_a = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 0.8,
            variation: 0.5,
            seed: 12345,
        };
        let params_b = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 0.8,
            variation: 0.5,
            seed: 54321,
        };
        let pattern_a = generate(&params_a, layout.as_ref());
        let pattern_b = generate(&params_b, layout.as_ref());
        assert!(
            !steps_equal(&pattern_a.steps, &pattern_b.steps),
            "Different seeds should produce different patterns"
        );
    }

    #[test]
    fn generate_kick_role_maps_by_kind_not_slot_index() {
        // Kick is deliberately at slot 1, not slot 0.
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Tom,
        ]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 1.0,
            variation: 0.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        // Slot 1 (Kick) should receive the kick role anchors.
        assert!(
            pattern.steps[0].instruments[1],
            "Kick slot should have kick anchor at step 0"
        );
        assert!(
            pattern.steps[8].instruments[1],
            "Kick slot should have kick anchor at step 8"
        );

        // Slot 0 (Snare) should not receive the kick role anchors.
        assert!(
            !pattern.steps[0].instruments[0],
            "Snare slot should not receive kick anchor at step 0"
        );
        assert!(
            !pattern.steps[8].instruments[0],
            "Snare slot should not receive kick anchor at step 8"
        );
    }

    #[test]
    fn generate_kick_role_maps_to_last_slot() {
        let mut slots = std::array::from_fn(|_| TrackSlot::inactive());
        slots[13] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        let layout = Arc::new(AtomicTrackLayout::from_state(&TrackLayoutState {
            slots,
            global_midi_channel: 10,
            global_base_note: 36,
        }));
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 1.0,
            variation: 0.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        assert!(
            pattern.steps[0].instruments[13],
            "Kick at slot 13 should have kick anchor at step 0"
        );
        assert!(
            pattern.steps[8].instruments[13],
            "Kick at slot 13 should have kick anchor at step 8"
        );
    }

    #[test]
    fn generate_no_kick_slot_has_no_kick_anchors() {
        // Layout with no Kick or HiHat/OpenHH/Ride: only roles whose anchors/candidates
        // never land on steps 0 or 8, so we can verify the kick role is not copied.
        let layout = layout_from_kinds(&[TrackInstrumentKind::Snare, TrackInstrumentKind::Clap]);
        let params = GeneratorParams {
            generator_type: GeneratorType::Probabilistic,
            style_primary: Style::Rock,
            style_secondary: Style::Rock,
            style_mix: 0.0,
            density: 1.0,
            variation: 0.0,
            seed: 42,
        };
        let pattern = generate(&params, layout.as_ref());

        for slot in 0..INSTRUMENT_COUNT {
            assert!(
                !pattern.steps[0].instruments[slot],
                "Slot {} should not have activity at kick anchor step 0",
                slot
            );
            assert!(
                !pattern.steps[8].instruments[slot],
                "Slot {} should not have activity at kick anchor step 8",
                slot
            );
        }
    }

    #[test]
    fn density_spans_from_sparse_to_busy_in_every_type() {
        // [283] Density used to barely move the hit count (+20 % across the
        // slider), ran backwards in Euclidean and did nothing in Classic at
        // Variation 0, and its first fix froze density 0 on the anchors alone.
        let layout = layout_from_kinds(&[
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::OpenHiHat,
            TrackInstrumentKind::Tom,
            TrackInstrumentKind::Clap,
            TrackInstrumentKind::Ride,
        ]);
        let mean_hits = |generator_type, density| {
            let mut total = 0;
            for seed in 0..40u64 {
                let params = GeneratorParams {
                    generator_type,
                    style_primary: Style::Rock,
                    style_secondary: Style::Rock,
                    style_mix: 0.0,
                    density,
                    variation: 0.0,
                    seed: seed * 7919 + 1,
                };
                let pattern = generate(&params, layout.as_ref());
                total += pattern
                    .steps
                    .iter()
                    .map(|s| s.instruments.iter().filter(|&&on| on).count())
                    .sum::<usize>();
            }
            total as f32 / 40.0
        };
        for generator_type in [
            GeneratorType::Probabilistic,
            GeneratorType::Markov,
            GeneratorType::Euclidean,
            GeneratorType::Classic,
        ] {
            // Sparse is not frozen: two GENERATE clicks still differ.
            let at_min = |seed| {
                let params = GeneratorParams {
                    generator_type,
                    style_primary: Style::Rock,
                    style_secondary: Style::Rock,
                    style_mix: 0.0,
                    density: 0.0,
                    variation: 0.0,
                    seed,
                };
                generate(&params, layout.as_ref())
            };
            assert!(
                !steps_equal(&at_min(1).steps, &at_min(2).steps),
                "{generator_type:?}: density 0 gives the same pattern on every click"
            );
            let sparse = mean_hits(generator_type, 0.0);
            let middle = mean_hits(generator_type, 0.5);
            let busy = mean_hits(generator_type, 1.0);
            assert!(
                sparse < middle && middle < busy && busy > 1.6 * sparse,
                "{generator_type:?}: {sparse} / {middle} / {busy} hits at density 0 / 0.5 / 1"
            );
        }
    }
}
