//! Pattern grid: lanes, step cells, fusion, lane reorder, page bar.

use crate::plock::PlockState;
use crate::sequencer::{FusedGroup, SharedPattern};
use crate::sound_settings::SoundSettingsState;
use crate::track::{TrackInstrumentKind, TrackLayoutState, TrackSlot};
use crate::ui::controls::*;
use crate::ui::editor_state::*;
use crate::ui::header::header_param_slider;
use crate::ui::help::{self, HelpExt};
use crate::ui::popups::draw_page_popup_if_any;
use crate::ui::slider;
use crate::ui::theme::*;
use crate::ui::widgets::*;
use crate::DrumFlashParams;
use nih_plug::{params::persist::PersistentField, prelude::*};
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

pub fn draw_grid_v2(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    external_midi_triggers: &[AtomicBool; crate::track::MAX_TRACKS],
    current_step: &AtomicU32,
    current_steps: &[AtomicU32; crate::track::MAX_TRACKS],
    sound_settings: &SoundSettingsState,
    plock: &PlockState,
    state: &mut EditorUIState,
) {
    // When the p-lock popup is open, any primary click this frame (menu item,
    // popup border, or outside) should be handled by the popup, not by the step
    // cell underneath it. Suppress step-cell toggles for this frame.
    state.suppress_step_cell_click = false;
    if state.plock_popup.is_some()
        && ui.input(|i| i.pointer.button_clicked(egui::PointerButton::Primary))
    {
        state.suppress_step_cell_click = true;
    }

    let master_length = params.pattern_length.value().clamp(1, 64) as usize;
    let play_step = current_step.load(Ordering::Relaxed) as usize;
    let play_page = play_step / 16;

    draw_page_bar_v2(
        ui,
        setter,
        params,
        pattern,
        sound_settings,
        plock,
        state,
        play_page,
        master_length,
    );

    if state.follow_mode && play_page < 4 {
        state.current_page = play_page;
    }

    let page_offset = state.current_page * 16;
    let fusion_mode_active = fusion_modifier_pressed(ui);
    // [210] A pending fusion start SURVIVES releasing Shift. It used to be wiped
    // the instant the modifier came up, so letting go for a fraction of a second
    // between the two clicks silently threw the gesture away - which is most of
    // why creating a fusion felt broken. Escape cancels it; so does a plain
    // click on a cell of that lane, and the lane operations that already do.
    if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
        for selection_start in state.fusion_selection_start.iter_mut() {
            *selection_start = None;
        }
    }

    // [283] A linked lane's own morphs follow the fused groups it borrows:
    // drop those whose group is gone (deleted, moved, lane unlinked).
    pattern.retain_linked_morphs(|slot| params.track_layout.state.grid_slot(slot));

    let mut fusion_editing_started_this_frame = false;

    let well_resp = egui::Frame::new()
        .fill(WELL_FILL)
        .stroke(egui::Stroke::new(1.0, PANEL_BORDER))
        .corner_radius(RADIUS_PANEL)
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = GAP_TIGHT;

            let row_w = ui.available_width();
            let grip_w = 14.0;
            // [289] Hum left the row for the Track tab: its column went to the
            // lane name and the volume bar (62 -> 88, 56 -> 81 = 44 + 7).
            let name_w = 88.0;
            let vol_w = 81.0;
            // [216] Two tags now (M / S): the audition button is gone.
            let mst_w = STEP_H * 2.0 + GAP_TIGHT;
            let extra_w = 44.0;
            let gap = 7.0;
            // [302] + the level meter at the end of each lane.
            let fixed_w = grip_w + name_w + vol_w + mst_w + extra_w * 2.0 + gap * 7.0 + LANE_METER_W;
            let steps_w = (row_w - fixed_w).max(320.0);
            let cell_w = ((steps_w - GAP_TIGHT * 15.0) / 16.0).floor().max(18.0);

            draw_seq_header_v2(
                ui,
                params,
                pattern,
                plock,
                state,
                page_offset,
                play_step,
                master_length,
                grip_w,
                name_w,
                vol_w,
                mst_w,
                extra_w,
                gap,
                cell_w,
            );

            let mixer_rows = mixer_rows(params);
            let pushes: [&FloatParam; crate::track::MAX_TRACKS] =
                std::array::from_fn(|i| params.pushes()[i]);
            let lengths: [&IntParam; crate::track::MAX_TRACKS] =
                std::array::from_fn(|i| params.lengths()[i]);
            state.selected_track_slot = state.selected_track_slot.min(crate::track::MAX_TRACKS - 1);
            let mut lane_row_rects: [Option<egui::Rect>; crate::track::MAX_TRACKS] =
                [None; crate::track::MAX_TRACKS];

            // Always render the full 14 rows (active lanes + styled empty lanes)
            // so the grid height is constant and the panels below never shift.
            for slot_idx in 0..crate::track::MAX_TRACKS {
                let Some(inst) = voice_idx_for_slot(params, slot_idx) else {
                    // Inactive slot: the +N chip opens the shared instrument
                    // picker (category ▸ kind) for this specific slot.
                    let row_response = draw_empty_slot_lane_v2(
                        ui,
                        setter,
                        params,
                        slot_idx,
                        page_offset,
                        grip_w,
                        name_w,
                        vol_w,
                        mst_w,
                        extra_w,
                        gap,
                        cell_w,
                        state,
                        sound_settings,
                        pattern,
                        plock,
                    );
                    lane_row_rects[slot_idx] = Some(row_response.rect);
                    continue;
                };
                let row = &mixer_rows[slot_idx];
                let fusions = pattern.load_fusions(slot_idx);
                let lane_length = effective_lane_length_for_ui(params, slot_idx, master_length);
                let lane_play_step = current_steps[slot_idx].load(Ordering::Relaxed) as usize;
                let row_response = draw_legacy_slot_lane_v2(
                    ui,
                    setter,
                    params,
                    pattern,
                    external_midi_triggers,
                    sound_settings,
                    plock,
                    state,
                    slot_idx,
                    inst,
                    row,
                    &fusions,
                    pushes[slot_idx],
                    lengths[slot_idx],
                    page_offset,
                    lane_play_step,
                    master_length,
                    lane_length,
                    fusion_mode_active,
                    grip_w,
                    name_w,
                    vol_w,
                    extra_w,
                    gap,
                    cell_w,
                    &mut fusion_editing_started_this_frame,
                );
                lane_row_rects[slot_idx] = Some(row_response.rect);
            }

            // [286] The effect buses stay on lanes 13 / 14: they are not dragged, and an
            // instrument never lands among them.
            let dragging_instrument = state.lane_drag_source.is_some_and(|from| {
                !crate::track::is_fx_slot(from)
                    || !PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| {
                        s.slots[from].kind.is_effect()
                    })
            });
            if dragging_instrument {
                if let Some(pointer_pos) = ui.input(|input| input.pointer.interact_pos()) {
                    if let Some(gap) = compute_reorder_gap(&lane_row_rects, pointer_pos) {
                        // [191] Same snap as the drop, so the line is drawn where
                        // the lane will actually land - never inside a chain.
                        let gap =
                            PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| {
                                s.snap_gap_out_of_chains(gap)
                            })
                            .min(crate::track::FX_FIRST_SLOT);
                        draw_lane_reorder_indicator(ui, &lane_row_rects, gap);
                    }
                }
            }

            handle_lane_reorder_drop(
                ui,
                setter,
                params,
                pattern,
                sound_settings,
                plock,
                state,
                &lane_row_rects,
            );

            let grid_well_rect = ui.min_rect().intersect(ui.clip_rect());
            handle_wav_drop(
                ui,
                params,
                sound_settings,
                state,
                pattern,
                plock,
                &lane_row_rects,
                grid_well_rect,
            );
        });

    // Recessed well shadow (top + sides) — one place: `skeuo::well_recess`.
    {
        let wr = well_resp.response.rect;
        crate::ui::skeuo::well_recess(ui, wr, RADIUS_PANEL as f32);
    }

    // Grid-shift warning: asks before breaking an edge fusion.
    draw_shift_grid_warning_if_any(ui, params, pattern, plock, state, master_length);

    let mut fusion_edit_box_rect = None;
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.spacing_mut().item_spacing.x = 12.0;
        let mode_label = ui.label(
            RichText::new("P-Lock Mode")
                .font(f_sans_sb(10.5))
                .color(INK3()),
        );
        let selected = if state.sequencer_mode { 1 } else { 0 };
        let new_selected = p_lock_mode_segmented(ui, selected);
        if new_selected != selected {
            state.sequencer_mode = new_selected == 1;
        }
        let mode_hint = ui.label(
            RichText::new("Right-click a step to edit its p-lock")
                .size(font_size(10.5))
                .color(INK3()),
        );
        help::rect(
            ui,
            mode_label.rect.union(mode_hint.rect),
            "plock_mode",
            help::PLOCK_MODE,
        );

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            fusion_edit_box_rect = Some(draw_fusion_edit_box(
                ui,
                pattern,
                params,
                sound_settings,
                state,
                fusion_mode_active,
            ));
            if let Some(rect) = fusion_edit_box_rect {
                help::rect(ui, rect, "fusion_box", help::FUSION_BOX);
            }
        });
    });

    if !fusion_editing_started_this_frame {
        close_fusion_editing_on_outside_click(ui, pattern, state, None, fusion_edit_box_rect);
    }

    draw_page_popup_if_any(ui, setter, pattern, params, plock, state);
}

