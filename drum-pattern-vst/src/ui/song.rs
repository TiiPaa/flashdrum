//! Song editor: 16 song blocks (pattern selector + repeat), clear all.

use crate::pattern_bank::{SLOT_COUNT, SONG_BLOCKS};
use crate::ui::editor_state::EditorUIState;
use crate::ui::help::{self, HelpExt};
use crate::ui::theme::*;
use crate::DrumFlashParams;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Vec2};
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc,
};

pub fn draw_song_editor(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    state: &mut EditorUIState,
    song_position: &Arc<AtomicU32>,
) {
    let current_song_pos = song_position.load(Ordering::Relaxed) as usize;
    let is_song_active = params.song_mode.value();

    let mut bank = match params.pattern_bank.bank.lock() {
        Ok(b) => b,
        Err(_) => return,
    };

    // Always use 16 song blocks and always loop.
    bank.song.length = SONG_BLOCKS as u8;
    bank.song.loop_enabled = true;

    let selected = state.song_selected_step.min(SONG_BLOCKS - 1);

    // Header: Song Mode checkbox, Clear All
    ui.horizontal(|ui| {
        ui.set_height(26.0);
        ui.spacing_mut().item_spacing.x = 6.0;

        let mut song_enabled = params.song_mode.value();
        if ui
            .checkbox(&mut song_enabled, "Song Mode")
            .help(help::SONG_MODE)
            .changed()
        {
            setter.set_parameter(&params.song_mode, song_enabled);
        }

        ui.add_space(8.0);
        // Keycap norm (chip_button): red label while confirming.
        if crate::ui::controls::confirm_armed(ui, &mut state.song_clear_confirm) {
            let confirm =
                crate::ui::controls::chip_button(ui, "Confirm?", true, RED(), egui::Sense::click())
                    .help(help::SONG_CLEAR);
            if confirm.clicked() {
                for step in 0..SONG_BLOCKS {
                    bank.song.set_step(step, -1);
                    bank.song.set_repeat(step, 1);
                }
                state.song_clear_confirm = None;
            } else {
                crate::ui::controls::confirm_disarm_elsewhere(
                    ui,
                    &mut state.song_clear_confirm,
                    &confirm,
                );
            }
        } else if crate::ui::controls::chip_button(
            ui,
            "Clear All",
            false,
            BLUE(),
            egui::Sense::click(),
        )
        .help(help::SONG_CLEAR)
        .clicked()
        {
            state.song_clear_confirm = Some(ui.input(|i| i.time));
        }
    });

    ui.add_space(4.0);

    // Step grid: 1 row of 16 editable blocks (pattern on top, repeat on bottom).
    let body_w = ui.available_width();
    let cell_h = 64.0;
    let cell_w = ((body_w - 2.0 * (SONG_BLOCKS as f32 - 1.0)) / SONG_BLOCKS as f32).max(18.0);
    let steps_per_row = SONG_BLOCKS;

    ui.horizontal(|ui| {
        ui.set_height(cell_h);
        ui.spacing_mut().item_spacing.x = 2.0;
        for step_idx in 0..steps_per_row {
            let is_current = step_idx == current_song_pos && is_song_active;
            let is_selected = step_idx == selected;
            let slot = bank.song.steps[step_idx];
            let occupied =
                slot >= 0 && (slot as usize) < SLOT_COUNT && bank.slots[slot as usize].occupied;

            // Playing block is signalled by a red play LED (top-right corner),
            // like the page bar — not by a blue fill. Selection keeps a blue border.
            let fill = if occupied { PANEL2() } else { SONG_EMPTY() };
            let stroke_color = if is_selected { BLUE() } else { LINE2() };

            let (rect, response) =
                ui.allocate_exact_size(Vec2::new(cell_w, cell_h), egui::Sense::click());
            ui.painter().rect_filled(rect, 3.0, fill);
            ui.painter().rect_stroke(
                rect,
                3.0,
                egui::Stroke::new(1.0, stroke_color),
                egui::StrokeKind::Inside,
            );

            // Top half: pattern selector.
            let inner = rect.shrink(2.0);
            let top_rect = egui::Rect::from_min_size(
                inner.min,
                Vec2::new(inner.width(), inner.height() * 0.5),
            );
            let top_response = ui
                .allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(top_rect)
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                    |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            // Skeuo dropdown (styled_select → skeuo::keycap). Options are
                            // "--" plus each occupied slot; map the picked index back.
                            let slot = bank.song.steps[step_idx];
                            let mut labels: Vec<String> = vec!["--".to_string()];
                            let mut values: Vec<i8> = vec![-1];
                            for i in 0..SLOT_COUNT {
                                if bank.slots[i].occupied {
                                    labels.push(format!("P{}", i + 1));
                                    values.push(i as i8);
                                }
                            }
                            let refs: Vec<&str> = labels.iter().map(|s| s.as_str()).collect();
                            let cur = values.iter().position(|v| *v == slot).unwrap_or(0);
                            let w = ui.available_width().max(20.0);
                            let (select, picked) = crate::ui::widgets::styled_select_centered(
                                ui,
                                format!("song_pattern_select_{}", step_idx),
                                cur,
                                &refs,
                                w,
                            );
                            if let Some(pick) = picked {
                                let new_slot = values.get(pick).copied().unwrap_or(-1);
                                if new_slot != bank.song.steps[step_idx] {
                                    bank.song.set_step(step_idx, new_slot);
                                }
                            }
                            select
                        })
                        .inner
                    },
                )
                .inner;

            // Bottom half: repeat editor.
            let bottom_rect = egui::Rect::from_min_size(
                egui::pos2(inner.left(), inner.top() + inner.height() * 0.5),
                Vec2::new(inner.width(), inner.height() * 0.5),
            );
            let repeat_response = ui
                .allocate_new_ui(
                    egui::UiBuilder::new()
                        .max_rect(bottom_rect)
                        .layout(egui::Layout::top_down(egui::Align::Center)),
                    |ui| {
                        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                            let mut repeat = bank.song.repeats[step_idx];
                            let drag = ui.add_sized(
                                Vec2::new(ui.available_width(), ui.available_height()),
                                egui::DragValue::new(&mut repeat)
                                    .range(1..=64)
                                    .speed(1.0)
                                    .fixed_decimals(0)
                                    .custom_formatter(|n, _| {
                                        if n <= 1.0 {
                                            "x1".to_string()
                                        } else {
                                            format!("x{}", n as i64)
                                        }
                                    }),
                            );
                            if repeat != bank.song.repeats[step_idx] {
                                bank.song.set_repeat(step_idx, repeat);
                            }
                            drag
                        })
                        .inner
                    },
                )
                .inner;

            // Red play LED in the top-right corner when this block is playing
            // (inset like the page bar, not hugging the edge).
            help::rect(ui, rect, ("song_block", step_idx), help::SONG_BLOCK);
            if is_current {
                crate::ui::skeuo::play_led(
                    ui.painter(),
                    egui::pos2(rect.right() - 11.0, rect.top() + 11.0),
                    3.0,
                );
            }

            if response.clicked() {
                state.song_selected_step = step_idx;
            }
            // [283] The block's menu opens on a right-click anywhere on it:
            // the pattern list and the xN field cover almost all of it.
            for r in [&response, &top_response, &repeat_response] {
                r.context_menu(|ui| song_block_menu(ui, &mut bank, state, step_idx));
            }
        }
    });

    // Publish any song change to the audio-thread lock-free controller.
    let current_song = bank.song;
    let song_changed = state.last_published_song != Some(current_song);
    drop(bank);
    if song_changed {
        params.pattern_bank.refresh_snapshot();
        params.song_controller.publish(current_song);
        state.last_published_song = Some(current_song);
    }
}

