//! MIDI file export (SMF Type 0)

use crate::{
    groove::GrooveType,
    plock::SequencerPlockState,
    sequencer::SharedPattern,
    track::{AtomicTrackLayout, MAX_TRACKS},
};
use std::path::Path;

const TICKS_PER_QUARTER: u32 = 480;
const TICKS_PER_STEP: u32 = TICKS_PER_QUARTER / 4;
const TICKS_PER_EIGHTH: u32 = TICKS_PER_QUARTER / 2;

/// Absolute tick offset of a step with swing/groove applied.
fn step_tick_offset(step: usize, swing: f32, groove_type: GrooveType) -> u32 {
    if groove_type == GrooveType::Straight {
        return step as u32 * TICKS_PER_STEP;
    }
    let ratio = crate::groove::swing_ratio_for(swing, groove_type).clamp(0.02, 0.98);
    let pair = step / 2;
    let is_odd = step % 2 == 1;
    let base = pair as u32 * TICKS_PER_EIGHTH;
    if is_odd {
        base + ((TICKS_PER_EIGHTH as f64 * ratio).round() as u32)
    } else {
        base
    }
}

fn encode_vlq(mut value: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        bytes.push((value & 0x7F) as u8);
        value >>= 7;
        if value == 0 {
            break;
        }
    }
    for byte in bytes.iter_mut().skip(1) {
        *byte |= 0x80;
    }
    bytes.reverse();
    bytes
}

fn midi_header(track_length: u32) -> Vec<u8> {
    let mut data = Vec::new();
    // MThd
    data.extend_from_slice(b"MThd");
    data.extend_from_slice(&[0x00, 0x00, 0x00, 0x06]); // chunk length = 6
    data.extend_from_slice(&[0x00, 0x00]); // format 0
    data.extend_from_slice(&[0x00, 0x01]); // 1 track
    data.extend_from_slice(&[0x01, 0xE0]); // 480 ticks per quarter
                                           // MTrk
    data.extend_from_slice(b"MTrk");
    data.extend_from_slice(&[
        (track_length >> 24) as u8,
        (track_length >> 16) as u8,
        (track_length >> 8) as u8,
        track_length as u8,
    ]);
    data
}

pub fn export_pattern_to_midi(
    pattern: &SharedPattern,
    track_layout: &AtomicTrackLayout,
    bpm: f32,
    pattern_length: usize,
    swing: f32,
    groove_type: GrooveType,
    seq_plock: &SequencerPlockState,
    path: &Path,
) -> std::io::Result<()> {
    std::fs::write(
        path,
        export_pattern_to_midi_data(
            pattern,
            track_layout,
            bpm,
            pattern_length,
            swing,
            groove_type,
            seq_plock,
        ),
    )
}

