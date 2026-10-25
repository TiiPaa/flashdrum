//! Popups: Add Module picker, page menu, global settings.

use crate::plock::PlockState;
use crate::sequencer::SharedPattern;
use crate::track::TrackLayoutState;
use crate::ui::editor_state::{EditorUIState, PageMenuAction};
use crate::ui::help::{self, HelpExt};
use crate::ui::local_param_slider::LocalParamSlider;
use crate::ui::menus::{page_menu_frame, page_menu_header, plock_menu_action_row};
use crate::ui::theme;
use crate::ui::theme::*;
use crate::ui::widgets::styled_select;
use crate::DrumFlashParams;
use nih_plug::{params::persist::PersistentField, prelude::*};
use nih_plug_egui::egui::{self, RichText, Vec2};

pub fn draw_page_popup_if_any(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    pattern: &SharedPattern,
    params: &DrumFlashParams,
    plock: &PlockState,
    state: &mut EditorUIState,
) {
    let Some(mut popup) = state.page_popup else {
        return;
    };

    let page = popup.page;
    let has_clipboard = state.page_clipboard.is_some();
    let confirm_action = popup.confirm_action;
    let accent = BLUE();

    let area_id = ui.id().with("page_popup");
    let response = egui::Area::new(area_id)
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(popup.screen_pos)
        .show(ui.ctx(), |ui| {
            page_menu_frame(ui, accent, |ui| {
                page_menu_header(ui, &format!("Page {}", page + 1), accent);

                // [212] Loop this page alone / release it. Greyed in Song
                // mode, where the loop is ignored.
                let looped = params.page_loop.value() == page as i32 + 1;
                let song_mode = params.song_mode.value();
                let loop_label = if looped {
                    "Stop page loop"
                } else {
                    "Loop this page"
                };
                let loop_color = if song_mode { INK3() } else { AMBER() };
                if plock_menu_action_row(ui, loop_label, loop_color)
                    .help(help::PAGE_MENU_LOOP)
                    .clicked()
                    && !song_mode
                {
                    crate::ui::grid::set_page_loop_param(
                        setter,
                        params,
                        if looped { None } else { Some(page) },
                    );
                    state.page_popup = None;
                }

                if plock_menu_action_row(ui, "Copy", accent)
                    .help(help::PAGE_MENU_COPY)
                    .clicked()
                {
                    state.page_clipboard = Some(crate::ui::grid::copy_page_to_clipboard(
                        pattern, plock, params, page,
                    ));
                    state.page_popup = None;
                }

                if confirm_action == Some(PageMenuAction::Paste) {
                    ui.label(
                        RichText::new("Overwrite page?")
                            .font(f_sans_med(10.0))
                            .color(INK3()),
                    );
                    if plock_menu_action_row(ui, "Yes, overwrite", AMBER()).clicked() {
                        if let Some(ref clipboard) = state.page_clipboard {
                            crate::ui::grid::paste_page_from_clipboard(
                                pattern, plock, params, page, clipboard,
                            );
                            // Auto-extend pattern length so the pasted page is actually played.
                            let required_len = ((page + 1) * 16).clamp(1, 64) as i32;
                            let current_len = params.pattern_length.value().clamp(1, 64) as i32;
                            if required_len > current_len {
                                setter.set_parameter(&params.pattern_length, required_len);
                            }
                        }
                        state.page_popup = None;
                    }
                    if plock_menu_action_row(ui, "No, cancel", INK3()).clicked() {
                        state.page_popup = None;
                    }
                } else {
                    let paste_enabled = has_clipboard;
                    let paste_color = if paste_enabled { AMBER() } else { INK3() };
                    if plock_menu_action_row(ui, "Paste", paste_color)
                        .help(help::PAGE_MENU_PASTE)
                        .clicked()
                        && paste_enabled
                    {
                        popup.confirm_action = Some(PageMenuAction::Paste);
                        state.page_popup = Some(popup);
                    }
                }

                if confirm_action == Some(PageMenuAction::Clear) {
                    ui.label(
                        RichText::new("Clear page?")
                            .font(f_sans_med(10.0))
                            .color(INK3()),
                    );
                    if plock_menu_action_row(ui, "Yes, clear", RED()).clicked() {
                        crate::ui::grid::clear_page_for_ui(pattern, plock, params, page);
                        state.page_popup = None;
                    }
                    if plock_menu_action_row(ui, "No, cancel", INK3()).clicked() {
                        state.page_popup = None;
                    }
                } else {
                    if plock_menu_action_row(ui, "Clear", RED())
                        .help(help::PAGE_MENU_CLEAR)
                        .clicked()
                    {
                        popup.confirm_action = Some(PageMenuAction::Clear);
                        state.page_popup = Some(popup);
                    }
                }
            });
        })
        .response;

    // Close popup when clicking outside.
    let clicked_outside = ui.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .map_or(false, |pos| !response.rect.contains(pos))
    });
    if clicked_outside {
        state.page_popup = None;
    }
}

