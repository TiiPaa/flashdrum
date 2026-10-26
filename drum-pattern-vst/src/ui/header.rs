//! Header bar: brand, master/swing sliders, groove select, source + toggles.

use crate::ui::controls::{enum_combo, toggle_led_param};
use crate::ui::editor_state::EditorUIState;
use crate::ui::help::{self, HelpExt};
use crate::ui::theme::*;
use crate::{DrumFlashParams, BUILD_ID};
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};
use std::sync::{
    atomic::{AtomicBool, AtomicU32},
    Arc,
};

pub fn header_param_slider<P: Param>(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &P,
    total_w: f32,
    label: &str,
    show_value: bool,
) {
    header_param_slider_col(ui, setter, param, total_w, label, show_value, 0.0);
}

/// [`header_param_slider`] whose groove starts `label_w` after its left edge, so
/// the sliders of a group line up whatever their labels (0 = right after the label).
pub fn header_param_slider_col<P: Param>(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &P,
    total_w: f32,
    label: &str,
    show_value: bool,
    label_w: f32,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total_w, CTL_HEIGHT), egui::Sense::hover());
    let cy = rect.center().y;
    let norm = param.unmodulated_normalized_value();

    let painter = ui.painter_at(rect);
    let track_left = if label.is_empty() {
        rect.left()
    } else {
        let label_rect = painter.text(
            egui::pos2(rect.left(), cy),
            egui::Align2::LEFT_CENTER,
            label,
            f_sans_med(11.5),
            INK2(),
        );
        if label_w > 0.0 {
            rect.left() + label_w
        } else {
            label_rect.right().max(rect.left() + 34.0) + 8.0
        }
    };
    let track_right = if show_value {
        let valstr = param.normalized_value_to_string(norm, true);
        let val_rect = painter.text(
            egui::pos2(rect.right(), cy),
            egui::Align2::RIGHT_CENTER,
            &valstr,
            f_mono(11.0),
            INK(),
        );
        (val_rect.left() - 8.0).max(track_left + 12.0)
    } else {
        rect.right()
    };
    // Reserve the Ø11 knob radius at both ends so it isn't clipped at the extremes.
    let knob_r = 6.0;
    let track_left = track_left.max(rect.left() + knob_r);
    let track_right = track_right
        .min(rect.right() - knob_r)
        .max(track_left + 12.0);
    let track = egui::Rect::from_min_max(
        egui::pos2(track_left, cy - 3.0),
        egui::pos2(track_right, cy + 3.0),
    );

    let resp = ui.interact(
        track.expand2(Vec2::new(0.0, 8.0)),
        ui.make_persistent_id(("hslider", label)),
        egui::Sense::click_and_drag(),
    );
    let mut frac = norm;
    let frac_at = |x: f32| ((x - track.left()) / track.width().max(1.0)).clamp(0.0, 1.0);
    if resp.drag_started() {
        setter.begin_set_parameter(param);
    }
    if (resp.dragged() || resp.drag_started()) && resp.interact_pointer_pos().is_some() {
        frac = frac_at(resp.interact_pointer_pos().unwrap().x);
        setter.set_parameter_normalized(param, frac);
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            frac = frac_at(p.x);
            setter.begin_set_parameter(param);
            setter.set_parameter_normalized(param, frac);
            setter.end_set_parameter(param);
        }
    }
    if resp.double_clicked() {
        frac = param.default_normalized_value();
        setter.begin_set_parameter(param);
        setter.set_parameter_normalized(param, frac);
        setter.end_set_parameter(param);
    }

    // All slider visuals live in one place: `skeuo::slider_track` (Len = with cap).
    crate::ui::skeuo::slider_track(ui, track, frac, BLUE(), true);
}

