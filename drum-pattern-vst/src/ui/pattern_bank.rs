//! Pattern bank bar: Export/Drag chips, Save/Load slots P1-P16, Clear.

use crate::ui::controls::chip_button;
use crate::ui::editor_state::EditorUIState;
use crate::ui::help::{self, HelpExt};
use crate::ui::midi::{export_midi_to_documents, start_external_midi_drag};
use crate::ui::theme::*;
use crate::{
    sequencer::{Pattern, SharedPattern},
    DrumFlashParams,
};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};
use std::sync::{atomic::AtomicU32, Arc};

pub fn draw_pattern_bank(
    ui: &mut egui::Ui,
    state: &mut EditorUIState,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    save_pattern_request: &Arc<AtomicU32>,
    load_pattern_request: &Arc<AtomicU32>,
    audio_last_loaded_slot: &Arc<AtomicU32>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Patterns").strong().size(12.0).color(INK()));
        ui.add_space(8.0);

        // Save (keycap; armed = pressed blue) — to the LEFT of the slots. Click,
        // then click a slot to store. (Clr + Export/Drag stay after the slots.)
        let save_state = if state.save_mode_active {
            crate::ui::widgets::KeycapState::PressedBlue
        } else {
            crate::ui::widgets::KeycapState::Rest
        };
        let save_response = crate::ui::controls::keycap_button(
            ui,
            "Save",
            46.0,
            save_state,
            true,
            f_mono_med(10.5),
        )
        .help_or(
            help::BANK_SAVE,
            RichText::new(if state.save_mode_active {
                "Click a slot (P1-P16) to save the current pattern there"
            } else {
                "Activate save mode, then click a slot to store the current pattern"
            })
            .size(11.0)
            .monospace(),
        );
        if save_response.clicked() {
            state.save_mode_active = !state.save_mode_active;
            state.clear_confirm_mode = false;
        }
        ui.add_space(8.0);

        // Determine if current pattern is dirty compared to last_loaded_slot
        let is_dirty = pattern_is_dirty(params, pattern, state);

        // Tighter horizontal rhythm so the 16 P-slots fit on one row alongside
        // the Save/Clr/Export/Drag keycaps.
        ui.spacing_mut().item_spacing.x = 3.0;

        // P1-P16 slots
        for i in 0..crate::pattern_bank::SLOT_COUNT {
            let occupied = params
                .pattern_bank
                .bank
                .lock()
                .map(|b| b.slots[i].occupied)
                .unwrap_or(false);
            let is_loaded = state.last_loaded_slot == Some(i);
            let show_star = is_dirty && is_loaded;
            let text = if show_star {
                format!("P{}*", i + 1)
            } else {
                format!("P{}", i + 1)
            };

            let btn_size = Vec2::new(26.0, 26.0);
            let kc_state = if is_loaded {
                crate::ui::widgets::KeycapState::PressedBlue
            } else {
                crate::ui::widgets::KeycapState::Rest
            };

            let response = ui
                .allocate_ui_with_layout(
                    btn_size,
                    egui::Layout::top_down(egui::Align::Center),
                    |ui| {
                        let (rect, response) =
                            ui.allocate_exact_size(btn_size, egui::Sense::click());
                        // Occupied slots read as raised keycaps with a bright
                        // label; empty slots are flat recessed boxes — the
                        // difference in shape language is obvious at a glance.
                        let is_empty = !occupied && !is_loaded;
                        if is_empty {
                            ui.painter().rect_filled(rect, 5.0, BG());
                            ui.painter().rect_stroke(
                                rect,
                                5.0,
                                egui::Stroke::new(1.0, LINE()),
                                egui::StrokeKind::Inside,
                            );
                        } else {
                            crate::ui::widgets::keycap_tex(ui, rect, kc_state);
                        }
                        if response.is_pointer_button_down_on() {
                            ui.painter()
                                .rect_filled(rect, 5.0, Color32::from_black_alpha(60));
                        }
                        let label_color = if is_loaded {
                            Color32::from_rgb(234, 246, 255)
                        } else if occupied {
                            INK_KEYCAP
                        } else {
                            FAINT()
                        };
                        ui.painter().text(
                            rect.center(),
                            egui::Align2::CENTER_CENTER,
                            &text,
                            egui::FontId::monospace(10.0),
                            label_color,
                        );
                        response
                    },
                )
                .inner;

            let tooltip = if state.save_mode_active {
                format!("Save current pattern to P{}", i + 1)
            } else if occupied {
                if is_loaded {
                    format!(
                        "P{}: current pattern (click to reload)\n* = unsaved changes",
                        i + 1
                    )
                } else {
                    format!("P{}: saved pattern (click to load)", i + 1)
                }
            } else {
                format!("P{}: empty", i + 1)
            };
            let response = response.help_or(
                help::BANK_SLOT,
                RichText::new(tooltip).size(11.0).monospace(),
            );

            if response.clicked_by(egui::PointerButton::Primary) {
                state.clear_confirm_mode = false;
                if state.save_mode_active {
                    save_pattern_request
                        .store((i + 1) as u32, std::sync::atomic::Ordering::Relaxed);
                    state.save_mode_active = false;
                    state.last_loaded_slot = Some(i);
                } else if is_dirty {
                    // Unsaved changes: warn before discarding them (loading an
                    // occupied slot OR positioning on an empty one both wipe
                    // the working grid).
                    state.pattern_load_confirm = Some(i);
                } else if occupied {
                    load_pattern_request
                        .store((i + 1) as u32, std::sync::atomic::Ordering::Relaxed);
                    state.last_loaded_slot = Some(i);
                } else {
                    // Empty slot: position on it — fresh empty grid, ready to
                    // build a new pattern (Save will store into this slot).
                    clear_current_grid(pattern, params);
                    state.last_loaded_slot = Some(i);
                    // The audio thread owns `audio_last_loaded_slot` and the UI
                    // re-syncs `last_loaded_slot` from it every frame. Positioning
                    // on an empty slot is a UI-only action (no load request), so we
                    // must publish it here — otherwise the next-frame sync reverts
                    // to the previously saved/loaded slot and, since the grid was
                    // just cleared, flags THAT slot dirty (phantom "*").
                    audio_last_loaded_slot.store(i as u32, std::sync::atomic::Ordering::Relaxed);
                }
            }
            if response.secondary_clicked() {
                if occupied {
                    // Copy slot to clipboard
                    if let Ok(bank) = params.pattern_bank.bank.lock() {
                        state.pattern_clipboard = Some(bank.slots[i].clone());
                    }
                } else if let Some(ref clipboard) = state.pattern_clipboard {
                    // Paste clipboard into empty slot
                    if let Ok(mut bank_mut) = params.pattern_bank.bank.lock() {
                        bank_mut.slots[i] = clipboard.clone();
                        drop(bank_mut);
                        params.pattern_bank.refresh_snapshot();
                    }
                }
            }
        }

        ui.add_space(10.0);

        // Clr (keycap; two-step — the confirm phase offers a choice: clear the
        // current GRID only, or clear the grid AND empty the bank SLOT).
        let is_clear_confirm = state.clear_confirm_mode;
        if is_clear_confirm {
            let grid_response = chip_button(ui, "Grid", true, BLUE(), egui::Sense::click())
                .help_or(
                    help::BANK_CLEAR_GRID,
                    RichText::new("Clear the current grid only — the slot keeps its saved pattern")
                        .size(11.0)
                        .monospace(),
                );
            if grid_response.clicked() {
                clear_current_grid(pattern, params);
                state.clear_confirm_mode = false;
            }
            if state.last_loaded_slot.is_some() {
                let slot_response = chip_button(ui, "Slot", true, RED(), egui::Sense::click())
                    .help_or(
                        help::BANK_CLEAR_SLOT,
                        RichText::new(
                            "Clear the current grid AND empty the bank slot it was loaded from",
                        )
                        .size(11.0)
                        .monospace(),
                    );
                if slot_response.clicked() {
                    clear_current_grid(pattern, params);
                    if let Some(i) = state.last_loaded_slot {
                        if let Ok(mut bank) = params.pattern_bank.bank.lock() {
                            bank.slots[i] = crate::pattern_bank::PatternSlot::default();
                            drop(bank);
                            params.pattern_bank.refresh_snapshot();
                        }
                    }
                    state.clear_confirm_mode = false;
                }
            }
            let cancel_response = chip_button(ui, "X", false, INK3(), egui::Sense::click())
                .help_or(
                    help::BANK_CLEAR_CANCEL,
                    RichText::new("Cancel").size(11.0).monospace(),
                );
            if cancel_response.clicked() {
                state.clear_confirm_mode = false;
            }
        } else {
            let clear_response = chip_button(ui, "Clr", false, BLUE(), egui::Sense::click())
                .help_or(
                    help::BANK_CLEAR,
                    RichText::new("Clear the current grid or empty the current bank slot")
                        .size(11.0)
                        .monospace(),
                );
            if clear_response.clicked() {
                state.clear_confirm_mode = true;
                state.save_mode_active = false;
            }
        }

        // Export / Drag (MIDI export, separate concern) pushed to the right edge.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let drag_response = chip_button(ui, "Drag", false, BLUE(), egui::Sense::click())
                .help_or(help::BANK_DRAG, "Drag the current pattern into your DAW");
            if drag_response.clicked() {
                let bpm = params.bpm.value();
                let pattern_length = params.pattern_length.value() as usize;
                let swing = params.swing.value();
                let groove_type = params.groove_type.value();
                match export_midi_to_documents(
                    pattern,
                    &params.track_layout.state,
                    bpm,
                    pattern_length,
                    swing,
                    groove_type,
                    &params.seq_plock_state.state,
                )
                .and_then(|path| start_external_midi_drag(&path).map(|_| path))
                {
                    Ok(path) => {
                        nih_log!("MIDI drag helper started from: {}", path.display());
                        state.last_midi_export_path = Some(path.display().to_string());
                        state.last_midi_export_error = None;
                    }
                    Err(e) => {
                        nih_log!("MIDI drag failed: {}", e);
                        state.last_midi_export_path = None;
                        state.last_midi_export_error = Some(e.to_string());
                    }
                }
            }
            if chip_button(ui, "Export", false, BLUE(), egui::Sense::click())
                .help(help::BANK_EXPORT)
                .clicked()
            {
                let bpm = params.bpm.value();
                let pattern_length = params.pattern_length.value() as usize;
                let swing = params.swing.value();
                let groove_type = params.groove_type.value();
                match export_midi_to_documents(
                    pattern,
                    &params.track_layout.state,
                    bpm,
                    pattern_length,
                    swing,
                    groove_type,
                    &params.seq_plock_state.state,
                ) {
                    Ok(path) => {
                        nih_log!("MIDI exported to: {}", path.display());
                        state.last_midi_export_path = Some(path.display().to_string());
                        state.last_midi_export_error = None;
                        // [271] Say where the file went, with an "Open folder"
                        // button (modal drawn below the bank).
                        state.midi_export_modal = Some(path);
                    }
                    Err(e) => {
                        nih_log!("MIDI export failed: {}", e);
                        state.last_midi_export_path = None;
                        state.last_midi_export_error = Some(e.to_string());
                    }
                }
            }
            if state.last_midi_export_error.is_some() {
                ui.label(RichText::new("Export failed").size(10.0).color(RED()));
            } else if state.last_midi_export_path.is_some() {
                ui.label(RichText::new("Exported").size(10.0).color(INK3()));
            }
        });
    });

    // Unsaved-changes confirmation (skeuo plate, foreground).
    crate::ui::popups::draw_pattern_load_warning_if_any(
        ui,
        params,
        pattern,
        state,
        load_pattern_request,
        audio_last_loaded_slot,
    );

    // [271] MIDI export confirmation.
    crate::ui::popups::draw_midi_export_modal_if_any(ui, state);
}