/// [283] Where the Lane 1 Root Note waits for Auto-assign. One fixed id: the
/// field and the button sit in different child `Ui`s, and `ui.id()` differs
/// from one to the other, so the button used to read nothing and always
/// started from 36.
fn root_note_id() -> egui::Id {
    egui::Id::new("flash_drum_midi_auto_assign_base")
}

/// Global settings popup (default analog value, MIDI settings, skin).
/// [230] Settings > Auto-assign outputs, ONE action on click: every active
/// lane goes to the aux output of its own number (lane N -> Out N). Through
/// `assign_slot_output`, so an assigned lane leaves the main mix exactly as a
/// hand assignment does; the user puts it back by hand if wanted. Nothing is
/// remembered: it is a shortcut for fourteen picks, not a mode.
pub fn assign_outputs_in_order(params: &DrumFlashParams) {
    let layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    let mut next = layout.clone();
    let mut changed = false;
    for (i, slot) in layout.slots.iter().enumerate() {
        // [286] An effect bus keeps its own routing (it returns into the main mix).
        if !slot.active || slot.kind.is_effect() {
            continue;
        }
        let want = crate::track::TrackAudioOut::Out(i as u8 + 1);
        if slot.routing.out_select != want {
            next.assign_slot_output(i, want);
            changed = true;
        }
    }
    if changed {
        PersistentField::<TrackLayoutState>::set(&params.track_layout, next);
    }
}

/// [239] Settings > Auto-assign MIDI notes, ONE action on click: the active
/// lanes, in slot order, take consecutive notes from `base` (first active lane
/// -> base, second -> base+1, ...). A shortcut for fourteen DragValue picks,
/// not a mode - and it clears note collisions between lanes by construction.
/// Returns whether anything changed (pure, testable without the params).
fn auto_assign_notes_in_order(layout: &mut TrackLayoutState, base: u8) -> bool {
    let mut note = base;
    let mut changed = false;
    for slot in layout.slots.iter_mut() {
        // [286] An effect bus has no MIDI note.
        if !slot.active || slot.kind.is_effect() {
            continue;
        }
        let want = note.min(127);
        if slot.midi_note != want {
            slot.midi_note = want;
            changed = true;
        }
        note = note.saturating_add(1);
    }
    changed
}

pub fn assign_midi_notes_in_order(params: &DrumFlashParams, base: u8) {
    let mut layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    if auto_assign_notes_in_order(&mut layout, base) {
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
    }
}

