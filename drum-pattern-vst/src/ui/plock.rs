//! P-lock menus: sound plocks, fusion morph, sequencer plocks, popup.

use crate::plock::PlockState;
use crate::sequencer::{FusedGroup, SharedPattern};
use crate::ui::editor_state::*;
use crate::ui::grid::preserve_step_active_from_plock_popup;
use crate::ui::help::{self, HelpExt};
use crate::ui::local_param_slider::LocalParamSlider;
use crate::ui::menus::*;
use crate::ui::theme::*;
use crate::DrumFlashParams;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};

fn draw_plock_menu(
    ui: &mut egui::Ui,
    pattern: &SharedPattern,
    plock: &PlockState,
    params: &DrumFlashParams,
    _setter: &ParamSetter,
    instrument: usize,
    step: usize,
    step_was_active: bool,
    state: &mut EditorUIState,
) {
    #[allow(non_snake_case)]
    let ACCENT: Color32 = PL_LINK();
    // `instrument` is a SLOT index (plock storage is per slot); registry and
    // special-param lookups go through the voice index of the slot's kind.
    let voice_idx = schema_voice_idx(params, instrument);
    let inst_def = &crate::instrument_registry::INSTRUMENTS[voice_idx];
    let title = format!("Plock {}", inst_def.label);

    preserve_step_active_from_plock_popup(pattern, state, instrument, step, step_was_active);

    // Copy and Paste finish the gesture, so the menu closes afterwards. Set
    // inside the frame closure and applied once it returns: the paste branches
    // hold a borrow on the clipboard, which lives in `state`.
    let mut close_after_action = false;

    plock_menu_frame(ui, ACCENT, |ui| {
        if plock_menu_header(ui, &title, step, ACCENT) {
            state.plock_popup = None;
        }

        let has_plock = plock.masks.is_active(instrument, step);

        // ------ Creation ------
        if !has_plock {
            // [211] One way to create a p-lock, so the choice is gone: a new
            // p-lock always starts LINKED - only the fields actually touched
            // override, the rest keep following the lane.
            //
            // "Snapshot Current Settings", which froze all 46 fields at once,
            // is no longer offered. The FORMAT is untouched: a snapshot saved
            // before this build still loads, still plays, and the Mode row
            // below still names it "Full Snapshot".
            if plock_menu_action_row(ui, "Create Plock", ACCENT)
                .help(help::PLOCK_CREATE)
                .clicked()
            {
                plock.masks.set_active(instrument, step, true);
                // [215] Creating a p-lock IS the start of editing it, so the
                // gesture goes straight to the Lane Editor instead of leaving a
                // second menu open in front of the panel that does the work.
                state.sound_edit_target = Some(crate::ui::editor_state::SelectedCell {
                    slot: instrument,
                    step,
                });
                // [303] A cell's locks are edited in Sound or Effects: only Track gives way.
                if state.sound_editor_tab == crate::ui::editor_state::SoundEditorTab::Track {
                    state.sound_editor_tab = crate::ui::editor_state::SoundEditorTab::Sound;
                }
                close_after_action = true;
            }
            if let Some(ref entry) = state.plock_clipboard {
                if entry.instrument == instrument {
                    if plock_menu_action_row(ui, "Paste Plock", ACCENT)
                        .help(help::PLOCK_PASTE)
                        .clicked()
                    {
                        plock.masks.set_active(instrument, step, true);
                        plock
                            .field_masks
                            .set_raw(instrument, step, entry.field_mask);
                        for (field, &value) in entry.values.iter().enumerate() {
                            plock.values.set(instrument, step, field, value);
                        }
                        // The gesture is done: the next one happens on another
                        // cell, so keep the grid clear.
                        close_after_action = true;
                    }
                }
            }
            return;
        }

        // [218] The "Mode" row is gone. Since [211] a p-lock is always created
        // linked, so the row said "Linked to Global" until the first parameter
        // was touched and "Mixed" ever after - never anything the user could
        // act on. Only a snapshot inherited from an older session was worth
        // naming, and those cannot be created any more.

        // [215] No "Edit In Panel" row: the right-click that opened this menu
        // already aimed the Lane Editor at this cell, so the button asked the
        // user to confirm something that had happened. The menu is down to the
        // two things the panel cannot do to a p-lock as a whole.
        //
        // "Paste Plock" went with it, on the same instruction. Overwriting a
        // p-locked cell from the clipboard now takes two steps: Clear, then
        // paste on the emptied cell.
        if plock_menu_action_row(ui, "Copy Plock", ACCENT)
            .help(help::PLOCK_COPY)
            .clicked()
        {
            let field_mask = plock.field_masks.get_raw(instrument, step);
            let mut values = Vec::with_capacity(crate::plock::FIELD_COUNT);
            for field in 0..crate::plock::FIELD_COUNT {
                values.push(plock.values.get(instrument, step, field));
            }
            state.plock_clipboard = Some(SinglePlockClipboard {
                instrument,
                field_mask,
                values,
            });
            close_after_action = true;
        }
        if plock_menu_action_row(ui, "Clear Plock", DANGER())
            .help(help::PLOCK_CLEAR)
            .clicked()
        {
            plock.clear(instrument, step);
            // [217] Let go of the cell in the Lane Editor as well.
            //
            // The right-click that opened this menu aimed the panel at this
            // cell, in Step scope. Clearing left it aimed there with no p-lock
            // to edit - and in Step scope, writing ANY row re-creates the
            // p-lock (`PlockSource::set` raises the step's active bit, which is
            // what makes a row able to create an override at all). So the next
            // control touched brought the p-lock straight back, and the Clear
            // looked like it had done nothing.
            if state
                .sound_edit_target
                .map(|c| c.slot == instrument && c.step == step)
                == Some(true)
            {
                state.sound_edit_target = None;
            }
            // [216] The gesture is finished and the cell has no p-lock left:
            // keeping the menu open would show options about something that no
            // longer exists.
            close_after_action = true;
        }
    });
    if close_after_action {
        state.plock_popup = None;
    }
}