/// [283] Copy / Paste / Duplicate / Clear of one song block.
fn song_block_menu(
    ui: &mut egui::Ui,
    bank: &mut crate::pattern_bank::PatternBank,
    state: &mut EditorUIState,
    step_idx: usize,
) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.set_min_width(110.0);
    ui.set_max_width(110.0);
    use crate::ui::menus::context_menu_button;
    if context_menu_button(ui, "Copy", INK(), true).clicked() {
        state.song_clipboard = Some((bank.song.steps[step_idx], bank.song.repeats[step_idx]));
        ui.close_menu();
    }
    let can_paste = state.song_clipboard.is_some();
    if context_menu_button(ui, "Paste", INK(), can_paste).clicked() && can_paste {
        if let Some((slot, repeat)) = state.song_clipboard {
            bank.song.set_step(step_idx, slot);
            bank.song.set_repeat(step_idx, repeat);
        }
        ui.close_menu();
    }
    if context_menu_button(ui, "Duplicate", INK(), true).clicked() {
        let next = step_idx + 1;
        if next < SONG_BLOCKS {
            let slot = bank.song.steps[step_idx];
            let repeat = bank.song.repeats[step_idx];
            bank.song.set_step(next, slot);
            bank.song.set_repeat(next, repeat);
        }
        ui.close_menu();
    }
    if context_menu_button(ui, "Clear", RED(), true).clicked() {
        bank.song.set_step(step_idx, -1);
        bank.song.set_repeat(step_idx, 1);
        ui.close_menu();
    }
}