pub fn draw_settings_popup_if_any(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    state: &mut EditorUIState,
) {
    if !state.settings_open {
        return;
    }

    // [282] Wide enough for its longest row with the +1 text size ("Number
    // lanes from the root note" + Auto-assign): narrower, the rows overflowed
    // and the close button sat mid-way.
    const SETTINGS_W: f32 = 280.0;
    let anchor = ui.max_rect().right_top() + Vec2::new(-(SETTINGS_W + 30.0), 60.0);
    let area_id = ui.id().with("settings_popup");

    let response = egui::Area::new(area_id)
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(anchor)
        .show(ui.ctx(), |ui| {
            page_menu_frame(ui, BLUE(), |ui| {
                ui.set_min_width(SETTINGS_W);
                ui.set_max_width(SETTINGS_W);

                // Header with close button
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Settings").font(f_sans_sb(11.0)).color(BLUE()));
                    ui.add_space((ui.available_width() - 22.0).max(0.0));
                    if crate::ui::controls::keycap_button(
                        ui,
                        "×",
                        22.0,
                        crate::ui::widgets::KeycapState::Rest,
                        true,
                        f_sans_med(12.0),
                    )
                    .help(help::SET_CLOSE)
                    .clicked()
                    {
                        state.settings_open = false;
                    }
                });
                ui.add_space(12.0);

                // ---- Audio ----
                ui.label(RichText::new("Audio").font(f_sans_sb(11.0)).color(BLUE()));
                ui.add_space(6.0);

                // [230] Auto-assign outputs: one click, lane N -> Out N.
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Outputs")
                            .font(f_sans_med(10.5))
                            .color(INK3()),
                    );
                    ui.add_space((ui.available_width() - 96.0).max(0.0));
                    if crate::ui::controls::keycap_button(
                        ui,
                        "Auto-assign",
                        96.0,
                        crate::ui::widgets::KeycapState::Rest,
                        true,
                        f_sans_med(9.5),
                    )
                    .help_or(
                        help::SET_OUTPUTS,
                        "Route every active lane to the aux output of its own number, now: lane 1 to Out 1, lane 2 to Out 2, and so on. Each assigned lane leaves the main mix, as with any aux assignment; switch its Main Mix back on by hand if you want both. A one-time action, not a mode.",
                    )
                    .clicked()
                    {
                        assign_outputs_in_order(params);
                    }
                });
                ui.add_space(10.0);

                // Default Analog
                let analog_label = ui.label(
                    RichText::new("Default Analog")
                        .font(f_sans_med(10.5))
                        .color(INK3()),
                );
                ui.add_space(4.0);
                let mut value = state.global_config.default_analog;
                let slider =
                    LocalParamSlider::new(&mut value, 0.0..=1.0).reset_value(0.5);
                let slider = ui.add(slider);
                help::rect(
                    ui,
                    analog_label.rect.union(slider.rect),
                    "set_default_analog",
                    help::SET_DEFAULT_ANALOG,
                );
                if slider.changed() {
                    state.global_config.default_analog = value.clamp(0.0, 1.0);
                    let _ = state.global_config.save();
                }

                ui.add_space(14.0);

                // ---- MIDI ----
                ui.label(RichText::new("MIDI").font(f_sans_sb(11.0)).color(BLUE()));
                ui.add_space(6.0);

                // Global MIDI Channel
                help::zone(ui, "set_midi_channel", help::SET_MIDI_CHANNEL, |ui| ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Global MIDI Channel")
                            .font(f_sans_med(10.5))
                            .color(INK3()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut channel = state.global_config.global_midi_channel as i32;
                        if ui
                            .add(egui::DragValue::new(&mut channel).range(1..=16).speed(1.0))
                            .changed()
                        {
                            let channel = channel.clamp(1, 16) as u8;
                            state.global_config.global_midi_channel = channel;
                            let _ = state.global_config.save();
                            // Also update the current track layout so the change
                            // is heard immediately without reloading the project.
                            let mut layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
                            layout.global_midi_channel = channel;
                            PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
                        }
                    });
                }));
                ui.add_space(8.0);

                // [239] Auto-assign MIDI notes: the active lanes take
                // consecutive notes starting from LANE 1's root note, ONE
                // action on click (same philosophy as [230] above).
                help::zone(ui, "set_root_note", help::SET_ROOT_NOTE, |ui| ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Lane 1 Root Note")
                            .font(f_sans_med(10.5))
                            .color(INK3()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let base_id = root_note_id();
                        let mut base = ui
                            .ctx()
                            .memory_mut(|m| m.data.get_temp::<i32>(base_id))
                            .unwrap_or(36);
                        if ui
                            .add(egui::DragValue::new(&mut base).range(0..=127).speed(1.0))
                            .changed()
                        {
                            base = base.clamp(0, 127);
                            ui.ctx().memory_mut(|m| m.data.insert_temp(base_id, base));
                        }
                    });
                }));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    // Label on the left, like the Macros row below — no
                    // full-width caption shifting the popup's layout.
                    ui.label(
                        RichText::new("Number lanes from the root note")
                            .font(f_sans_med(10.5))
                            .color(INK3()),
                    );
                    ui.add_space((ui.available_width() - 96.0).max(0.0));
                    if crate::ui::controls::keycap_button(
                        ui,
                        "Auto-assign",
                        96.0,
                        crate::ui::widgets::KeycapState::Rest,
                        true,
                        f_sans_med(9.5),
                    )
                    .help_or(
                        help::SET_AUTO_NOTES,
                        "Number every active lane's MIDI note from the Lane 1 Root Note, in lane order: lane 1 gets the root, lane 2 gets root+1, and so on. A one-time action, not a mode.",
                    )
                    .clicked()
                    {
                        let base = ui
                            .ctx()
                            .memory_mut(|m| {
                                m.data.get_temp::<i32>(root_note_id())
                            })
                            .unwrap_or(36);
                        assign_midi_notes_in_order(params, base.clamp(0, 127) as u8);
                    }
                });

                ui.add_space(10.0);

                // [242] Macro assignments live in their own modal.
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Macros").font(f_sans_med(10.5)).color(INK3()));
                    ui.add_space((ui.available_width() - 96.0).max(0.0));
                    if crate::ui::controls::keycap_button(
                        ui,
                        "Edit...",
                        96.0,
                        crate::ui::widgets::KeycapState::Rest,
                        true,
                        f_sans_med(9.5),
                    )
                    .help_or(
                        help::SET_MACROS,
                        "Assign each Macro knob (visible to your DAW as an automatable parameter) to one sound parameter of a lane.",
                    )
                    .clicked()
                    {
                        state.macros_open = true;
                    }
                });

                ui.add_space(14.0);

                // ---- [285] Randomize Lane: the density each instrument type starts with ----
                ui.label(RichText::new("Randomize Lane").font(f_sans_sb(11.0)).color(BLUE()));
                ui.add_space(6.0);
                let kinds: Vec<crate::track::TrackInstrumentKind> = crate::track::TrackInstrumentKind::ALL
                    .iter()
                    .copied()
                    .filter(|kind| !kind.is_effect())
                    .collect();
                let kind_names: Vec<&str> = kinds.iter().map(|kind| kind.default_name()).collect();
                let kind_id = egui::Id::new("settings_randomize_kind");
                let kind_idx = ui
                    .ctx()
                    .memory_mut(|m| m.data.get_temp::<usize>(kind_id))
                    .unwrap_or(0)
                    .min(kinds.len().saturating_sub(1));
                let kind_label = ui.label(
                    RichText::new("Instrument")
                        .font(f_sans_med(10.5))
                        .color(INK3()),
                );
                ui.add_space(4.0);
                let (response, picked) =
                    styled_select(ui, "settings_randomize_kind_menu", kind_idx, &kind_names, 120.0);
                help::rect(
                    ui,
                    kind_label.rect.union(response.rect),
                    "set_randomize_kind",
                    help::SET_RANDOMIZE_KIND,
                );
                let kind_idx = picked.map_or(kind_idx, |idx| idx.min(kinds.len().saturating_sub(1)));
                if picked.is_some() {
                    ui.ctx().memory_mut(|m| m.data.insert_temp(kind_id, kind_idx));
                }
                ui.add_space(6.0);
                let density_label = ui.label(
                    RichText::new("Default Density")
                        .font(f_sans_med(10.5))
                        .color(INK3()),
                );
                ui.add_space(4.0);
                let kind = kinds[kind_idx];
                let mut density = state.global_config.randomize_density_for(kind);
                let slider = ui.add(
                    LocalParamSlider::new(&mut density, 0.05..=1.0)
                        .reset_value(crate::track::DEFAULT_RANDOMIZE_DENSITY),
                );
                help::rect(
                    ui,
                    density_label.rect.union(slider.rect),
                    "set_randomize_density",
                    help::SET_RANDOMIZE_DENSITY,
                );
                if slider.changed() {
                    state.global_config.set_randomize_density_for(kind, density);
                    let _ = state.global_config.save();
                }

                ui.add_space(14.0);

                // ---- Others ----
                ui.label(RichText::new("Others").font(f_sans_sb(11.0)).color(BLUE()));
                ui.add_space(6.0);

                // Auto-Edit (moved here from the header).
                help::zone(ui, "set_auto_edit", help::SET_AUTO_EDIT, |ui| ui.horizontal(|ui| {
                    ui.label(RichText::new("Auto-Edit").font(f_sans_med(10.5)).color(INK3()));
                    ui.add_space((ui.available_width() - 34.0).max(0.0));
                    let checked = params.auto_edit.value();
                    if ui.add(crate::ui::widgets::ToggleSwitch::new(checked)).clicked() {
                        crate::ui::controls::set_bool_param_if_changed(setter, &params.auto_edit, !checked);
                    }
                }));
                ui.add_space(8.0);

                // Skin selector
                ui.label(
                    RichText::new("Skin")
                        .font(f_sans_med(10.5))
                        .color(INK3()),
                );
                ui.add_space(4.0);
                let skin_names: Vec<&str> =
                    theme::SKINS.iter().map(|(name, _)| *name).collect();
                let current_idx = skin_names
                    .iter()
                    .position(|n| *n == theme::skin_name())
                    .unwrap_or(0);
                let (response, picked) =
                    styled_select(ui, "settings_skin", current_idx, &skin_names, 120.0);
                response.help(help::SET_SKIN);
                if let Some(idx) = picked {
                    let name = skin_names[idx.min(skin_names.len().saturating_sub(1))];
                    theme::set_skin(name);
                    state.global_config.skin = name.to_string();
                    let _ = state.global_config.save();
                }
                ui.add_space(8.0);

                // [282] Zoom selector (window + text size).
                ui.label(
                    RichText::new("Zoom")
                        .font(f_sans_med(10.5))
                        .color(INK3()),
                );
                ui.add_space(4.0);
                let zoom_labels: Vec<String> = crate::config::ZOOM_STEPS
                    .iter()
                    .map(|percent| format!("{percent}%"))
                    .collect();
                let zoom_names: Vec<&str> = zoom_labels.iter().map(String::as_str).collect();
                let current_zoom = crate::config::ZOOM_STEPS
                    .iter()
                    .position(|&percent| percent == state.global_config.zoom_percent)
                    .unwrap_or(0);
                let (response, picked) =
                    styled_select(ui, "settings_zoom", current_zoom, &zoom_names, 120.0);
                response.help(help::SET_ZOOM);
                if let Some(idx) = picked {
                    state.global_config.zoom_percent = crate::config::ZOOM_STEPS
                        [idx.min(crate::config::ZOOM_STEPS.len() - 1)];
                    let _ = state.global_config.save();
                }

                ui.add_space(14.0);
                ui.separator();
                ui.add_space(8.0);

                // About — credit for the ported *(AC) voices (MIT license:
                // attribution required, and well deserved).
                ui.label(RichText::new("About").font(f_sans_sb(11.0)).color(BLUE()));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "The (AC) drum voices are ported from '606 Inspired Synth Drums' by Matthew Fecher (analogcode / AudioKit Pro), MIT License. Thank you Matthew!",
                    )
                    .font(f_sans_med(9.5))
                    .color(INK3()),
                );
                ui.add_space(2.0);
                ui.hyperlink_to(
                    RichText::new("github.com/analogcode/606-Inspired-Synth-Drums")
                        .font(f_sans_med(9.5))
                        .color(BLUE()),
                    "https://github.com/analogcode/606-Inspired-Synth-Drums",
                );
            });
        })
        .response;

    // Close popup when clicking outside.
    let clicked_outside = ui.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .map_or(false, |pos| !response.rect.contains(pos))
    });
    if clicked_outside {
        state.settings_open = false;
    }
}