/// Fusion group actions (Morphing / Edit Fusion Steps / Delete Fusion),
/// shared by the sound-plock and sequencer-plock popup branches.
fn draw_fusion_group_menu(
    ui: &mut egui::Ui,
    pattern: &SharedPattern,
    params: &DrumFlashParams,
    inst: usize,
    // [283] The lane whose fused groups `inst` plays (itself unless linked):
    // the group's cells and pulses are edited there, shared by the chain.
    grid: usize,
    idx: usize,
    group: FusedGroup,
    step: usize,
    state: &mut EditorUIState,
) {
    plock_menu_frame(ui, PL_LINK(), |ui| {
        if plock_menu_header(
            ui,
            &format!("Fusion {}-{}", group.start_cell + 1, group.end_cell + 1),
            step,
            PL_LINK(),
        ) {
            state.plock_popup = None;
        }

        let morph_active = group.morph_count > 0;
        let morph_label = if morph_active {
            let morphable =
                crate::instrument_registry::morphable_fields(schema_voice_idx(params, inst));
            let names: Vec<&str> = group.morph_targets[..group.morph_count as usize]
                .iter()
                .map(|t| {
                    morphable
                        .iter()
                        .find(|f| f.field_index == t.field as usize)
                        .map(|f| f.label)
                        .unwrap_or("?")
                })
                .collect();
            format!("Morphing ({})", names.join(", "))
        } else {
            "Morphing".to_string()
        };
        // [184] ph. 4 — the morph is edited in the Lane Editor now: this row just
        // aims the panel at it and closes the popup.
        if plock_menu_action_row(ui, &morph_label, PL_LINK())
            .help(help::FUSION_MORPH)
            .clicked()
        {
            state.sound_edit_target =
                Some(crate::ui::editor_state::SelectedCell { slot: inst, step });
            state.fusion_tab = crate::ui::editor_state::FusionTab::End;
            // [303] A cell's locks are edited in Sound or Effects: only Track gives way.
                if state.sound_editor_tab == crate::ui::editor_state::SoundEditorTab::Track {
                    state.sound_editor_tab = crate::ui::editor_state::SoundEditorTab::Sound;
                }
            state.plock_popup = None;
        }
        if plock_menu_action_row(ui, "Edit Fusion Steps", PL_LINK())
            .help(help::FUSION_EDIT_STEPS)
            .clicked()
        {
            state.fusion_editing = Some((grid, idx));
            state.fusion_editing_lane = inst;
            state.fusion_edit_steps = group.step_count;
            state.fusion_edit_focus_request = true;
            state.plock_popup = None;
        }
        if plock_menu_action_row(ui, "Delete Fusion", DANGER())
            .help(help::FUSION_DELETE)
            .clicked()
        {
            let mut new_fusions = pattern.load_fusions(grid);
            if idx < new_fusions.len() {
                new_fusions.remove(idx);
                pattern.store_fusions(grid, &new_fusions);
            }
            state.mark_pattern_dirty();
            state.plock_popup = None;
        }
    });
}

