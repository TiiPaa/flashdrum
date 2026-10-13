use nih_plug::prelude::*;
use nih_plug_egui::{
    create_egui_editor,
    egui::{self, Vec2},
};
use std::sync::{
    atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering},
    Arc,
};

use crate::{
    config::GlobalConfig, pattern_bank::SLOT_COUNT, plock::PlockState, sequencer::SharedPattern,
    sound_settings::SoundSettingsState, DrumFlashParams,
};

mod bottom_panel;
mod controls;
mod editor_state;
mod envelope_viz;
mod fmt;
mod grid;
mod header;
mod help;
mod local_param_slider;
mod macros_panel;
mod menus;
pub(crate) mod midi;
mod pads;
pub mod param_source;
mod pattern_bank;
mod plock;
mod popups;
mod preset_browser;
mod skeuo;
mod slider;
mod song;
mod sound_editor;
mod theme;
mod widgets;

use bottom_panel::draw_bottom_panel;
use editor_state::*;
use grid::draw_grid_v2;
use header::draw_header_bar;
use pattern_bank::{draw_pattern_bank, save_current_pattern_to_bank_slot};
use plock::draw_plock_popup;
use popups::*;
use sound_editor::draw_sound_panel;
use theme::*;

fn install_egui_fonts(ctx: &egui::Context) {
    use egui::FontFamily;
    let mut fonts = egui::FontDefinitions::default();

    // Default fallback chains (emoji / missing-glyph coverage) kept after our faces.
    let prop_fallback = fonts
        .families
        .get(&FontFamily::Proportional)
        .cloned()
        .unwrap_or_default();
    let mono_fallback = fonts
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();

    // IBM Plex faces — matches the design's weight scale (Sans 400/500/600/700, Mono 400/500/600).
    let faces: [(&str, &[u8]); 7] = [
        (
            "sans_400",
            include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf"),
        ),
        (
            "sans_500",
            include_bytes!("../assets/fonts/IBMPlexSans-Medium.ttf"),
        ),
        (
            "sans_600",
            include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf"),
        ),
        (
            "sans_700",
            include_bytes!("../assets/fonts/IBMPlexSans-Bold.ttf"),
        ),
        (
            "mono_400",
            include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf"),
        ),
        (
            "mono_500",
            include_bytes!("../assets/fonts/IBMPlexMono-Medium.ttf"),
        ),
        (
            "mono_600",
            include_bytes!("../assets/fonts/IBMPlexMono-SemiBold.ttf"),
        ),
    ];
    for (name, bytes) in faces {
        fonts.font_data.insert(
            name.to_string(),
            Arc::new(egui::FontData::from_static(bytes)),
        );
    }

    let with_fallback = |face: &str, fallback: &[String]| {
        let mut v = vec![face.to_string()];
        v.extend_from_slice(fallback);
        v
    };

    // Built-in families default to Regular (with the original fallbacks appended).
    fonts.families.insert(
        FontFamily::Proportional,
        with_fallback("sans_400", &prop_fallback),
    );
    fonts.families.insert(
        FontFamily::Monospace,
        with_fallback("mono_400", &mono_fallback),
    );

    // Named weight families — used via FontId::new(size, FontFamily::Name(..)).
    let named: [(&str, &str, &[String]); 5] = [
        ("sans_med", "sans_500", &prop_fallback),
        ("sans_sb", "sans_600", &prop_fallback),
        ("sans_bold", "sans_700", &prop_fallback),
        ("mono_med", "mono_500", &mono_fallback),
        ("mono_sb", "mono_600", &mono_fallback),
    ];
    for (alias, face, fb) in named {
        fonts
            .families
            .insert(FontFamily::Name(alias.into()), with_fallback(face, fb));
    }

    ctx.set_fonts(fonts);
}

/// Editor size at 100 % zoom, in logical pixels.
pub const EDITOR_SIZE: (u32, u32) = (1480, 800);

/// [282] Window size for a Zoom setting in percent.
fn editor_size_for(zoom_percent: u16) -> (u32, u32) {
    let scale = |v: u32| (v * zoom_percent as u32 + 50) / 100;
    (scale(EDITOR_SIZE.0), scale(EDITOR_SIZE.1))
}

