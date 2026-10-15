//! [286] P-locks on an effect lane (Fx1 / Fx2).
//!
//! An instrument reads its p-locks when a note fires. An effect has no notes, so
//! its locks are read from the transport position instead: at any moment the
//! value of a parameter is a pure function of where the playhead is and of the
//! locks placed on that parameter's lane. Two modes, chosen per lane:
//!
//! - **Step** (abrupt): the value of the most recent lock holds until the next
//!   one is reached, then jumps to it.
//! - **Glide** (progressive): between two locks the value moves in a straight
//!   line, arriving exactly on the second one's step. The slider is the starting
//!   point: with a single lock, the value rises from the slider at the start of
//!   the pattern to that lock.
//!
//! In both modes, after the LAST lock the value holds until the pattern loops, and
//! the slider is back at the start: nothing glides across the loop point. In Step
//! mode the pattern starts from the slider and stays there until the first lock. A
//! parameter with no lock at all keeps its slider. Being stateless, the evaluation
//! needs no bookkeeping across seeks, loops or a stop/start.
//!
//! The lane can switch the **restart** off: the loop point then stops being special.
//! There is no slider start and no return to it; the locks form a circle, the last one
//! holding (Step) or gliding (Glide) into the first of the next loop.

use crate::groove::{beat_to_step, step_start_beat, GrooveType};

/// How steps map to beats for the pattern being played.
#[derive(Clone, Copy, Debug)]
pub struct StepClock {
    /// Pattern length in steps (1..=64).
    pub steps: usize,
    pub swing: f32,
    pub groove: GrooveType,
}

/// [`locked_value_restart`] with the restart on: the pattern starts from the slider.
#[cfg(test)]
pub fn locked_value(
    global: f32,
    beat: f64,
    clock: StepClock,
    glide: bool,
    lock_at: impl Fn(usize) -> Option<f32>,
) -> f32 {
    locked_value_restart(global, beat, clock, glide, true, lock_at)
}