#[allow(clippy::too_many_arguments)]
fn draw_legacy_slot_lane_v2(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    external_midi_triggers: &[AtomicBool; crate::track::MAX_TRACKS],
    sound_settings: &SoundSettingsState,
    plock: &PlockState,
    state: &mut EditorUIState,
    slot_idx: usize,
    voice_idx: usize,
    row: &MixerRow<'_>,
    fusions: &[FusedGroup],
    push_param: &FloatParam,
    length_param: &IntParam,
    page_offset: usize,
    play_step: usize,
    master_length: usize,
    lane_length: usize,
    fusion_mode_active: bool,
    grip_w: f32,
    name_w: f32,
    vol_w: f32,
    extra_w: f32,
    gap: f32,
    cell_w: f32,
    // No longer written since fusion editing moved to the right-click menu;
    // kept in the signature to avoid churning the call site. Always false now,
    // so the outside-click close guard simply always runs.
    _fusion_editing_started_this_frame: &mut bool,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.set_height(LANE_H);

        let layout_state =
            PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
        // [286] An effect bus: its cells carry p-locks, never steps.
        let is_fx = layout_state.slots[slot_idx].kind.is_effect();
        let fusion_mode_active = fusion_mode_active && !is_fx;

        // [191] A link is positional, so a follower cannot travel on its own:
        // moving it, or slipping a lane above it, would re-point the link at
        // another instrument. Its grip refuses the drag and says why; the lane
        // it follows stays draggable and takes the whole run along.
        let is_follower = layout_state.is_grid_follower(slot_idx);
        let grip_response = draw_seq_grip_v2(ui, grip_w, LANE_H);
        let grip_response = if is_follower {
            grip_response
                .on_hover_cursor(egui::CursorIcon::NotAllowed)
                .help_or(help::LANE_GRIP_FOLLOWER, help::LANE_GRIP_FOLLOWER)
        } else {
            grip_response
                .on_hover_cursor(egui::CursorIcon::Grab)
                .help(help::LANE_GRIP)
        };
        if !is_follower
            && (grip_response.is_pointer_button_down_on() || grip_response.drag_started())
        {
            state.lane_drag_source = Some(slot_idx);
            select_legacy_track(state, slot_idx);
        }
        if state.lane_drag_source == Some(slot_idx) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }

        let selected = state.selected_track_slot == slot_idx;

        // Grid linking (layering): a lane linked to the one above SHOWS and EDITS
        // that lane's steps + fusions. All pattern step/fusion reads and writes
        // below go through `grid_idx`; sound, plocks, tags and routing stay on the
        // real `slot_idx` so they remain independent per lane.
        let grid_idx = layout_state.grid_slot(slot_idx);
        let is_linked = grid_idx != slot_idx;
        // [283] The other lanes of this link chain (same shown steps), as
        // a bit mask: their p-locks are drawn here as pips.
        let partner_lanes: u16 = (0..crate::track::MAX_TRACKS)
            .filter(|&s| {
                s != slot_idx && layout_state.slots[s].active && layout_state.grid_slot(s) == grid_idx
            })
            .fold(0, |mask, s| mask | (1 << s));
        let linked_fusions_storage;
        let fusions: &[FusedGroup] = if is_linked {
            // [283] The shared groups, with this lane's own morph.
            linked_fusions_storage = pattern.load_lane_fusions(slot_idx, grid_idx);
            &linked_fusions_storage
        } else {
            fusions
        };

        // [191] Link indicator: a return arrow on the FOLLOWER only, saying its
        // grid comes from the lane above. The lane it borrows from carries no
        // mark - the arrow alone means "linked", and its direction says where
        // from, which the old 2 px edge stripe could not.
        //
        // It lives in the free space between the grip's dot matrix and the name
        // plate: the matrix is ~6 px wide inside a 14 px grip, so the right
        // margin plus the 7 px item spacing give ~10 px, taken from nothing.
        if is_linked {
            let r = grip_response.rect;
            let slot = egui::Rect::from_min_max(
                egui::pos2(r.right() - 2.0, r.top() + 4.0),
                egui::pos2(r.right() + gap, r.bottom() - 4.0),
            );
            crate::ui::skeuo::link_arrow(ui.painter(), slot, BLUE());
            help::rect(ui, slot, ("lane_link", slot_idx), help::LANE_LINK_ARROW);
        }

        let slot_name = {
            let name = if layout_state.slots[slot_idx].name.is_empty() {
                crate::instrument_registry::INSTRUMENTS[voice_idx]
                    .label
                    .to_string()
            } else {
                layout_state.slots[slot_idx].name.clone()
            };
            name.chars().take(8).collect::<String>()
        };
        // [238] MIDI activity flashes the NAME PLATE itself (the amber LED of
        // [216] is gone). The same timer also confirms Paste Lane, Paste Grid
        // and Randomize Lane, which set it for longer.
        //
        // Repaint trap ([216]): the editor only repaints on events, so the
        // frame that lit the flash was often the last one drawn and the lit
        // pixels stayed on screen. Nothing ever asked for the frame that would
        // turn it off - scheduling that repaint is the fix.
        let now = ui.ctx().input(|i| i.time);
        if external_midi_triggers[slot_idx].swap(false, Ordering::Acquire) {
            state.slot_flash_until[slot_idx] = now + 0.12;
        }
        let flash_until = state.slot_flash_until[slot_idx];
        let flash = if now < flash_until {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(
                    (flash_until - now).min(0.12).max(0.01),
                ));
            ((flash_until - now) / 0.12).min(1.0) as f32
        } else {
            0.0
        };
        let name_response =
            draw_lane_name_v2(ui, name_w, selected, &slot_name, flash, is_fx).help(help::LANE_NAME);

        if name_response.clicked() {
            select_legacy_track(state, slot_idx);
            // [184] Clicking a lane's NAME means "show me this lane": it always
            // hands the panel back to the lane's own sound, even when the cell
            // already selected belongs to this very lane.
            state.sound_edit_target = None;
        }
        if name_response.double_clicked() {
            select_legacy_track(state, slot_idx);
            state.sound_editor_tab = SoundEditorTab::Track;
            state.track_name_input = layout_state.slots[slot_idx].name.clone();
            state.track_name_input_slot = Some(slot_idx);
            state.track_name_focus_request = true;
        }

        name_response.context_menu(|ui| {
            use crate::ui::menus::{context_menu_button, context_menu_separator};
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.set_min_width(148.0);
            ui.set_max_width(148.0);
            // Instrument type, cascading submenus by category:
            // Instrument ▸ BD/SD/HH/PERC/FX/OTHER ▸ kinds. Same semantics as
            // the Track tab Type dropdown.
            let current_kind = layout_state.slots[slot_idx].kind;
            let mut picked_kind = None;
            let instrument_menu = ui.menu_button(
                RichText::new("Instrument").font(f_sans_med(10.5)).color(INK()),
                |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.set_min_width(110.0);
                    ui.set_max_width(110.0);
                    // Shared category▸kind picker (identical everywhere).
                    picked_kind = crate::ui::menus::instrument_category_menu(ui, Some(current_kind), slot_idx);
                },
            );
            instrument_menu.response.help(help::MENU_INSTRUMENT);
            if let Some(kind) = picked_kind {
                change_slot_kind(params, sound_settings, state, slot_idx, kind);
                ui.close_menu();
            }
            context_menu_separator(ui);
            if context_menu_button(ui, "Copy Lane", INK(), true)
                .help(help::MENU_COPY_LANE)
                .clicked()
            {
                state.copy_lane(params, slot_idx, sound_settings, pattern, plock);
                state.lane_clear_grid_confirm = None;
                state.lane_delete_confirm = None;
                ui.close_menu();
            }
            let can_paste = state.lane_clipboard.is_some();
            if context_menu_button(ui, "Paste Lane", INK(), can_paste)
                .help(help::MENU_PASTE_LANE)
                .clicked()
                && can_paste
            {
                if state.paste_lane(setter, params, slot_idx, sound_settings, pattern, plock) {
                    // Flash visual feedback
                    state.slot_flash_until[slot_idx] = ui.ctx().input(|i| i.time) + 0.5;
                }
                state.lane_clear_grid_confirm = None;
                state.lane_delete_confirm = None;
                ui.close_menu();
            }
            if context_menu_button(ui, "Paste Grid", INK(), can_paste)
                .help(help::MENU_PASTE_GRID)
                .clicked()
                && can_paste
            {
                if state.paste_grid(params, slot_idx, pattern) {
                    // Flash visual feedback
                    state.slot_flash_until[slot_idx] = ui.ctx().input(|i| i.time) + 0.5;
                }
                state.lane_clear_grid_confirm = None;
                state.lane_delete_confirm = None;
                ui.close_menu();
            }
            // Grid link (layering): mirror the steps + fusions of the lane above.
            if layout_state.slots[slot_idx].linked_up {
                if context_menu_button(ui, "Unlink steps", INK(), true)
                    .help(help::MENU_UNLINK)
                    .clicked()
                {
                    set_lane_linked_up(params, slot_idx, false);
                    ui.close_menu();
                }
            } else if layout_state.can_link_up(slot_idx) {
                if context_menu_button(ui, "Link steps to lane above", INK(), true)
                    .help(help::MENU_LINK)
                    .clicked()
                {
                    set_lane_linked_up(params, slot_idx, true);
                    ui.close_menu();
                }
            }
            context_menu_separator(ui);
            let confirm_clear_grid = state.lane_clear_grid_confirm == Some(slot_idx);
            if context_menu_button(
                ui,
                if confirm_clear_grid { "Confirm Clear Lane?" } else { "Clear Lane" },
                RED(),
                true,
            )
            .help_or(help::MENU_CLEAR_LANE, if confirm_clear_grid {
                "Click again to clear this lane's steps, fusions and plocks"
            } else {
                "Clear this lane's steps, fusions and plocks; keeps instrument, sound, routing and lane controls"
            })
            .clicked()
            {
                if confirm_clear_grid {
                    state.clear_lane(params, slot_idx, pattern, plock);
                    ui.close_menu();
                } else {
                    state.lane_clear_grid_confirm = Some(slot_idx);
                    state.lane_delete_confirm = None;
                }
            }
            let confirm_delete = state.lane_delete_confirm == Some(slot_idx);
            if context_menu_button(
                ui,
                if confirm_delete { "Confirm Delete Lane?" } else { "Delete Lane" },
                RED(),
                true,
            )
            .help_or(
                help::MENU_DELETE_LANE,
                "Deactivate this lane; the slot becomes empty and can be reactivated later",
            )
            .clicked()
            {
                if confirm_delete {
                    deactivate_slot(setter, params, state, slot_idx);
                    ui.close_menu();
                } else {
                    state.lane_delete_confirm = Some(slot_idx);
                    state.lane_clear_grid_confirm = None;
                }
            }
            // [286] Nothing to randomize on an effect bus.
            if !is_fx {
                // Randomize density (drives the Randomize Lane row below). [285] The
                // lane's own; double-click = its instrument type's default (Settings).
                {
                    let mut density = state.lane_randomize_density(params, slot_idx);
                    let type_default = params
                        .track_layout
                        .state
                        .kind_for_slot(slot_idx)
                        .map_or(crate::track::DEFAULT_RANDOMIZE_DENSITY, |kind| {
                            state.global_config.randomize_density_for(kind)
                        });
                    let response = ui.horizontal(|ui| {
                        ui.label(RichText::new("Density").font(f_sans_med(10.0)).color(INK3()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add(
                                crate::ui::local_param_slider::LocalParamSlider::new(
                                    &mut density,
                                    0.05..=1.0,
                                )
                                .with_width(72.0)
                                .without_value()
                                .reset_value(type_default),
                            )
                        })
                        .inner
                    });
                    help::rect(ui, response.response.rect, "menu_density", help::MENU_DENSITY);
                    if response.inner.changed() {
                        EditorUIState::set_lane_randomize_density(params, slot_idx, density);
                    }
                }
                if context_menu_button(ui, "Randomize Lane", INK(), true)
                    .help_or(
                        help::MENU_RANDOMIZE,
                        "Fill this lane with random steps (clears fusions and plocks) at the Density above",
                    )
                    .clicked()
                {
                    state.randomize_lane(params, slot_idx, pattern, plock);
                    // Flash visual feedback
                    state.slot_flash_until[slot_idx] = ui.ctx().input(|i| i.time) + 0.5;
                    state.lane_delete_confirm = None;
                    ui.close_menu();
                }
            }
        });

        let inst_state = &sound_settings.instruments[slot_idx];
        // [288] Displayed 0..1; the stored gain keeps its 0..2 range.
        let mut lane_vol =
            f32::from_bits(inst_state.volume.load(Ordering::Relaxed)) * 0.5;
        let lane_vol_response =
            draw_mini_value_slider(
                ui,
                &mut lane_vol,
                0.0,
                1.0,
                0.5,
                vol_w,
                if is_fx { AMBER() } else { BLUE() },
                "",
            )
                .help(help::LANE_VOL);
        if lane_vol_response.clicked() || lane_vol_response.dragged() {
            select_legacy_track(state, slot_idx);
        }
        if lane_vol_response.changed() {
            inst_state
                .volume
                .store((lane_vol * 2.0).to_bits(), Ordering::Relaxed);
            sound_settings.bump_version();
        }

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP_TIGHT;
            if draw_tag_param_v2(
                ui,
                setter,
                row.mute,
                "M",
                RED(),
                Color32::WHITE,
                "",
            )
            .help(if is_fx { help::LANE_FX_MUTE } else { help::LANE_MUTE })
            .clicked()
                && params.auto_edit.value()
            {
                select_legacy_track(state, slot_idx);
            }
            if is_fx {
                // [286] A bus has no solo; keep its column.
                let _ = ui.allocate_exact_size(Vec2::splat(TAG_SIZE), egui::Sense::hover());
            } else {
                if draw_tag_param_v2(
                    ui,
                    setter,
                    row.solo,
                    "S",
                    GREEN(),
                    SOLO_FILL(),
                    "",
                )
                .help(help::LANE_SOLO)
                .clicked()
                    && params.auto_edit.value()
                {
                    select_legacy_track(state, slot_idx);
                }
            }
            // [216] Two tags instead of three: the "T" audition button is gone.
            // Its MIDI lamp moved to the name plate, above.
        });

        let steps_rect = ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP_TIGHT;

            // Long-press drag: update target, apply move on release, and activate
            // after the ~0.5 s hold threshold.
            let mut drag_just_completed = false;
            if let Some(drag) = state.step_drag.as_mut() {
                if drag.slot == slot_idx {
                    if !drag.active {
                        let now = ui.ctx().input(|i| i.time);
                        if now - drag.start_time >= 0.5 {
                            drag.active = true;
                        }
                    }
                    if drag.active {
                        if let Some(pos) = ui.input(|i| i.pointer.interact_pos()) {
                            let delta_x = pos.x - drag.source_rect.center().x;
                            let delta_steps = (delta_x / (cell_w + GAP_TIGHT)).round() as i32;
                            let min_step = page_offset as i32;
                            let max_step = (page_offset + 15).min(crate::plock::STEP_COUNT - 1) as i32;
                            let target =
                                (drag.source_step as i32 + delta_steps).clamp(min_step, max_step)
                                    as usize;
                            drag.current_target = target;
                        }
                    }
                    // Release ends the gesture whether or not the 0.5 s long-press
                    // threshold was reached. A quick tap (drag never activated) MUST
                    // clear the pending drag here — otherwise it keeps counting and
                    // "activates" later, while the button is already up, spawning a
                    // ghost cell that follows the cursor. A non-active release then
                    // falls through to a normal click (toggle).
                    if ui.input(|i| i.pointer.any_released()) {
                        let (was_active, from, to) =
                            (drag.active, drag.source_step, drag.current_target);
                        state.step_drag = None;
                        if was_active {
                            if from != to {
                                move_step_with_plocks(
                                    pattern, plock, params, slot_idx, grid_idx, from, to,
                                );
                                state.mark_pattern_dirty();
                            }
                            drag_just_completed = true;
                        }
                    }
                }
            }

            // [282] Ctrl / Cmd + click selects a cell for the Lane Editor. Shift wins
            // when both are held: it is the fusion gesture already in progress.
            // [286] On an effect bus a click always selects the cell (to edit its
            // p-lock): there is no step to switch on.
            let select_cell_mode = is_fx
                || (!fusion_mode_active && crate::ui::controls::select_cell_modifier_pressed(ui));
            for local_step in 0..16 {
                let global_step = page_offset + local_step;
                let beyond_len = global_step >= lane_length;
                let fusion_info = fusion_containing(fusions, global_step);
                let fusion_group = fusion_info.map(|(_, group)| group);
                let source_step = fusion_group
                    .map(|group| group.start_cell as usize)
                    .unwrap_or(global_step);
                let is_fusion_start = fusion_group
                    .map(|group| group.is_start(global_step))
                    .unwrap_or(false);
                let is_fusion_mid = fusion_group.is_some() && !is_fusion_start;

                let active = !beyond_len && pattern.is_active(source_step, grid_idx);
                // Inside a fused group, the playhead marker lives on the START
                // cell only — it rings the whole fused block, not the steps.
                let is_current = if let Some(group) = fusion_group {
                    group.contains(play_step) && group.is_start(global_step)
                } else {
                    play_step == global_step
                };
                let has_sound_plock_base = !beyond_len && plock.masks.is_active(slot_idx, source_step);
                let has_morph = !beyond_len
                    && fusion_group.map(|g| g.morph_active()).unwrap_or(false);
                let has_sound_plock = has_sound_plock_base || has_morph;
                let field_mask = if has_sound_plock_base {
                    plock.field_masks.get(slot_idx, source_step)
                } else {
                    0
                };
                let all_bits = (1u64 << crate::plock::FIELD_COUNT) - 1;
                let is_snapshot = has_sound_plock_base && field_mask == all_bits;
                let has_seq_plock = !beyond_len
                    && params
                        .seq_plock_state
                        .state
                        .is_active(slot_idx, source_step);
                let has_solo = !beyond_len
                    && params
                        .seq_plock_state
                        .state
                        .is_solo(slot_idx, source_step);
                // [283] Linked lanes share their steps but keep their own
                // p-locks: a p-lock on a partner lane at this step shows here
                // as a pip in its colour.
                let partner = |pred: &dyn Fn(usize) -> bool| {
                    !beyond_len && (0..crate::track::MAX_TRACKS).any(|s| partner_lanes & (1 << s) != 0 && pred(s))
                };
                let partner_sound = partner(&|s| plock.masks.is_active(s, source_step));
                let partner_seq =
                    partner(&|s| params.seq_plock_state.state.is_active(s, source_step));
                // [210] Not gated on the modifier: the mark must stay visible
                // while the user reaches for the second cell.
                let selection_start =
                    state.fusion_selection_start[slot_idx] == Some(global_step);

                let is_editing = state
                    .fusion_editing
                    .map(|(ei, eidx)| {
                        // Fusions belong to the grid owner (`grid_idx`), so a
                        // linked lane and its master both reflect the same edit.
                        ei == grid_idx && fusion_info.map(|(idx, _)| idx == eidx).unwrap_or(false)
                    })
                    .unwrap_or(false);
                // Keep animating the edit-pulse while a fusion is being edited
                // (the reactive editor would otherwise freeze the pulse).
                if is_editing {
                    ui.ctx().request_repaint();
                }

                let is_drag_source = state.step_drag.as_ref().map_or(false, |d| {
                    d.active && d.slot == slot_idx && d.source_step == global_step
                });
                let is_drag_target = state.step_drag.as_ref().map_or(false, |d| {
                    d.active && d.slot == slot_idx && d.current_target == global_step
                });

                // [286] On an effect bus a cell with a lock reads as "on".
                let (fill, mut stroke) = step_colors_v2(
                    ui.ctx(),
                    state.sequencer_mode && !is_fx,
                    local_step,
                    active || (is_fx && has_sound_plock),
                    has_sound_plock,
                    is_snapshot,
                    has_seq_plock,
                    is_current,
                    beyond_len,
                    selection_start,
                    is_fusion_start,
                    is_fusion_mid,
                    is_editing,
                );
                if is_drag_target {
                    stroke = egui::Stroke::new(2.0, DRAG_TARGET());
                }
                // [286] The first cell of an effect lane carries the loop-restart mark: a
                // green icon while the lane goes back to its own values at every loop,
                // a struck-out grey one when that restart is switched off.
                let fx_loop_start = is_fx && global_step == 0 && !has_sound_plock && !beyond_len;
                let fx_restart_on = sound_settings.instruments[slot_idx]
                    .special_value(crate::synthesis::reverb_sp::NO_RESTART)
                    < 0.5;
                let text = if is_fusion_start {
                    fusion_group
                        .map(|g| g.step_count.to_string())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let fusion_span = if is_fusion_start {
                    fusion_group.map(|g| g.cell_span())
                } else {
                    None
                };
                let response = draw_step_cell_v2(
                    ui,
                    Vec2::new(cell_w, STEP_H),
                    fill,
                    stroke,
                    &text,
                    !beyond_len,
                    is_current && !beyond_len,
                    is_fusion_mid,
                    fusion_span,
                    beyond_len,
                    active,
                    is_editing,
                );

                if fx_loop_start {
                    if fx_restart_on {
                        draw_loop_reset_icon(ui.painter(), response.rect, PL_LINK(), false);
                        help::rect(ui, response.rect, ("fx_loop_start", slot_idx), help::FX_LOOP_START);
                    } else {
                        draw_loop_reset_icon(ui.painter(), response.rect, INK3(), false);
                        help::rect(ui, response.rect, ("fx_loop_start", slot_idx), help::FX_LOOP_HOLD);
                    }
                }

                // [210] The `[` that opens the span, over the cell that is
                // waiting for its end.
                if selection_start {
                    crate::ui::skeuo::fusion_start_bracket(
                        ui.painter(),
                        response.rect,
                        BLUE(),
                    );
                }

                // [220] The cell carries the OTHER kind of p-lock too: a pip
                // in that kind's colour, whichever mode the grid is in. Sound
                // p-locks are light green, sequencer p-locks violet, so the pip
                // never repeats the fill it sits on.
                //
                // A fused block's cells all read the START cell's p-locks, so the
                // pip landed on every one of them - five dots across one block.
                // One block, one pip, on its LAST cell: the block's own top-right
                // corner. A block running off the page keeps its pip on the last
                // cell still visible.
                let pip_cell = match fusion_group {
                    Some(group) => {
                        let last = group.start_cell as usize + group.cell_span() - 1;
                        global_step == last.min(page_offset + 15)
                    }
                    None => true,
                };
                if !beyond_len && pip_cell {
                    // A kind already shown by the cell's own fill needs no pip.
                    let sound_filled = !state.sequencer_mode && has_sound_plock;
                    let seq_filled = state.sequencer_mode && has_seq_plock;
                    let mut pips = [Color32::TRANSPARENT; 2];
                    let mut n = 0;
                    if (has_sound_plock || partner_sound) && !sound_filled {
                        pips[n] = PL_LINK();
                        n += 1;
                    }
                    if (has_seq_plock || partner_seq) && !seq_filled {
                        pips[n] = SEQPL();
                        n += 1;
                    }
                    crate::ui::skeuo::plock_pips(ui.painter(), response.rect, &pips[..n]);
                }

                // Step-solo marker: a small 'S' in the top-left corner of soloed
                // seq-plock cells. Shown only in sequencer mode (matches when the
                // violet seq-plock colouring is visible).
                if state.sequencer_mode && has_solo {
                    let r = response.rect;
                    let c = egui::pos2(r.left() + 5.5, r.top() + 5.5);
                    ui.painter()
                        .circle_filled(c, 4.5, Color32::from_rgb(24, 16, 40));
                    ui.painter().text(
                        c,
                        egui::Align2::CENTER_CENTER,
                        "S",
                        f_sans_sb(8.0),
                        Color32::WHITE,
                    );
                }

                // Start a long-press drag when the left button is held on an active,
                // single, non-fused cell outside fusion mode.
                //
                // Guard on the PRIMARY button + no open popup: `is_pointer_button_down_on`
                // is also true on a right-click, so without this a right-click that opens
                // the plock popup would start a phantom drag; then reading the menu (>0.5 s)
                // activates it and clicking a popup control releases it, silently MOVING the
                // step (and its solo/plock) to wherever the popup sits.
                if !beyond_len
                    && active
                    && !fusion_mode_active
                    && !select_cell_mode
                    && fusion_group.is_none()
                    && response.is_pointer_button_down_on()
                    && ui.input(|i| i.pointer.primary_down())
                    && state.plock_popup.is_none()
                {
                    if state.step_drag.is_none() {
                        let now = ui.ctx().input(|i| i.time);
                        state.step_drag = Some(DragStepState {
                            slot: slot_idx,
                            source_step: global_step,
                            source_rect: response.rect,
                            start_time: now,
                            active: false,
                            current_target: global_step,
                        });
                    }
                }

                let drag_active = state.step_drag.as_ref().map_or(false, |d| d.active);
                let suppress_click = drag_active || drag_just_completed || state.suppress_step_cell_click || state.plock_popup.is_some();

                if is_drag_target {
                    ui.painter().rect_filled(
                        response.rect,
                        4.0,
                        Color32::from_rgba_unmultiplied(255, 200, 80, 50),
                    );
                }
                if is_drag_source {
                    let t = ui.ctx().input(|i| i.time) as f32;
                    let pulse = ((t * 8.0).sin() + 1.0) * 0.5;
                    let alpha = (140.0 + 115.0 * pulse) as u8;
                    let fill_alpha = (50.0 + 50.0 * pulse) as u8;
                    ui.painter().rect_filled(
                        response.rect,
                        4.0,
                        Color32::from_rgba_unmultiplied(255, 255, 255, fill_alpha),
                    );
                    ui.painter().rect_stroke(
                        response.rect.shrink(1.0),
                        4.0,
                        egui::Stroke::new(2.5, Color32::from_rgba_unmultiplied(255, 255, 255, alpha)),
                        egui::StrokeKind::Inside,
                    );
                }

                // Fusion editing is done via the right-click menu (Edit Fusion
                // Steps / Delete Fusion), not a double-click — simpler and avoids
                // the toggle-vs-edit conflict on the same cell.
                if !beyond_len && !suppress_click && response.clicked_by(egui::PointerButton::Primary) && fusion_mode_active {
                    select_legacy_track(state, slot_idx);
                    handle_fusion_shift_click(
                        pattern,
                        params,
                        plock,
                        grid_idx,
                        global_step,
                        master_length,
                        fusions,
                        &mut state.fusion_selection_start[slot_idx],
                    );
                } else if fx_loop_start
                    && !suppress_click
                    && response.clicked_by(egui::PointerButton::Primary)
                {
                    // [286] The loop-restart arrow of an effect lane: a click switches the
                    // restart on or off for the lane (green / grey). A right-click still
                    // opens the cell's lock menu.
                    let lane = &sound_settings.instruments[slot_idx];
                    let off = lane.special_value(crate::synthesis::reverb_sp::NO_RESTART) >= 0.5;
                    lane.set_special(
                        crate::synthesis::reverb_sp::NO_RESTART,
                        if off { 0.0 } else { 1.0 },
                    );
                    sound_settings.bump_version();
                    select_legacy_track(state, slot_idx);
                    ui.ctx().request_repaint();
                } else if !beyond_len
                    && !suppress_click
                    && response.clicked_by(egui::PointerButton::Primary)
                    && select_cell_mode
                {
                    // [282] Select the cell for the Lane Editor: nothing is
                    // toggled, no menu opens. A plain click turned the step off
                    // when all the user wanted was to edit its sound. Song
                    // mode stays locked out, as for the right-click: the
                    // pattern changes under you there.
                    if !params.song_mode.value() {
                        select_legacy_track(state, slot_idx);
                        state.sound_edit_target = Some(crate::ui::editor_state::SelectedCell {
                            slot: slot_idx,
                            step: source_step,
                        });
                        // [303] A cell's locks are edited in Sound or Effects: only Track gives way.
                if state.sound_editor_tab == crate::ui::editor_state::SoundEditorTab::Track {
                    state.sound_editor_tab = crate::ui::editor_state::SoundEditorTab::Sound;
                }
                        ui.ctx().request_repaint();
                    }
                } else if !beyond_len && !suppress_click && response.clicked_by(egui::PointerButton::Primary) {
                    // Clicking outside the currently edited fusion exits edit mode.
                    if let Some((edit_inst, edit_idx)) = state.fusion_editing {
                        let editing_this_group = fusion_info
                            .map(|(idx, _)| edit_inst == grid_idx && edit_idx == idx)
                            .unwrap_or(false);
                        if !editing_this_group {
                            finish_fusion_editing_for_ui(pattern, state);
                        }
                    }

                    if let Some(group) = fusion_group {
                        toggle_fusion_for_ui(pattern, group, grid_idx);
                    } else {
                        toggle_step_for_ui(pattern, global_step, grid_idx);
                    }
                    state.mark_pattern_dirty();
                    // Reactive editor: force a redraw so the toggle (esp. the
                    // fused-block on/off dim) shows immediately when stopped.
                    ui.ctx().request_repaint();
                    if params.auto_edit.value() {
                        select_legacy_track(state, slot_idx);
                    }
                    state.fusion_selection_start[slot_idx] = None;
                }

                if !beyond_len && response.secondary_clicked() {
                    // P-lock editing is allowed in Pattern mode, Follow ON included
                    // ([184]): the cell under the cursor AT CLICK TIME is the cell
                    // the user aimed at, the popup anchors to that point, and the
                    // panel's badge names the step even once its page has scrolled
                    // away. Song mode stays locked out — there the whole PATTERN
                    // changes under you, so the step could belong to another
                    // pattern a second later, which is a real hazard rather than a
                    // cosmetic one.
                    if !params.song_mode.value() {
                        select_legacy_track(state, slot_idx);
                        // [184] The same gesture points the Lane Editor at this
                        // cell. Only in Sound p-lock mode: a right-click in
                        // Sequencer mode is about probability/stutter/nudge and
                        // must not silently retarget the sound panel.
                        // [199] It must also CLEAR any cell selection left by an
                        // earlier Sound-mode right-click on the same lane —
                        // otherwise creating a seq plock leaves the panel stuck
                        // on the sound plock Step tab.
                        if is_fx || !state.sequencer_mode {
                            state.sound_edit_target =
                                Some(crate::ui::editor_state::SelectedCell {
                                    slot: slot_idx,
                                    step: source_step,
                                });
                            state.sound_editor_tab =
                                crate::ui::editor_state::SoundEditorTab::Sound;
                        } else {
                            state.sound_edit_target = None;
                        }
                        if let Some(pos) = response.interact_pointer_pos() {
                            // [282] The menu opens under the cell, so it never covers
                            // the cell it edits; on a fusion, under the whole block
                            // (its part on this page).
                            let pitch = cell_w + GAP_TIGHT;
                            let mut anchor = response.rect;
                            if let Some(group) = fusion_group {
                                let first = (group.start_cell as usize).max(page_offset);
                                let last = (group.end_cell as usize).min(page_offset + 15);
                                anchor.set_left(
                                    response.rect.left()
                                        - global_step.saturating_sub(first) as f32 * pitch,
                                );
                                anchor.set_right(
                                    response.rect.right()
                                        + last.saturating_sub(global_step) as f32 * pitch,
                                );
                            }
                            if let Some(to_global) =
                                ui.ctx().layer_transform_to_global(ui.layer_id())
                            {
                                anchor = to_global * anchor;
                            }
                            state.plock_popup = Some(PlockPopup {
                                instrument: slot_idx,
                                step: source_step,
                                step_was_active: active,
                                screen_pos: pos,
                                anchor: Some(anchor),
                                just_opened: true,
                                // [213] Seeded from the grid mode, then free to
                                // change inside the menu.
                                sequencer: state.sequencer_mode && !is_fx,
                            });
                        }
                    }
                }
            }
        })
        .response
        .rect;
        help::rect(ui, steps_rect, ("lane_steps", slot_idx), if is_fx { help::LANE_FX_STEPS } else { help::LANE_STEPS });

        if is_fx {
            // [286] A bus has no push/pull and no length of its own: keep the columns.
            draw_empty_lane_chip_v2(ui, extra_w, "");
            draw_empty_lane_chip_v2(ui, extra_w, "");
        } else {
            let push_response = draw_param_mini_slider_with_value(
                ui,
                setter,
                push_param,
                -50.0,
                50.0,
                extra_w,
                BLUE(),
                "Push/Pull",
                |value| format!("{:+.0} ms", value),
            )
            .help(help::LANE_PUSH);
            if push_response.clicked() || push_response.dragged() || push_response.double_clicked() {
                select_legacy_track(state, slot_idx);
            }
            if draw_track_length_control(ui, setter, params, length_param, slot_idx, master_length) {
                select_legacy_track(state, slot_idx);
            }
        }
        // [302] The lane's level meter, at the very end of the lane.
        draw_lane_meter(ui, sound_settings, slot_idx, STEP_H);
    })
    .response
}

