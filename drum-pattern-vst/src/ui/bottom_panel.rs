//! Bottom panel: Generator | Song tabs, the Generator (PATTERN / STYLE / CREATE).

use crate::generator::{self, GeneratorType, Style};
use crate::pattern_bank::SONG_BLOCKS;
use crate::sequencer::{Pattern, SharedPattern};
use crate::ui::controls::{enum_combo_compact, generator_song_segmented};
use crate::ui::editor_state::{EditorUIState, GenAction};
use crate::ui::header::header_param_slider_col;
use crate::ui::help;
use crate::ui::pattern_bank::load_pattern_for_ui_with_length;
use crate::ui::song::draw_song_editor;
use crate::ui::theme::*;
use crate::DrumFlashParams;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Vec2};
use std::sync::{
    atomic::{AtomicBool, AtomicU32},
    Arc,
};

pub fn draw_bottom_panel(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    state: &mut EditorUIState,
    _song_mode: &Arc<AtomicBool>,
    song_position: &Arc<AtomicU32>,
) {
    let panel_w = ui.available_width();
    // Compact height sized for the 800px window (designer target): the
    // Generator view needs ~70px of body and the Song editor ~110px.
    let panel_h = 168.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(panel_w, panel_h), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, RADIUS_PANEL, PANEL());
    painter.rect_stroke(
        rect,
        RADIUS_PANEL,
        egui::Stroke::new(1.0, LINE()),
        egui::StrokeKind::Inside,
    );
    const BOTTOM_HEADER_H: f32 = 42.0;
    const BOTTOM_SEPARATOR_Y: f32 = 42.0;
    const BOTTOM_BODY_TOP: f32 = 44.0;
    const BOTTOM_PAD_X: f32 = 12.0;

    painter.hline(
        rect.x_range(),
        rect.top() + BOTTOM_SEPARATOR_Y,
        egui::Stroke::new(1.0, LINE()),
    );

    let header_rect = egui::Rect::from_min_size(rect.min, Vec2::new(panel_w, BOTTOM_HEADER_H));
    let switch_top = rect.top() + ((BOTTOM_HEADER_H - CTL_HEIGHT) * 0.5).floor();
    let switch_rect = egui::Rect::from_min_size(
        egui::pos2(rect.left() + BOTTOM_PAD_X, switch_top),
        Vec2::new(150.0, CTL_HEIGHT),
    );
    let selected = state.bottom_panel_tab.min(1);
    let switch_right = ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(switch_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Min)),
        |ui| {
            // Generator | Song segmented tabs: view only, no longer toggle song mode.
            ui.set_clip_rect(header_rect);
            ui.spacing_mut().item_spacing.x = 0.0;
            let new_selected = help::zone(ui, "bottom_tabs", help::BOTTOM_TABS, |ui| {
                generator_song_segmented(ui, selected)
            });
            if new_selected != selected {
                state.bottom_panel_tab = new_selected;
            }
        },
    )
    .response
    .rect
    .right();

    let meta = if state.bottom_panel_tab == 1 {
        if let Ok(bank) = params.pattern_bank.bank.lock() {
            let blocks = (bank.song.length as usize).min(SONG_BLOCKS);
            let total_reps = bank.song.steps[..blocks]
                .iter()
                .filter(|&&s| s >= 0)
                .count();
            format!("{} blocks · {} patterns", blocks, total_reps)
        } else {
            "Song chain".to_string()
        }
    } else {
        let gen_type = params.generator_type.value();
        let type_name = GeneratorType::variants()[gen_type.to_index()];
        let style_a = Style::variants()[params.style_primary.value().to_index()];
        // Only Probabilistic blends A into B: the others never hear B.
        if gen_type == GeneratorType::Probabilistic {
            let style_b = Style::variants()[params.style_secondary.value().to_index()];
            format!("{type_name} · {style_a} -> {style_b}")
        } else {
            format!("{type_name} · {style_a}")
        }
    };
    painter.text(
        // [282] After the switch's real width: a fixed x (142) let the
        // switch, wider since the +1 font bump, run under the text.
        egui::pos2(switch_right + 12.0, switch_rect.center().y),
        egui::Align2::LEFT_CENTER,
        meta,
        f_mono_med(10.5),
        INK3(),
    );

    let body_rect = egui::Rect::from_min_max(
        egui::pos2(rect.left() + BOTTOM_PAD_X, rect.top() + BOTTOM_BODY_TOP),
        egui::pos2(rect.right() - BOTTOM_PAD_X, rect.bottom() - 8.0),
    );
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(body_rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.set_clip_rect(body_rect);
            ui.set_width(body_rect.width());
            ui.set_height(body_rect.height());
            if state.bottom_panel_tab == 1 {
                draw_song_editor(ui, setter, params, state, song_position);
            } else {
                draw_generator_panel_content(ui, setter, params, pattern, state);
            }
        },
    );
}