fn export_pattern_to_midi_data(
    pattern: &SharedPattern,
    track_layout: &AtomicTrackLayout,
    bpm: f32,
    pattern_length: usize,
    swing: f32,
    groove_type: GrooveType,
    seq_plock: &SequencerPlockState,
) -> Vec<u8> {
    let microseconds_per_quarter = (60_000_000.0 / bpm).round() as u32;
    let steps = pattern_length.clamp(1, 64);

    let mut events: Vec<(u32, Vec<u8>)> = Vec::new();

    // Set tempo (meta event)
    events.push((
        0,
        vec![
            0xFF,
            0x51,
            0x03,
            (microseconds_per_quarter >> 16) as u8,
            (microseconds_per_quarter >> 8) as u8,
            microseconds_per_quarter as u8,
        ],
    ));

    // Emit a note-on/off pair at `tick`, note-off after `dur` ticks.
    let emit = |events: &mut Vec<(u32, Vec<u8>)>, tick: u32, dur: u32, note: u8| {
        events.push((tick, vec![0x99, note, 100]));
        events.push((tick + dur.max(1), vec![0x89, note, 0]));
    };

    // Per-cell microtiming (ms) → signed tick shift at the export tempo.
    let nudge_ticks = |slot: usize, step: usize| -> i64 {
        let ms = seq_plock
            .get(slot, step)
            .map(|p| p.microtiming_ms.clamp(-100.0, 100.0))
            .unwrap_or(0.0);
        (ms as f64 * bpm as f64 / 60000.0 * TICKS_PER_QUARTER as f64).round() as i64
    };
    let shifted = |base: u32, nudge: i64| -> u32 { (base as i64 + nudge).max(0) as u32 };

    for slot in 0..MAX_TRACKS {
        if !track_layout.is_active(slot) {
            continue;
        }
        // [286] An effect bus plays no note.
        if track_layout.kind_for_slot(slot).is_some_and(|k| k.is_effect()) {
            continue;
        }
        let note = track_layout.midi_note_for_slot(slot);
        // A linked lane plays the steps + fusions of the lane it follows
        // (like the sequencer); microtiming and stutter stay its own.
        let grid = track_layout.grid_slot(slot);
        let fusions = pattern.load_fusions(grid);
        for step in 0..steps {
            let bit = (pattern.load_step_mask(step) & (1u16 << grid)) != 0;

            // A cell covered by a fusion plays nothing on its own; only the
            // fusion's START cell emits (matching the sequencer).
            if let Some(group) = fusions.iter().find(|g| g.contains(step)) {
                if !group.is_start(step) || !bit {
                    continue;
                }
                // `step_count` pulses evenly spaced over the whole fused span.
                let pulses = (group.step_count.max(1)) as u32;
                let span = group.cell_span() as u32;
                let total = (span * TICKS_PER_STEP).max(1);
                let base = step_tick_offset(group.start_cell as usize, swing, groove_type);
                let base = shifted(base, nudge_ticks(slot, group.start_cell as usize));
                let dur = (total / pulses).saturating_sub(1).clamp(1, 10);
                for k in 0..pulses {
                    let tick = base + (k as f32 * total as f32 / pulses as f32).round() as u32;
                    emit(&mut events, tick, dur, note);
                }
            } else if bit {
                // Normal cell: sequencer-plock stutter retriggers over one step.
                let n = seq_plock
                    .get(slot, step)
                    .map(|p| p.stutter_count.max(1))
                    .unwrap_or(1) as u32;
                let base = step_tick_offset(step, swing, groove_type);
                let base = shifted(base, nudge_ticks(slot, step));
                let dur = (TICKS_PER_STEP / n).saturating_sub(1).clamp(1, 10);
                for k in 0..n {
                    let tick = base + (k as f32 * TICKS_PER_STEP as f32 / n as f32).round() as u32;
                    emit(&mut events, tick, dur, note);
                }
            }
        }
    }

    // Sort by absolute tick
    events.sort_by_key(|e| e.0);

    // Convert to delta times
    let mut track_data = Vec::new();
    let mut last_tick = 0u32;
    for (tick, data) in events {
        let delta = tick - last_tick;
        last_tick = tick;
        track_data.extend_from_slice(&encode_vlq(delta));
        track_data.extend_from_slice(&data);
    }

    // End of track
    track_data.extend_from_slice(&encode_vlq(0));
    track_data.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    let mut file_data = midi_header(track_data.len() as u32);
    file_data.extend_from_slice(&track_data);
    file_data
}