/// Value of one parameter at `beat` (position inside the pattern, in beats).
/// `lock_at(step)` returns the lock on this parameter at that step, if any.
/// `restart`: the loop point resets to the slider (see the module docs).
/// Allocation-free; the cost is two scans of at most `steps` entries.
pub fn locked_value_restart(
    global: f32,
    beat: f64,
    clock: StepClock,
    glide: bool,
    restart: bool,
    lock_at: impl Fn(usize) -> Option<f32>,
) -> f32 {
    let steps = clock.steps.clamp(1, 64);
    let loop_beats = steps as f64 * 0.25;
    let beat = if beat.is_finite() {
        beat.rem_euclid(loop_beats)
    } else {
        0.0
    };
    let current = beat_to_step(beat, clock.swing, clock.groove).min(steps - 1);
    let start = |step: usize| step_start_beat(step, clock.swing, clock.groove);

    // No restart: the locks form a circle.
    if !restart {
        let Some((prev_step, prev_value)) = (0..steps)
            .find_map(|k| {
                let step = (current + steps - k) % steps;
                lock_at(step).map(|value| (step, value))
            })
        else {
            return global;
        };
        if !glide {
            return prev_value;
        }
        let Some((next_step, next_value)) = (1..=steps).find_map(|k| {
            let step = (current + k) % steps;
            lock_at(step).map(|value| (step, value))
        }) else {
            return prev_value;
        };
        if next_step == prev_step {
            return prev_value;
        }
        let from = start(prev_step) - if prev_step > current { loop_beats } else { 0.0 };
        let to = start(next_step) + if next_step <= current { loop_beats } else { 0.0 };
        let span = to - from;
        if span <= 1.0e-9 {
            return prev_value;
        }
        let t = ((beat - from) / span).clamp(0.0, 1.0) as f32;
        return prev_value + (next_value - prev_value) * t;
    }

    // In Glide mode the slider stands in for a lock on the first step, so the value
    // can rise from it to the first real lock.
    let lock_at = |step: usize| -> Option<f32> {
        lock_at(step).or((glide && step == 0).then_some(global))
    };

    // The most recent lock at or before the current step; none = the slider.
    let Some((prev_step, prev_value)) =
        (0..=current).rev().find_map(|step| lock_at(step).map(|value| (step, value)))
    else {
        return global;
    };
    if !glide {
        return prev_value;
    }

    // The next lock before the loop ends; none = the last value holds.
    let Some((next_step, next_value)) =
        (current + 1..steps).find_map(|step| lock_at(step).map(|value| (step, value)))
    else {
        return prev_value;
    };
    let span = start(next_step) - start(prev_step);
    if span <= 1.0e-9 {
        return prev_value;
    }
    let t = ((beat - start(prev_step)) / span).clamp(0.0, 1.0) as f32;
    prev_value + (next_value - prev_value) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLOCK: StepClock = StepClock {
        steps: 16,
        swing: 0.0,
        groove: GrooveType::Straight,
    };

    /// Locks of 1.0 at step 2 and 5.0 at step 6 (straight: a step is 0.25 beat).
    fn two_locks(step: usize) -> Option<f32> {
        match step {
            2 => Some(1.0),
            6 => Some(5.0),
            _ => None,
        }
    }

    fn at_step(step: usize, fraction: f64) -> f64 {
        (step as f64 + fraction) * 0.25
    }

    #[test]
    fn no_lock_keeps_the_slider() {
        for glide in [false, true] {
            assert_eq!(locked_value(0.7, 1.0, CLOCK, glide, |_| None), 0.7);
        }
    }

    #[test]
    fn step_mode_holds_each_lock_until_the_next() {
        let v = |step, frac| locked_value(9.0, at_step(step, frac), CLOCK, false, two_locks);
        assert_eq!(v(0, 0.0), 9.0, "the pattern starts from the slider");
        assert_eq!(v(1, 0.5), 9.0, "held until the first lock");
        assert_eq!(v(2, 0.0), 1.0, "on the first lock");
        assert_eq!(v(4, 0.5), 1.0, "held between the locks");
        assert_eq!(v(6, 0.0), 5.0, "jumps on the second lock");
        assert_eq!(v(15, 0.9), 5.0, "held until the loop ends");
    }

    #[test]
    fn glide_mode_is_a_straight_line_between_locks() {
        let v = |step, frac| locked_value(9.0, at_step(step, frac), CLOCK, true, two_locks);
        assert_eq!(v(2, 0.0), 1.0);
        assert!((v(4, 0.0) - 3.0).abs() < 1.0e-5, "halfway between 1 and 5");
        assert!((v(5, 0.0) - 4.0).abs() < 1.0e-5);
        assert!((v(5, 0.999) - 5.0).abs() < 0.01, "almost there");
        assert_eq!(v(6, 0.0), 5.0, "arrives exactly on the lock");
    }

    #[test]
    fn glide_rises_from_the_slider_to_the_first_lock_and_holds_after_the_last() {
        let v = |step, frac| locked_value(9.0, at_step(step, frac), CLOCK, true, two_locks);
        assert_eq!(v(0, 0.0), 9.0, "starts on the slider");
        assert!((v(1, 0.0) - 5.0).abs() < 1.0e-5, "half way from 9 to the lock of 1 on step 3");
        assert_eq!(v(2, 0.0), 1.0, "arrives on the first lock");
        for step in 6..16 {
            assert_eq!(v(step, 0.5), 5.0, "the last lock holds to the end of the loop");
        }
    }

    #[test]
    fn a_single_lock_does_not_take_over_the_whole_lane() {
        let one = |step: usize| (step == 8).then_some(2.5);
        // Step: the slider until the lock, then the lock to the end of the loop.
        assert_eq!(locked_value(9.0, at_step(0, 0.0), CLOCK, false, one), 9.0);
        assert_eq!(locked_value(9.0, at_step(4, 0.0), CLOCK, false, one), 9.0);
        assert_eq!(locked_value(9.0, at_step(8, 0.0), CLOCK, false, one), 2.5);
        assert_eq!(locked_value(9.0, at_step(15, 0.5), CLOCK, false, one), 2.5);
        // Glide: from the slider to the lock, then the lock to the end of the loop.
        assert_eq!(locked_value(9.0, at_step(0, 0.0), CLOCK, true, one), 9.0);
        let half = locked_value(9.0, at_step(4, 0.0), CLOCK, true, one);
        assert!((half - 5.75).abs() < 1.0e-5, "half way from 9 to 2.5, got {half}");
        assert_eq!(locked_value(9.0, at_step(8, 0.0), CLOCK, true, one), 2.5);
        assert_eq!(locked_value(9.0, at_step(15, 0.5), CLOCK, true, one), 2.5);
        // A lock on the very first step IS the start value: no slider involved.
        let first = |step: usize| (step == 0).then_some(2.5);
        assert_eq!(locked_value(9.0, at_step(5, 0.0), CLOCK, false, first), 2.5);
    }

    #[test]
    fn works_with_swing_and_short_patterns() {
        let clock = StepClock {
            steps: 8,
            swing: 0.4,
            groove: GrooveType::Swing16,
        };
        let lock = |step: usize| match step {
            0 => Some(0.0),
            4 => Some(1.0),
            _ => None,
        };
        let mut last = 0.0f32;
        // Monotone up from step 0 to step 4, always finite and in range.
        for i in 0..=100 {
            let beat = start_beat_of(clock, 0) + (start_beat_of(clock, 4) * i as f64 / 100.0);
            let v = locked_value(9.0, beat.min(start_beat_of(clock, 4)), clock, true, lock);
            assert!(v.is_finite() && (-1.0e-6..=1.000001).contains(&v), "{v}");
            assert!(v + 1.0e-5 >= last, "not monotone: {last} -> {v}");
            last = v;
        }
    }

    fn start_beat_of(clock: StepClock, step: usize) -> f64 {
        step_start_beat(step, clock.swing, clock.groove)
    }

    #[test]
    fn out_of_range_beats_and_lengths_are_safe() {
        let clock = StepClock {
            steps: 0,
            swing: 0.0,
            groove: GrooveType::Straight,
        };
        assert!(locked_value(1.0, f64::NAN, clock, true, two_locks).is_finite());
        assert!(locked_value(1.0, 1.0e9, CLOCK, true, two_locks).is_finite());
        assert!(locked_value(1.0, -3.0, CLOCK, false, two_locks).is_finite());
    }

    #[test]
    fn without_restart_the_locks_form_a_circle() {
        let v = |step, frac, glide| {
            locked_value_restart(9.0, at_step(step, frac), CLOCK, glide, false, two_locks)
        };
        // Step: the last lock (5.0, step 6) holds through the loop point, up to the first (1.0, step 2).
        assert_eq!(v(0, 0.0, false), 5.0, "no slider at the loop start");
        assert_eq!(v(1, 0.5, false), 5.0);
        assert_eq!(v(2, 0.0, false), 1.0);
        assert_eq!(v(15, 0.5, false), 5.0);
        // Glide: 5.0 at step 6 glides to 1.0 at step 2 of the next loop, 12 steps later.
        assert!((v(12, 0.0, true) - 3.0).abs() < 1.0e-5, "half way round");
        assert!((v(0, 0.0, true) - (5.0 - 4.0 * 10.0 / 12.0)).abs() < 1.0e-4);
        assert_eq!(v(2, 0.0, true), 1.0);
        // No lock at all: the slider, restart or not.
        assert_eq!(locked_value_restart(9.0, 1.0, CLOCK, true, false, |_| None), 9.0);
        // A single lock holds everywhere.
        let one = |step: usize| (step == 8).then_some(2.5);
        for glide in [false, true] {
            for step in [0usize, 4, 8, 15] {
                assert_eq!(locked_value_restart(9.0, at_step(step, 0.0), CLOCK, glide, false, one), 2.5);
            }
        }
    }
}
