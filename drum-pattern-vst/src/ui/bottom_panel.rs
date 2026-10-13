//! Bottom panel: Generator | Song tabs, generator controls, preset chips.

use crate::generator::{self, GeneratorType, Style};
use crate::pattern_bank::SONG_BLOCKS;
use crate::sequencer::{Pattern, SharedPattern};
use crate::track::{TrackInstrumentKind, TrackLayoutState};
use crate::ui::controls::{
    chip_button, compact_chip, enum_combo_compact, generator_song_segmented, genrow_label,
};
use crate::ui::editor_state::EditorUIState;
use crate::ui::header::header_param_slider;
use crate::ui::help::{self, HelpExt};
use crate::ui::pattern_bank::load_pattern_for_ui_with_length;
use crate::ui::song::draw_song_editor;
use crate::ui::theme::*;
use crate::DrumFlashParams;
use nih_plug::{params::persist::PersistentField, prelude::*};
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
        format!(
            "{} · {} -> {}",
            GeneratorType::variants()[params.generator_type.value().to_index()],
            Style::variants()[params.style_primary.value().to_index()],
            Style::variants()[params.style_secondary.value().to_index()]
        )
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

fn draw_generator_panel_content(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    state: &mut EditorUIState,
) {
    ui.vertical(|ui| {
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.y = 12.0;
        draw_generator_bar(ui, setter, params, pattern);
        draw_preset_bar(ui, pattern, params, setter, state);
    });
}

fn draw_preset_bar(
    ui: &mut egui::Ui,
    pattern: &SharedPattern,
    params: &DrumFlashParams,
    setter: &ParamSetter,
    state: &mut EditorUIState,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 7.0;
        ui.set_height(CTL_HEIGHT);
        genrow_label(ui, "Presets", 62.0);
        let pattern_length = params.pattern_length.value() as usize;
        if compact_chip(ui, "Rock", false)
            .help(help::PRESET_CHIPS_BASIC)
            .clicked()
        {
            params.plock_state.state.clear_all();
            params.seq_plock_state.state.clear_all();
            crate::ui::grid::clear_all_fusions(pattern);
            load_pattern_for_ui_with_length(pattern, &Pattern::rock_pattern(), pattern_length);
        }
        if compact_chip(ui, "Funk", false)
            .help(help::PRESET_CHIPS_BASIC)
            .clicked()
        {
            params.plock_state.state.clear_all();
            params.seq_plock_state.state.clear_all();
            crate::ui::grid::clear_all_fusions(pattern);
            load_pattern_for_ui_with_length(pattern, &Pattern::funk_pattern(), pattern_length);
        }
        if compact_chip(ui, "Disco", false)
            .help(help::PRESET_CHIPS_BASIC)
            .clicked()
        {
            params.plock_state.state.clear_all();
            params.seq_plock_state.state.clear_all();
            crate::ui::grid::clear_all_fusions(pattern);
            load_pattern_for_ui_with_length(pattern, &Pattern::disco_pattern(), pattern_length);
        }
        // Style presets: install the matching kit (lanes) + load the fixed groove.
        use TrackInstrumentKind as K;
        let style_presets: [(&str, &[TrackInstrumentKind], fn() -> Pattern); 6] = [
            (
                "House",
                &[K::Kick, K::Clap, K::HiHat, K::OpenHiHat, K::Perc1],
                Pattern::house_pattern,
            ),
            (
                "Dub",
                &[K::Kick, K::Snare, K::HiHat, K::BassDrum808, K::Perc1],
                Pattern::dub_pattern,
            ),
            (
                "DnB",
                &[K::Kick, K::Snare, K::HiHat, K::Snare606, K::BassDrum808],
                Pattern::dnb_pattern,
            ),
            (
                "Bossa",
                &[K::Kick, K::Snare, K::Ride, K::HiHat, K::Perc1],
                Pattern::bossa_pattern,
            ),
            (
                "Afro",
                &[K::Kick, K::Snare, K::HiHat, K::Ride, K::Tom, K::Perc1],
                Pattern::afrobeat_pattern,
            ),
            (
                "Break",
                &[K::Kick, K::Snare, K::HiHat, K::Snare606, K::Ride],
                Pattern::breakbeat_pattern,
            ),
        ];
        for (label, kinds, make) in style_presets {
            if compact_chip(ui, label, false)
                .help(help::PRESET_CHIPS_KIT)
                .clicked()
            {
                apply_style_preset(
                    params,
                    setter,
                    state,
                    pattern,
                    kinds,
                    make(),
                    pattern_length,
                );
            }
        }
        ui.add_space(8.0);
        if chip_button(ui, "⟳ Random", true, AMBER(), egui::Sense::click())
            .help(help::PRESET_RANDOM)
            .clicked()
        {
            params.plock_state.state.clear_all();
            params.seq_plock_state.state.clear_all();
            crate::ui::grid::clear_all_fusions(pattern);
            load_pattern_for_ui_with_length(pattern, &Pattern::random_pattern(), pattern_length);
        }
    });
}