/// A 2px vertical separator (height 22) with 13pt horizontal padding on each side.
fn header_vbar(ui: &mut egui::Ui) {
    // [282] 9 px each side (was 13): the +1 text size made the header 30 px
    // too wide, which pushed Settings out and widened the lane editor too.
    ui.add_space(HEADER_VBAR_GAP);
    let (r, _) = ui.allocate_exact_size(Vec2::new(2.0, 22.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, LINE());
    ui.add_space(HEADER_VBAR_GAP);
}

const HEADER_VBAR_GAP: f32 = 9.0;

/// Clear the whole program: grid (steps + fusions + sound/seq plocks on every
/// lane), all 16 pattern-bank slots, and the song. Lane kinds, sounds and
/// routing are kept.
fn clear_entire_program(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &crate::sequencer::SharedPattern,
    state: &mut EditorUIState,
) {
    // Grid.
    pattern.load_step_masks(&[0u16; crate::sequencer::pattern::STEP_COUNT]);
    crate::ui::grid::clear_all_fusions(pattern);
    params.plock_state.state.clear_all();
    params.seq_plock_state.state.clear_all();

    // No mute/solo may survive on lanes that are about to become invisible.
    crate::ui::controls::clear_all_mutes_solos(setter, params);

    // Pattern bank + song.
    if let Ok(mut bank) = params.pattern_bank.bank.lock() {
        for slot in bank.slots.iter_mut() {
            *slot = crate::pattern_bank::PatternSlot::default();
        }
        bank.song = crate::pattern_bank::SongSequence::default();
        drop(bank);
        params.pattern_bank.refresh_snapshot();
    }
    let empty_song = crate::pattern_bank::SongSequence::default();
    params.song_controller.publish(empty_song);
    state.last_published_song = Some(empty_song);

    // Lanes: deactivate every slot (the audio thread gates them off via the
    // kind-change watch).
    nih_plug::params::persist::PersistentField::<crate::track::TrackLayoutState>::set(
        &params.track_layout,
        crate::track::TrackLayoutState::empty_layout(),
    );
    state.selected_track_slot = 0;
    state.selected_instrument = 0;

    state.clear_program_confirm = None;
}

pub fn draw_header_bar(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &crate::sequencer::SharedPattern,
    state: &mut EditorUIState,
    _save_pattern_request: &Arc<AtomicU32>,
    _load_pattern_request: &Arc<AtomicU32>,
    _song_mode: &Arc<AtomicBool>,
    _song_position: &Arc<AtomicU32>,
) {
    let available = ui.available_size_before_wrap();
    let header_height = HEADER_H;
    let (rect, _) = ui.allocate_exact_size(
        egui::Vec2::new(available.x, header_height),
        egui::Sense::hover(),
    );

    // Skeuo metal header: vertical gradient + top highlight + bottom border.
    let painter = ui.painter_at(rect);
    crate::ui::widgets::vgrad(
        &painter,
        rect,
        &[(0.0, HEADER_TOP), (0.6, HEADER_MID), (1.0, HEADER_BOT)],
        0.0,
    );
    painter.line_segment(
        [rect.left_top(), rect.right_top()],
        egui::Stroke::new(1.0, Color32::from_white_alpha(30)),
    );
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, LINE()),
    );

    // Contenu avec padding horizontal
    let content_rect = rect.shrink2(egui::Vec2::new(14.0, 0.0));
    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(content_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.set_height(content_rect.height());
                ui.spacing_mut().item_spacing.x = 0.0;

                // Logotype = baked bitmap (FLASH grey + DRUM white, transparent),
                // tight-cropped via uv; version stays live text after it.
                {
                    // logotype.png = 164×48 canvas, glyph content x3..162 / y12..36
                    // INCLUSIVE — the uv max must cover the LAST content pixel
                    // (+1), otherwise the bottom pixel row is cropped off.
                    let uv = egui::Rect::from_min_max(
                        egui::pos2(3.0 / 164.0, 12.0 / 48.0),
                        egui::pos2(163.0 / 164.0, 37.0 / 48.0),
                    );
                    // Content = 160×25 px, displayed 1:1 (no mipmaps → any
                    // rescale aliases).
                    let logo_h = 25.0;
                    let logo_w = 160.0;
                    let ver = format!("v{} · {}", env!("CARGO_PKG_VERSION"), BUILD_ID);
                    let ver_font = f_mono(9.5);
                    let ver_w = ui.fonts(|f| {
                        f.layout_no_wrap(ver.clone(), ver_font.clone(), INK3())
                            .size()
                            .x
                    });
                    let total = logo_w + 12.0 + ver_w;
                    let (rect, _) =
                        ui.allocate_exact_size(Vec2::new(total, logo_h), egui::Sense::hover());
                    let img_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.left(), rect.center().y - logo_h * 0.5),
                        Vec2::new(logo_w, logo_h),
                    );
                    egui::Image::new(egui::include_image!("../../assets/logotype.png"))
                        .uv(uv)
                        .paint_at(ui, img_rect);
                    ui.painter().text(
                        egui::pos2(rect.left() + logo_w + 12.0, rect.center().y + 1.0),
                        egui::Align2::LEFT_CENTER,
                        ver,
                        ver_font,
                        INK3(),
                    );
                    help::rect(ui, rect, "hdr_version", help::HDR_VERSION);
                }

                header_vbar(ui);

                // Master / Swing sliders + Groove select
                help::zone(ui, "hdr_master", help::HDR_MASTER, |ui| {
                    header_param_slider(ui, setter, &params.master_volume, 142.0, "Master", true)
                });
                header_vbar(ui);
                help::zone(ui, "hdr_swing", help::HDR_SWING, |ui| {
                    header_param_slider(ui, setter, &params.swing, 142.0, "Swing", true)
                });
                ui.add_space(8.0);
                help::zone(ui, "hdr_groove", help::HDR_GROOVE, |ui| {
                    enum_combo(ui, setter, &params.groove_type, "")
                });

                header_vbar(ui);

                // Sequencer source GROUP: Internal/Ext MIDI + MIDI Pat together.
                let internal = params.use_internal_sequencer.value();
                let sel = help::zone(ui, "hdr_seq", help::HDR_SEQ, |ui| {
                    ui.label(RichText::new("Seq").font(f_sans_sb(10.5)).color(INK3()));
                    ui.add_space(8.0);
                    // Symmetric halves (segmented_equal).
                    crate::ui::skeuo::segmented_equal(
                        ui,
                        "seq_mode",
                        &["Internal", "Ext MIDI"],
                        if internal { 0 } else { 1 },
                    )
                });
                let want_internal = sel == 0;
                if want_internal != internal {
                    setter.begin_set_parameter(&params.use_internal_sequencer);
                    setter.set_parameter(&params.use_internal_sequencer, want_internal);
                    setter.end_set_parameter(&params.use_internal_sequencer);
                    // MIDI pattern switching only makes sense in internal sequencer mode.
                    if !want_internal && params.midi_pattern_switch.value() {
                        setter.begin_set_parameter(&params.midi_pattern_switch);
                        setter.set_parameter(&params.midi_pattern_switch, false);
                        setter.end_set_parameter(&params.midi_pattern_switch);
                    }
                }
                ui.add_space(6.0);
                // MIDI Pat is part of the Seq group (internal-sequencer only).
                let midi_pat_enabled = params.use_internal_sequencer.value();
                help::zone(ui, "hdr_midi_pat", help::HDR_MIDI_PAT, |ui| {
                    toggle_led_param(
                        ui,
                        setter,
                        &params.midi_pattern_switch,
                        "MIDI Pat",
                        midi_pat_enabled,
                    )
                });

                // Choke moved to per-slot choke groups in the Track tab (the
                // legacy global HH→OH param is hidden but still loaded).

                header_vbar(ui);

                // Push the Help + Presets + Clear All + Settings keycaps to the
                // right edge. Trailing block: Help (52) + gap (6) + Presets (80)
                // + vbar + Clear All (72) + vbar + Settings (80).
                // [283] Help took its room from Master and Swing (172 -> 142).
                let trailing =
                    52.0 + 6.0 + 80.0 + 72.0 + 80.0 + 2.0 * (2.0 * HEADER_VBAR_GAP + 2.0);
                ui.add_space((ui.available_width() - trailing).max(0.0));
                let help_on = state.global_config.help_mode;
                if crate::ui::controls::keycap_button(
                    ui,
                    "Help",
                    52.0,
                    if help_on {
                        crate::ui::widgets::KeycapState::PressedAmber
                    } else {
                        crate::ui::widgets::KeycapState::Rest
                    },
                    true,
                    f_sans_med(10.5),
                )
                .on_hover_text(crate::ui::help::HELP_KEYCAP)
                .clicked()
                {
                    state.global_config.help_mode = !help_on;
                    crate::ui::help::set(ui.ctx(), !help_on);
                    let _ = state.global_config.save();
                }
                ui.add_space(6.0);
                if crate::ui::controls::keycap_button(
                    ui,
                    "Presets",
                    80.0,
                    crate::ui::widgets::KeycapState::Rest,
                    true,
                    f_sans_med(10.5),
                )
                .help(help::HDR_PRESETS)
                .clicked()
                {
                    crate::ui::preset_browser::open(state);
                }
                header_vbar(ui);
                // Clear the whole program (grid + pattern bank + song + lanes)
                // — two clicks, the second one confirms.
                let clear_armed =
                    crate::ui::controls::confirm_armed(ui, &mut state.clear_program_confirm);
                let clear_response = crate::ui::controls::keycap_button(
                    ui,
                    if clear_armed { "Sure?" } else { "Clear All" },
                    72.0,
                    if clear_armed {
                        crate::ui::widgets::KeycapState::PressedAmber
                    } else {
                        crate::ui::widgets::KeycapState::Rest
                    },
                    true,
                    f_sans_med(10.5),
                )
                .help_or(
                    help::HDR_CLEAR_ALL,
                    "Clear everything: grid, pattern slots, song AND lanes (click twice)",
                );
                if clear_response.clicked() {
                    if clear_armed {
                        clear_entire_program(setter, params, pattern, state);
                    } else {
                        state.clear_program_confirm = Some(ui.input(|i| i.time));
                    }
                } else {
                    crate::ui::controls::confirm_disarm_elsewhere(
                        ui,
                        &mut state.clear_program_confirm,
                        &clear_response,
                    );
                }
                header_vbar(ui);
                if crate::ui::controls::keycap_button(
                    ui,
                    "Settings",
                    80.0,
                    crate::ui::widgets::KeycapState::Rest,
                    true,
                    f_sans_med(10.5),
                )
                .help(help::HDR_SETTINGS)
                .clicked()
                {
                    state.settings_open = true;
                }
            });
        },
    );
}