/// Export pattern to MIDI bytes in memory — the tests' entry point (the
/// plugin writes the file through `export_pattern_to_midi`).
#[cfg(test)]
pub fn export_pattern_to_midi_bytes(
    pattern: &SharedPattern,
    track_layout: &AtomicTrackLayout,
    bpm: f32,
    pattern_length: usize,
    swing: f32,
    groove_type: GrooveType,
    seq_plock: &SequencerPlockState,
) -> std::io::Result<Vec<u8>> {
    Ok(export_pattern_to_midi_data(
        pattern,
        track_layout,
        bpm,
        pattern_length,
        swing,
        groove_type,
        seq_plock,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sequencer::pattern::Pattern,
        track::{AtomicTrackLayout, TrackInstrumentKind, TrackLayoutState},
    };

    fn legacy_layout() -> std::sync::Arc<AtomicTrackLayout> {
        AtomicTrackLayout::from_state(&TrackLayoutState::from_legacy_13())
    }

    fn count_note_ons(bytes: &[u8], note: u8) -> usize {
        bytes
            .windows(3)
            .filter(|w| w[0] == 0x99 && w[1] == note && w[2] == 100)
            .count()
    }

    /// Absolute ticks of every note-on (velocity 100) in an exported SMF0 file.
    fn note_on_ticks(bytes: &[u8]) -> Vec<u32> {
        let mut ticks = Vec::new();
        let mut tick = 0u32;
        let mut i = 22; // skip MThd header (14) + MTrk header (8)
        while i < bytes.len() {
            let mut delta = 0u32;
            loop {
                let b = bytes[i];
                i += 1;
                delta = (delta << 7) | (b & 0x7F) as u32;
                if b & 0x80 == 0 {
                    break;
                }
            }
            tick += delta;
            match bytes[i] {
                status @ (0x99 | 0x89) => {
                    if status == 0x99 && bytes[i + 2] == 100 {
                        ticks.push(tick);
                    }
                    i += 3;
                }
                0xFF => {
                    // Meta event: FF <type> <len> <data…>
                    let len = bytes[i + 2] as usize;
                    i += 3 + len;
                }
                _ => i += 1,
            }
        }
        ticks
    }

    #[test]
    fn midi_export_applies_microtiming_nudge() {
        // Step 2, +50 ms at 120 BPM = 0.1 quarter = 48 ticks after the base
        // tick (2 * 120 = 240) → 288; -50 ms → 192.
        for (nudge_ms, expected_tick) in [(50.0f32, 288u32), (-50.0, 192)] {
            let pattern = SharedPattern::new(&Pattern::empty());
            pattern.set_step_mask(2, 1u16 << 0);
            let layout = legacy_layout();
            let seq = SequencerPlockState::new();
            seq.set_microtiming(0, 2, nudge_ms);

            let bytes = export_pattern_to_midi_bytes(
                &pattern,
                &layout,
                120.0,
                16,
                0.0,
                GrooveType::Straight,
                &seq,
            )
            .expect("MIDI export should succeed");
            assert_eq!(
                note_on_ticks(&bytes),
                vec![expected_tick],
                "nudge {nudge_ms} ms should move the note-on to tick {expected_tick}"
            );
        }
    }

    /// [286] An effect bus plays no note, even if a step bit was stamped on it.
    #[test]
    fn midi_export_skips_effect_buses() {
        let pattern = SharedPattern::new(&Pattern::empty());
        pattern.set_step_mask(0, (1u16 << 0) | (1u16 << 12));
        let mut state = TrackLayoutState::empty_layout();
        state.slots[0] = crate::track::TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        state.slots[12] = crate::track::TrackSlot::active_with_kind(TrackInstrumentKind::Reverb);
        let layout = AtomicTrackLayout::from_state(&state);
        let seq = SequencerPlockState::new();
        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &seq,
        )
        .expect("MIDI export should succeed");
        assert_eq!(count_note_ons(&bytes, 36), 1);
        assert_eq!(count_note_ons(&bytes, 0), 0, "the bus must not emit a note");
    }

    #[test]
    fn midi_export_linked_lane_plays_the_followed_lanes_steps() {
        let pattern = SharedPattern::new(&Pattern::empty());
        pattern.set_step_mask(0, 1u16 << 0); // Kick (followed lane) on step 0
        pattern.set_step_mask(4, 1u16 << 1); // Snare's own hidden step: ignored
        let mut state = TrackLayoutState::from_legacy_13();
        state.slots[1].linked_up = true; // Snare follows Kick
        let layout = AtomicTrackLayout::from_state(&state);
        let seq = SequencerPlockState::new();

        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &seq,
        )
        .expect("MIDI export should succeed");
        assert_eq!(count_note_ons(&bytes, 36), 1, "the followed lane keeps its note");
        assert_eq!(
            count_note_ons(&bytes, 38),
            1,
            "the linked lane plays the followed lane's step, not its own hidden one"
        );
    }

    #[test]
    fn midi_export_expands_stutter_into_multiple_notes() {
        let pattern = SharedPattern::new(&Pattern::empty());
        pattern.set_step_mask(0, 1u16 << 0); // Kick on step 0
        let layout = legacy_layout();
        let seq = SequencerPlockState::new();
        seq.set_stutter(0, 0, 4); // 4 retriggers on the kick step

        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &seq,
        )
        .expect("MIDI export should succeed");
        assert_eq!(
            count_note_ons(&bytes, 36),
            4,
            "stutter=4 should emit 4 kick note-ons"
        );
    }

    #[test]
    fn midi_export_expands_fusion_into_pulses() {
        use crate::sequencer::pattern::{FusedGroup, MorphTarget};
        let pattern = SharedPattern::new(&Pattern::empty());
        pattern.set_step_mask(0, 1u16 << 0); // Kick on the fusion start cell
        pattern.store_fusions(
            0,
            &[FusedGroup {
                start_cell: 0,
                end_cell: 1,
                step_count: 3,
                morph_count: 0,
                morph_targets: [MorphTarget::default(); 4],
            }],
        );
        let layout = legacy_layout();
        let seq = SequencerPlockState::new();

        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &seq,
        )
        .expect("MIDI export should succeed");
        assert_eq!(
            count_note_ons(&bytes, 36),
            3,
            "fusion step_count=3 should emit 3 kick pulses"
        );
    }

    #[test]
    fn midi_export_uses_slot_midi_note_including_fourteenth_slot() {
        let mut layout_state = TrackLayoutState::default_layout();
        layout_state.slots[13].active = true;
        layout_state.slots[13].kind = TrackInstrumentKind::Kick;
        layout_state.slots[13].midi_note = 99;
        let layout = AtomicTrackLayout::from_state(&layout_state);

        let pattern = SharedPattern::new(&Pattern::empty());
        pattern.set_step_mask(0, 1u16 << 13);

        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &SequencerPlockState::new(),
        )
        .expect("MIDI export should succeed");

        assert!(
            bytes.windows(3).any(|window| window == [0x99, 99, 100]),
            "14th slot note-on should use its custom MIDI note"
        );
        assert!(
            bytes.windows(3).any(|window| window == [0x89, 99, 0]),
            "14th slot note-off should use its custom MIDI note"
        );
    }

    #[test]
    fn midi_export_includes_perc1_thirteenth_instrument() {
        let pattern = SharedPattern::new(&Pattern::empty());
        let perc1_slot = 12;
        pattern.set_step_mask(0, 1u16 << perc1_slot);
        let layout = legacy_layout();

        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &SequencerPlockState::new(),
        )
        .expect("MIDI export should succeed");

        assert!(
            bytes.windows(3).any(|window| window == [0x99, 37, 100]),
            "Perc1 note-on event should be exported"
        );
        assert!(
            bytes.windows(3).any(|window| window == [0x89, 37, 0]),
            "Perc1 note-off event should be exported"
        );
    }

    #[test]
    fn midi_export_includes_steps_beyond_first_page() {
        let pattern = SharedPattern::new(&Pattern::empty());
        pattern.set_step_mask(32, 1u16 << 0); // Kick at step 32
        pattern.set_step_mask(63, 1u16 << 1); // Snare at step 63
        let layout = legacy_layout();

        let bytes = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            64,
            0.0,
            GrooveType::Straight,
            &SequencerPlockState::new(),
        )
        .expect("MIDI export should succeed");

        // Kick note = 36, Snare note = 38
        assert!(
            bytes.windows(3).any(|window| window == [0x99, 36, 100]),
            "Kick note-on at step 32 should be exported"
        );
        assert!(
            bytes.windows(3).any(|window| window == [0x99, 38, 100]),
            "Snare note-on at step 63 should be exported"
        );
    }

    #[test]
    fn midi_export_applies_swing16_to_odd_steps() {
        let pattern = SharedPattern::new(&Pattern::empty());
        // Put a kick on step 1 (the first off-beat 16th)
        pattern.set_step_mask(1, 1u16 << 0);
        let layout = legacy_layout();

        let straight = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.0,
            GrooveType::Straight,
            &SequencerPlockState::new(),
        )
        .expect("MIDI export should succeed");
        let swung = export_pattern_to_midi_bytes(
            &pattern,
            &layout,
            120.0,
            16,
            0.5,
            GrooveType::Swing16,
            &SequencerPlockState::new(),
        )
        .expect("MIDI export should succeed");

        // Straight step 1 is at tick 120; with +50 % swing16 ratio = 2/3,
        // the odd step moves to tick 160.
        assert!(
            straight.windows(3).any(|window| window == [0x99, 36, 100]),
            "Kick should be present in straight export"
        );
        // Look for the note-on delta preceding the kick note in the swung export.
        // Step 1 straight delta from step 0 off = 120; swung delta = 160.
        let note_on_pos = swung
            .windows(3)
            .position(|window| window == [0x99, 36, 100])
            .expect("swung export should contain kick note-on");
        // Decode the VLQ delta immediately preceding the note-on.
        let mut delta_start = note_on_pos - 1;
        while delta_start > 0 && (swung[delta_start - 1] & 0x80) != 0 {
            delta_start -= 1;
        }
        let mut delta = 0u32;
        for b in &swung[delta_start..note_on_pos] {
            delta = (delta << 7) | ((b & 0x7F) as u32);
        }
        assert_eq!(
            delta, 160,
            "swung step 1 should land at tick 160, got delta {}",
            delta
        );
    }
}