/// Place `add` in `rect`, left to right, vertically centred.
fn at<R>(ui: &mut egui::Ui, rect: egui::Rect, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            add(ui)
        },
    )
    .inner
}

/// A row label in the sliders' face.
fn row_label(ui: &egui::Ui, x: f32, y: f32, text: &str) {
    ui.painter().text(
        egui::pos2(x, y + CTL_HEIGHT * 0.5),
        egui::Align2::LEFT_CENTER,
        text,
        f_sans_med(11.5),
        INK2(),
    );
}

/// The Generator, in three groups: **PATTERN** (Type, Density, Variation),
/// **STYLE** (A and B side by side, Mix A/B under them) and **CREATE** (a tall
/// GENERATE and Full random under it, same width). Every row label of a group
/// sits in one column, so its controls start at the same x.
fn draw_generator_panel_content(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    state: &mut EditorUIState,
) {
    const LABEL_W: f32 = 64.0;
    const PATTERN_W: f32 = 268.0;
    const STYLE_W: f32 = 332.0;
    const GROUP_GAP: f32 = 16.0;
    const ROW_GAP: f32 = 7.0;
    let body = ui.max_rect();
    let title_y = body.top() + 11.0;
    let row = |i: f32| body.top() + 22.0 + i * (CTL_HEIGHT + ROW_GAP);
    let painter = ui.painter().clone();
    let divider = |x: f32| {
        painter.vline(x, (body.top() + 6.0)..=(body.bottom() - 2.0), egui::Stroke::new(1.0, LINE()));
    };

    // PATTERN
    let g1 = body.left() + 2.0;
    crate::ui::skeuo::group_title(&painter, egui::pos2(g1, title_y), "PATTERN");
    let type_rect = egui::Rect::from_min_size(egui::pos2(g1, row(0.0)), Vec2::new(PATTERN_W, CTL_HEIGHT));
    at(ui, type_rect, |ui| {
        help::zone(ui, "gen_type", help::GEN_TYPE, |ui| {
            row_label(ui, g1, row(0.0), "Type");
            ui.add_space(LABEL_W);
            enum_combo_compact(ui, setter, &params.generator_type, "gen_type", PATTERN_W - LABEL_W);
        });
    });
    let density_rect = egui::Rect::from_min_size(egui::pos2(g1, row(1.0)), Vec2::new(PATTERN_W, CTL_HEIGHT));
    at(ui, density_rect, |ui| {
        help::zone(ui, "gen_density", help::GEN_DENSITY, |ui| {
            header_param_slider_col(ui, setter, &params.gen_density, PATTERN_W, "Density", false, LABEL_W);
        });
    });
    let variation_rect = egui::Rect::from_min_size(egui::pos2(g1, row(2.0)), Vec2::new(PATTERN_W, CTL_HEIGHT));
    at(ui, variation_rect, |ui| {
        help::zone(ui, "gen_variation", help::GEN_VARIATION, |ui| {
            header_param_slider_col(ui, setter, &params.gen_variation, PATTERN_W, "Variation", false, LABEL_W);
        });
    });
    let d1 = g1 + PATTERN_W + GROUP_GAP;
    divider(d1);

    // STYLE
    let g2 = d1 + GROUP_GAP;
    crate::ui::skeuo::group_title(&painter, egui::pos2(g2, title_y), "STYLE");
    // [283] Only Probabilistic blends A with B: Mix and B are greyed for the others.
    let blends = params.generator_type.value() == GeneratorType::Probabilistic;
    let half = (STYLE_W - 12.0) * 0.5;
    let a_rect = egui::Rect::from_min_size(egui::pos2(g2, row(0.0)), Vec2::new(half, CTL_HEIGHT));
    at(ui, a_rect, |ui| {
        help::zone(ui, "style_a", help::GEN_STYLE_A, |ui| {
            row_label(ui, g2, row(0.0), "A");
            ui.add_space(16.0);
            enum_combo_compact(ui, setter, &params.style_primary, "style_a", half - 16.0);
        });
    });
    let bx = g2 + half + 12.0;
    let b_rect = egui::Rect::from_min_size(egui::pos2(bx, row(0.0)), Vec2::new(half, CTL_HEIGHT));
    at(ui, b_rect, |ui| {
        probabilistic_only(ui, "style_b", help::GEN_STYLE_B, blends, |ui| {
            row_label(ui, bx, row(0.0), "B");
            ui.add_space(16.0);
            enum_combo_compact(ui, setter, &params.style_secondary, "style_b", half - 16.0);
        });
    });
    let mix_rect = egui::Rect::from_min_size(egui::pos2(g2, row(1.0)), Vec2::new(STYLE_W, CTL_HEIGHT));
    at(ui, mix_rect, |ui| {
        probabilistic_only(ui, "style_mix", help::GEN_MIX, blends, |ui| {
            header_param_slider_col(ui, setter, &params.style_mix, STYLE_W, "Mix A/B", false, LABEL_W);
        });
    });
    painter.text(
        egui::pos2(g2, row(2.0) + CTL_HEIGHT * 0.5),
        egui::Align2::LEFT_CENTER,
        "Mix blends A into B (Probabilistic only)",
        f_sans(10.5),
        INK3(),
    );
    let d2 = g2 + STYLE_W + GROUP_GAP;
    divider(d2);

    // CREATE. Both actions replace the grid: on an unsaved pattern the first
    // click turns the key into "Sure?", like every two-click button, and a second
    // click on the SAME key goes ahead (no pointer travel); it disarms after
    // `CONFIRM_TIMEOUT_S` or on a click anywhere else.
    use crate::ui::controls::CONFIRM_TIMEOUT_S;
    let now = ui.input(|i| i.time);
    if let Some((_, armed_at)) = state.generate_confirm {
        if now - armed_at >= CONFIRM_TIMEOUT_S {
            state.generate_confirm = None;
        }
    }
    let armed = |action: GenAction| matches!(state.generate_confirm, Some((a, _)) if a == action);
    let generate_armed = armed(GenAction::Generate);
    let random_armed = armed(GenAction::FullRandom);
    if generate_armed || random_armed {
        // So the key reverts on its own.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(CONFIRM_TIMEOUT_S));
    }
    let g3 = d2 + GROUP_GAP;
    let g3w = (body.right() - 2.0 - g3).max(80.0);
    crate::ui::skeuo::group_title(&painter, egui::pos2(g3, title_y), "CREATE");
    let gen_rect = egui::Rect::from_min_max(egui::pos2(g3, row(0.0)), egui::pos2(g3 + g3w, row(1.0) + CTL_HEIGHT));
    let generate = at(ui, gen_rect, |ui| {
        help::zone(ui, "generate", help::GEN_GENERATE, |ui| {
            if generate_armed {
                crate::ui::controls::keycap_button_sized(
                    ui,
                    "Sure?",
                    gen_rect.size(),
                    crate::ui::widgets::KeycapState::PressedAmber,
                    true,
                    f_sans_sb(13.0),
                )
            } else {
                crate::ui::controls::keycap_button_sized(
                    ui,
                    "GENERATE",
                    gen_rect.size(),
                    crate::ui::widgets::KeycapState::PressedAmber,
                    true,
                    f_sans_sb(13.0),
                )
            }
        })
    });
    let random_rect = egui::Rect::from_min_size(egui::pos2(g3, row(2.0)), Vec2::new(g3w, CTL_HEIGHT));
    let random = at(ui, random_rect, |ui| {
        help::zone(ui, "full_random", help::PRESET_RANDOM, |ui| {
            crate::ui::controls::full_random_button(ui, random_rect.size(), random_armed)
        })
    });
    let clicked = if generate.clicked() {
        Some((GenAction::Generate, generate_armed))
    } else if random.clicked() {
        Some((GenAction::FullRandom, random_armed))
    } else {
        None
    };
    match clicked {
        Some((action, was_armed)) => {
            if !was_armed && crate::ui::pattern_bank::pattern_is_dirty(params, pattern, state) {
                state.generate_confirm = Some((action, now));
            } else {
                state.generate_confirm = None;
                match action {
                    GenAction::Generate => generate_pattern(params, pattern),
                    GenAction::FullRandom => {
                        params.plock_state.state.clear_all();
                        params.seq_plock_state.state.clear_all();
                        crate::ui::grid::clear_all_fusions(pattern);
                        let pattern_length = params.pattern_length.value() as usize;
                        // An effect lane takes no trigger (like GENERATE).
                        let layout = params.track_layout.state.as_ref();
                        let random = Pattern::random_pattern(|slot| {
                            !layout.kind_for_slot(slot).is_some_and(|kind| kind.is_effect())
                        });
                        load_pattern_for_ui_with_length(pattern, &random, pattern_length);
                    }
                }
            }
        }
        None => {
            // (The press of the confirming click lands on the key itself: only a
            // press elsewhere cancels.)
            if state.generate_confirm.is_some()
                && ui.input(|i| i.pointer.any_pressed())
                && !generate.hovered()
                && !random.hovered()
            {
                // Clicked anywhere else while armed: cancel.
                state.generate_confirm = None;
            }
        }
    }
}