/// [282] Where the cell menu opens: under its cell (`anchor`), left-aligned with
/// it, so the cell stays visible; above it when the window has no room below
/// for the menu (`menu_h`, its height last frame); at the click point when there
/// is no cell.
fn popup_placement(
    anchor: Option<egui::Rect>,
    click: egui::Pos2,
    menu_h: Option<f32>,
    screen: egui::Rect,
) -> (egui::Pos2, egui::Align2) {
    const GAP: f32 = 3.0;
    let Some(cell) = anchor else {
        return (click, egui::Align2::LEFT_TOP);
    };
    let h = menu_h.unwrap_or(0.0);
    let fits_below = cell.bottom() + GAP + h <= screen.bottom();
    let fits_above = cell.top() - GAP - h >= screen.top();
    if !fits_below && fits_above {
        (cell.left_top() - egui::vec2(0.0, GAP), egui::Align2::LEFT_BOTTOM)
    } else {
        (cell.left_bottom() + egui::vec2(0.0, GAP), egui::Align2::LEFT_TOP)
    }
}
pub fn draw_plock_popup(
    ctx: &egui::Context,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    plock: &PlockState,
    state: &mut EditorUIState,
) {
    let popup = match state.plock_popup {
        Some(p) => p,
        None => return,
    };

    let area_id = egui::Id::new("plock_popup");
    let (pos, pivot) = popup_placement(
        popup.anchor,
        popup.screen_pos,
        ctx.memory(|m| m.area_rect(area_id)).map(|r| r.height()),
        ctx.screen_rect(),
    );
    let response = egui::Area::new(area_id)
        .kind(egui::UiKind::Menu)
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(pos)
        .sense(egui::Sense::click())
        .show(ctx, |ui| {
            // Outer border: draw a slightly larger rounded rect behind the panel.
            let content_response = egui::Frame::NONE
                .fill(P_ACTIVE())
                .corner_radius(RADIUS_PANEL)
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_min_width(260.0);
                    ui.set_max_width(350.0);

                    let inst = popup.instrument;
                    let step = popup.step;
                    // [283] A linked lane shows the fused groups of the lane
                    // it follows (shared), with its own morph.
                    let grid = params.track_layout.state.grid_slot(inst);
                    let fusions = pattern.load_lane_fusions(inst, grid);
                    let fusion_info = fusions.iter().enumerate().find(|(_, g)| {
                        (g.start_cell as usize) <= step && step <= (g.end_cell as usize)
                    });

                    // [299] An effect lane has no notes, so no sequencer p-lock
                    // (stutter, micro-timing, probability, condition): only its
                    // sound locks, and no Sound / Sequencer choice.
                    let effect_lane = params
                        .track_layout
                        .state
                        .kind_for_slot(inst)
                        .is_some_and(|k| k.is_effect());
                    let popup = if effect_lane {
                        crate::ui::editor_state::PlockPopup { sequencer: false, ..popup }
                    } else {
                        popup
                    };
                    // [213] Pick the p-lock type here, on the cell, instead of
                    // flipping the grid-wide "P-Lock Mode" first. Same two
                    // labels as that switch, so they read as the same choice;
                    // this one is local to the popup and leaves the grid alone.
                    if !effect_lane {
                    help::zone(ui, "step_menu_type", help::STEP_MENU_TYPE, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 8.0;
                            ui.label(RichText::new("P-Lock").font(f_sans_med(10.0)).color(INK3()));
                            let selected = if popup.sequencer { 1 } else { 0 };
                            let picked = crate::ui::skeuo::segmented(
                                ui,
                                ("plock_popup_type", inst, step),
                                &["Sound", "Sequencer"],
                                selected,
                            );
                            if picked != selected {
                                let sequencer = picked == 1;
                                state.plock_popup = Some(crate::ui::editor_state::PlockPopup {
                                    sequencer,
                                    ..popup
                                });
                                // The Lane Editor follows: it edits sound p-locks and
                                // has nothing to say about a sequencer one.
                                if sequencer {
                                    state.sound_edit_target = None;
                                } else {
                                    state.sound_edit_target =
                                        Some(crate::ui::editor_state::SelectedCell {
                                            slot: inst,
                                            step,
                                        });
                                    state.sound_editor_tab =
                                        crate::ui::editor_state::SoundEditorTab::Sound;
                                }
                            }
                        })
                    });
                    ui.add_space(8.0);
                    }

                    if popup.sequencer {
                        if let Some((idx, group)) = fusion_info {
                            // Fused cell: fusion actions (morph/edit/delete)
                            // on top, the seq-plock menu below â€” same as
                            // the sound-plock branch.
                            draw_fusion_group_menu(
                                ui, pattern, params, inst, grid, idx, *group, step, state,
                            );
                            ui.separator();
                            draw_sequencer_plock_menu(
                                ui,
                                pattern,
                                params,
                                setter,
                                inst,
                                step,
                                popup.step_was_active,
                                state,
                                true,
                            );
                        } else {
                            draw_sequencer_plock_menu(
                                ui,
                                pattern,
                                params,
                                setter,
                                inst,
                                step,
                                popup.step_was_active,
                                state,
                                false,
                            );
                        }
                    } else {
                        if let Some((idx, group)) = fusion_info {
                            draw_fusion_group_menu(
                                ui, pattern, params, inst, grid, idx, *group, step, state,
                            );
                            ui.separator();

                            // Also show the source-step plock menu below.
                            draw_plock_menu(
                                ui,
                                pattern,
                                plock,
                                params,
                                setter,
                                inst,
                                step,
                                popup.step_was_active,
                                state,
                            );
                        } else {
                            draw_plock_menu(
                                ui,
                                pattern,
                                plock,
                                params,
                                setter,
                                inst,
                                step,
                                popup.step_was_active,
                                state,
                            );
                        }
                    }
                })
                .response;

            let border_rect = content_response.rect.expand2(egui::Vec2::new(1.0, 1.0));
            ui.painter().rect_stroke(
                border_rect,
                RADIUS_PANEL + 1.0,
                egui::Stroke::new(1.0, LINE2()),
                egui::StrokeKind::Inside,
            );
            content_response
        })
        .response;

    // Close popup on click outside.
    //
    // [282] Not in the frame that opens it: the right-click release that
    // opened the menu is still "a click" then. The Area did not exist the
    // frame before (not hovered), and its rect is rounded to 1/32 pt, so with
    // a fractional pointer (Zoom 125 %, Windows scaling) the click point
    // could fall just outside its top-left corner and the menu closed as it
    // opened: "shows one time in three".
    if !popup.just_opened && response.clicked_elsewhere() {
        state.plock_popup = None;
    }

    // [218] A right-click ON THE MENU closes it: the gesture that opens it is
    // the one that dismisses it.
    //
    // Two guards, both needed. The click must land inside the menu, or a
    // right-click on another cell would close this popup instead of moving it
    // there. And it must not be the click that OPENED the menu: the popup is
    // drawn at the pointer in the very same frame, so that first click is both
    // "clicked" and inside the rect - the menu would vanish on sight.
    if !popup.just_opened && ctx.input(|i| i.pointer.button_clicked(egui::PointerButton::Secondary))
    {
        let inside = ctx
            .input(|i| i.pointer.interact_pos())
            .is_some_and(|pos| response.rect.contains(pos));
        if inside {
            state.plock_popup = None;
        }
    }
    if let Some(open) = state.plock_popup.as_mut() {
        open.just_opened = false;
    }

    // Close popup on click in the popup border/padding (consume the click so it
    // does not pass through to the step cell underneath).
    if state.plock_popup.is_some() && response.clicked() {
        state.plock_popup = None;
    }
}