/// Install a style preset: replace the lane layout with `kinds` (reseeding each
/// lane's sound to the kind's defaults) and load the deterministic `groove`.
/// Destructive — like the per-lane Type change it goes through
/// `PersistentField::set`, so the audio thread reinitialises the voices.
fn apply_style_preset(
    params: &DrumFlashParams,
    setter: &ParamSetter,
    state: &mut EditorUIState,
    pattern: &SharedPattern,
    kinds: &[TrackInstrumentKind],
    groove: Pattern,
    pattern_length: usize,
) {
    let mut layout = TrackLayoutState::from_kinds(kinds);
    // [283] The kit replaces the lanes, not the MIDI channel set in Settings.
    let current = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    layout.keep_globals_of(&current);
    PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
    // The layout replaced the lanes: no mute/solo may survive on a lane that
    // changed instrument or disappeared.
    crate::ui::controls::clear_all_mutes_solos(setter, params);
    let analog = state.global_config.default_analog;
    for (slot, &kind) in kinds.iter().enumerate() {
        params
            .sound_settings
            .state
            .reset_slot_to_defaults(slot, kind, analog);
    }
    params.plock_state.state.clear_all();
    params.seq_plock_state.state.clear_all();
    crate::ui::grid::clear_all_fusions(pattern);
    load_pattern_for_ui_with_length(pattern, &groove, pattern_length);
    state.selected_instrument = 0;
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

fn draw_generator_bar(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
) {
    const GEN_TYPE_W: f32 = 116.0;
    // Fits the longest style name ("Drum'n'Bass") next to the caret.
    const STYLE_W: f32 = 104.0;
    const GEN_BTN_W: f32 = 104.0;

    // Two rows: (1) Type · style A → Mix → B   (2) Density · Variation · GENERATE.
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 6.0;

        // Row 1 — algorithm + style morph.
        ui.horizontal(|ui| {
            ui.set_height(CTL_HEIGHT);
            ui.spacing_mut().item_spacing.x = 0.0;
            // [283] Each control explains itself in help mode.
            help::zone(ui, "gen_type", help::GEN_TYPE, |ui| {
                genrow_label(ui, "Type", 40.0);
                enum_combo_compact(ui, setter, &params.generator_type, "gen_type", GEN_TYPE_W);
            });
            ui.add_space(16.0);
            help::zone(ui, "style_a", help::GEN_STYLE_A, |ui| {
                genrow_label(ui, "A", 12.0);
                ui.add_space(5.0);
                enum_combo_compact(ui, setter, &params.style_primary, "style_a", STYLE_W);
            });
            // [283] Only Probabilistic blends A with B: greyed for the others.
            let blends = params.generator_type.value() == GeneratorType::Probabilistic;
            ui.add_space(14.0);
            probabilistic_only(ui, "style_mix", help::GEN_MIX, blends, |ui| {
                header_param_slider(ui, setter, &params.style_mix, 110.0, "Mix A/B", false);
            });
            ui.add_space(14.0);
            probabilistic_only(ui, "style_b", help::GEN_STYLE_B, blends, |ui| {
                genrow_label(ui, "B", 12.0);
                ui.add_space(5.0);
                enum_combo_compact(ui, setter, &params.style_secondary, "style_b", STYLE_W);
            });
        });

        // Row 2 — amounts + GENERATE (right).
        ui.horizontal(|ui| {
            ui.set_height(CTL_HEIGHT);
            ui.spacing_mut().item_spacing.x = 0.0;
            help::zone(ui, "gen_density", help::GEN_DENSITY, |ui| {
                header_param_slider(ui, setter, &params.gen_density, 150.0, "Density", false);
            });
            ui.add_space(18.0);
            help::zone(ui, "gen_variation", help::GEN_VARIATION, |ui| {
                header_param_slider(ui, setter, &params.gen_variation, 150.0, "Variation", false);
            });

            let space = (ui.available_width() - GEN_BTN_W).max(10.0);
            ui.add_space(space);
            let gen_btn_response = help::zone(ui, "generate", help::GEN_GENERATE, |ui| {
                crate::ui::controls::keycap_button(
                    ui,
                    "GENERATE",
                    GEN_BTN_W,
                    crate::ui::widgets::KeycapState::PressedAmber,
                    true,
                    f_sans_sb(11.0),
                )
            });
            if gen_btn_response.clicked() {
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
                let generated =
                    generator::generate(&gen_params, params.track_layout.state.as_ref());
                let pattern_length = params.pattern_length.value() as usize;
                crate::ui::grid::clear_all_fusions(pattern);
                // [283] The grid stays on the slot it was loaded from (P3*):
                // a generated pattern is an edit of that slot, like any other
                // (user decision; MIDI Pat saves it there before switching).
                load_pattern_for_ui_with_length(pattern, &generated, pattern_length);
            }
        });
    });
}