#[allow(clippy::too_many_arguments)]
/// Returns the click position when the `+N` chip was clicked (opens the
/// instrument picker for this slot).
fn draw_empty_slot_lane_v2(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    slot_idx: usize,
    _page_offset: usize,
    grip_w: f32,
    name_w: f32,
    vol_w: f32,
    mst_w: f32,
    extra_w: f32,
    gap: f32,
    cell_w: f32,
    state: &mut EditorUIState,
    sound_settings: &SoundSettingsState,
    pattern: &SharedPattern,
    plock: &PlockState,
) -> egui::Response {
    let inner = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.set_height(LANE_H);

        // Empty lanes are draggable too: moving one just reorders slot data
        // (`apply_lane_reorder_move` is slot-generic). No track selection here —
        // an inactive slot has no Sound Editor tab to select.
        let grip_response = draw_seq_grip_v2(ui, grip_w, LANE_H)
            .on_hover_cursor(egui::CursorIcon::Grab)
            .help(help::LANE_GRIP);
        if grip_response.is_pointer_button_down_on() || grip_response.drag_started() {
            state.lane_drag_source = Some(slot_idx);
        }
        if state.lane_drag_source == Some(slot_idx) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        // The +N chip opens the SHARED category▸kind picker (native menu root,
        // so the submenus work) — identical to the Track-tab Type field and the
        // lane right-click menu.
        // [286] Lanes 13 and 14 are reserved for effects: their empty chip says so, in amber.
        let fx_slot = crate::track::is_fx_slot(slot_idx);
        let add_btn = egui::Button::new(
            RichText::new(if fx_slot {
                format!("Fx{}", slot_idx + 1 - crate::track::FX_FIRST_SLOT)
            } else {
                format!("+{}", slot_idx + 1)
            })
            .font(f_mono_sb(11.0))
            .color(if fx_slot { AMBER() } else { FAINT() }),
        )
        .fill(BG())
        .stroke(egui::Stroke::new(1.0, if fx_slot { AMBER() } else { LINE() }))
        .corner_radius(RADIUS_PAD)
        .min_size(Vec2::new(name_w, 21.0));
        let picker = egui::menu::menu_custom_button(ui, add_btn, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.set_min_width(130.0);
            crate::ui::menus::instrument_category_menu(ui, None, slot_idx)
        });
        if let Some(kind) = picker.inner.flatten() {
            activate_slot(
                params,
                sound_settings,
                state,
                slot_idx,
                kind,
                pattern,
                plock,
            );
        }
        picker
            .response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .help(if crate::track::is_fx_slot(slot_idx) { help::EMPTY_FX_SLOT } else { help::EMPTY_SLOT })
            .context_menu(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.set_min_width(118.0);
                ui.set_max_width(118.0);
                let can_paste = state.lane_clipboard.is_some();
                if crate::ui::menus::context_menu_button(ui, "Paste Lane", INK(), can_paste)
                    .clicked()
                    && can_paste
                {
                    if state.paste_lane(setter, params, slot_idx, sound_settings, pattern, plock) {
                        state.slot_flash_until[slot_idx] = ui.ctx().input(|i| i.time) + 0.5;
                    }
                    ui.close_menu();
                }
            });
        draw_empty_lane_chip_v2(ui, vol_w, "Empty");
        draw_empty_lane_chip_v2(ui, mst_w, "");

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP_TIGHT;
            for _local_step in 0..16 {
                // Empty lane: flat near-invisible cells — no dashed outline
                // (unlike the out-of-length cells, which keep their dashes).
                draw_step_cell_v2(
                    ui,
                    Vec2::new(cell_w, STEP_H),
                    CELL_DISABLED(),
                    egui::Stroke::new(1.0, Color32::TRANSPARENT),
                    "",
                    false,
                    false,
                    false,
                    None,
                    true,
                    false,
                    false,
                );
            }
        });

        draw_empty_lane_chip_v2(ui, extra_w, "");
        draw_empty_lane_chip_v2(ui, extra_w, "");
    });
    inner.response
}

fn draw_empty_lane_chip_v2(ui: &mut egui::Ui, width: f32, label: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 21.0), egui::Sense::hover());
    // Borderless dark boxes: they only reserve the column space.
    ui.painter().rect_filled(rect, RADIUS_CTL, BG());
    if !label.is_empty() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            label,
            f_sans_med(9.5),
            FAINT(),
        );
    }
    response
}

/// [191] Permutation deplacant un **bloc** de `len` lanes consecutives, pour
/// qu'une maitresse emporte ses esclaves. `to` est l'index final de la premiere
/// lane du bloc, la meme convention que `lane_move_order`, dont ceci est la
/// generalisation (`len == 1` donne exactement le meme resultat - test dedie).
fn lane_move_order_block(from: usize, len: usize, to: usize) -> [usize; crate::track::MAX_TRACKS] {
    let count = crate::track::MAX_TRACKS;
    if len == 0 || from >= count || from + len > count {
        return std::array::from_fn(|i| i);
    }
    let mut ids: Vec<usize> = (0..count).collect();
    let block: Vec<usize> = ids.drain(from..from + len).collect();
    let insert = to.min(ids.len());
    for (k, id) in block.into_iter().enumerate() {
        ids.insert(insert + k, id);
    }
    std::array::from_fn(|i| ids[i])
}

fn moved_slot_index(order: &[usize; crate::track::MAX_TRACKS], old_idx: usize) -> usize {
    order
        .iter()
        .position(|&idx| idx == old_idx)
        .unwrap_or(old_idx.min(crate::track::MAX_TRACKS - 1))
}

fn remap_slot_index(order: &[usize; crate::track::MAX_TRACKS], slot: usize) -> usize {
    if slot >= crate::track::MAX_TRACKS {
        slot
    } else {
        moved_slot_index(order, slot)
    }
}

fn move_mask_bits(mask: u16, order: &[usize; crate::track::MAX_TRACKS]) -> u16 {
    let mut new_mask = 0u16;
    for (new_idx, &old_idx) in order.iter().enumerate() {
        if (mask & (1u16 << old_idx)) != 0 {
            new_mask |= 1u16 << new_idx;
        }
    }
    new_mask
}

fn compute_reorder_gap(
    lane_row_rects: &[Option<egui::Rect>; crate::track::MAX_TRACKS],
    pointer_pos: egui::Pos2,
) -> Option<usize> {
    let mut count = 0usize;
    for rect in lane_row_rects.iter() {
        if let Some(rect) = rect {
            if pointer_pos.y < rect.center().y {
                return Some(count);
            }
            count += 1;
        }
    }
    Some(count)
}

fn draw_lane_reorder_indicator(
    ui: &mut egui::Ui,
    lane_row_rects: &[Option<egui::Rect>; crate::track::MAX_TRACKS],
    gap: usize,
) {
    let top = lane_row_rects.get(gap).and_then(|r| *r);
    let bottom = gap
        .checked_sub(1)
        .and_then(|i| lane_row_rects.get(i))
        .and_then(|r| *r);
    let y = match (top, bottom) {
        (Some(t), Some(_)) => t.top(),
        (Some(t), None) => t.top(),
        (None, Some(b)) => b.bottom(),
        (None, None) => return,
    };
    let x_min = lane_row_rects
        .iter()
        .find_map(|rect| rect.map(|r| r.left()))
        .unwrap_or(0.0);
    let x_max = lane_row_rects
        .iter()
        .find_map(|rect| rect.map(|r| r.right()))
        .unwrap_or(0.0);
    ui.painter().line_segment(
        [egui::pos2(x_min, y), egui::pos2(x_max, y)],
        egui::Stroke::new(2.0, BLUE()),
    );
}

