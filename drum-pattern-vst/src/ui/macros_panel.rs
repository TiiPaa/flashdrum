//! [242] Macros modal (build 2): assign each host-visible "Macro N" knob to
//! one SOUND parameter of an instantiated lane. The knobs themselves are
//! driven from the DAW (Control Link / automation); this modal only says
//! what they drive. The engine, the pruning rules and the persistence live
//! in `crate::macros` (build 1).

use crate::instrument_registry::StandardField;
use crate::macros::{MacroTarget, MACRO_COUNT};
use crate::track::{TrackLayoutState, MAX_TRACKS};
use crate::ui::controls::keycap_button;
use crate::ui::editor_state::EditorUIState;
use crate::ui::help::{self, HelpExt};
use crate::ui::menus::page_menu_frame;
use crate::ui::theme::*;
use crate::ui::widgets::{styled_select, KeycapState};
use crate::DrumFlashParams;
use nih_plug::params::persist::PersistentField;
use nih_plug::prelude::{FloatParam, Param, ParamSetter};
use nih_plug_egui::egui::{self, RichText, Vec2};

/// [242 build 3] Macro knob: a miniature of the PROVEN `header_param_slider`
/// interaction (Master/Swing) — begin at drag start, `set_parameter_normalized`
/// per frame (value = pointer position, no accumulated delta), end at drag
/// stop. Studio One's MIDI learn follows the gestures, and the knob reflects
/// DAW writes the rest of the time. The first attempt used nih_plug's
/// `ParamSlider` and froze S1 ~1 min; the second (one-shot commit) broke the
/// drag entirely. This is the same code path as the header, which never did
/// either.
fn macro_knob(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &FloatParam,
    macro_idx: usize,
) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(96.0, 14.0), egui::Sense::hover());
    let id = ui.id().with(("macro_knob", macro_idx));
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());

    let frac_at = |x: f32| ((x - rect.left()) / rect.width()).clamp(0.0, 1.0);
    if response.drag_started() {
        setter.begin_set_parameter(param);
    }
    if (response.dragged() || response.drag_started()) && response.interact_pointer_pos().is_some()
    {
        setter.set_parameter_normalized(param, frac_at(response.interact_pointer_pos().unwrap().x));
    }
    if response.drag_stopped() {
        setter.end_set_parameter(param);
    }
    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            setter.begin_set_parameter(param);
            setter.set_parameter_normalized(param, frac_at(pos.x));
            setter.end_set_parameter(param);
        }
    }

    let norm = param.unmodulated_normalized_value();
    let p = ui.painter();
    p.rect_filled(rect, 3.0, crate::ui::theme::WELL_FILL);
    let fill = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * norm, rect.height()));
    p.rect_filled(fill, 3.0, BLUE());
    p.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, crate::ui::theme::PANEL_BORDER),
        egui::StrokeKind::Inside,
    );
    response
}

/// Display name of a lane, same rule as the grid: custom name, else the
/// instrument's label.
fn lane_name(layout: &TrackLayoutState, slot: usize) -> String {
    let s = &layout.slots[slot];
    if s.name.is_empty() {
        crate::instrument_registry::INSTRUMENTS[s.kind.drum_voice_index()]
            .label
            .to_string()
    } else {
        s.name.clone()
    }
}

/// Every assignable parameter of a lane's kind: its declared standards (Freq,
/// Decay, Volume, …) then its declared specials (Offset, Gate Rate, …).
fn param_options(
    slot: usize,
    kind: crate::track::TrackInstrumentKind,
) -> Vec<(MacroTarget, String)> {
    let def = kind.instrument_def();
    let mut out: Vec<(MacroTarget, String)> = def
        .standard_params
        .iter()
        .map(|d| (MacroTarget::Std(slot, d.field), d.label.to_string()))
        .collect();
    out.extend(def.special_params.iter().map(|d| {
        (
            MacroTarget::Special(slot, d.special_index),
            d.label.to_string(),
        )
    }));
    out
}