fn draw_sequencer_plock_menu(
    ui: &mut egui::Ui,
    pattern: &SharedPattern,
    params: &DrumFlashParams,
    _setter: &ParamSetter,
    instrument: usize,
    step: usize,
    step_was_active: bool,
    state: &mut EditorUIState,
    stutter_disabled: bool,
) {
    use crate::plock::{SequencerStepParams, StepCondition};

    #[allow(non_snake_case)]
    let ACCENT: Color32 = SEQPL();
    // `instrument` is a SLOT index; the label comes from the slot's voice schema.
    let inst_def = &crate::instrument_registry::INSTRUMENTS[schema_voice_idx(params, instrument)];
    let title = format!("Seq Plock {}", inst_def.label);

    preserve_step_active_from_plock_popup(pattern, state, instrument, step, step_was_active);

    plock_menu_frame(ui, ACCENT, |ui| {
        if plock_menu_header(ui, &title, step, ACCENT) {
            state.plock_popup = None;
        }

        let seq_plock = &params.seq_plock_state.state;
        let has_seq_plock = seq_plock.is_active(instrument, step);
        let current = seq_plock.get(instrument, step).unwrap_or_default();
        let mut changed_this_frame = false;

        // Mode indicator
        help::zone(ui, "seq_mode", help::SEQ_MODE, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Mode").font(f_sans_med(10.0)).color(INK3()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mode_text = if has_seq_plock { "Active" } else { "Inactive" };
                    ui.label(
                        RichText::new(mode_text)
                            .font(f_mono_med(10.0))
                            .color(ACCENT),
                    );
                });
            })
        });
        ui.add_space(8.0);

        // Solo â€” mutes every other lane ONLY while the playhead sits on this
        // cell (its step, or the whole span of a fused cell). Per-cell toggle;
        // remove it by toggling the same cell off. Turning it off clears the
        // whole seq-plock when solo was the only thing set, so a solo-only cell
        // does not linger as an empty (paramless) seq-plock.
        {
            let solo_now = current.solo;
            let solo_response = help::zone(ui, "seq_solo", help::SEQ_SOLO, |ui| {
                plock_menu_row(ui, "Solo", ACCENT, solo_now, None, |ui| {
                    ui.add(crate::ui::widgets::ToggleSwitch::new(solo_now))
                })
            });
            if solo_response.clicked() {
                if solo_now {
                    seq_plock.set_solo(instrument, step, false);
                    if let Some(p) = seq_plock.get(instrument, step) {
                        let no_other_params = p.probability == 1.0
                            && p.stutter_count == 1
                            && p.condition == StepCondition::Always
                            && p.microtiming_ms == 0.0;
                        if no_other_params {
                            seq_plock.clear(instrument, step);
                        }
                    }
                } else {
                    seq_plock.set_solo(instrument, step, true);
                }
                changed_this_frame = true;
            }
        }
        ui.add_space(4.0);

        // Probability
        {
            let mut prob = current.probability;
            let prob_text = format!("{:.0}%", prob * 100.0);
            let prob_response = help::zone(ui, "seq_prob", help::SEQ_PROBABILITY, |ui| {
                plock_menu_row(
                    ui,
                    "Probability",
                    ACCENT,
                    has_seq_plock,
                    Some(&prob_text),
                    |ui| {
                        ui.add(
                            LocalParamSlider::new(&mut prob, 0.0..=1.0)
                                .with_width(86.0)
                                .without_value()
                                .reset_value(1.0),
                        )
                    },
                )
            });
            if prob_response.changed() {
                seq_plock.set_probability(instrument, step, prob);
                changed_this_frame = true;
            }
        }

        // Stutter
        if stutter_disabled {
            if has_seq_plock && current.stutter_count != 1 {
                let mut fixed = current;
                fixed.stutter_count = 1;
                seq_plock.set(instrument, step, &fixed);
            }
            help::zone(ui, "seq_stutter", help::SEQ_STUTTER, |ui| {
                plock_menu_row(ui, "Stutter", ACCENT, false, None, |ui| {
                    ui.label(
                        RichText::new("disabled on fusion")
                            .font(f_sans_med(10.0))
                            .color(INK3()),
                    );
                    ui.allocate_response(Vec2::new(1.0, 1.0), egui::Sense::hover())
                })
            });
        } else {
            let mut stutter = current.stutter_count.max(1) as f32;
            let stutter_text = format!("{}x", stutter as i32);
            let stutter_response = help::zone(ui, "seq_stutter", help::SEQ_STUTTER, |ui| {
                plock_menu_row(
                    ui,
                    "Stutter",
                    ACCENT,
                    has_seq_plock && current.stutter_count != 1,
                    Some(&stutter_text),
                    |ui| {
                        ui.add(
                            LocalParamSlider::new(&mut stutter, 1.0..=16.0)
                                .with_width(86.0)
                                .without_value()
                                .reset_value(1.0),
                        )
                    },
                )
            });
            if stutter_response.changed() {
                let new_stutter = stutter.round() as u8;
                seq_plock.set_stutter(instrument, step, new_stutter);
                changed_this_frame = true;
            }
        }

        // Microtiming (nudge): shifts the whole cell (stutter/fusion pulses
        // included) by -100..+100 ms around its step boundary.
        {
            let mut nudge = current.microtiming_ms.clamp(-100.0, 100.0);
            let nudge_text = if nudge.abs() < 0.5 {
                "0 ms".to_string()
            } else {
                format!("{:+.0} ms", nudge)
            };
            let nudge_response = help::zone(ui, "seq_nudge", help::SEQ_NUDGE, |ui| {
                plock_menu_row(
                    ui,
                    "Nudge",
                    ACCENT,
                    has_seq_plock && current.microtiming_ms != 0.0,
                    Some(&nudge_text),
                    |ui| {
                        ui.add(
                            LocalParamSlider::new(&mut nudge, -100.0..=100.0)
                                .with_width(86.0)
                                .without_value()
                                .reset_value(0.0),
                        )
                    },
                )
            });
            if nudge_response.changed() {
                seq_plock.set_microtiming(instrument, step, nudge);
                changed_this_frame = true;
            }
        }

        // Condition
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Condition").font(f_sans_sb(10.0)).color(INK2()))
                .help(help::SEQ_CONDITION);
            // [218] "Not" inverts whichever condition is selected: "3/4"
            // becomes "every loop except the 3rd of four". It replaces the old
            // "Not 1st loop" entry, which was this modifier hard-wired onto one
            // condition. Greyed on "Always", where inverting would mean "never".
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let inert = current.condition == StepCondition::Always;
                let on = current.condition_negate && !inert;
                let color = if on { ACCENT } else if inert { INK3() } else { INK2() };
                let response = ui.add_enabled(
                    !inert,
                    egui::Button::new(RichText::new("Not").font(f_sans_med(9.5)).color(color))
                        .fill(PANEL2())
                        .stroke(egui::Stroke::new(1.0, if on { ACCENT } else { LINE2() }))
                        .corner_radius(RADIUS_CTL)
                        .min_size(Vec2::new(44.0, 22.0)),
                );
                if response.clicked() {
                    seq_plock.set_condition_negate(instrument, step, !on);
                    changed_this_frame = true;
                }
                response.help_or(
                    help::SEQ_NOT,
                    "Invert the whole condition: the step plays on every loop EXCEPT the ones it selects.",
                );

                // [219] A second condition, ANDed with the first. Off, the
                // section is exactly as before; on, a second grid appears.
                let and_on = current.condition_and.is_some();
                let and_response = ui.add(
                    egui::Button::new(
                        RichText::new("And")
                            .font(f_sans_med(9.5))
                            .color(if and_on { ACCENT } else { INK2() }),
                    )
                    .fill(PANEL2())
                    .stroke(egui::Stroke::new(1.0, if and_on { ACCENT } else { LINE2() }))
                    .corner_radius(RADIUS_CTL)
                    .min_size(Vec2::new(44.0, 22.0)),
                );
                if and_response.clicked() {
                    let next = if and_on { None } else { Some(current.condition) };
                    seq_plock.set_condition_and(instrument, step, next);
                    changed_this_frame = true;
                }
                and_response.help_or(
                    help::SEQ_AND,
                    "Add a second condition: the step plays only on the loops where BOTH hold.",
                );
            });
        });
        ui.add_space(6.0);

        let all_conditions = StepCondition::all();
        let grid_id = format!("condition_grid_{}_{}", instrument, step);
        let available_w = ui.available_width();
        let button_w = (available_w - 16.0) / 3.0;
        egui::Grid::new(grid_id)
            .num_columns(3)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                for (idx, cond) in all_conditions.iter().copied().enumerate() {
                    let selected = current.condition == cond;
                    let text_color = if selected { ACCENT } else { INK2() };
                    let fill = if selected { PANEL2() } else { PANEL2() };
                    let stroke_color = if selected { ACCENT } else { LINE2() };
                    if ui
                        .add_sized(
                            Vec2::new(button_w.max(1.0), 26.0),
                            egui::Button::new(
                                RichText::new(cond.label())
                                    .font(f_sans_med(9.5))
                                    .color(text_color),
                            )
                            .fill(fill)
                            .stroke(egui::Stroke::new(1.0, stroke_color))
                            .corner_radius(RADIUS_CTL),
                        )
                        .help(help::SEQ_CONDITION)
                        .clicked()
                    {
                        seq_plock.set_condition(instrument, step, cond);
                        changed_this_frame = true;
                    }
                    if (idx + 1) % 3 == 0 {
                        ui.end_row();
                    }
                }
            });

        if let Some(second) = current.condition_and {
            ui.add_space(8.0);
            ui.label(RichText::new("And").font(f_sans_sb(10.0)).color(INK2()));
            ui.add_space(6.0);
            let grid_id = format!("condition_and_grid_{}_{}", instrument, step);
            egui::Grid::new(grid_id)
                .num_columns(3)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    for (idx, cond) in all_conditions.iter().copied().enumerate() {
                        let selected = second == cond;
                        let text_color = if selected { ACCENT } else { INK2() };
                        let stroke_color = if selected { ACCENT } else { LINE2() };
                        if ui
                            .add_sized(
                                Vec2::new(button_w.max(1.0), 26.0),
                                egui::Button::new(
                                    RichText::new(cond.label())
                                        .font(f_sans_med(9.5))
                                        .color(text_color),
                                )
                                .fill(PANEL2())
                                .stroke(egui::Stroke::new(1.0, stroke_color))
                                .corner_radius(RADIUS_CTL),
                            )
                            .help(help::SEQ_AND)
                            .clicked()
                        {
                            seq_plock.set_condition_and(instrument, step, Some(cond));
                            changed_this_frame = true;
                        }
                        if (idx + 1) % 3 == 0 {
                            ui.end_row();
                        }
                    }
                });
        }

        // Actions
        ui.add_space(8.0);
        if has_seq_plock || changed_this_frame {
            if plock_menu_action_row(ui, "Clear Seq Plock", DANGER())
                .help(help::SEQ_CLEAR)
                .clicked()
            {
                seq_plock.clear(instrument, step);
            }
        } else {
            if plock_menu_action_row(ui, "Create Seq Plock", ACCENT)
                .help(help::SEQ_CREATE)
                .clicked()
            {
                seq_plock.set(instrument, step, &SequencerStepParams::default());
            }
        }
    });
}

#[cfg(test)]
mod tests {
    /// [282] The cell menu opens under its cell, above it when there is no room
    /// below, at the click point without a cell.
    #[test]
    fn the_cell_menu_opens_under_its_cell_or_above_it() {
        use nih_plug_egui::egui::{pos2, vec2, Align2, Rect};
        let screen = Rect::from_min_max(pos2(0.0, 0.0), pos2(1000.0, 600.0));
        let cell = Rect::from_min_max(pos2(100.0, 200.0), pos2(130.0, 226.0));
        let click = pos2(115.0, 210.0);
        assert_eq!(
            super::popup_placement(Some(cell), click, Some(300.0), screen),
            (pos2(100.0, 229.0), Align2::LEFT_TOP)
        );
        let low = cell.translate(vec2(0.0, 300.0));
        assert_eq!(
            super::popup_placement(Some(low), click, Some(300.0), screen),
            (pos2(100.0, 497.0), Align2::LEFT_BOTTOM)
        );
        assert_eq!(
            super::popup_placement(None, click, Some(300.0), screen),
            (click, Align2::LEFT_TOP)
        );
    }
}