#[allow(clippy::too_many_arguments)]
fn handle_lane_reorder_drop(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    plock: &PlockState,
    state: &mut EditorUIState,
    lane_row_rects: &[Option<egui::Rect>; crate::track::MAX_TRACKS],
) {
    let Some(from) = state.lane_drag_source else {
        return;
    };

    if ui.input(|input| input.pointer.primary_down()) {
        return;
    }

    state.lane_drag_source = None;
    let Some(pointer_pos) = ui.input(|input| input.pointer.interact_pos()) else {
        return;
    };
    let Some(gap) = compute_reorder_gap(lane_row_rects, pointer_pos) else {
        return;
    };
    let layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    // [191] Never land between a master and its followers: that lane would come
    // between them and steal the link. The gap snaps to the nearer chain end -
    // the same snap the drop indicator draws, so what is shown is what happens.
    let gap = layout.snap_gap_out_of_chains(gap);
    let len = layout.chain_len(from).max(1);
    // [286] An effect lane never moves; an instrument stays above the buses.
    if from < crate::track::MAX_TRACKS && layout.slots[from].kind.is_effect() {
        return;
    }
    let to = gap.min(crate::track::FX_FIRST_SLOT.saturating_sub(len));

    // Dropping a chain inside itself changes nothing.
    if to >= from && to < from + len {
        return;
    }
    if from != to {
        apply_lane_reorder_move(
            setter,
            params,
            pattern,
            sound_settings,
            plock,
            state,
            from,
            to,
            len,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn apply_lane_reorder_move(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    plock: &PlockState,
    state: &mut EditorUIState,
    from: usize,
    to: usize,
    len: usize,
) {
    if from >= crate::track::MAX_TRACKS || to >= crate::track::MAX_TRACKS || from == to {
        return;
    }
    // [191] A master carries its followers, so the whole run moves as one.
    let order = lane_move_order_block(from, len.max(1), to);

    let old_step_masks = pattern.step_masks();
    // [283] With their linked-lane morphs.
    let old_fusions: [(Vec<FusedGroup>, Vec<FusedGroup>); crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| (pattern.load_fusions(slot), pattern.load_linked_morphs(slot)));
    for (step, mask) in old_step_masks.iter().copied().enumerate() {
        pattern.set_step_mask(step, move_mask_bits(mask, &order));
    }
    for (new_idx, &old_idx) in order.iter().enumerate() {
        let (groups, morphs) = &old_fusions[old_idx];
        pattern.store_fusions_and_morphs(new_idx, groups, morphs);
    }

    let sound_values = sound_settings.read_all();
    let sound_stride = crate::sound_settings::FIELDS_PER_INSTRUMENT_V3;
    let mut new_sound_values = sound_values.clone();
    for (new_idx, &old_idx) in order.iter().enumerate() {
        let dst = new_idx * sound_stride;
        let src = old_idx * sound_stride;
        new_sound_values[dst..dst + sound_stride]
            .copy_from_slice(&sound_values[src..src + sound_stride]);
    }
    sound_settings.write_all(&new_sound_values);

    let old_plock_masks: [u64; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| plock.masks.masks[slot].load(Ordering::Relaxed));
    let old_plock_field_masks: Vec<Vec<u64>> = (0..crate::track::MAX_TRACKS)
        .map(|slot| {
            (0..crate::plock::STEP_COUNT)
                .map(|step| plock.field_masks.get_raw(slot, step))
                .collect()
        })
        .collect();
    let old_plock_values: Vec<Vec<Vec<f32>>> = (0..crate::track::MAX_TRACKS)
        .map(|slot| {
            (0..crate::plock::STEP_COUNT)
                .map(|step| {
                    (0..crate::plock::FIELD_COUNT)
                        .map(|field| plock.values.get(slot, step, field))
                        .collect()
                })
                .collect()
        })
        .collect();
    for (new_idx, &old_idx) in order.iter().enumerate() {
        plock.masks.masks[new_idx].store(old_plock_masks[old_idx], Ordering::Relaxed);
        for step in 0..crate::plock::STEP_COUNT {
            plock
                .field_masks
                .set_raw(new_idx, step, old_plock_field_masks[old_idx][step]);
            for field in 0..crate::plock::FIELD_COUNT {
                plock
                    .values
                    .set(new_idx, step, field, old_plock_values[old_idx][step][field]);
            }
        }
    }

    let seq = &params.seq_plock_state.state;
    let old_seq_masks: [u64; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| seq.masks[slot].load(Ordering::Relaxed));
    let old_seq_solo_masks: [u64; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| seq.solo_masks[slot].load(Ordering::Relaxed));
    let old_seq_probabilities: Vec<Vec<u32>> = (0..crate::track::MAX_TRACKS)
        .map(|slot| {
            (0..crate::plock::STEP_COUNT)
                .map(|step| seq.probabilities[slot][step].load(Ordering::Relaxed))
                .collect()
        })
        .collect();
    let old_seq_stutters: Vec<Vec<u32>> = (0..crate::track::MAX_TRACKS)
        .map(|slot| {
            (0..crate::plock::STEP_COUNT)
                .map(|step| seq.stutters[slot][step].load(Ordering::Relaxed))
                .collect()
        })
        .collect();
    let old_seq_conditions: Vec<Vec<u32>> = (0..crate::track::MAX_TRACKS)
        .map(|slot| {
            (0..crate::plock::STEP_COUNT)
                .map(|step| seq.conditions[slot][step].load(Ordering::Relaxed))
                .collect()
        })
        .collect();
    let old_seq_microtimings: Vec<Vec<u32>> = (0..crate::track::MAX_TRACKS)
        .map(|slot| {
            (0..crate::plock::STEP_COUNT)
                .map(|step| seq.microtimings[slot][step].load(Ordering::Relaxed))
                .collect()
        })
        .collect();
    for (new_idx, &old_idx) in order.iter().enumerate() {
        seq.masks[new_idx].store(old_seq_masks[old_idx], Ordering::Relaxed);
        seq.solo_masks[new_idx].store(old_seq_solo_masks[old_idx], Ordering::Relaxed);
        for step in 0..crate::plock::STEP_COUNT {
            seq.probabilities[new_idx][step]
                .store(old_seq_probabilities[old_idx][step], Ordering::Relaxed);
            seq.stutters[new_idx][step].store(old_seq_stutters[old_idx][step], Ordering::Relaxed);
            seq.conditions[new_idx][step]
                .store(old_seq_conditions[old_idx][step], Ordering::Relaxed);
            seq.microtimings[new_idx][step]
                .store(old_seq_microtimings[old_idx][step], Ordering::Relaxed);
        }
    }

    let mute_values: [bool; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.mutes()[slot].value());
    let solo_values: [bool; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.solos()[slot].value());
    let mix_values: [bool; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.mixes()[slot].value());
    let algo_values: [i32; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.algos()[slot].value());
    let humanize_values: [f32; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.humanizes()[slot].value());
    let push_values: [f32; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.pushes()[slot].value());
    let length_values: [i32; crate::track::MAX_TRACKS] =
        std::array::from_fn(|slot| params.lengths()[slot].value());

    for (new_idx, &old_idx) in order.iter().enumerate() {
        set_bool_param_if_changed(setter, params.mutes()[new_idx], mute_values[old_idx]);
        set_bool_param_if_changed(setter, params.solos()[new_idx], solo_values[old_idx]);
        set_bool_param_if_changed(setter, params.mixes()[new_idx], mix_values[old_idx]);
        set_int_param_if_changed(setter, params.algos()[new_idx], algo_values[old_idx]);
        set_float_param_if_changed(
            setter,
            params.humanizes()[new_idx],
            humanize_values[old_idx],
        );
        set_float_param_if_changed(setter, params.pushes()[new_idx], push_values[old_idx]);
        set_int_param_if_changed(setter, params.lengths()[new_idx], length_values[old_idx]);
    }

    let old_lock_mask = PersistentField::<u16>::map(&params.lane_length_locks, |mask| *mask);
    PersistentField::<u16>::set(
        &params.lane_length_locks,
        move_mask_bits(old_lock_mask, &order),
    );

    // [228] The lane's custom texture moves with it (was left behind on the
    // old number, then dropped by the "no Texture menu here" rule).
    params.user_textures.reorder(&order);

    // [242] Macro assignments point at lane NUMBERS packed inside each entry:
    // they move with their lane like every other per-slot store.
    params.macro_map_state.state.reorder(&order);

    let old_selection = state.selected_track_slot;
    let old_selected_instrument = state.selected_instrument;
    let old_fusion_selection = state.fusion_selection_start;
    let old_slot_flash_until = state.slot_flash_until;
    state.selected_track_slot = remap_slot_index(&order, old_selection);
    state.selected_instrument = remap_slot_index(&order, old_selected_instrument);
    state.fusion_selection_start =
        std::array::from_fn(|new_idx| old_fusion_selection[order[new_idx]]);
    state.slot_flash_until = std::array::from_fn(|new_idx| old_slot_flash_until[order[new_idx]]);
    state.fusion_editing = state
        .fusion_editing
        .map(|(slot, group)| (remap_slot_index(&order, slot), group));
    state.plock_popup = state.plock_popup.map(|mut popup| {
        popup.instrument = remap_slot_index(&order, popup.instrument);
        popup
    });
    // [184] The cell selection follows its lane too, or it would suddenly point
    // at whichever lane took that slot.
    state.sound_edit_target = state.sound_edit_target.map(|mut cell| {
        cell.slot = remap_slot_index(&order, cell.slot);
        cell
    });

    let mut new_layout =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    let old_slots = new_layout.slots.clone();
    for (new_idx, &old_idx) in order.iter().enumerate() {
        new_layout.slots[new_idx] = old_slots[old_idx].clone();
    }
    PersistentField::<TrackLayoutState>::set(&params.track_layout, new_layout);

    // The 16 saved patterns hold their own copy of every lane's steps, fusions
    // and p-locks. Permute them with the same order, or recalling a pattern
    // would put each instrument's steps back on the lane it used to occupy.
    params.pattern_bank.permute_lanes_in_saved_patterns(&order);
}

/// Change an active slot's instrument kind (lane-name context menu). Same
/// semantics as the Track tab Type dropdown: default name + MIDI note, and the
/// slot's sound settings are reseeded from the new kind's defaults (the audio
/// thread reinitializes the voice on kind change).
pub fn change_slot_kind(
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    slot_idx: usize,
    kind: TrackInstrumentKind,
) {
    if slot_idx >= crate::track::MAX_TRACKS {
        return;
    }
    let mut new_state =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    let slot = &mut new_state.slots[slot_idx];
    if !slot.active || slot.kind == kind {
        return;
    }
    slot.kind = kind;
    slot.name = kind.default_name().to_string();
    // [304] The LFO syncs belonged to the old instrument's LFOs.
    slot.lfo_sync = [0; crate::track::LFO_SYNC_SLOTS];
    // [285] A new instrument: its type's Randomize density.
    slot.randomize_density = state.global_config.randomize_density_for(kind);
    slot.midi_note = kind.default_midi_note();
    PersistentField::<TrackLayoutState>::set(&params.track_layout, new_state);
    sound_settings.reset_slot_to_defaults(slot_idx, kind, state.global_config.default_analog);
    state.selected_instrument = slot_idx;
}

/// Activate a specific inactive slot with the chosen instrument kind.
/// Triggered from the empty-lane instrument picker.
pub fn activate_slot(
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    slot_idx: usize,
    kind: TrackInstrumentKind,
    pattern: &SharedPattern,
    plock: &PlockState,
) {
    if slot_idx >= crate::track::MAX_TRACKS {
        return;
    }
    let mut new_state =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    if new_state.slots[slot_idx].active {
        return;
    }
    new_state.slots[slot_idx] = TrackSlot::active_with_kind(kind);
    // [285] The lane starts with its instrument type's Randomize density (Settings).
    new_state.slots[slot_idx].randomize_density = state.global_config.randomize_density_for(kind);
    // [298] An effect lane is named after its effect (Reverb, Delay, Rumble), like any lane
    // is named after its instrument: `active_with_kind` already did it.
    PersistentField::<TrackLayoutState>::set(&params.track_layout, new_state);
    // The slot's settings still hold whatever they were initialized with
    // (legacy defaults of the same index) — align them with the new kind.
    sound_settings.reset_slot_to_defaults(slot_idx, kind, state.global_config.default_analog);
    // Deleting a lane leaves its musical data behind on purpose, so a slot
    // reused for a new instrument would inherit the previous lane's steps.
    // Wipe them here, in the live pattern and in the saved ones alike, so a
    // freshly placed instrument starts blank on all 16 patterns.
    clear_lane_musical_data(params, pattern, plock, slot_idx);
    select_legacy_track(state, slot_idx);
}

/// Wipe one lane's steps, fusions and p-locks from the live pattern AND from
/// the 16 saved patterns.
pub fn clear_lane_musical_data(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    plock: &PlockState,
    slot_idx: usize,
) {
    crate::ui::editor_state::clear_grid_steps(pattern, slot_idx);
    pattern.store_fusions(slot_idx, &[]);
    crate::ui::editor_state::clear_grid_sound_plocks(plock, slot_idx);
    crate::ui::editor_state::clear_grid_seq_plocks(&params.seq_plock_state.state, slot_idx);
    params.pattern_bank.clear_lane_in_saved_patterns(slot_idx);
}

/// Deactivate an active slot so it becomes an empty lane again.
/// Set/clear the grid-link flag on a lane (mirrors the lane above for layering).
fn set_lane_linked_up(params: &DrumFlashParams, slot_idx: usize, linked: bool) {
    if slot_idx >= crate::track::MAX_TRACKS {
        return;
    }
    let mut new_state =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    new_state.slots[slot_idx].linked_up = linked;
    PersistentField::<TrackLayoutState>::set(&params.track_layout, new_state);
}

/// Triggered from the lane title context menu.
fn deactivate_slot(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    state: &mut EditorUIState,
    slot_idx: usize,
) {
    if slot_idx >= crate::track::MAX_TRACKS {
        return;
    }
    let mut new_state =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    if !new_state.slots[slot_idx].active {
        return;
    }
    new_state.slots[slot_idx].active = false;
    let next_selected = new_state.active_slot_indices().next().unwrap_or(0);
    PersistentField::<TrackLayoutState>::set(&params.track_layout, new_state);

    // The lane becomes invisible: its mute/solo must not keep acting.
    crate::ui::controls::set_bool_param_if_changed(setter, params.mutes()[slot_idx], false);
    crate::ui::controls::set_bool_param_if_changed(setter, params.solos()[slot_idx], false);

    state.fusion_selection_start[slot_idx] = None;
    state.fusion_editing = state.fusion_editing.filter(|(idx, _)| *idx != slot_idx);
    state.plock_popup = state
        .plock_popup
        .filter(|popup| popup.instrument != slot_idx);
    // [184] The lane's p-locks go with it, so the cell selection must too.
    state.sound_edit_target = state.sound_edit_target.filter(|cell| cell.slot != slot_idx);
    state.lane_clear_grid_confirm = None;
    state.lane_delete_confirm = None;

    if state.selected_track_slot == slot_idx {
        state.selected_track_slot = next_selected;
        state.selected_instrument = next_selected;
    }
}

/// [212] Write the page-loop parameter: `None` = off, `Some(page)` = loop it.
pub fn set_page_loop_param(setter: &ParamSetter, params: &DrumFlashParams, page: Option<usize>) {
    let value = page.map(|p| p as i32 + 1).unwrap_or(0);
    if params.page_loop.value() != value {
        setter.begin_set_parameter(&params.page_loop);
        setter.set_parameter(&params.page_loop, value);
        setter.end_set_parameter(&params.page_loop);
    }
}

fn draw_page_bar_v2(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    _sound_settings: &SoundSettingsState,
    plock: &PlockState,
    state: &mut EditorUIState,
    play_page: usize,
    master_length: usize,
) {
    let page_count = (master_length + 15) / 16;
    // [212] Which page loops (param 0 = none, 1-4 = page). Beyond the pattern
    // the loop is dead: the param is put back to 0 so the ring goes away too.
    let song_mode = params.song_mode.value();
    let looped_page = match params.page_loop.value() {
        p if p >= 1 => Some(p as usize - 1),
        _ => None,
    };
    if let Some(p) = looped_page {
        if p >= page_count.max(1) {
            set_page_loop_param(setter, params, None);
        }
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.allocate_ui_with_layout(
            Vec2::new(50.0, CTL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.label(RichText::new("Page").font(f_sans_sb(10.5)).color(INK3()));
            },
        );
        for page in 0..4 {
            let enabled = page < page_count.max(1);
            let active = state.current_page == page;
            let response = crate::ui::controls::keycap_button(
                ui,
                &format!("{}", page + 1),
                28.0,
                if active {
                    crate::ui::widgets::KeycapState::PressedBlue
                } else {
                    crate::ui::widgets::KeycapState::Rest
                },
                enabled,
                f_mono_med(10.5),
            );
            if play_page == page {
                // Red play LED incrusted in the button's top-right corner.
                let r = response.rect;
                crate::ui::skeuo::play_led(ui.painter(), egui::pos2(r.right() - 7.0, r.top() + 7.0), 2.6);
            }
            // [212] Amber ring = this page loops. Greyed in Song mode, where
            // the loop is ignored.
            if looped_page == Some(page) && enabled {
                let ring = if song_mode { INK3() } else { AMBER() };
                ui.painter().rect_stroke(
                    response.rect.expand(1.5),
                    5.0,
                    egui::Stroke::new(1.5, ring),
                    egui::StrokeKind::Outside,
                );
            }
            let response = response.help_or(help::PAGE_KEYCAP, if looped_page == Some(page) {
                if song_mode {
                    "This page is set to loop, but Song mode plays the chain: the loop is ignored until Song is off. Double-click to release."
                } else {
                    "This page loops. Double-click to release it and play the whole pattern again."
                }
            } else {
                "Click: show this page. Double-click: loop this page alone (the playhead stays in it, keeping its phase). Right-click: copy / paste / clear / loop."
            });
            if response.double_clicked() && enabled {
                let next = if looped_page == Some(page) { None } else { Some(page) };
                set_page_loop_param(setter, params, next);
            }
            if response.clicked() {
                state.current_page = page;
            }
            if response.secondary_clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    state.page_popup = Some(PagePopup {
                        page,
                        screen_pos: pos,
                        confirm_action: None,
                    });
                }
            }
        }

        let follow_label = if state.follow_mode {
            "Follow ON"
        } else {
            "Follow OFF"
        };
        let follow_state = if state.follow_mode {
            crate::ui::widgets::KeycapState::PressedBlue
        } else {
            crate::ui::widgets::KeycapState::Rest
        };
        if crate::ui::controls::keycap_button(ui, follow_label, 78.0, follow_state, true, f_sans_sb(11.0))
            .help(help::PAGE_FOLLOW)
            .clicked()
        {
            state.follow_mode = !state.follow_mode;
        }
        const LEN_GROUP_W: f32 = 468.0;
        const LEN_VALUE_W: f32 = 64.0;
        ui.add_space((ui.available_width() - LEN_GROUP_W).max(0.0));
        ui.allocate_ui_with_layout(
            Vec2::new(LEN_GROUP_W, CTL_HEIGHT),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.label(RichText::new("Len").font(f_sans_sb(10.5)).color(INK3()));
                help::zone(ui, "pattern_len", help::PATTERN_LEN, |ui| {
                    header_param_slider(ui, setter, &params.pattern_length, 132.0, "", false);
                    draw_len_value_fixed(ui, master_length, LEN_VALUE_W);
                });
                for &len in &[16, 32, 48, 64] {
                    let active = master_length == len;
                    if crate::ui::controls::keycap_button(
                        ui,
                        &format!("{}", len),
                        36.0,
                        if active {
                            crate::ui::widgets::KeycapState::PressedBlue
                        } else {
                            crate::ui::widgets::KeycapState::Rest
                        },
                        true,
                        f_mono_med(10.5),
                    )
                    .help(help::PATTERN_LEN)
                    .clicked()
                    {
                        setter.set_parameter(&params.pattern_length, len as i32);
                    }
                }
                let can_double = master_length <= 32;
                if crate::ui::controls::keycap_button(
                    ui,
                    "x2",
                    36.0,
                    crate::ui::widgets::KeycapState::Rest,
                    can_double,
                    f_mono_med(10.5),
                )
                .help(help::PATTERN_X2)
                .clicked()
                {
                    for i in 0..master_length {
                        pattern.set_step_mask(master_length + i, pattern.load_step_mask(i));
                        for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
                            // [231] The sequencer p-locks double with the cells.
                            params
                                .seq_plock_state
                                .state
                                .copy_step(inst, i, master_length + i);
                            if plock.masks.is_active(inst, i) {
                                let field_mask = plock.field_masks.get_raw(inst, i);
                                plock.masks.set_active(inst, master_length + i, true);
                                plock
                                    .field_masks
                                    .set_raw(inst, master_length + i, field_mask);
                                for field in 0..crate::plock::FIELD_COUNT {
                                    let value = plock.values.get(inst, i, field);
                                    plock.values.set(inst, master_length + i, field, value);
                                }
                            }
                        }
                    }
                    duplicate_fusions_for_x2(pattern, params, plock, master_length);
                    setter.set_parameter(&params.pattern_length, (master_length * 2) as i32);
                }
            },
        );
    });
}

fn draw_len_value_fixed(ui: &mut egui::Ui, master_length: usize, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, CTL_HEIGHT), egui::Sense::hover());
    let number_text = format!("{:>2}", master_length);
    let number_pos = egui::pos2(rect.left(), rect.center().y);
    ui.painter().text(
        number_pos,
        egui::Align2::LEFT_CENTER,
        number_text,
        f_mono(12.0),
        INK(),
    );
    ui.painter().text(
        egui::pos2(rect.left() + 23.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "steps",
        f_sans(9.5),
        INK3(),
    );
}

fn draw_seq_header_v2(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    plock: &PlockState,
    state: &mut EditorUIState,
    page_offset: usize,
    play_step: usize,
    master_length: usize,
    grip_w: f32,
    name_w: f32,
    vol_w: f32,
    mst_w: f32,
    extra_w: f32,
    gap: f32,
    cell_w: f32,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.add_sized(Vec2::new(grip_w, 16.0), egui::Label::new(""));

        // ‹ › above the lane names: shift the whole grid by one cell
        // (steps, fusions and p-locks wrap inside the pattern length).
        // The column is reserved at its FULL width first: `allocate_ui` kept
        // only what the two 14 px arrows use, which pulled every heading after
        // it (Vol, M / S, the step numbers, Push / Len) ~32 px left of
        // its column.
        let (name_rect, _) = ui.allocate_exact_size(Vec2::new(name_w, 16.0), egui::Sense::hover());
        let mut arrows = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(name_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        arrows.spacing_mut().item_spacing.x = GAP_TIGHT;
        for (glyph, delta, hover) in [
            ("‹", -1isize, "Shift the whole grid one step left (steps, fusions and p-locks wrap inside the pattern length)"),
            ("›", 1isize, "Shift the whole grid one step right (steps, fusions and p-locks wrap inside the pattern length)"),
        ] {
            let (rect, resp) =
                arrows.allocate_exact_size(Vec2::new(14.0, 16.0), egui::Sense::click());
            let color = if resp.hovered() { INK() } else { FAINT() };
            arrows.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                glyph,
                f_mono(12.0),
                color,
            );
            let resp = resp.help_or(help::GRID_SHIFT, hover);
            if resp.clicked() {
                let len = master_length.clamp(1, 64);
                if straddling_fusions(pattern, len, delta).is_empty() {
                    shift_grid(params, pattern, plock, state, master_length, delta);
                } else {
                    // A fusion sits at the edge we push toward: ask
                    // before breaking it (reduced to its first cell).
                    state.shift_grid_confirm = Some(delta);
                }
            }
        }
        ui.add_sized(
            Vec2::new(vol_w, 16.0),
            egui::Label::new(RichText::new("Vol").font(f_sans_sb(9.5)).color(INK3())),
        )
        .help(help::LANE_VOL);
        // M / S / T column headings (aligned with the lane tags below)
        ui.allocate_ui(Vec2::new(mst_w, 16.0), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = GAP_TIGHT;
                for (t, text) in [("M", help::LANE_MUTE), ("S", help::LANE_SOLO)] {
                    ui.add_sized(
                        Vec2::new(STEP_H, 16.0),
                        egui::Label::new(RichText::new(t).font(f_mono(9.0)).color(INK3())),
                    )
                    .help(text);
                }
            });
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = GAP_TIGHT;
            for local in 0..16 {
                let step = page_offset + local;
                let color = if play_step == step {
                    BLUE()
                } else if local % 4 == 0 {
                    INK2()
                } else {
                    FAINT()
                };
                ui.add_sized(
                    Vec2::new(cell_w, 16.0),
                    egui::Label::new(
                        RichText::new(format!("{}", step + 1))
                            .font(f_mono(9.0))
                            .color(color),
                    ),
                )
                .help(help::GRID_STEP_NUMBERS);
            }
        });
        for (label, text) in [
            ("Push", help::LANE_PUSH),
            ("Len", help::LANE_LEN),
        ] {
            ui.add_sized(
                Vec2::new(extra_w, 16.0),
                egui::Label::new(RichText::new(label).font(f_sans_sb(9.5)).color(INK3())),
            )
            .help(text);
        }
    });
}