/// Capture the current pattern, plocks and fusions into the given pattern-bank slot.
pub fn save_current_pattern_to_bank_slot(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    slot: usize,
) {
    if slot >= crate::pattern_bank::SLOT_COUNT {
        return;
    }
    let Ok(mut bank) = params.pattern_bank.bank.lock() else {
        return;
    };
    let pattern_length = params.pattern_length.value() as u8;
    bank.slots[slot].capture(
        pattern,
        &params.plock_state.state,
        &params.seq_plock_state.state,
        pattern_length,
    );
    drop(bank);
    params.pattern_bank.refresh_snapshot();
}

pub fn load_pattern_for_ui(pattern_for_ui: &SharedPattern, pattern: &Pattern) {
    let masks = pattern.step_masks();
    pattern_for_ui.load_step_masks(&masks);
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        pattern_for_ui.store_fusions(inst, &pattern.fusions[inst]);
    }
}

/// Wipe the working grid (steps, sound/seq plocks, fusions) without touching
/// any bank slot — used by Clr>Grid and when positioning on an empty slot.
pub(crate) fn clear_current_grid(pattern: &SharedPattern, params: &DrumFlashParams) {
    load_pattern_for_ui(pattern, &crate::sequencer::pattern::Pattern::empty());
    params.plock_state.state.clear_all();
    params.seq_plock_state.state.clear_all();
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        pattern.store_fusions(inst, &[]);
    }
}