pub fn draw_pattern_load_warning_if_any(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    state: &mut EditorUIState,
    load_pattern_request: &std::sync::Arc<std::sync::atomic::AtomicU32>,
    audio_last_loaded_slot: &std::sync::Arc<std::sync::atomic::AtomicU32>,
) {
    let Some(slot) = state.pattern_load_confirm else {
        return;
    };

    let screen_rect = ui.ctx().screen_rect();
    let panel_w = 338.0;
    let pos = egui::pos2(
        screen_rect.center().x - panel_w * 0.5,
        screen_rect.center().y - 40.0,
    );
    egui::Area::new(ui.id().with("pattern_load_warning"))
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            let bg = ui.painter().add(egui::Shape::Noop);
            let resp = egui::Frame::NONE
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(panel_w);
                    ui.label(RichText::new("Warning").font(f_sans_sb(12.0)).color(RED()));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "The current pattern has unsaved changes. Switching to P{} will discard them.",
                            slot + 1
                        ))
                        .font(f_sans_med(10.5))
                        .color(INK2()),
                    );
                    ui.add_space(10.0);
                    // Shared tail of the two confirming actions: switch to the
                    // target slot (loading its pattern, or a fresh empty grid).
                    let switch_to_slot = |state: &mut EditorUIState| {
                        let occupied = params
                            .pattern_bank
                            .bank
                            .lock()
                            .map(|b| b.slots[slot].occupied)
                            .unwrap_or(false);
                        if occupied {
                            load_pattern_request
                                .store((slot + 1) as u32, std::sync::atomic::Ordering::Relaxed);
                        } else {
                            crate::ui::pattern_bank::clear_current_grid(pattern, params);
                            // [283] Positioning on an empty slot is UI-only (no
                            // load request): publish it, or the next frame's
                            // sync puts the grid back on the previous slot -
                            // which MIDI Pat would then overwrite with it.
                            audio_last_loaded_slot
                                .store(slot as u32, std::sync::atomic::Ordering::Relaxed);
                        }
                        state.last_loaded_slot = Some(slot);
                        state.pattern_load_confirm = None;
                    };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        // Save the current pattern into ITS slot first, then switch.
                        if let Some(current) = state.last_loaded_slot {
                            if crate::ui::controls::chip_button(
                                ui,
                                "Save & Load",
                                true,
                                BLUE(),
                                egui::Sense::click(),
                            )
                            .help(help::LOAD_WARN_SAVE)
                            .clicked()
                            {
                                crate::ui::pattern_bank::save_current_pattern_to_bank_slot(
                                    params, pattern, current,
                                );
                                switch_to_slot(state);
                            }
                        }
                        if crate::ui::controls::chip_button(
                            ui,
                            format!("Discard & Load P{}", slot + 1).as_str(),
                            true,
                            RED(),
                            egui::Sense::click(),
                        )
                        .help(help::LOAD_WARN_DISCARD)
                        .clicked()
                        {
                            switch_to_slot(state);
                        }
                        if crate::ui::controls::chip_button(ui, "Cancel", false, INK2(), egui::Sense::click())
                            .help(help::LOAD_WARN_CANCEL)
                            .clicked()
                        {
                            state.pattern_load_confirm = None;
                        }
                    });
                });
            ui.painter()
                .set(bg, crate::ui::skeuo::plate_shape(resp.response.rect, RADIUS_PANEL as f32));
        });
}