fn draw_seq_grip_v2(ui: &mut egui::Ui, width: f32, height: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), egui::Sense::drag());
    // 2x3 dot matrix (drag-handle look; avoids relying on braille glyph coverage)
    let c = rect.center();
    let dot_color = if response.dragged() || response.hovered() {
        INK2()
    } else {
        FAINT()
    };
    for col in 0..2 {
        for row in 0..3 {
            let p = egui::pos2(
                c.x + (col as f32 - 0.5) * 4.0,
                c.y + (row as f32 - 1.0) * 3.0,
            );
            ui.painter().circle_filled(p, 1.0, dot_color);
        }
    }
    response
}

/// [243] A WAV dropped from the OS onto the grid creates a One-Shot lane with
/// the file loaded on it (same path as File > Load... — the file is the
/// lane's own [228] file: persisted, follows the lane, dropped on kind
/// change). Target: the empty lane under the pointer, else the first free
/// slot. [270] Dropped on a One-Shot lane, the file replaces that lane's file
/// (its steps and settings stay); any other occupied lane is never replaced.
/// The lane flash confirms the creation or the replacement.
#[allow(clippy::too_many_arguments)]
fn handle_wav_drop(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    pattern: &SharedPattern,
    plock: &PlockState,
    lane_row_rects: &[Option<egui::Rect>; crate::track::MAX_TRACKS],
    grid_rect: egui::Rect,
) {
    let active: [bool; crate::track::MAX_TRACKS] =
        std::array::from_fn(|i| params.track_layout.state.is_active(i));
    let oneshot = oneshot_lanes(params, &active);
    let modal_open = state.preset_browser.is_some() || state.macros_open || state.settings_open;
    let available = (active.iter().take(crate::track::FX_FIRST_SLOT).any(|a| !*a)
        || oneshot.iter().any(|o| *o))
        && !modal_open;
    // [270] Under the dragged file: what a drop there would do. Where it would do
    // nothing (a full grid, a lane other than One-Shot) the target is withdrawn, so
    // the OS shows its ⊘ and the reason below.
    let hover_target = nih_plug_egui::file_drop::hover_position(ui.ctx())
        .filter(|pos| grid_rect.contains(*pos))
        .map(|pos| (pos, pick_drop_lane(pos, lane_row_rects, &active, &oneshot)));
    let accepting = available && !matches!(hover_target, Some((_, None)));
    nih_plug_egui::file_drop::set_target(ui.ctx(), accepting.then_some(grid_rect), &["wav"]);

    // [273] A rejected drop says WHY next to the bare OS ⊘ cursor: full grid
    // (only a One-Shot lane takes a file) or an open modal blocks the target.
    if !accepting {
        if let Some(pos) = nih_plug_egui::file_drop::rejected_hover_position(ui.ctx()) {
            if grid_rect.contains(pos) {
                let reason = if state.preset_browser.is_some() {
                    "Close the Presets dialog to drop files."
                } else if state.macros_open {
                    "Close the Macros dialog to drop files."
                } else if state.settings_open {
                    "Close the Settings dialog to drop files."
                } else if oneshot.iter().any(|o| *o) {
                    "Grid full - only a One-Shot lane takes a dropped file (it replaces its file). Remove a lane to create a new one."
                } else {
                    "Grid full - an occupied lane is never replaced; remove one first."
                };
                wav_drop_hint(ui, pos, reason);
            }
        }
    } else if let Some((_, Some(DropTarget::Replace(slot)))) = hover_target {
        // [270] A drop here replaces a file: say so before it happens, ON the lane
        // (a note by the cursor went unseen during the drag).
        if let Some(row) = lane_row_rects[slot] {
            wav_replace_banner(ui, row.intersect(grid_rect));
        }
    }

    for drop in nih_plug_egui::file_drop::take_dropped(ui.ctx()) {
        if !available || !grid_rect.contains(drop.position) {
            continue;
        }
        let Some(path) = drop.paths.iter().find(|p| {
            p.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        }) else {
            continue;
        };
        let active: [bool; crate::track::MAX_TRACKS] =
            std::array::from_fn(|i| params.track_layout.state.is_active(i));
        let oneshot = oneshot_lanes(params, &active);
        let Some(target) = pick_drop_lane(drop.position, lane_row_rects, &active, &oneshot) else {
            continue;
        };
        let (slot, done) = match target {
            DropTarget::Create(slot) => (
                slot,
                create_oneshot_from_drop(params, sound_settings, state, pattern, plock, slot, path),
            ),
            DropTarget::Replace(slot) => (
                slot,
                replace_oneshot_file(params, sound_settings, slot, path),
            ),
        };
        if done {
            state.slot_flash_until[slot] = ui.ctx().input(|i| i.time) + 0.5;
            ui.ctx().request_repaint();
        }
    }
}

/// [273] [270] A note next to the cursor during a file drag over the grid.
fn wav_drop_hint(ui: &mut egui::Ui, pos: egui::Pos2, text: &str) {
    egui::Area::new(ui.id().with("wav_drop_hint"))
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + egui::vec2(14.0, 14.0))
        .show(ui.ctx(), |ui| {
            let bg = ui.painter().add(egui::Shape::Noop);
            let resp = egui::Frame::NONE
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(RichText::new(text).font(f_sans_med(9.5)).color(INK()));
                });
            ui.painter().set(
                bg,
                crate::ui::skeuo::plate_shape(resp.response.rect, RADIUS_PANEL as f32),
            );
        });
}

/// [270] The warning over the One-Shot lane a dragged WAV would replace the file of:
/// the row framed in amber, the text across it.
fn wav_replace_banner(ui: &egui::Ui, row: egui::Rect) {
    let painter = ui.ctx().layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        ui.id().with("wav_replace_banner"),
    ));
    painter.rect(
        row,
        3.0,
        Color32::from_black_alpha(190),
        egui::Stroke::new(1.5, AMBER()),
        egui::StrokeKind::Inside,
    );
    painter.text(
        row.center(),
        egui::Align2::CENTER_CENTER,
        WAV_REPLACE_WARNING,
        f_sans_med(10.0),
        AMBER(),
    );
}

const WAV_REPLACE_WARNING: &str = "Drop here REPLACES this One-Shot lane's file";

/// [270] The lanes a dropped WAV can replace the file of: the active One-Shot lanes.
fn oneshot_lanes(
    params: &DrumFlashParams,
    active: &[bool; crate::track::MAX_TRACKS],
) -> [bool; crate::track::MAX_TRACKS] {
    std::array::from_fn(|i| {
        active[i]
            && params.track_layout.state.kind_for_slot(i) == Some(TrackInstrumentKind::OneShot)
    })
}

/// [270] Where a dropped WAV goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropTarget {
    /// A new One-Shot lane on this empty slot.
    Create(usize),
    /// This One-Shot lane's file is replaced.
    Replace(usize),
}

/// [270] A WAV dropped on a One-Shot lane replaces its file, like File > Load in its
/// Sound tab: the steps, locks and settings stay; a stereo file comes up in stereo.
fn replace_oneshot_file(
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    slot: usize,
    path: &std::path::Path,
) -> bool {
    if params.track_layout.state.kind_for_slot(slot) != Some(TrackInstrumentKind::OneShot)
        || !params.track_layout.state.is_active(slot)
    {
        return false;
    }
    if params.user_textures.load(slot, path).is_err() {
        return false;
    }
    if params.user_textures.is_stereo(slot) {
        sound_settings.instruments[slot]
            .set_standard(crate::instrument_registry::StandardField::Stereo, 1.0);
    }
    sound_settings.bump_version();
    true
}

fn create_oneshot_from_drop(
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    pattern: &SharedPattern,
    plock: &PlockState,
    slot: usize,
    path: &std::path::Path,
) -> bool {
    if slot >= crate::track::MAX_TRACKS || params.track_layout.state.is_active(slot) {
        return false;
    }
    // change_slot_kind deliberately ignores inactive slots; creation must
    // use the same activation/cleanup path as the empty-lane picker.
    activate_slot(
        params,
        sound_settings,
        state,
        slot,
        TrackInstrumentKind::OneShot,
        pattern,
        plock,
    );
    if params.user_textures.load(slot, path).is_ok() {
        sound_settings.instruments[slot].set_standard(
            crate::instrument_registry::StandardField::Stereo,
            if params.user_textures.is_stereo(slot) {
                1.0
            } else {
                0.0
            },
        );
        sound_settings.bump_version();
    }
    true
}

/// The drop target: the row UNDER the pointer when it hosts no active lane,
/// else the first inactive slot. [270] A One-Shot row under the pointer gets
/// its file replaced; any other occupied row is never replaced, and the
/// effect-bus rows ([286]) never take an instrument.
fn pick_drop_lane(
    pos: egui::Pos2,
    rects: &[Option<egui::Rect>],
    active: &[bool],
    oneshot: &[bool],
) -> Option<DropTarget> {
    for (i, rect) in rects.iter().enumerate() {
        if let Some(rect) = rect {
            if rect.contains(pos) && !crate::track::is_fx_slot(i) {
                if oneshot.get(i).copied().unwrap_or(false) {
                    return Some(DropTarget::Replace(i));
                }
                if !active.get(i).copied().unwrap_or(true) {
                    return Some(DropTarget::Create(i));
                }
            }
        }
    }
    active
        .iter()
        .take(crate::track::FX_FIRST_SLOT)
        .position(|a| !*a)
        .map(DropTarget::Create)
}

fn draw_lane_name_v2(
    ui: &mut egui::Ui,
    width: f32,
    selected: bool,
    label: &str,
    flash: f32,
    fx: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 21.0), egui::Sense::click());
    crate::ui::skeuo::lane_name(ui, rect, label, selected, flash, fx);
    response
}

fn draw_tag_button_v2(
    ui: &mut egui::Ui,
    label: &str,
    color: Color32,
    text_on: Color32,
    active: bool,
    tooltip: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TAG_SIZE), egui::Sense::click());
    crate::ui::skeuo::tag(ui, rect, label, active, color, text_on);
    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

fn draw_tag_param_v2(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &BoolParam,
    label: &str,
    color: Color32,
    text_on: Color32,
    tooltip: &str,
) -> egui::Response {
    let value = param.value();
    let response = draw_tag_button_v2(ui, label, color, text_on, value, tooltip);
    if response.clicked() {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, !value);
        setter.end_set_parameter(param);
    }
    response
}

fn draw_step_cell_v2(
    ui: &mut egui::Ui,
    size: Vec2,
    fill: Color32,
    stroke: egui::Stroke,
    text: &str,
    enabled: bool,
    playhead: bool,
    is_fusion_mid: bool,
    fusion_span: Option<usize>,
    dashed_border: bool,
    // Fused-block state (the atlas sprite is always the lit "hit" variant, so
    // muted/editing states are shown by overlays below).
    fusion_active: bool,
    fusion_editing: bool,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);

    // Fusion block: the start cell draws one continuous rectangle spanning all
    // merged cells (including the gaps between them). Mid cells stay transparent
    // so the start cell's block remains unbroken.
    let block_rect = if let Some(span) = fusion_span {
        let mut r = rect;
        let extra = (size.x + GAP_TIGHT) * (span.saturating_sub(1) as f32);
        r.set_right(r.right() + extra);
        r
    } else {
        rect
    };

    let hover = hover_t(
        ui.ctx(),
        response.id,
        response.hovered() && enabled && !is_fusion_mid,
    );
    let stroke = if hover > 0.01 {
        egui::Stroke::new(1.0, lerp_color(stroke.color, BLUE(), hover))
    } else {
        stroke
    };

    if !is_fusion_mid {
        if dashed_border {
            // Empty-lane placeholder: flat recessed fill + dotted outline.
            ui.painter().rect_filled(block_rect, 4.0, fill);
            if stroke.width > 0.0 && stroke.color.a() > 0 {
                draw_dashed_rect(ui.painter(), block_rect.shrink(0.5));
            }
        } else {
            // Step cell = designer baked bitmap atlas (state + fusion length,
            // bleed-corrected). Overlays (playhead ring, pulse count) drawn below.
            crate::ui::pads::draw_pad(ui, block_rect, fill, fusion_span, enabled);
            // Downbeats (1/5/9/13): a faint light wash makes the off cells read
            // as anchors — the baked pad-off-beat sprite alone is too subtle.
            if fill == CELL_EMPTY_BEAT() {
                ui.painter().rect_filled(block_rect, 4.0, white_a(6));
            }
            // Fused-block state overlays (the sprite is always the lit "hit"
            // variant, so muted/editing states can't come from the sprite).
            if fusion_span.is_some() {
                if fusion_editing {
                    // Editing: a bright outline that BLINKS on/off (~1 Hz) around
                    // the whole block.
                    let t = ui.ctx().input(|i| i.time) as f32;
                    if (t * 6.0).sin() > 0.0 {
                        ui.painter().rect_stroke(
                            block_rect,
                            egui::epaint::CornerRadius::same(4),
                            egui::Stroke::new(2.0, Color32::from_rgb(140, 190, 255)),
                            egui::StrokeKind::Inside,
                        );
                    }
                } else if !fusion_active {
                    // Muted (toggled off): a dark overlay dims the whole block.
                    ui.painter()
                        .rect_filled(block_rect, 4.0, Color32::from_black_alpha(165));
                }
            }
            // Hover: a subtle white outline that fades in (state-agnostic, so it
            // never tints an orange/red pad blue).
            if hover > 0.01 {
                ui.painter().rect_stroke(
                    block_rect,
                    egui::epaint::CornerRadius::same(4),
                    egui::Stroke::new(1.0, Color32::from_white_alpha((hover * 90.0) as u8)),
                    egui::StrokeKind::Inside,
                );
            }
        }
    }

    // Playhead: an inset white ring drawn ON TOP of the state border, with a
    // subtle pulse so the playing column reads at a glance. On a fused cell the
    // ring wraps the WHOLE fusion block, not the individual start cell.
    if playhead {
        let time = ui.ctx().input(|i| i.time) as f32;
        let pulse = ((time * 2.5).sin() + 1.0) * 0.5;
        let alpha = 120 + (pulse * 80.0) as u8;
        let ring_rect = if fusion_span.is_some() {
            block_rect
        } else {
            rect
        };
        // The ring stays WHITE on every pad, light green included: the playhead
        // has to read as one continuous column, and a ring that changes colour
        // cell by cell breaks that more than a low contrast ever did.
        ui.painter().rect_stroke(
            ring_rect.shrink(0.75),
            egui::epaint::CornerRadius::same(3),
            egui::Stroke::new(1.5, white_a(alpha)),
            egui::StrokeKind::Inside,
        );
    }
    if !text.is_empty() {
        ui.painter().text(
            block_rect.center(),
            egui::Align2::CENTER_CENTER,
            text,
            f_mono_sb(10.0),
            if is_light_pad(fill) {
                Color32::from_rgb(12, 32, 26)
            } else {
                Color32::WHITE
            },
        );
    }
    response
}

/// True for a pad bright enough that a white overlay would vanish on it - the
/// light-green sound-p-lock fill. Only the fusion pulse count flips to
/// near-black on those; the playhead ring stays white everywhere.
fn is_light_pad(fill: Color32) -> bool {
    let y = 0.2126 * fill.r() as f32 + 0.7152 * fill.g() as f32 + 0.0722 * fill.b() as f32;
    y > 170.0
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn draw_dashed_rect(painter: &egui::Painter, rect: egui::Rect) {
    let stroke = egui::Stroke::new(2.5, Color32::from_rgba_unmultiplied(0, 0, 0, 235));
    let dash = 6.0;
    let gap = 3.0;
    let step = dash + gap;
    let corner = 4.0;

    let mut x = rect.left() + corner;
    while x < rect.right() - corner {
        let end = (x + dash).min(rect.right() - corner);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(end, rect.top())],
            stroke,
        );
        painter.line_segment(
            [egui::pos2(x, rect.bottom()), egui::pos2(end, rect.bottom())],
            stroke,
        );
        x += step;
    }

    let mut y = rect.top() + corner;
    while y < rect.bottom() - corner {
        let end = (y + dash).min(rect.bottom() - corner);
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.left(), end)],
            stroke,
        );
        painter.line_segment(
            [egui::pos2(rect.right(), y), egui::pos2(rect.right(), end)],
            stroke,
        );
        y += step;
    }
}

fn step_colors_v2(
    ctx: &egui::Context,
    sequencer_mode: bool,
    local_step: usize,
    active: bool,
    has_sound_plock: bool,
    is_snapshot: bool,
    has_seq_plock: bool,
    is_current: bool,
    disabled: bool,
    selection_start: bool,
    is_fusion_start: bool,
    is_fusion_mid: bool,
    is_editing: bool,
) -> (Color32, egui::Stroke) {
    if disabled {
        return (CELL_DISABLED(), egui::Stroke::new(1.0, Color32::BLACK));
    }
    if selection_start {
        // [210] The old pair was invisible: a fill of rgb(20,34,58) against an
        // empty cell at rgb(27,27,34) - same brightness, barely bluer - behind
        // a 1.5 px line. The fill is now lifted towards the accent and the
        // outline is solid, and `fusion_start_bracket` draws the `[` on top.
        let f = FUSION_FILL();
        let b = BLUE();
        let lift = |a: u8, t: u8| (a as f32 + (t as f32 - a as f32) * 0.30) as u8;
        let fill = Color32::from_rgb(lift(f.r(), b.r()), lift(f.g(), b.g()), lift(f.b(), b.b()));
        return (fill, egui::Stroke::new(2.0, b));
    }

    let empty = if local_step % 4 == 0 {
        CELL_EMPTY_BEAT()
    } else {
        CELL_EMPTY_OFF()
    };

    // Compute the base color as if the cell were not fused.
    let (fill, border) = if sequencer_mode {
        if active && has_seq_plock {
            (SEQPL(), SEQPL())
        } else if has_seq_plock {
            (CELL_SEQPL_OFF(), SEQPL_DIM())
        } else if active {
            (BLUE(), BLUE())
        } else if is_current {
            (CELL_CURRENT(), LINE())
        } else {
            (empty, LINE())
        }
    } else if active && has_sound_plock {
        (PL_LINK(), PL_LINK())
    } else if active {
        (BLUE(), BLUE())
    } else if has_sound_plock {
        if is_snapshot {
            (CELL_PL_SNAP_OFF(), PL_SNAP_DIM())
        } else {
            (CELL_PL_LINK_OFF(), PL_LINK_DIM())
        }
    } else if is_current {
        (CELL_CURRENT(), LINE())
    } else {
        (empty, LINE())
    };

    // Fusion overrides: keep the plock color if the fused cell carries a sound
    // plock or a morph (modulation), so fused cells are not visually muted.
    if is_fusion_start && !is_editing {
        if has_sound_plock || has_seq_plock {
            return (fill, egui::Stroke::new(1.0, BLUE()));
        }
        return (FUSION_FILL(), egui::Stroke::new(1.0, BLUE()));
    }
    if is_fusion_mid && !is_editing {
        return (Color32::TRANSPARENT, egui::Stroke::NONE);
    }

    if is_editing {
        let time = ctx.input(|i| i.time) as f32;
        let pulse = ((time * 4.0).sin() + 1.0) * 0.5;
        // When editing a fused group, pulse every cell of the group using the
        // same blue fusion base so the block flashes as one unit.
        let fusion_fill = if is_fusion_start || is_fusion_mid {
            FUSION_FILL()
        } else {
            fill
        };
        let edit_color = Color32::from_rgba_unmultiplied(
            (BLUE().r() as f32 * pulse + fusion_fill.r() as f32 * (1.0 - pulse)) as u8,
            (BLUE().g() as f32 * pulse + fusion_fill.g() as f32 * (1.0 - pulse)) as u8,
            (BLUE().b() as f32 * pulse + fusion_fill.b() as f32 * (1.0 - pulse)) as u8,
            255,
        );
        return (edit_color, egui::Stroke::new(1.5, BLUE()));
    }

    (fill, egui::Stroke::new(1.0, border))
}