/// True when the working grid differs from the pattern stored in the loaded
/// slot: steps, length, fusions, sound or sequencer p-locks ([283]: it used to
/// compare only steps and length, so an edit of p-locks or fusions alone was
/// lost on a slot switch without a warning). Drives the `P1*` marker and the
/// unsaved-changes warning on slot switch.
pub(crate) fn pattern_is_dirty(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    state: &EditorUIState,
) -> bool {
    let current_masks = pattern.step_masks();
    let grid_has_content = current_masks.iter().any(|&m| m != 0);
    let current_len = params.pattern_length.value() as u8;
    let matches = |slot: &crate::pattern_bank::PatternSlot| {
        slot.matches_live(
            pattern,
            &params.plock_state.state,
            &params.seq_plock_state.state,
            current_len,
        )
    };
    match state.last_loaded_slot {
        // No slot association (e.g. a project reopened): unsaved ONLY if the grid has content AND doesn't already
        // match a stored slot. A reopened project whose pattern was saved to
        // some Pn before quitting matches that slot → NOT dirty (no false alert).
        None => {
            if !grid_has_content {
                return false;
            }
            if let Ok(bank) = params.pattern_bank.bank.lock() {
                let saved_somewhere = bank.slots.iter().any(|s| s.occupied && matches(s));
                !saved_somewhere
            } else {
                false
            }
        }
        Some(slot_idx) => {
            if let Ok(bank) = params.pattern_bank.bank.lock() {
                let slot = &bank.slots[slot_idx];
                if !slot.occupied {
                    // Positioned on an EMPTY slot: dirty if the grid has content.
                    return grid_has_content;
                }
                !matches(slot)
            } else {
                false
            }
        }
    }
}

pub fn load_pattern_for_ui_with_length(
    pattern_for_ui: &SharedPattern,
    pattern: &Pattern,
    length: usize,
) {
    let mut masks = pattern.step_masks();
    // Clear steps beyond the current pattern length
    for step in length..masks.len() {
        masks[step] = 0;
    }
    pattern_for_ui.load_step_masks(&masks);
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        pattern_for_ui.store_fusions(inst, &pattern.fusions[inst]);
    }
}