/// GENERATE: a new pattern from the Generator's settings, on the loaded slot.
fn generate_pattern(params: &DrumFlashParams, pattern: &SharedPattern) {
    params.plock_state.state.clear_all();
    params.seq_plock_state.state.clear_all();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let gen_params = generator::GeneratorParams {
        generator_type: params.generator_type.value(),
        style_primary: params.style_primary.value(),
        style_secondary: params.style_secondary.value(),
        style_mix: params.style_mix.value(),
        density: params.gen_density.value(),
        variation: params.gen_variation.value(),
        seed,
    };
    let generated = generator::generate(&gen_params, params.track_layout.state.as_ref());
    let pattern_length = params.pattern_length.value() as usize;
    crate::ui::grid::clear_all_fusions(pattern);
    // [283] The grid stays on the slot it was loaded from (P3*):
    // a generated pattern is an edit of that slot, like any other
    // (user decision; MIDI Pat saves it there before switching).
    load_pattern_for_ui_with_length(pattern, &generated, pattern_length);
}

/// [283] A generator control only the Probabilistic type reads: greyed with
/// the other types, saying why (help mode shows its full text instead).
fn probabilistic_only(
    ui: &mut egui::Ui,
    id_salt: &str,
    help_text: &str,
    enabled: bool,
    add: impl FnOnce(&mut egui::Ui),
) {
    let rect = ui.add_enabled_ui(enabled, add).response.rect;
    help::rect(ui, rect, id_salt, help_text);
    if !enabled && !help::is_on(ui.ctx()) {
        ui.interact(
            rect,
            ui.id().with(("gen_unused", id_salt)),
            egui::Sense::hover(),
        )
        .on_hover_text_at_pointer(help::GEN_PROBABILISTIC_ONLY);
    }
}