/// [286] A small "back to the start" mark: three quarters of a circle with an arrow head,
/// drawn with the painter (no font glyph to depend on).
fn draw_loop_reset_icon(painter: &egui::Painter, rect: egui::Rect, color: Color32, struck: bool) {
    let c = rect.center();
    let r = (rect.height().min(rect.width()) * 0.27).clamp(4.0, 7.5);
    let start = -0.35f32 * std::f32::consts::PI; // where the arrow head sits
    let sweep = 1.55f32 * std::f32::consts::PI;
    let points: Vec<egui::Pos2> = (0..=14)
        .map(|i| {
            let a = start - sweep * i as f32 / 14.0;
            egui::pos2(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(2.0, color)));
    // Arrow head at the start of the arc, pointing along the sweep's reverse.
    let tip = egui::pos2(c.x + r * start.cos(), c.y + r * start.sin());
    let tangent = egui::vec2(-start.sin(), start.cos());
    let normal = egui::vec2(start.cos(), start.sin());
    let a = tip + tangent * (r * 0.55);
    painter.add(egui::Shape::convex_polygon(
        vec![a, tip + normal * (r * 0.5), tip - normal * (r * 0.5)],
        color,
        egui::Stroke::NONE,
    ));
    // Struck out: restart switched off.
    if struck {
        let d = r + 2.0;
        painter.line_segment(
            [egui::pos2(c.x - d, c.y + d), egui::pos2(c.x + d, c.y - d)],
            egui::Stroke::new(2.0, color),
        );
    }
}

/// [302] Width of the level meter at the end of a lane.
const LANE_METER_W: f32 = 6.0;

/// [302] A lane's level meter: a thin vertical bar, -48 dBFS at the bottom to 0 at the top,
/// green, amber over -6 dBFS, red at full scale. It jumps to each new peak and falls back at
/// 24 dB/s (the audio thread keeps the block's max until the editor reads it).
fn draw_lane_meter(ui: &mut egui::Ui, sound_settings: &SoundSettingsState, slot: usize, height: f32) {
    const FLOOR_DB: f32 = -48.0;
    const FALL_DB_PER_S: f32 = 24.0;
    let id = egui::Id::new(("lane_meter", slot));
    let now = ui.ctx().input(|i| i.time);
    let (shown_db, last) = ui.ctx().data(|d| d.get_temp::<(f32, f64)>(id)).unwrap_or((FLOOR_DB, now));
    let fallen = (shown_db - FALL_DB_PER_S * (now - last) as f32).max(FLOOR_DB);
    let peak = sound_settings.take_lane_peak(slot);
    let peak_db = if peak > 0.0 { 20.0 * peak.log10() } else { FLOOR_DB };
    let level_db = peak_db.max(fallen);
    ui.ctx().data_mut(|d| d.insert_temp(id, (level_db, now)));
    if level_db > FLOOR_DB {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(LANE_METER_W, height.max(8.0)), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 1.5, Color32::from_rgb(16, 16, 20));
    let frac = ((level_db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
    if frac > 0.0 {
        let bar = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 1.0, rect.bottom() - 1.0 - (rect.height() - 2.0) * frac),
            egui::pos2(rect.right() - 1.0, rect.bottom() - 1.0),
        );
        let color = if level_db >= -0.1 {
            RED()
        } else if level_db > -6.0 {
            AMBER()
        } else {
            GREEN()
        };
        painter.rect_filled(bar, 1.0, color);
    }
}

fn draw_mini_value_slider(
    ui: &mut egui::Ui,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    width: f32,
    fill: Color32,
    tooltip: &str,
) -> egui::Response {
    let style = slider::TrackStyle {
        fill,
        ..slider::TrackStyle::mini()
    };
    let response = slider::draw_track(ui, value, min, max, default, false, width, style);

    if tooltip.is_empty() {
        response
    } else {
        response.on_hover_text(tooltip)
    }
}

fn draw_param_mini_slider_with_value(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &FloatParam,
    min: f32,
    max: f32,
    width: f32,
    fill: Color32,
    tooltip: &str,
    format: impl Fn(f32) -> String,
) -> egui::Response {
    let mut value = param.value();
    let response = draw_mini_value_slider(
        ui,
        &mut value,
        min,
        max,
        param.default_plain_value(),
        width,
        fill,
        "",
    );
    if response.changed() {
        setter.set_parameter(param, value.clamp(min, max));
    }
    if response.hovered() || response.dragged() {
        draw_mini_slider_value_tooltip(ui, &response, &format!("{}: {}", tooltip, format(value)));
    }
    response
}

fn draw_mini_slider_value_tooltip(ui: &egui::Ui, response: &egui::Response, text: &str) {
    let mut pos = response.rect.center_top() + egui::vec2(0.0, -7.0);
    if let Some(to_global) = ui.ctx().layer_transform_to_global(ui.layer_id()) {
        pos = to_global * pos;
    }
    egui::Area::new(response.id.with("mini_slider_value_tooltip"))
        .kind(egui::UiKind::Tooltip)
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            egui::Frame::NONE
                .fill(P_ACTIVE())
                .stroke(egui::Stroke::new(1.0, LINE2()))
                .corner_radius(RADIUS_CTL)
                .inner_margin(egui::Margin {
                    left: 8,
                    right: 8,
                    top: 5,
                    bottom: 5,
                })
                .show(ui, |ui| {
                    ui.label(RichText::new(text).font(f_mono_med(10.5)).color(INK()));
                });
        });
}

fn toggle_step_for_ui(pattern_for_ui: &SharedPattern, step: usize, instrument: usize) {
    let current_mask = pattern_for_ui.load_step_mask(step);
    let bit = 1u16 << instrument;
    let next_mask = current_mask ^ bit;
    pattern_for_ui.set_step_mask(step, next_mask);
}

fn set_step_active_for_ui(
    pattern_for_ui: &SharedPattern,
    step: usize,
    instrument: usize,
    active: bool,
) {
    let current_mask = pattern_for_ui.load_step_mask(step);
    let bit = 1u16 << instrument;
    let next_mask = if active {
        current_mask | bit
    } else {
        current_mask & !bit
    };
    pattern_for_ui.set_step_mask(step, next_mask);
}

pub fn preserve_step_active_from_plock_popup(
    pattern_for_ui: &SharedPattern,
    state: &mut EditorUIState,
    instrument: usize,
    step: usize,
    step_was_active: bool,
) {
    if step_was_active && !pattern_for_ui.is_active(step, instrument) {
        set_step_active_for_ui(pattern_for_ui, step, instrument, true);
        state.mark_pattern_dirty();
    }
}

/// Fusions that would straddle the wrap if the grid shifted by `delta`:
/// after rotating, their start would land after their end, which the fusion
/// model cannot represent. The UI warns before breaking them — on confirm,
/// each is reduced to its first cell (a plain active step), which then
/// rotates normally.
fn straddling_fusions(
    pattern: &SharedPattern,
    len: usize,
    delta: isize,
) -> Vec<(usize, FusedGroup)> {
    let mut out = Vec::new();
    for slot in 0..crate::track::MAX_TRACKS {
        for group in pattern.load_fusions(slot) {
            let start = (group.start_cell as isize + delta).rem_euclid(len as isize);
            let end = (group.end_cell as isize + delta).rem_euclid(len as isize);
            if start > end {
                out.push((slot, group));
            }
        }
    }
    out
}

/// Grid-shift warning (skeuo plate, foreground): a fusion straddles the wrap
/// in the pending direction. On confirm it is reduced to its first cell (a
/// plain active step) and the grid shifts; on cancel nothing happens.
pub fn draw_shift_grid_warning_if_any(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    plock: &PlockState,
    state: &mut EditorUIState,
    master_length: usize,
) {
    let Some(delta) = state.shift_grid_confirm else {
        return;
    };
    let len = master_length.clamp(1, 64);
    let straddling = straddling_fusions(pattern, len, delta);
    if straddling.is_empty() {
        state.shift_grid_confirm = None;
        return;
    }

    let screen_rect = ui.ctx().screen_rect();
    let panel_w = 360.0;
    let pos = egui::pos2(
        screen_rect.center().x - panel_w * 0.5,
        screen_rect.center().y - 30.0,
    );
    egui::Area::new(ui.id().with("shift_grid_warning"))
        .kind(egui::UiKind::Popup)
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ui.ctx(), |ui| {
            let bg = ui.painter().add(egui::Shape::Noop);
            let resp = egui::Frame::NONE
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(panel_w);
                    ui.label(RichText::new("Shift grid").font(f_sans_sb(12.0)).color(RED()));
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "A fusion at the pattern edge will be broken and reduced to its first cell:",
                        )
                        .font(f_sans_med(10.5))
                        .color(INK2()),
                    );
                    ui.add_space(4.0);
                    for (slot, group) in &straddling {
                        ui.label(
                            RichText::new(format!(
                                "Lane {}: cells {}-{}",
                                slot + 1,
                                group.start_cell + 1,
                                group.end_cell + 1
                            ))
                            .font(f_mono_med(9.5))
                            .color(INK3()),
                        );
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        if crate::ui::controls::chip_button(
                            ui,
                            "Break & Shift",
                            true,
                            RED(),
                            egui::Sense::click(),
                        )
                        .clicked()
                        {
                            shift_grid(params, pattern, plock, state, master_length, delta);
                            state.shift_grid_confirm = None;
                        }
                        if crate::ui::controls::chip_button(
                            ui,
                            "Cancel",
                            false,
                            INK2(),
                            egui::Sense::click(),
                        )
                        .clicked()
                        {
                            state.shift_grid_confirm = None;
                        }
                    });
                });
            ui.painter().set(
                bg,
                crate::ui::skeuo::plate_shape(resp.response.rect, RADIUS_PANEL as f32),
            );
        });
}

/// Shift every lane's grid by one cell — steps, fusions, sound p-locks and
/// sequencer p-locks rotate together so nothing desyncs. Wraps inside the
/// pattern length (nothing is lost). `delta` is +1 (right) or -1 (left).
/// A fusion straddling the wrap is broken: its first cell stays as a plain
/// active step. Callers warn first via `straddling_fusions` (the ‹ › arrows
/// ask before breaking). Saved bank slots are frozen snapshots and stay
/// untouched.
fn shift_grid(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    plock: &PlockState,
    state: &mut EditorUIState,
    master_length: usize,
    delta: isize,
) {
    let len = master_length.clamp(1, 64);
    let wrap = |cell: isize| -> usize { cell.rem_euclid(len as isize) as usize };

    // 1) Step masks — the u16 packs every lane, so all lanes shift at once.
    let masks = pattern.step_masks();
    for step in 0..len {
        pattern.set_step_mask(step, masks[wrap(step as isize - delta)]);
    }

    // 2) Fusions: shift both endpoints; a group that would straddle the wrap
    // is dropped (start must stay <= end). A linked lane's own morphs ([283])
    // shift with the groups they belong to.
    let shift = |groups: Vec<FusedGroup>| -> Vec<FusedGroup> {
        let mut shifted = Vec::with_capacity(groups.len());
        for group in groups {
            let start = wrap(group.start_cell as isize + delta) as u8;
            let end = wrap(group.end_cell as isize + delta) as u8;
            if start <= end {
                let mut group = group;
                group.start_cell = start;
                group.end_cell = end;
                shifted.push(group);
            }
        }
        shifted
    };
    for slot in 0..crate::track::MAX_TRACKS {
        pattern.store_fusions_and_morphs(
            slot,
            &shift(pattern.load_fusions(slot)),
            &shift(pattern.load_linked_morphs(slot)),
        );
    }

    // 3) Sound p-locks: values + activity masks + field masks rotate per step
    // (link/snapshot mode preserved, like move_step_with_plocks).
    for slot in 0..crate::track::MAX_TRACKS {
        let active: [bool; 64] = std::array::from_fn(|s| plock.masks.is_active(slot, s));
        let field_masks: [u64; 64] = std::array::from_fn(|s| plock.field_masks.get(slot, s));
        let values: [[f32; crate::plock::FIELD_COUNT]; 64] =
            std::array::from_fn(|s| std::array::from_fn(|f| plock.values.get(slot, s, f)));
        for step in 0..len {
            let src = wrap(step as isize - delta);
            if active[src] {
                plock.masks.set_active(slot, step, true);
                plock.field_masks.set_raw(slot, step, field_masks[src]);
                for field in 0..crate::plock::FIELD_COUNT {
                    plock.values.set(slot, step, field, values[src][field]);
                }
            } else {
                plock.clear(slot, step);
            }
        }
    }

    // 4) Sequencer p-locks (probability, stutter, conditions, microtiming,
    // solo) rotate the same way.
    let seq = &params.seq_plock_state.state;
    for slot in 0..crate::track::MAX_TRACKS {
        let cell_params: [Option<crate::plock::SequencerStepParams>; 64] =
            std::array::from_fn(|s| seq.get(slot, s));
        for step in 0..len {
            let src = wrap(step as isize - delta);
            match &cell_params[src] {
                Some(cell_params) => seq.set(slot, step, cell_params),
                None => seq.clear(slot, step),
            }
        }
    }

    state.mark_pattern_dirty();
}

/// Move a single step (with its sound and sequencer plocks) from `source` to
/// `target` within the same slot. Fused cells are left untouched.
/// `grid` is the lane whose steps `slot` shows (itself, or the lane above for
/// a linked lane, [283]); the p-locks are `slot`'s own.
fn move_step_with_plocks(
    pattern: &SharedPattern,
    plock: &PlockState,
    params: &DrumFlashParams,
    slot: usize,
    grid: usize,
    source: usize,
    target: usize,
) {
    if source == target {
        return;
    }

    // Copy step state.
    let active = pattern.is_active(source, grid);
    set_step_active_for_ui(pattern, target, grid, active);
    set_step_active_for_ui(pattern, source, grid, false);

    // Copy sound plock field-by-field so link/snapshot mode is preserved.
    if plock.masks.is_active(slot, source) {
        let field_mask = plock.field_masks.get(slot, source);
        plock.masks.set_active(slot, target, true);
        plock.field_masks.set_raw(slot, target, field_mask);
        for field in 0..crate::plock::FIELD_COUNT {
            plock
                .values
                .set(slot, target, field, plock.values.get(slot, source, field));
        }
        plock.clear(slot, source);
    } else {
        plock.clear(slot, target);
    }

    // Copy sequencer plock.
    let seq = &params.seq_plock_state.state;
    if let Some(seq_params) = seq.get(slot, source) {
        seq.set(slot, target, &seq_params);
    }
    seq.clear(slot, source);
}

fn fusion_containing(fusions: &[FusedGroup], step: usize) -> Option<(usize, FusedGroup)> {
    fusions
        .iter()
        .copied()
        .enumerate()
        .find(|(_, group)| group.contains(step))
}

fn reset_stutter_on_fusion(
    params: &DrumFlashParams,
    instrument: usize,
    start_cell: usize,
    end_cell: usize,
) {
    let seq_plock = &params.seq_plock_state.state;
    for step in start_cell..=end_cell {
        if let Some(mut seq_params) = seq_plock.get(instrument, step) {
            if seq_params.stutter_count != 1 {
                seq_params.stutter_count = 1;
                seq_plock.set(instrument, step, &seq_params);
            }
        }
    }
}

fn clear_covered_plocks_for_fusion(
    plock: &PlockState,
    params: &DrumFlashParams,
    instrument: usize,
    start_cell: usize,
    end_cell: usize,
) {
    // The fusion start cell is the only source for sound/seq plocks. Plocks on
    // covered cells would be invisible and inactive while the fusion exists.
    for step in (start_cell + 1)..=end_cell {
        plock.clear(instrument, step);
        params.seq_plock_state.state.clear(instrument, step);
    }
}

fn normalize_fusion_cells_for_ui(
    pattern_for_ui: &SharedPattern,
    instrument: usize,
    start_cell: usize,
    end_cell: usize,
) {
    for step in start_cell..=end_cell {
        set_step_active_for_ui(pattern_for_ui, step, instrument, false);
    }
    set_step_active_for_ui(pattern_for_ui, start_cell, instrument, true);
}

fn toggle_fusion_for_ui(pattern_for_ui: &SharedPattern, group: FusedGroup, instrument: usize) {
    let start = group.start_cell as usize;
    let end = group.end_cell as usize;
    let next_active = !pattern_for_ui.is_active(start, instrument);
    for step in start..=end {
        set_step_active_for_ui(pattern_for_ui, step, instrument, false);
    }
    set_step_active_for_ui(pattern_for_ui, start, instrument, next_active);
}

fn edited_fusion_for_ui(
    pattern_for_ui: &SharedPattern,
    state: &EditorUIState,
) -> Option<(usize, usize, FusedGroup)> {
    let (instrument, index) = state.fusion_editing?;
    pattern_for_ui
        .load_fusions(instrument)
        .get(index)
        .copied()
        .map(|group| (instrument, index, group))
}