pub fn create_editor(
    params: Arc<DrumFlashParams>,
    current_step: Arc<AtomicU32>,
    current_steps: Arc<[AtomicU32; crate::track::MAX_TRACKS]>,
    pattern: Arc<SharedPattern>,
    external_midi_triggers: Arc<[AtomicBool; crate::track::MAX_TRACKS]>,
    sound_settings_state: Arc<SoundSettingsState>,
    plock_state: Arc<PlockState>,
    save_pattern_request: Arc<AtomicU32>,
    load_pattern_request: Arc<AtomicU32>,
    song_mode: Arc<AtomicBool>,
    song_position: Arc<AtomicU32>,
    pending_pattern_length: Arc<AtomicI32>,
    audio_last_loaded_slot: Arc<AtomicU32>,
    global_config: GlobalConfig,
) -> Option<Box<dyn Editor>> {
    let params_for_ui = params.clone();
    let editor_state = params.editor_state.clone();
    let pattern_for_ui = pattern.clone();
    let external_midi_triggers_for_ui = external_midi_triggers.clone();
    let sound_settings_for_ui = sound_settings_state.clone();
    let current_steps_for_ui = current_steps.clone();
    let plock_for_ui = plock_state.clone();
    let song_mode_for_ui = song_mode.clone();
    let song_position_for_ui = song_position.clone();
    let pending_pattern_length_for_ui = pending_pattern_length.clone();
    let audio_last_loaded_slot_for_ui = audio_last_loaded_slot.clone();

    theme::set_skin(&global_config.skin);
    // [282] Open straight at the zoomed size (the host asks for the size
    // before the window exists).
    let zoomed_size = editor_size_for(global_config.zoom_percent);
    if editor_state.size() != zoomed_size {
        editor_state.set_requested_size(zoomed_size);
    }

    create_egui_editor(
        params.editor_state.clone(),
        {
            let mut initial_state = EditorUIState::default();
            initial_state.global_config = global_config;
            initial_state
        },
        |egui_ctx, _state| {
            install_egui_fonts(egui_ctx);
            // [280] First editor open: copy the starter presets into the
            // user's preset folders (no-op once done).
            crate::presets::seed_default_user_presets();
            // PNG loader for the skeuo pad textures (egui::include_image!).
            egui_extras::install_image_loaders(egui_ctx);

            // Style global sombre
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = BG();
            visuals.window_fill = BG();
            visuals.extreme_bg_color = BG();
            visuals.widgets.inactive.bg_fill = PANEL2();
            visuals.widgets.hovered.bg_fill = P_HOVER();
            visuals.widgets.active.bg_fill = P_ACTIVE();
            visuals.selection.bg_fill = BLUE();
            visuals.faint_bg_color = PANEL();
            visuals.extreme_bg_color = BG();
            // egui-native context menus + tooltips get a raised skeuo panel look
            // (plate-mid fill, dark border, soft drop shadow). Our custom Area
            // popups use Frame::NONE + a hand-painted plate, so they're unaffected.
            visuals.window_fill = egui::Color32::from_rgb(41, 42, 47);
            visuals.window_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(20, 20, 24));
            visuals.popup_shadow = egui::epaint::Shadow {
                offset: [0, 4],
                blur: 14,
                spread: 0,
                color: egui::Color32::from_black_alpha(110),
            };
            visuals.menu_corner_radius = egui::CornerRadius::same(RADIUS_PANEL as u8);
            visuals.widgets.noninteractive.bg_fill = BG();

            // Chrome tokens: rounded corners, hairline strokes, no hover-expansion.
            let cr = egui::CornerRadius::same(6);
            visuals.widgets.noninteractive.corner_radius = cr;
            visuals.widgets.inactive.corner_radius = cr;
            visuals.widgets.hovered.corner_radius = cr;
            visuals.widgets.active.corner_radius = cr;
            visuals.widgets.open.corner_radius = cr;
            visuals.widgets.inactive.expansion = 0.0;
            visuals.widgets.hovered.expansion = 0.0;
            visuals.widgets.active.expansion = 0.0;
            visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE());
            visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, LINE2());
            visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BLUE());
            visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, BLUE());
            visuals.widgets.open.bg_stroke = egui::Stroke::new(1.0, LINE2());
            visuals.selection.stroke = egui::Stroke::new(1.0, BLUE());
            egui_ctx.set_visuals(visuals);
            // [282] Ctrl +/- would zoom the content without growing the window:
            // the zoom is Settings → Zoom only.
            egui_ctx.options_mut(|options| options.zoom_with_keyboard = false);
            // [282] egui's own text styles (plain labels, tooltips) grow like
            // the rest of the UI.
            egui_ctx.style_mut(|style| {
                for font in style.text_styles.values_mut() {
                    font.size = theme::font_size(font.size);
                }
            });
        },
        move |egui_ctx, setter, state| {
            #[cfg(target_os = "windows")]
            nih_plug_egui::set_keyboard_focus(egui_ctx.wants_keyboard_input());

            // [281] Opt-in resource sampling (FLASH_DRUM_PERF_LOG=1) for the
            // Bitwig progressive-lag report. One cached flag load when off.
            crate::perf_log::sample_frame();

            // [253] Free the textures the audio thread retired since the last
            // frame (the queue push is RT-safe, the drop is not). Cheap when
            // empty: one atomic load.
            params_for_ui.user_textures.pool.drain_retired();

            // Apply any pending pattern length update from a slot load.
            let pending_len = pending_pattern_length_for_ui.swap(0, Ordering::Relaxed);
            if pending_len >= 1 && pending_len <= 64 {
                setter.set_parameter(&params_for_ui.pattern_length, pending_len as i32);
            }

            // Sync last_loaded_slot with the audio thread so the UI always
            // reflects the slot that was actually loaded (prevents divergence
            // when clicking rapidly while the audio thread is still restoring).
            let audio_slot = audio_last_loaded_slot_for_ui.load(Ordering::Relaxed);
            if audio_slot == u32::MAX {
                state.last_loaded_slot = None;
            } else if (audio_slot as usize) < SLOT_COUNT {
                state.last_loaded_slot = Some(audio_slot as usize);
            }

            // [283] Help mode follows the persisted setting (the header keycap
            // flips both).
            help::set(egui_ctx, state.global_config.help_mode);

            // [282] Zoom: the window follows the setting (asked once per change;
            // a host that refuses leaves everything as is), and egui's zoom
            // follows the size the host actually gave.
            let target_size = editor_size_for(state.global_config.zoom_percent);
            if state.zoom_requested_size != Some(target_size) {
                state.zoom_requested_size = Some(target_size);
                if editor_state.size() != target_size {
                    editor_state.set_requested_size(target_size);
                }
            }
            let zoom = (editor_state.size().0 as f32 / EDITOR_SIZE.0 as f32).clamp(0.5, 3.0);
            if (egui_ctx.zoom_factor() - zoom).abs() > 0.001 {
                egui_ctx.set_zoom_factor(zoom);
            }

            egui::CentralPanel::default().show(egui_ctx, |panel_ui| {
                    // The whole window, panel margins included.
                    let full_rect = panel_ui.clip_rect();
                    let mut content_ui = panel_ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(full_rect)
                            .layout(*panel_ui.layout()),
                    );
                    let ui = &mut content_ui;
                    draw_header_bar(
                        ui,
                        setter,
                        &params_for_ui,
                        &pattern_for_ui,
                        state,
                        &save_pattern_request,
                        &load_pattern_request,
                        &song_mode_for_ui,
                        &song_position_for_ui,
                    );

                    let body_h = ui.available_height();
                    let body_w = ui.available_width();
                    let (body_rect, _) =
                        ui.allocate_exact_size(Vec2::new(body_w, body_h), egui::Sense::hover());
                    crate::ui::widgets::vgrad(
                        ui.painter(),
                        body_rect,
                        &[(0.0, WINDOW_BG_TOP), (1.0, WINDOW_BG_BOT)],
                        0.0,
                    );

                    let right_w = 568.0;
                    let left_w = (body_rect.width() - right_w).max(0.0);
                    let left_rect =
                        egui::Rect::from_min_size(body_rect.min, Vec2::new(left_w, body_h));
                    let right_rect = egui::Rect::from_min_size(
                        egui::pos2(left_rect.right(), body_rect.top()),
                        Vec2::new(right_w, body_h),
                    );

                    ui.painter().rect_filled(right_rect, 0.0, PANEL_SKEUO);

                    ui.allocate_new_ui(
                        egui::UiBuilder::new()
                            .max_rect(left_rect.shrink2(Vec2::new(14.0, 14.0)))
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                        |ui| {
                            ui.set_clip_rect(left_rect.shrink2(Vec2::new(1.0, 0.0)));
                            ui.set_width(left_rect.width() - 28.0);
                            ui.set_height(left_rect.height() - 28.0);
                            // Tighter vertical rhythm to fit the 800px window.
                            ui.spacing_mut().item_spacing.y = 10.0;
                            draw_grid_v2(
                                ui,
                                setter,
                                &params_for_ui,
                                &pattern_for_ui,
                                &external_midi_triggers_for_ui,
                                &current_step,
                                &current_steps_for_ui,
                                &sound_settings_for_ui,
                                &plock_for_ui,
                                state,
                            );
                            draw_pattern_bank(
                                ui,
                                state,
                                &params_for_ui,
                                &pattern_for_ui,
                                &save_pattern_request,
                                &load_pattern_request,
                                &audio_last_loaded_slot_for_ui,
                            );
                            draw_bottom_panel(
                                ui,
                                setter,
                                &params_for_ui,
                                &pattern_for_ui,
                                state,
                                &song_mode_for_ui,
                                &song_position_for_ui,
                            );
                        },
                    );

                    ui.allocate_new_ui(
                        egui::UiBuilder::new()
                            .max_rect(right_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                        |ui| {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, PANEL_SKEUO);
                            ui.set_width(right_w);
                            ui.set_height(body_h);
                            draw_sound_panel(
                                ui,
                                &sound_settings_for_ui,
                                &params_for_ui,
                                setter,
                                state,
                                // [184] The panel edits either the lane global or
                                // the selected cell's p-lock; the pattern is what
                                // tells it whether that cell is inside a fusion.
                                &plock_for_ui,
                                &pattern_for_ui,
                            );
                        },
                    );

                    // Seam between the left column (recessed grid area) and the
                    // raised Lane Editor panel — drawn LAST so it stays above the
                    // panel fill: a dark crease + a 1px light highlight on the
                    // panel side (skeuo bevel). A plain hairline was invisible now
                    // that both surfaces share the same skeuo grey.
                    let seam_x = right_rect.left();
                    ui.painter().vline(
                        seam_x,
                        body_rect.y_range(),
                        egui::Stroke::new(1.0, PANEL_BORDER),
                    );
                    ui.painter().vline(
                        seam_x + 1.0,
                        body_rect.y_range(),
                        egui::Stroke::new(1.0, egui::Color32::from_rgb(58, 59, 66)),
                    );

                    // Custom plock popup to avoid egui context_menu chrome.
                    draw_plock_popup(
                        egui_ctx,
                        setter,
                        &params_for_ui,
                        &pattern_for_ui,
                        &plock_for_ui,
                        state,
                    );

                    // Global settings popup (default analog, MIDI prefs, etc.).
                    draw_settings_popup_if_any(ui, setter, &params_for_ui, state);

                    // Presets modal (instruments / patterns / songs).
                    preset_browser::draw_preset_browser_if_any(
                        ui,
                        setter,
                        &params_for_ui,
                        &pattern_for_ui,
                        &sound_settings_for_ui,
                        state,
                    );

                    // [242] Macros assignment modal (CC -> lane sound param).
                    macros_panel::draw_macros_modal_if_any(ui, setter, &params_for_ui, state);

                    // Auto-save pattern edits to the current bank slot when Song Mode is active.
                    // This prevents edits from being lost when the song advances to the next pattern.
                    if params_for_ui.song_mode.value() {
                        if let Some(slot) = state.pattern_dirty_slot.take() {
                            save_current_pattern_to_bank_slot(
                                &params_for_ui,
                                &pattern_for_ui,
                                slot,
                            );
                        }
                    } else {
                        state.pattern_dirty_slot = None;
                    }
                });
        },
    )
}
// Pattern grid with per-row Hum/Push/Len
// ---------------------------------------------------------------------------------------------------------------