/// [271] After a MIDI export, say where the file went and offer to open the
/// folder — the exports directory is otherwise invisible to the user.
pub fn draw_midi_export_modal_if_any(ui: &mut egui::Ui, state: &mut EditorUIState) {
    let Some(path) = state.midi_export_modal.clone() else {
        return;
    };

    let screen_rect = ui.ctx().screen_rect();
    let panel_w = 430.0;
    let pos = egui::pos2(
        screen_rect.center().x - panel_w * 0.5,
        screen_rect.center().y - 30.0,
    );
    egui::Area::new(ui.id().with("midi_export_modal"))
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            let bg = ui.painter().add(egui::Shape::Noop);
            let resp = egui::Frame::NONE
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(panel_w);
                    ui.label(
                        RichText::new("MIDI exported")
                            .font(f_sans_sb(12.0))
                            .color(INK()),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(path.display().to_string())
                            .font(f_mono_med(9.5))
                            .color(INK2()),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if crate::ui::controls::chip_button(
                            ui,
                            "Open folder",
                            true,
                            BLUE(),
                            egui::Sense::click(),
                        )
                        .help(help::EXPORT_OPEN_FOLDER)
                        .clicked()
                        {
                            let folder = path
                                .parent()
                                .map(|p| p.to_path_buf())
                                .unwrap_or_else(|| path.clone());
                            let _ = open::that_detached(&folder);
                            state.midi_export_modal = None;
                        }
                        if crate::ui::controls::chip_button(
                            ui,
                            "OK",
                            false,
                            INK2(),
                            egui::Sense::click(),
                        )
                        .clicked()
                        {
                            state.midi_export_modal = None;
                        }
                    });
                });
            ui.painter().set(
                bg,
                crate::ui::skeuo::plate_shape(resp.response.rect, RADIUS_PANEL as f32),
            );
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_assign_notes_numbers_active_lanes_from_base() {
        let mut layout = TrackLayoutState::modular_default_layout();
        // Factory layout: 4 active lanes (slots 0..=3), the rest inactive.
        assert!(auto_assign_notes_in_order(&mut layout, 36));
        let notes: Vec<u8> = layout
            .slots
            .iter()
            .filter(|s| s.active)
            .map(|s| s.midi_note)
            .collect();
        assert_eq!(notes, [36, 37, 38, 39]);
        // Inactive slots are untouched.
        assert!(!layout.slots[4].active);

        // No-op when already assigned: reports no change.
        assert!(!auto_assign_notes_in_order(&mut layout, 36));

        // Saturates at 127 near the top of the range.
        assert!(auto_assign_notes_in_order(&mut layout, 126));
        let notes: Vec<u8> = layout
            .slots
            .iter()
            .filter(|s| s.active)
            .map(|s| s.midi_note)
            .collect();
        assert_eq!(notes, [126, 127, 127, 127]);
    }
}