fn finish_fusion_editing_for_ui(pattern_for_ui: &SharedPattern, state: &mut EditorUIState) {
    if let Some((instrument, _, group)) = edited_fusion_for_ui(pattern_for_ui, state) {
        set_step_active_for_ui(pattern_for_ui, group.start_cell as usize, instrument, true);
    }
    state.mark_pattern_dirty();
    state.fusion_editing = None;
}

fn fusion_text_w(ui: &egui::Ui, s: &str, font: egui::FontId) -> f32 {
    ui.fonts(|f| {
        f.layout_no_wrap(s.to_string(), font, Color32::WHITE)
            .size()
            .x
    })
}

/// Raised sub-panel background for the fusion strip (relief + border + top liseré).
fn fusion_strip_bg(ui: &egui::Ui, rect: egui::Rect) {
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 6.0, rgb(38, 39, 44));
    grad3(
        &p,
        rect.shrink(2.5),
        rgb(45, 46, 51),
        rgb(40, 41, 46),
        rgb(37, 38, 43),
        0.55,
    );
    p.rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, rgb(20, 20, 24)),
        egui::StrokeKind::Inside,
    );
    plateau_line(
        &p,
        egui::Rect::from_min_size(
            rect.min + Vec2::new(10.0, 1.5),
            Vec2::new((rect.width() - 20.0).max(0.0), 1.0),
        ),
        (255, 255, 255),
        16,
    );
}

/// Compact keycap button for the fusion strip (h19, so it never touches the strip
/// edges). Returns the click response.
fn fusion_key(
    ui: &mut egui::Ui,
    w: f32,
    label: &str,
    state: KeycapState,
    tc: Color32,
    sense: egui::Sense,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 19.0), sense);
    crate::ui::skeuo::keycap(ui, rect, state);
    if resp.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, 5.0, Color32::from_black_alpha(60));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        f_sans_sb(11.0),
        tc,
    );
    resp
}

fn draw_fusion_edit_box(
    ui: &mut egui::Ui,
    pattern_for_ui: &SharedPattern,
    params: &DrumFlashParams,
    _sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    fusion_mode_active: bool,
) -> egui::Rect {
    let box_size = Vec2::new(380.0, 28.0);
    // Fixed outer size so the parent row never grows between idle/edit states.
    let (rect, response) = ui.allocate_exact_size(box_size, egui::Sense::hover());
    fusion_strip_bg(ui, rect);

    let font = f_sans_med(11.0);
    let g = 8.0_f32;

    if let Some((instrument, index, group)) = edited_fusion_for_ui(pattern_for_ui, state) {
        let range = format!("Fusion {}–{}", group.start_cell + 1, group.end_cell + 1);
        // [283] The morph of the lane the box was opened from: a linked lane
        // shares the group but keeps its own morph.
        let lane = if params
            .track_layout
            .state
            .grid_slot(state.fusion_editing_lane)
            == instrument
        {
            state.fusion_editing_lane
        } else {
            instrument
        };
        let morph_group = pattern_for_ui
            .load_lane_fusions(lane, instrument)
            .get(index)
            .copied()
            .unwrap_or(group);
        let morph = if morph_group.morph_count > 0 {
            let morphable =
                crate::instrument_registry::morphable_fields(schema_voice_idx(params, lane));
            let names: Vec<&str> = morph_group.morph_targets[..morph_group.morph_count as usize]
                .iter()
                .map(|t| {
                    morphable
                        .iter()
                        .find(|f| f.field_index == t.field as usize)
                        .map(|f| f.label)
                        .unwrap_or("?")
                })
                .collect();
            format!("Morph: {}", names.join(", "))
        } else {
            "Morph: Off".to_string()
        };

        let (fw, delw, xw) = (40.0_f32, 44.0_f32, 28.0_f32);
        let content = fusion_text_w(ui, &range, font.clone())
            + g
            + fusion_text_w(ui, "Steps", font.clone())
            + g
            + fw
            + g
            + fusion_text_w(ui, &morph, font.clone())
            + g
            + delw
            + g
            + xw;
        let lead = ((box_size.x - content) * 0.5).max(6.0);

        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(lead);
                ui.label(
                    RichText::new(range.as_str())
                        .font(font.clone())
                        .color(Color32::WHITE),
                );
                ui.add_space(g);
                ui.label(RichText::new("Steps").font(font.clone()).color(INK3()));
                ui.add_space(g);
                let step_drag = egui::DragValue::new(&mut state.fusion_edit_steps)
                    .range(1..=64)
                    .speed(1.0)
                    .fixed_decimals(0);
                let step_response = ui.add_sized(Vec2::new(fw, 18.0), step_drag);
                if state.fusion_edit_focus_request {
                    state.fusion_edit_focus_request = false;
                    step_response.request_focus();
                }
                if step_response.lost_focus() {
                    let mut new_fusions = pattern_for_ui.load_fusions(instrument);
                    if let Some(group) = new_fusions.get_mut(index) {
                        group.step_count = state.fusion_edit_steps;
                        pattern_for_ui.store_fusions(instrument, &new_fusions);
                    }
                    finish_fusion_editing_for_ui(pattern_for_ui, state);
                }
                ui.add_space(g);
                ui.label(
                    RichText::new(morph.as_str())
                        .font(font.clone())
                        .color(INK2()),
                );
                ui.add_space(g);
                if fusion_key(
                    ui,
                    delw,
                    "Del",
                    KeycapState::Rest,
                    rgb(230, 120, 110),
                    egui::Sense::click(),
                )
                .clicked()
                {
                    let mut new_fusions = pattern_for_ui.load_fusions(instrument);
                    if index < new_fusions.len() {
                        new_fusions.remove(index);
                        pattern_for_ui.store_fusions(instrument, &new_fusions);
                    }
                    state.fusion_editing = None;
                    state.mark_pattern_dirty();
                    // Reactive editor: force a redraw so the cell (which was drawn
                    // earlier this frame with the now-deleted fusion) refreshes.
                    ui.ctx().request_repaint();
                }
                ui.add_space(g);
                if fusion_key(ui, xw, "×", KeycapState::Rest, INK(), egui::Sense::click()).clicked()
                {
                    finish_fusion_editing_for_ui(pattern_for_ui, state);
                }
            },
        );
    } else {
        if state.fusion_editing.is_some() {
            state.fusion_editing = None;
        }
        let mono = f_mono_med(10.5);
        // [283] English like the rest of the plugin, and the real gesture:
        // Shift + click on two cells (it used to say "drag", in French).
        let hint = if fusion_mode_active {
            "click 2 cells, same page"
        } else {
            "+ click 2 cells to fuse"
        };
        let majw = 46.0_f32;
        let content = fusion_text_w(ui, "FUSION", mono.clone())
            + g
            + majw
            + g
            + fusion_text_w(ui, hint, font.clone());
        let lead = ((box_size.x - content) * 0.5).max(6.0);
        let hint_color = if fusion_mode_active { BLUE() } else { INK3() };

        ui.allocate_new_ui(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(lead);
                ui.label(RichText::new("FUSION").font(mono.clone()).color(INK3()));
                ui.add_space(g);
                fusion_key(
                    ui,
                    majw,
                    "Shift",
                    KeycapState::PressedAmber,
                    rgb(255, 240, 214),
                    egui::Sense::hover(),
                );
                ui.add_space(g);
                ui.label(RichText::new(hint).font(font.clone()).color(hint_color));
            },
        );
    }

    response.rect
}

fn close_fusion_editing_on_outside_click(
    ui: &egui::Ui,
    pattern_for_ui: &SharedPattern,
    state: &mut EditorUIState,
    inline_rect: Option<egui::Rect>,
    edit_box_rect: Option<egui::Rect>,
) {
    if state.fusion_editing.is_none() {
        return;
    }

    let clicked_outside = ui.input(|input| {
        if !input.pointer.any_pressed() {
            return false;
        }
        let Some(pos) = input.pointer.interact_pos() else {
            return false;
        };

        let inside_inline = inline_rect.map(|rect| rect.contains(pos)).unwrap_or(false);
        let inside_box = edit_box_rect
            .map(|rect| rect.contains(pos))
            .unwrap_or(false);

        !inside_inline && !inside_box
    });

    if clicked_outside {
        finish_fusion_editing_for_ui(pattern_for_ui, state);
    }
}

fn fusion_inside_range(group: FusedGroup, start: usize, end: usize) -> bool {
    (group.start_cell as usize) >= start && (group.end_cell as usize) < end
}

fn fusion_overlaps_range(group: FusedGroup, start: usize, end: usize) -> bool {
    (group.start_cell as usize) < end && (group.end_cell as usize) >= start
}

fn normalize_existing_fusion_cells_for_ui(
    pattern_for_ui: &SharedPattern,
    instrument: usize,
    start_cell: usize,
    end_cell: usize,
) {
    let was_active = pattern_for_ui.is_active(start_cell, instrument);
    for step in start_cell..=end_cell {
        set_step_active_for_ui(pattern_for_ui, step, instrument, false);
    }
    set_step_active_for_ui(pattern_for_ui, start_cell, instrument, was_active);
}

fn clear_page_fusions_for_ui(pattern_for_ui: &SharedPattern, page: usize) {
    let page_start = page * 16;
    let page_end = (page_start + 16).min(crate::sequencer::pattern::STEP_COUNT);
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        let fusions = pattern_for_ui.load_fusions(inst);
        let retained: Vec<_> = fusions
            .into_iter()
            .filter(|group| !fusion_inside_range(*group, page_start, page_end))
            .collect();
        pattern_for_ui.store_fusions(inst, &retained);
    }
}

pub fn copy_page_to_clipboard(
    pattern_for_ui: &SharedPattern,
    plock: &PlockState,
    params: &DrumFlashParams,
    page: usize,
) -> PageClipboard {
    let page_start = page * 16;
    let page_end = (page_start + 16).min(crate::sequencer::pattern::STEP_COUNT);
    let mut triggers = [0u16; 16];
    for (i, step) in (page_start..page_end).enumerate() {
        triggers[i] = pattern_for_ui.load_step_mask(step);
    }

    let mut plocks = Vec::new();
    let mut seq_plocks = Vec::new();
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        for (i, step) in (page_start..page_end).enumerate() {
            if plock.masks.is_active(inst, step) {
                let field_mask = plock.field_masks.get_raw(inst, step);
                let mut values = vec![0.0f32; crate::plock::FIELD_COUNT];
                for field in 0..crate::plock::FIELD_COUNT {
                    values[field] = plock.values.get(inst, step, field);
                }
                plocks.push(PlockClipboardEntry {
                    instrument: inst,
                    step: i,
                    field_mask,
                    values,
                });
            }
            // [231] Sequencer p-locks travel with the page too.
            if let Some(snapshot) = params.seq_plock_state.state.snapshot(inst, step) {
                seq_plocks.push(crate::ui::editor_state::SeqPlockClipboardEntry {
                    instrument: inst,
                    step: i,
                    snapshot,
                });
            }
        }
    }

    let mut fusions = Vec::new();
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        for group in pattern_for_ui.load_fusions(inst) {
            let group_start = group.start_cell as usize;
            let group_end = group.end_cell as usize;
            if group_start >= page_start && group_end < page_end {
                fusions.push(FusionClipboardEntry {
                    instrument: inst,
                    start_step: group_start - page_start,
                    end_step: group_end - page_start,
                    step_count: group.step_count,
                });
            }
        }
    }

    PageClipboard {
        triggers,
        plocks,
        fusions,
        seq_plocks,
    }
}

pub fn paste_page_from_clipboard(
    pattern_for_ui: &SharedPattern,
    plock: &PlockState,
    params: &DrumFlashParams,
    page: usize,
    clipboard: &PageClipboard,
) {
    let page_start = page * 16;
    let page_end = (page_start + 16).min(crate::sequencer::pattern::STEP_COUNT);

    // Triggers
    for (i, step) in (page_start..page_end).enumerate() {
        pattern_for_ui.set_step_mask(step, clipboard.triggers[i]);
    }

    // Clear existing plocks on the page
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        for step in page_start..page_end {
            plock.clear(inst, step);
            params.seq_plock_state.state.clear(inst, step);
        }
    }

    // Sound plocks
    for entry in &clipboard.plocks {
        let step = page_start + entry.step;
        if step >= page_end {
            continue;
        }
        plock.masks.set_active(entry.instrument, step, true);
        plock
            .field_masks
            .set_raw(entry.instrument, step, entry.field_mask);
        for (field, &value) in entry.values.iter().enumerate() {
            plock.values.set(entry.instrument, step, field, value);
        }
    }

    // [231] Sequencer p-locks (the page's were cleared above).
    for entry in &clipboard.seq_plocks {
        let step = page_start + entry.step;
        if step >= page_end {
            continue;
        }
        params
            .seq_plock_state
            .state
            .restore(entry.instrument, step, &entry.snapshot);
    }

    // Fusions
    replace_page_fusions_for_ui(pattern_for_ui, params, plock, page, &clipboard.fusions);
}

pub fn clear_page_for_ui(
    pattern_for_ui: &SharedPattern,
    plock: &PlockState,
    params: &DrumFlashParams,
    page: usize,
) {
    let page_start = page * 16;
    let page_end = (page_start + 16).min(crate::sequencer::pattern::STEP_COUNT);

    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        for step in page_start..page_end {
            set_step_active_for_ui(pattern_for_ui, step, inst, false);
            plock.clear(inst, step);
            params.seq_plock_state.state.clear(inst, step);
        }
    }

    clear_page_fusions_for_ui(pattern_for_ui, page);
}

fn replace_page_fusions_for_ui(
    pattern_for_ui: &SharedPattern,
    params: &DrumFlashParams,
    plock: &PlockState,
    page: usize,
    entries: &[FusionClipboardEntry],
) {
    let page_start = page * 16;
    let page_end = (page_start + 16).min(crate::sequencer::pattern::STEP_COUNT);

    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        let mut new_fusions: Vec<_> = pattern_for_ui
            .load_fusions(inst)
            .into_iter()
            .filter(|group| !fusion_inside_range(*group, page_start, page_end))
            .collect();

        for entry in entries.iter().filter(|entry| entry.instrument == inst) {
            if entry.start_step >= entry.end_step || entry.end_step >= 16 {
                continue;
            }

            let start_cell = page_start + entry.start_step;
            let end_cell = page_start + entry.end_step;
            let group = FusedGroup {
                start_cell: start_cell as u8,
                end_cell: end_cell as u8,
                step_count: entry.step_count,
                ..Default::default()
            };
            if !group.is_valid() {
                continue;
            }

            normalize_existing_fusion_cells_for_ui(pattern_for_ui, inst, start_cell, end_cell);
            reset_stutter_on_fusion(params, inst, start_cell, end_cell);
            clear_covered_plocks_for_fusion(plock, params, inst, start_cell, end_cell);
            new_fusions.push(group);
        }

        new_fusions.sort_by_key(|group| group.start_cell);
        pattern_for_ui.store_fusions(inst, &new_fusions);
    }
}

fn duplicate_fusions_for_x2(
    pattern_for_ui: &SharedPattern,
    params: &DrumFlashParams,
    plock: &PlockState,
    current_len: usize,
) {
    let source_start = 0;
    let source_end = current_len.min(crate::sequencer::pattern::STEP_COUNT);
    let destination_start = current_len;
    let destination_end = (current_len * 2).min(crate::sequencer::pattern::STEP_COUNT);
    if source_end == 0 || destination_start >= destination_end {
        return;
    }

    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        let existing = pattern_for_ui.load_fusions(inst);
        let mut new_fusions: Vec<_> = existing
            .iter()
            .copied()
            .filter(|group| !fusion_overlaps_range(*group, destination_start, destination_end))
            .collect();

        for group in existing
            .iter()
            .copied()
            .filter(|group| fusion_inside_range(*group, source_start, source_end))
        {
            let start_cell = group.start_cell as usize + current_len;
            let end_cell = group.end_cell as usize + current_len;
            if end_cell >= destination_end {
                continue;
            }

            let shifted = FusedGroup {
                start_cell: start_cell as u8,
                end_cell: end_cell as u8,
                step_count: group.step_count,
                ..Default::default()
            };
            if !shifted.is_valid() {
                continue;
            }

            normalize_existing_fusion_cells_for_ui(pattern_for_ui, inst, start_cell, end_cell);
            reset_stutter_on_fusion(params, inst, start_cell, end_cell);
            clear_covered_plocks_for_fusion(plock, params, inst, start_cell, end_cell);
            new_fusions.push(shifted);
        }

        new_fusions.sort_by_key(|group| group.start_cell);
        pattern_for_ui.store_fusions(inst, &new_fusions);
    }
}

fn handle_fusion_shift_click(
    pattern_for_ui: &SharedPattern,
    params: &DrumFlashParams,
    plock: &PlockState,
    instrument: usize,
    clicked_step: usize,
    master_length: usize,
    fusions: &[FusedGroup],
    selection_start: &mut Option<usize>,
) {
    if let Some(start) = *selection_start {
        let (start_cell, end_cell) = if start < clicked_step {
            (start, clicked_step)
        } else {
            (clicked_step, start)
        };
        let span = end_cell - start_cell + 1;
        let same_page = start_cell / 16 == end_cell / 16;

        if span >= 2 && same_page && end_cell < master_length {
            normalize_fusion_cells_for_ui(pattern_for_ui, instrument, start_cell, end_cell);
            reset_stutter_on_fusion(params, instrument, start_cell, end_cell);
            clear_covered_plocks_for_fusion(plock, params, instrument, start_cell, end_cell);

            let mut new_fusions: Vec<_> = fusions
                .iter()
                .copied()
                .filter(|g| g.end_cell < start_cell as u8 || g.start_cell > end_cell as u8)
                .collect();
            new_fusions.push(FusedGroup {
                start_cell: start_cell as u8,
                end_cell: end_cell as u8,
                step_count: span.min(64) as u8,
                ..Default::default()
            });
            new_fusions.sort_by_key(|g| g.start_cell);
            pattern_for_ui.store_fusions(instrument, &new_fusions);
        }
        *selection_start = None;
    } else if clicked_step < master_length {
        *selection_start = Some(clicked_step);
    }
}

/// Clear all fusions for all instruments (used by Clear, presets, generator).
pub fn clear_all_fusions(pattern: &SharedPattern) {
    for inst in 0..crate::sequencer::pattern::INSTRUMENT_COUNT {
        pattern.store_fusions(inst, &[]);
    }
}

struct MixerRow<'a> {
    mute: &'a BoolParam,
    solo: &'a BoolParam,
}

fn mixer_rows(params: &DrumFlashParams) -> [MixerRow<'_>; crate::track::MAX_TRACKS] {
    std::array::from_fn(|i| MixerRow {
        mute: params.mutes()[i],
        solo: params.solos()[i],
    })
}