pub fn draw_macros_modal_if_any(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    state: &mut EditorUIState,
) {
    if !state.macros_open {
        return;
    }

    let screen = ui.ctx().screen_rect();
    // Sized on the content: 52+96+108+168+22 + spacing = 470 wide, sixteen
    // rows tall - no empty band on the right or below (user feedback).
    let size = Vec2::new(505.0, 570.0);
    let origin = egui::pos2(
        screen.center().x - size.x / 2.0,
        (screen.center().y - size.y / 2.0).max(screen.top() + 8.0),
    );
    let area_id = ui.id().with("macros_modal");

    let response = egui::Area::new(area_id)
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(origin)
        .show(ui.ctx(), |ui| {
            page_menu_frame(ui, BLUE(), |ui| {
                ui.set_min_width(size.x - 24.0);
                ui.set_max_width(size.x - 24.0);
                ui.set_height(size.y - 24.0);

                // Header
                let mut close = false;
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Macros").font(f_sans_sb(11.0)).color(BLUE()));
                    ui.add_space((ui.available_width() - 22.0).max(0.0));
                    if keycap_button(ui, "x", 22.0, KeycapState::Rest, true, f_sans_med(12.0))
                        .clicked()
                    {
                        close = true;
                    }
                });
                if close {
                    state.macros_open = false;
                    return;
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Drive a Macro knob from your DAW (MIDI CC / automation); it writes the assigned parameter like a hand drag - p-locks still win per step.",
                    )
                    .font(f_sans_med(9.5))
                    .color(INK3()),
                );
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                let layout =
                    PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
                let active_lanes: Vec<usize> = (0..MAX_TRACKS)
                    .filter(|&i| layout.slots[i].active)
                    .collect();
                let map = &params.macro_map_state.state;

                for macro_idx in 0..MACRO_COUNT {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        // Fixed-width label column, RIGHT-aligned: the units
                        // digits of "Macro 1".."Macro 16" line up against the
                        // knob instead of ragged left-aligned tens.
                        ui.allocate_ui_with_layout(
                            Vec2::new(52.0, 16.0),
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.label(
                                    RichText::new(format!("Macro {}", macro_idx + 1))
                                        .font(f_mono_med(9.5))
                                        .color(INK3()),
                                );
                            },
                        );
                        // See `macro_knob`: the header slider's interaction,
                        // learnable from Studio One.
                        let macro_param = params.macro_params()[macro_idx];
                        macro_knob(ui, setter, macro_param, macro_idx).help_or(
                            help::MACRO_KNOB,
                            "Drag to set. Studio One's MIDI learn sees this knob.",
                        );
                        let current = map.get(macro_idx);

                                        // Lane select: "-" (unassigned) + the active lanes.
                                        let mut lane_labels: Vec<String> = vec!["-".to_string()];
                                        lane_labels.extend(active_lanes.iter().map(|&s| {
                                            format!("{} {}", s + 1, lane_name(&layout, s))
                                        }));
                                        let lane_refs: Vec<&str> =
                                            lane_labels.iter().map(|s| s.as_str()).collect();
                                        let lane_sel = current
                                            .map(|t| {
                                                active_lanes
                                                    .iter()
                                                    .position(|&s| s == t.slot())
                                                    .map(|p| p + 1)
                                                    .unwrap_or(0)
                                            })
                                            .unwrap_or(0);
                                        let (lane_response, lane_pick) = styled_select(
                                            ui,
                                            ("macro_lane", macro_idx),
                                            lane_sel,
                                            &lane_refs,
                                            108.0,
                                        );
                                        lane_response.help(help::MACRO_LANE);

                                        let lane_after = match lane_pick {
                                            Some(0) => {
                                                map.set(macro_idx, None);
                                                None
                                            }
                                            Some(p) => {
                                                let slot = active_lanes[p - 1];
                                                // New lane: start on its first
                                                // standard parameter.
                                                let first = layout.slots[slot]
                                                    .kind
                                                    .instrument_def()
                                                    .standard_params
                                                    .first()
                                                    .map(|d| d.field)
                                                    .unwrap_or(StandardField::Freq);
                                                map.set(
                                                    macro_idx,
                                                    Some(MacroTarget::Std(slot, first)),
                                                );
                                                Some(slot)
                                            }
                                            None => current.map(|t| t.slot()),
                                        };

                                        // Param select, for the lane the row
                                        // points at. Unassigned rows get a
                                        // DISABLED select (greyed, not hidden:
                                        // the rows must not reflow).
                                        let Some(slot) = lane_after else {
                                            ui.add_enabled_ui(false, |ui| {
                                                let (_, _) = styled_select(
                                                    ui,
                                                    ("macro_param", macro_idx),
                                                    0,
                                                    &["-"],
                                                    168.0,
                                                );
                                            });
                                            ui.add_enabled_ui(false, |ui| {
                                                let _ = keycap_button(
                                                    ui, "x", 22.0,
                                                    KeycapState::Rest, true, f_sans_med(10.0),
                                                );
                                            });
                                            return;
                                        };
                                        let options = param_options(slot, layout.slots[slot].kind);
                                        let option_refs: Vec<&str> =
                                            options.iter().map(|(_, l)| l.as_str()).collect();
                                        let now = map.get(macro_idx);
                                        let param_sel = now
                                            .and_then(|t| {
                                                options.iter().position(|(o, _)| match (o, t) {
                                                    (MacroTarget::Std(_, f), MacroTarget::Std(_, tf)) => *f == tf,
                                                    (MacroTarget::Special(_, i), MacroTarget::Special(_, ti)) => *i == ti,
                                                    _ => false,
                                                })
                                            })
                                            .unwrap_or(0);
                                        let (param_response, param_pick) = styled_select(
                                            ui,
                                            ("macro_param", macro_idx),
                                            param_sel,
                                            &option_refs,
                                            168.0,
                                        );
                                        param_response.help(help::MACRO_PARAM);
                                        if let Some(p) = param_pick {
                                            map.set(macro_idx, Some(options[p].0));
                                        }

                                        if keycap_button(
                                            ui,
                                            "x",
                                            22.0,
                                            KeycapState::Rest,
                                            true,
                                            f_sans_med(10.0),
                                        )
                                        .help_or(help::MACRO_CLEAR, "Clear this assignment")
                                        .clicked()
                                        {
                                            map.set(macro_idx, None);
                                        }
                                    });
                                }
            });
        })
        .response;

    // Close when clicking outside the modal.
    let clicked_outside = ui.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .map_or(false, |pos| !response.rect.contains(pos))
    });
    if clicked_outside {
        state.macros_open = false;
    }
}