// ---------------------------------------------------------------------------------------------------------------
// Plock context menu
// ---------------------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::sequencer::FusedGroup;
    use crate::track::{TrackInstrumentKind, TrackLayoutState, TrackSlot, MAX_TRACKS};

    /// The column headings must sit over their columns: the header row takes
    /// exactly the widths the lane rows use (grip, name, Vol, M / S, 16 cells,
    /// Push / Len). The ‹ › arrows once shrank the name column to their
    /// own width and pulled every heading after it ~32 px left.
    #[test]
    fn grid_header_keeps_every_column_at_its_width() {
        use super::*;
        let params = crate::DrumFlashParams::default();
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let mut state = crate::ui::editor_state::EditorUIState::default();
        let ctx = egui::Context::default();
        crate::ui::install_egui_fonts(&ctx);
        let (grip_w, name_w, vol_w, extra_w, gap, cell_w) = (14.0, 88.0, 81.0, 44.0, 7.0, 30.0);
        let mst_w = STEP_H * 2.0 + GAP_TIGHT;
        let mut width = 0.0;
        // A few passes: egui settles fonts and sizes over the first frames.
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1400.0, 400.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    // A horizontal hugs its content: its rect IS the header's.
                    let header = ui.horizontal(|ui| {
                        draw_seq_header_v2(
                            ui,
                            &params,
                            &pattern,
                            plock,
                            &mut state,
                            0,
                            usize::MAX,
                            16,
                            grip_w,
                            name_w,
                            vol_w,
                            mst_w,
                            extra_w,
                            gap,
                            cell_w,
                        );
                    });
                    width = header.response.rect.width();
                });
            });
        }
        let expected = grip_w
            + name_w
            + vol_w
            + mst_w
            + 16.0 * cell_w
            + 15.0 * GAP_TIGHT
            + 2.0 * extra_w
            + 6.0 * gap;
        assert!(
            (width - expected).abs() < 0.5,
            "header {width} px wide, its columns {expected} px"
        );
    }

    #[test]
    fn shift_grid_rotates_steps_plocks_and_fusions_with_wrap() {
        use nih_plug::params::persist::PersistentField;
        let params = crate::DrumFlashParams::default();
        PersistentField::<TrackLayoutState>::set(
            &params.track_layout,
            TrackLayoutState::modular_default_layout(),
        );
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let mut state = crate::ui::editor_state::EditorUIState::default();
        let master_length = 16;

        // Start from an empty grid (the default params carry the rock pattern).
        pattern.load_step_masks(&[0u16; 64]);

        // Steps on lane 0 at 0 and 15; a plock at 0; a seq plock at 15.
        pattern.set_step_mask(0, 0b0001);
        pattern.set_step_mask(15, 0b0101);
        plock.set_field(0, 0, 2, 0.42);
        params.seq_plock_state.state.set_probability(0, 15, 0.25);
        pattern.store_fusions(
            0,
            &[FusedGroup {
                start_cell: 4,
                end_cell: 6,
                step_count: 3,
                ..Default::default()
            }],
        );

        // Right by one: 0→1, 15→0 (wrap), fusion 4-6 → 5-7.
        super::shift_grid(&params, &pattern, plock, &mut state, master_length, 1);
        assert_eq!(pattern.step_masks()[1], 0b0001);
        assert_eq!(pattern.step_masks()[0], 0b0101, "step 15 wrapped to 0");
        assert_eq!(
            pattern.step_masks()[15],
            0,
            "step 14 moved to 15? no: only 0 and 15 were on"
        );
        assert_eq!(plock.values.get(0, 1, 2), 0.42, "plock followed its step");
        assert!(!plock.masks.is_active(0, 0));
        let prob = f32::from_bits(
            params.seq_plock_state.state.probabilities[0][0]
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        assert_eq!(prob, 0.25, "seq plock wrapped with its step");
        let fusions = pattern.load_fusions(0);
        assert_eq!(fusions.len(), 1);
        assert_eq!((fusions[0].start_cell, fusions[0].end_cell), (5, 7));

        // Left by one restores the original layout.
        super::shift_grid(&params, &pattern, plock, &mut state, master_length, -1);
        assert_eq!(pattern.step_masks()[0], 0b0001);
        assert_eq!(pattern.step_masks()[15], 0b0101);
        assert_eq!(plock.values.get(0, 0, 2), 0.42);
        let fusions = pattern.load_fusions(0);
        assert_eq!((fusions[0].start_cell, fusions[0].end_cell), (4, 6));
    }

    #[test]
    fn straddling_fusions_detects_only_the_edge_groups_in_the_push_direction() {
        let params = crate::DrumFlashParams::default();
        let pattern = params.pattern_state.shared();
        pattern.store_fusions(
            0,
            &[
                FusedGroup {
                    start_cell: 4,
                    end_cell: 6,
                    step_count: 3,
                    ..Default::default()
                },
                FusedGroup {
                    start_cell: 14,
                    end_cell: 15,
                    step_count: 2,
                    ..Default::default()
                },
            ],
        );
        // Pushing right: only the 14-15 group straddles.
        let straddling = super::straddling_fusions(&pattern, 16, 1);
        assert_eq!(straddling.len(), 1);
        assert_eq!(
            (straddling[0].1.start_cell, straddling[0].1.end_cell),
            (14, 15)
        );
        // Pushing left: nothing straddles (14-15 → 13-14, 4-6 → 3-5, both fine).
        assert!(super::straddling_fusions(&pattern, 16, -1).is_empty());
        // A group at the start straddles only when pushing left.
        pattern.store_fusions(
            1,
            &[FusedGroup {
                start_cell: 0,
                end_cell: 1,
                step_count: 2,
                ..Default::default()
            }],
        );
        let left = super::straddling_fusions(&pattern, 16, -1);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, 1);
    }

    #[test]
    fn shift_grid_drops_a_fusion_straddling_the_wrap() {
        let params = crate::DrumFlashParams::default();
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let mut state = crate::ui::editor_state::EditorUIState::default();
        // Fusion 14-15 shifted right by one would become 15-0 (start > end):
        // unrepresentable, so it is dropped.
        pattern.store_fusions(
            0,
            &[FusedGroup {
                start_cell: 14,
                end_cell: 15,
                step_count: 2,
                ..Default::default()
            }],
        );
        super::shift_grid(&params, &pattern, plock, &mut state, 16, 1);
        assert!(
            pattern.load_fusions(0).is_empty(),
            "a fusion straddling the wrap must be dropped"
        );
    }

    #[test]
    fn wav_drop_activates_the_empty_slot_loads_its_file_and_preserves_other_lanes() {
        use nih_plug::params::persist::PersistentField;
        let params = crate::DrumFlashParams::default();
        let layout = TrackLayoutState::modular_default_layout();
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let settings = &params.sound_settings.state;
        let mut state = crate::ui::editor_state::EditorUIState::default();
        let slot = 5;
        // A previously deleted lane can still have notes and locks in storage.
        pattern.set_step_mask(0, 1 | (1 << slot));
        plock.set_field(slot, 0, 3, 200.0);
        let wav = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ch606.wav");
        assert!(super::create_oneshot_from_drop(
            &params, settings, &mut state, &pattern, plock, slot, &wav
        ));
        assert_eq!(
            params.track_layout.state.kind_for_slot(slot),
            Some(TrackInstrumentKind::OneShot)
        );
        assert_eq!(state.selected_track_slot, slot);
        assert!(params.user_textures.pool.is_loaded(slot));
        assert_eq!(
            params.user_textures.path(slot).as_deref(),
            Some(wav.as_path())
        );
        assert_eq!(
            pattern.step_masks()[0],
            1,
            "new lane is blank, existing lane is kept"
        );
        assert!(!plock.masks.is_active(slot, 0));
        assert_eq!(
            params.track_layout.state.kind_for_slot(0),
            Some(TrackInstrumentKind::Kick)
        );
        assert!(!super::create_oneshot_from_drop(
            &params, settings, &mut state, &pattern, plock, 0, &wav
        ));
        assert!(
            params.user_textures.path(0).is_none(),
            "occupied lane must not receive the file"
        );
        // [270] Only a One-Shot lane takes a replacement file.
        assert!(!super::replace_oneshot_file(&params, settings, 0, &wav));
        assert!(params.user_textures.path(0).is_none());
    }

    /// [285] A new lane starts with its instrument type's Randomize density (Settings),
    /// then keeps its own; a lane with none stored follows its type's default.
    #[test]
    fn a_new_lane_takes_its_type_randomize_density() {
        use crate::ui::editor_state::EditorUIState;
        use nih_plug::params::persist::PersistentField;
        let params = crate::DrumFlashParams::default();
        let layout = TrackLayoutState::modular_default_layout();
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let settings = &params.sound_settings.state;
        let mut state = EditorUIState::default();
        state.global_config.set_randomize_density_for(TrackInstrumentKind::HiHat, 0.3);
        let slot = 5;
        assert!(!params.track_layout.state.is_active(slot));
        super::activate_slot(
            &params, settings, &mut state, slot, TrackInstrumentKind::HiHat, &pattern, plock,
        );
        assert_eq!(state.lane_randomize_density(&params, slot), 0.3);
        // A later Settings change leaves the existing lane alone.
        state.global_config.set_randomize_density_for(TrackInstrumentKind::HiHat, 0.9);
        assert_eq!(state.lane_randomize_density(&params, slot), 0.3);
        EditorUIState::set_lane_randomize_density(&params, slot, 0.6);
        assert_eq!(state.lane_randomize_density(&params, slot), 0.6);
        // Slot 0 (the default layout's Kick) holds none: its type's default.
        assert_eq!(params.track_layout.state.randomize_density(0), 0.0);
        assert_eq!(
            state.lane_randomize_density(&params, 0),
            crate::track::DEFAULT_RANDOMIZE_DENSITY
        );
        state.global_config.set_randomize_density_for(TrackInstrumentKind::Kick, 0.5);
        assert_eq!(state.lane_randomize_density(&params, 0), 0.5);
    }

    /// [270] A file dragged over a One-Shot lane shows the replacement warning on
    /// that lane; over an empty lane, nothing.
    #[test]
    fn dragging_a_wav_over_a_oneshot_lane_warns_of_the_replacement() {
        use nih_plug::params::persist::PersistentField;
        use nih_plug_egui::egui;
        let params = crate::DrumFlashParams::default();
        PersistentField::<TrackLayoutState>::set(
            &params.track_layout,
            TrackLayoutState::modular_default_layout(),
        );
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let settings = &params.sound_settings.state;
        let mut state = crate::ui::editor_state::EditorUIState::default();
        let slot = 5;
        let wav = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/ch606.wav");
        assert!(super::create_oneshot_from_drop(
            &params, settings, &mut state, &pattern, plock, slot, &wav
        ));
        let rects: [Option<egui::Rect>; MAX_TRACKS] = std::array::from_fn(|i| {
            Some(egui::Rect::from_min_size(
                egui::pos2(0.0, i as f32 * 21.0),
                egui::vec2(800.0, 20.0),
            ))
        });
        let grid = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 400.0));
        let ctx = egui::Context::default();
        crate::ui::install_egui_fonts(&ctx);
        let mut warned_over = |row: usize| {
            // The bridge's hover position (`hover_id` in vendor file_drop.rs).
            let pos = egui::pos2(50.0, row as f32 * 21.0 + 5.0);
            ctx.data_mut(|d| d.insert_temp(egui::Id::new("egui_baseview_file_drop_hover"), pos));
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 600.0),
                )),
                ..Default::default()
            };
            let out = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    super::handle_wav_drop(
                        ui, &params, settings, &mut state, &pattern, plock, &rects, grid,
                    );
                });
            });
            out.shapes.iter().any(|c| {
                matches!(&c.shape, egui::Shape::Text(t)
                    if t.galley.text() == super::WAV_REPLACE_WARNING)
            })
        };
        assert!(warned_over(slot), "over the One-Shot lane");
        assert!(!warned_over(MAX_TRACKS - 3), "over an empty lane");
    }

    /// [270] A WAV dropped on a One-Shot lane replaces its file; its steps and locks stay.
    #[test]
    fn wav_drop_on_a_oneshot_lane_replaces_its_file() {
        use nih_plug::params::persist::PersistentField;
        let params = crate::DrumFlashParams::default();
        let layout = TrackLayoutState::modular_default_layout();
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let settings = &params.sound_settings.state;
        let mut state = crate::ui::editor_state::EditorUIState::default();
        let slot = 5;
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let first = assets.join("ch606.wav");
        assert!(super::create_oneshot_from_drop(
            &params, settings, &mut state, &pattern, plock, slot, &first
        ));
        pattern.set_step_mask(0, 1 << slot);
        plock.set_field(slot, 0, 3, 200.0);
        let second = std::fs::read_dir(&assets)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .find(|p| {
                p != &first && p.extension().is_some_and(|x| x.eq_ignore_ascii_case("wav"))
            })
            .expect("a second embedded WAV");
        assert!(super::replace_oneshot_file(&params, settings, slot, &second));
        assert_eq!(params.user_textures.path(slot).as_deref(), Some(second.as_path()));
        assert_eq!(
            params.track_layout.state.kind_for_slot(slot),
            Some(TrackInstrumentKind::OneShot)
        );
        assert_eq!(pattern.step_masks()[0], 1 << slot, "the lane's steps stay");
        assert!(plock.masks.is_active(slot, 0), "the lane's locks stay");
    }

    /// [243] Drop target selection: the empty row under the pointer wins,
    /// else the first free slot; occupied rows are never replaced and a full
    /// grid gives no target. [270] A One-Shot row under the pointer is replaced.
    #[test]
    fn pick_drop_lane_prefers_the_empty_row_under_the_pointer() {
        use nih_plug_egui::egui;
        let rect =
            |y: f32| egui::Rect::from_min_max(egui::pos2(0.0, y), egui::pos2(100.0, y + 20.0));
        let rects: [Option<egui::Rect>; MAX_TRACKS] =
            std::array::from_fn(|i| Some(rect(i as f32 * 21.0)));
        use super::DropTarget::{Create, Replace};
        let mut active = [false; MAX_TRACKS];
        active[1] = true;
        active[2] = true;
        let mut oneshot = [false; MAX_TRACKS];
        oneshot[2] = true;
        // Pointer over row 1 (active, not a One-Shot): the first free slot wins, NOT row 1.
        let pos = egui::pos2(50.0, 25.0);
        assert_eq!(super::pick_drop_lane(pos, &rects, &active, &oneshot), Some(Create(0)));
        // Pointer over row 2 (a One-Shot lane): its file is replaced.
        let row2 = egui::pos2(50.0, 2.0 * 21.0 + 5.0);
        assert_eq!(super::pick_drop_lane(row2, &rects, &active, &oneshot), Some(Replace(2)));
        // Pointer over row 4 (empty): row 4 wins even though slot 0 is free.
        let pos = egui::pos2(50.0, 4.0 * 21.0 + 5.0);
        assert_eq!(super::pick_drop_lane(pos, &rects, &active, &oneshot), Some(Create(4)));
        // Full grid: no target, except on a One-Shot lane.
        let active = [true; MAX_TRACKS];
        assert_eq!(super::pick_drop_lane(pos, &rects, &active, &oneshot), None);
        assert_eq!(super::pick_drop_lane(row2, &rects, &active, &oneshot), Some(Replace(2)));
    }

    /// The permutation the lane drag used before [191], kept as the reference
    /// the block version must reproduce for a single lane - proof that giving a
    /// master its followers did not change how one lane alone drags.
    fn reference_single_move(from: usize, to: usize) -> [usize; MAX_TRACKS] {
        let mut order = std::array::from_fn(|i| i);
        if from >= MAX_TRACKS || to >= MAX_TRACKS || from == to {
            return order;
        }

        let moved = order[from];
        if from < to {
            for idx in from..to {
                order[idx] = order[idx + 1];
            }
        } else {
            for idx in (to + 1..=from).rev() {
                order[idx] = order[idx - 1];
            }
        }
        order[to] = moved;
        order
    }

    #[test]
    fn a_single_lane_block_move_matches_the_old_permutation() {
        for from in 0..MAX_TRACKS {
            for to in 0..MAX_TRACKS {
                assert_eq!(
                    super::lane_move_order_block(from, 1, to),
                    reference_single_move(from, to),
                    "from {from} to {to}"
                );
            }
        }
    }

    /// [191] A master travels with its followers, in order and contiguous.
    #[test]
    fn a_master_carries_its_followers_as_one_block() {
        // Lanes 4, 5, 6 are one chain; drag it to the top.
        let order = super::lane_move_order_block(4, 3, 0);
        assert_eq!(&order[..3], &[4, 5, 6], "the run stays whole and in order");
        assert_eq!(&order[3..7], &[0, 1, 2, 3], "the lanes above shift down");

        // ...and downwards.
        let order = super::lane_move_order_block(0, 2, 5);
        assert_eq!(&order[5..7], &[0, 1]);
        assert_eq!(&order[..5], &[2, 3, 4, 5, 6]);
    }

    fn layout_with_chain() -> TrackLayoutState {
        // 0 free, 1 master with followers 2 and 3, 4 free.
        let mut layout = TrackLayoutState::empty_layout();
        for slot in 0..5 {
            layout.slots[slot] = TrackSlot::active_with_kind(TrackInstrumentKind::Kick);
        }
        layout.slots[2].linked_up = true;
        layout.slots[3].linked_up = true;
        layout
    }

    #[test]
    fn a_chain_reports_its_length_and_its_followers() {
        let layout = layout_with_chain();
        assert!(!layout.is_grid_follower(1), "the master is not a follower");
        assert!(layout.is_grid_follower(2));
        assert!(layout.is_grid_follower(3));
        assert!(!layout.is_grid_follower(4));
        assert_eq!(layout.chain_len(1), 3, "the master carries two followers");
        assert_eq!(
            layout.chain_len(2),
            2,
            "seen from a follower, the run below"
        );
        assert_eq!(layout.chain_len(0), 1, "a lone lane travels alone");
        assert_eq!(layout.chain_len(4), 1);
    }

    /// A drop between a master and its followers would steal the link, so the
    /// gap snaps to the nearer end of the chain.
    #[test]
    fn a_drop_point_never_lands_inside_a_chain() {
        let layout = layout_with_chain();
        // Gaps 2 and 3 are interior: before follower 2, and between 2 and 3.
        assert_eq!(
            layout.snap_gap_out_of_chains(2),
            1,
            "snaps above the master"
        );
        assert_eq!(layout.snap_gap_out_of_chains(3), 4, "snaps below the run");
        // Everything outside is left alone.
        for gap in [0usize, 1, 4, 5, MAX_TRACKS] {
            assert_eq!(
                layout.snap_gap_out_of_chains(gap),
                gap,
                "gap {gap} is outside any chain"
            );
        }
    }
}
