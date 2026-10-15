//! Presets modal ([150]): save/load user presets + factory presets for
//! instrument sounds, full patterns and songs.

use crate::pattern_bank::restore_from_buffers;
use crate::presets::{self, PresetKind};
use crate::sequencer::SharedPattern;
use crate::sound_settings::SoundSettingsState;
use crate::track::{TrackInstrumentKind, TrackLayoutState, TrackSlot};
use crate::ui::controls::keycap_button;
use crate::ui::editor_state::{EditorUIState, PresetBrowserState};
use crate::ui::grid::change_slot_kind;
use crate::ui::help::{self, HelpExt};
use crate::ui::menus::{page_menu_frame, plock_menu_action_row};
use crate::ui::sound_editor::{apply_lane_layout_preset, store_field};
use crate::ui::theme::*;
use crate::ui::widgets::KeycapState;
use crate::DrumFlashParams;
use nih_plug::{params::persist::PersistentField, prelude::*};
use nih_plug_egui::egui::{self, RichText, Vec2};

/// Open the modal (pattern bank "Presets" button).
pub fn open(state: &mut EditorUIState) {
    state.preset_browser = Some(PresetBrowserState::default());
}

#[allow(clippy::too_many_arguments)]
pub fn draw_preset_browser_if_any(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
) {
    if state.preset_browser.is_none() {
        return;
    }

    let screen = ui.ctx().screen_rect();
    let size = Vec2::new(420.0, 460.0);
    let origin = egui::pos2(
        screen.center().x - size.x / 2.0,
        (screen.center().y - size.y / 2.0).max(screen.top() + 8.0),
    );
    let area_id = ui.id().with("preset_browser");

    // Actions are collected while drawing and applied afterwards, once the
    // borrow on the browser state is released.
    let mut save_requested = false;
    #[cfg(debug_assertions)]
    let mut export_requested = false;
    let mut load_user: Option<std::path::PathBuf> = None;
    let mut load_factory: Option<&'static str> = None;
    let mut delete_user: Option<std::path::PathBuf> = None;
    let mut rename_requested: Option<(std::path::PathBuf, String)> = None;
    let mut apply_builtin_grid: Option<usize> = None; // 0=Clear All, 1=4, 2=12, 3=AC 4, 4=AC 12

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
                    ui.label(RichText::new("Presets").font(f_sans_sb(11.0)).color(BLUE()));
                    ui.add_space((ui.available_width() - 22.0).max(0.0));
                    if keycap_button(ui, "x", 22.0, KeycapState::Rest, true, f_sans_med(12.0))
                        .clicked()
                    {
                        close = true;
                    }
                });
                if close {
                    state.preset_browser = None;
                    return;
                }
                let Some(browser) = state.preset_browser.as_mut() else {
                    return;
                };

                ui.add_space(8.0);

                // Tabs
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for kind in PresetKind::ALL {
                        let active = browser.kind == kind;
                        let kc = if active {
                            KeycapState::PressedBlue
                        } else {
                            KeycapState::Rest
                        };
                        let tab_help = match kind {
                            PresetKind::Instrument => help::PRESET_TAB_INSTRUMENT,
                            PresetKind::Pattern => help::PRESET_TAB_PATTERN,
                            PresetKind::Grid => help::PRESET_TAB_GRID,
                            PresetKind::Song => help::PRESET_TAB_SONG,
                        };
                        if keycap_button(ui, kind.label(), 88.0, kc, true, f_sans_med(10.5))
                            .help(tab_help)
                            .clicked()
                        {
                            browser.kind = kind;
                            browser.confirm_delete = None;
                            browser.renaming = None;
                        }
                    }
                });
                ui.add_space(10.0);

                // Save row: capture the current instrument / pattern / song.
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").font(f_sans_med(10.5)).color(INK3()));
                    ui.add(
                        egui::TextEdit::singleline(&mut browser.name_input)
                            .desired_width(180.0)
                            .font(f_sans_med(11.0)),
                    );
                    let can_save = !browser.name_input.trim().is_empty();
                    if plock_menu_action_row(
                        ui,
                        "Save current",
                        if can_save { BLUE() } else { INK3() },
                    )
                    .help(help::PRESET_SAVE)
                    .clicked()
                        && can_save
                    {
                        save_requested = true;
                    }
                });
                // [261] A failed save/rename/delete says so — the old code
                // swallowed the error and cleared the name as if it worked.
                if let Some(err) = &browser.last_error {
                    ui.label(
                        RichText::new(err.as_str())
                            .font(f_sans_med(9.5))
                            .color(egui::Color32::from_rgb(224, 122, 95)),
                    );
                }
                // Patterns: choose whether loading also installs the lane kit.
                if browser.kind == PresetKind::Pattern {
                    ui.add_space(4.0);
                    help::zone(ui, "preset_load_lanes", help::PRESET_LOAD_LANES, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Load lanes too")
                                    .font(f_sans_med(10.5))
                                    .color(INK3()),
                            );
                            ui.add_space((ui.available_width() - 34.0).max(0.0));
                            let on = browser.load_with_kit;
                            if ui.add(crate::ui::widgets::ToggleSwitch::new(on)).clicked() {
                                browser.load_with_kit = !on;
                            }
                        })
                    });
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);

                egui::ScrollArea::vertical()
                    // Always reserved: the bar never pops in and shifts the
                    // rows, and the inner right margin keeps the Del buttons
                    // off it ([235]).
                    .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysVisible)
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin {
                                right: 8,
                                ..Default::default()
                            })
                            .show(ui, |ui| {
                                // Grid tab: the built-in lane layouts (ex page-bar dropdown)
                                // live here as factory grid presets.
                                if browser.kind == PresetKind::Grid {
                                    ui.label(
                                        RichText::new("Factory")
                                            .font(f_sans_sb(10.0))
                                            .color(INK3()),
                                    );
                                    ui.add_space(4.0);
                                    for (idx, label) in
                                        ["Clear All", "4 Lanes", "12 Lanes", "AC 4", "AC 12"]
                                            .iter()
                                            .enumerate()
                                    {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(*label)
                                                    .font(f_sans_med(10.5))
                                                    .color(INK2()),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    // Clear All wipes the grid: two clicks.
                                                    let confirming =
                                                        idx == 0 && browser.confirm_clear_all_grid;
                                                    let caption =
                                                        if confirming { "Sure?" } else { "Load" };
                                                    if keycap_button(
                                                        ui,
                                                        caption,
                                                        44.0,
                                                        KeycapState::Rest,
                                                        true,
                                                        f_sans_med(9.5),
                                                    )
                                                    .help(help::PRESET_GRID_FACTORY)
                                                    .clicked()
                                                    {
                                                        if idx == 0 && !confirming {
                                                            browser.confirm_clear_all_grid = true;
                                                        } else {
                                                            apply_builtin_grid = Some(idx);
                                                        }
                                                    }
                                                },
                                            );
                                        });
                                    }
                                    ui.add_space(8.0);
                                }

                                // Factory presets (embedded, read-only).
                                let factory = presets::factory_presets(browser.kind);
                                if !factory.is_empty() {
                                    ui.label(
                                        RichText::new("Factory")
                                            .font(f_sans_sb(10.0))
                                            .color(INK3()),
                                    );
                                    ui.add_space(4.0);
                                    for (name, json) in factory {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(name)
                                                    .font(f_sans_med(10.5))
                                                    .color(INK2()),
                                            );
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if keycap_button(
                                                        ui,
                                                        "Load",
                                                        44.0,
                                                        KeycapState::Rest,
                                                        true,
                                                        f_sans_med(9.5),
                                                    )
                                                    .help(help::PRESET_LOAD)
                                                    .clicked()
                                                    {
                                                        load_factory = Some(json);
                                                    }
                                                },
                                            );
                                        });
                                    }
                                    ui.add_space(8.0);
                                }

                                // User presets. [262] Cached: re-read on open, tab
                                // change or mutation — not on every frame.
                                ui.label(RichText::new("User").font(f_sans_sb(10.0)).color(INK3()));
                                ui.add_space(4.0);
                                let files = browser.cached_files();
                                if files.is_empty() {
                                    ui.label(
                                        RichText::new("No user preset yet")
                                            .font(f_sans_med(10.0))
                                            .color(FAINT()),
                                    );
                                }
                                for info in files {
                                    ui.horizontal(|ui| {
                                        let is_renaming =
                                            browser.renaming.as_ref() == Some(&info.path);
                                        if is_renaming {
                                            // [237] Inline rename: Enter commits, Esc cancels.
                                            let resp = ui.add(
                                                egui::TextEdit::singleline(
                                                    &mut browser.rename_input,
                                                )
                                                .desired_width(180.0)
                                                .font(f_sans_med(11.0)),
                                            );
                                            if browser.rename_focus_request {
                                                resp.request_focus();
                                                browser.rename_focus_request = false;
                                            }
                                            let commit = resp.lost_focus()
                                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                            let cancel =
                                                ui.input(|i| i.key_pressed(egui::Key::Escape));
                                            if commit {
                                                let new_name =
                                                    browser.rename_input.trim().to_string();
                                                if !new_name.is_empty() {
                                                    rename_requested =
                                                        Some((info.path.clone(), new_name));
                                                }
                                                browser.renaming = None;
                                            } else if cancel {
                                                browser.renaming = None;
                                            }
                                            return;
                                        }
                                        ui.label(
                                            RichText::new(&info.name)
                                                .font(f_sans_med(10.5))
                                                .color(INK()),
                                        );
                                        // [236] Save date from the file mtime (format untouched).
                                        if let Some(m) = info.modified {
                                            ui.label(
                                                RichText::new(presets::format_date(m))
                                                    .font(f_mono_med(9.0))
                                                    .color(FAINT()),
                                            );
                                        }
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let confirming = browser.confirm_delete.as_ref()
                                                    == Some(&info.path);
                                                if keycap_button(
                                                    ui,
                                                    if confirming { "Sure?" } else { "Del" },
                                                    44.0,
                                                    KeycapState::Rest,
                                                    true,
                                                    f_sans_med(9.5),
                                                )
                                                .help(help::PRESET_DELETE)
                                                .clicked()
                                                {
                                                    if confirming {
                                                        delete_user = Some(info.path.clone());
                                                    } else {
                                                        browser.confirm_delete =
                                                            Some(info.path.clone());
                                                    }
                                                }
                                                ui.add_space(4.0);
                                                if keycap_button(
                                                    ui,
                                                    "Load",
                                                    44.0,
                                                    KeycapState::Rest,
                                                    true,
                                                    f_sans_med(9.5),
                                                )
                                                .help(help::PRESET_LOAD)
                                                .clicked()
                                                {
                                                    load_user = Some(info.path.clone());
                                                }
                                                ui.add_space(4.0);
                                                if keycap_button(
                                                    ui,
                                                    "Ren",
                                                    44.0,
                                                    KeycapState::Rest,
                                                    true,
                                                    f_sans_med(9.5),
                                                )
                                                .help(help::PRESET_RENAME)
                                                .clicked()
                                                {
                                                    browser.renaming = Some(info.path.clone());
                                                    browser.rename_input = info.name.clone();
                                                    browser.rename_focus_request = true;
                                                    browser.confirm_delete = None;
                                                }
                                            },
                                        );
                                    });
                                }

                                // Debug factory authoring: export the current state into
                                // the `_factory` staging dir (then committed into
                                // `assets/presets/` + registered in factory_presets.rs).
                                #[cfg(debug_assertions)]
                                {
                                    ui.add_space(8.0);
                                    ui.separator();
                                    ui.add_space(4.0);
                                    let can_export = !browser.name_input.trim().is_empty();
                                    if plock_menu_action_row(
                                        ui,
                                        "Export factory (dev)",
                                        if can_export { AMBER() } else { INK3() },
                                    )
                                    .clicked()
                                        && can_export
                                    {
                                        export_requested = true;
                                    }
                                }
                            });
                    });
            });
        })
        .response;

    // -- Deferred actions ----------------------------------------------------
    let browser_snapshot = state
        .preset_browser
        .as_ref()
        .map(|b| (b.kind, b.name_input.trim().to_string()));
    if let Some((kind, name)) = browser_snapshot {
        if save_requested && !name.is_empty() {
            let result = save_current(params, pattern, sound_settings, state, kind, name.clone());
            // [261] The name field clears on success only; errors are shown.
            if let Some(b) = state.preset_browser.as_mut() {
                match result {
                    Ok(()) => {
                        b.name_input.clear();
                        b.last_error = None;
                        b.invalidate_files();
                    }
                    Err(e) => b.last_error = Some(e),
                }
            }
        }
        #[cfg(debug_assertions)]
        if export_requested && !name.is_empty() {
            export_factory(params, pattern, sound_settings, state, kind, name);
        }
        if let Some(path) = delete_user {
            let result = presets::delete_file(&path);
            if let Some(b) = state.preset_browser.as_mut() {
                match result {
                    Ok(()) => {
                        b.confirm_delete = None;
                        b.last_error = None;
                        b.invalidate_files();
                    }
                    Err(e) => b.last_error = Some(e),
                }
            }
        }
        if let Some((path, new_name)) = rename_requested {
            let result = presets::rename_preset(&path, &new_name, kind);
            if let Some(b) = state.preset_browser.as_mut() {
                match &result {
                    Ok(_) => {
                        b.last_error = None;
                        b.invalidate_files();
                    }
                    Err(e) => b.last_error = Some(e.clone()),
                }
            }
            // The Track-tab instrument loader caches its list by name.
            if matches!(kind, PresetKind::Instrument) {
                state.track_preset_cache_key = None;
            }
        }
        if let Some(path) = load_user {
            load_preset(setter, params, pattern, sound_settings, state, kind, &path);
        }
        if let Some(json) = load_factory {
            load_preset_json(setter, params, pattern, sound_settings, state, kind, json);
        }
        if let Some(idx) = apply_builtin_grid {
            let (layout, clear) = match idx {
                0 => (TrackLayoutState::empty_layout(), true),
                1 => (TrackLayoutState::modular_default_layout(), false),
                2 => (TrackLayoutState::preset_12_layout(), false),
                3 => (TrackLayoutState::preset_ac4_layout(), false),
                _ => (TrackLayoutState::preset_ac12_layout(), false),
            };
            apply_lane_layout_preset(
                setter,
                params,
                sound_settings,
                pattern,
                state,
                layout,
                clear,
            );
            if let Some(b) = state.preset_browser.as_mut() {
                b.confirm_clear_all_grid = false;
            }
        }
    }

    // Close when clicking outside the modal.
    let clicked_outside = ui.input(|input| {
        input.pointer.any_pressed()
            && input
                .pointer
                .interact_pos()
                .map_or(false, |pos| !response.rect.contains(pos))
    });
    if clicked_outside {
        state.preset_browser = None;
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

fn save_current(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    kind: PresetKind,
    name: String,
) -> Result<(), String> {
    let result = match kind {
        PresetKind::Instrument => {
            let slot = state.selected_instrument.min(crate::track::MAX_TRACKS - 1);
            let Some(slot_kind) = params.track_layout.state.kind_for_slot(slot) else {
                return Err("no instrument on the selected lane".to_string());
            };
            let algo = params.algos()[slot].value();
            presets::save_instrument(&presets::capture_instrument(
                name,
                slot_kind,
                &sound_settings.instruments[slot],
                algo,
                presets::user_texture_path(slot_kind, slot, &params.user_textures),
            ))
        }
        PresetKind::Pattern => {
            let layout =
                PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
            let mut algos = [0i32; crate::track::MAX_TRACKS];
            for (i, a) in algos.iter_mut().enumerate() {
                *a = params.algos()[i].value();
            }
            presets::save_pattern(&presets::capture_pattern(
                name,
                &layout,
                pattern,
                &params.plock_state.state,
                &params.seq_plock_state.state,
                params.pattern_length.value().clamp(1, 64) as u8,
                sound_settings,
                &algos,
                &params.user_textures,
            ))
        }
        PresetKind::Song => {
            let Ok(bank) = params.pattern_bank.bank.lock() else {
                return Err("pattern bank busy — try again".to_string());
            };
            presets::save_song(&presets::capture_song(name, bank.song))
        }
        PresetKind::Grid => {
            let layout =
                PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
            presets::save_grid(&presets::capture_grid(name, &layout))
        }
    };
    // A newly saved instrument preset must show up in the Track-tab loader.
    if result.is_ok() && matches!(kind, PresetKind::Instrument) {
        state.track_preset_cache_key = None;
    }
    result.map(|_| ())
}

/// Load a user preset file of the active kind.
#[allow(clippy::too_many_arguments)]
fn load_preset(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    kind: PresetKind,
    path: &std::path::Path,
) {
    let Ok(json) = std::fs::read_to_string(path) else {
        return;
    };
    load_preset_json(setter, params, pattern, sound_settings, state, kind, &json);
}

#[allow(clippy::too_many_arguments)]
fn load_preset_json(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    kind: PresetKind,
    json: &str,
) {
    match kind {
        PresetKind::Instrument => {
            let Ok(preset) = serde_json::from_str::<presets::InstrumentPreset>(json) else {
                return;
            };
            apply_instrument(setter, params, sound_settings, state, &preset);
        }
        PresetKind::Pattern => {
            let Ok(preset) = serde_json::from_str::<presets::PatternPreset>(json) else {
                return;
            };
            apply_pattern(params, pattern, sound_settings, state, setter, &preset);
        }
        PresetKind::Song => {
            let Ok(preset) = serde_json::from_str::<presets::SongPreset>(json) else {
                return;
            };
            if let Ok(mut bank) = params.pattern_bank.bank.lock() {
                bank.song = preset.song;
                drop(bank);
                params.pattern_bank.refresh_snapshot();
                params.song_controller.publish(preset.song);
                state.last_published_song = Some(preset.song);
            }
        }
        PresetKind::Grid => {
            let Ok(preset) = serde_json::from_str::<presets::GridPreset>(json) else {
                return;
            };
            let layout = presets::layout_from_kit(&preset.kit);
            apply_lane_layout_preset(
                setter,
                params,
                sound_settings,
                pattern,
                state,
                layout,
                false,
            );
        }
    }
}

/// Apply an instrument preset to the selected slot; switches the lane kind
/// first when the preset targets another instrument.
fn apply_instrument(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    preset: &presets::InstrumentPreset,
) {
    let slot = state.selected_instrument.min(crate::track::MAX_TRACKS - 1);
    apply_instrument_preset_to_slot(setter, params, sound_settings, state, slot, preset);
}

/// Apply an instrument preset onto a specific slot (switching the slot's kind
/// first if it differs). Public so the Track-tab quick loader can reuse it.
pub fn apply_instrument_preset_to_slot(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    slot: usize,
    preset: &presets::InstrumentPreset,
) {
    if slot >= crate::track::MAX_TRACKS {
        return;
    }
    let Some(saved_kind) = TrackInstrumentKind::from_index(preset.kind) else {
        return;
    };
    // [203] [204] A preset captured for a retired voice lands on its
    // replacement. Its VALUES are not written: they were captured against the
    // old voice's parameter shape, and its 32 specials mean something else on
    // the new one. The lane therefore comes up on the replacement's factory
    // sound - the same prudence the pattern-preset path already applies by
    // skipping a lane whose kind does not match.
    let retired = saved_kind.retired_replacement();
    let kind = retired.unwrap_or(saved_kind);
    if params.track_layout.state.kind_for_slot(slot) != Some(kind) {
        change_slot_kind(params, sound_settings, state, slot, kind);
    }
    if retired.is_none() {
        write_slot_sound(
            setter,
            params,
            sound_settings,
            slot,
            kind,
            &preset.standards,
            preset.algo,
            &preset.specials,
            preset.user_texture.as_deref(),
        );
    }
    sound_settings.bump_version();
}

/// Write one lane's captured sound (13 standards + algo + specials) into its
/// slot. Shared by the Instrument-preset and Pattern-preset apply paths.
#[allow(clippy::too_many_arguments)]
fn write_slot_sound(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    slot: usize,
    kind: TrackInstrumentKind,
    standards: &[f32; 13],
    algo: u8,
    specials: &[f32],
    user_texture: Option<&str>,
) {
    use crate::instrument_registry::StandardField as F;
    const ORDER: [F; 13] = [
        F::Freq,
        F::Decay,
        F::Volume,
        F::FilterFreq,
        F::Attack,
        F::Release,
        F::DecayCurve,
        F::ReleaseCurve,
        F::Hold,
        F::FilterEnvAmount,
        F::FilterEnvDecay,
        F::Analog,
        F::Stereo,
    ];
    let inst = &sound_settings.instruments[slot];
    for (field, value) in ORDER.iter().zip(standards.iter()) {
        store_field(inst, *field, *value);
    }
    setter.set_parameter(params.algos()[slot], algo as i32);
    // [228] A preset captured with a custom texture brings its file along:
    // the lane gets the file loaded (or shows "missing" when the file is
    // gone - "Custom" then plays Noise, never silence). A preset without one
    // clears the lane's file: the lane sounds like the preset, not like what
    // it had before.
    let has_texture_menu = kind
        .instrument_def()
        .special_params
        .iter()
        .any(|d| d.name.ends_with("_texture"));
    if has_texture_menu {
        match user_texture {
            Some(path) => {
                // [259] A preset from a third party is an automatic
                // resolution: local files only, network paths go "missing".
                let _ = params
                    .user_textures
                    .load_restored(slot, std::path::Path::new(path));
            }
            None => params.user_textures.clear(slot),
        }
    }
    for (i, def) in kind.instrument_def().special_params.iter().enumerate() {
        if let Some(value) = specials.get(i) {
            inst.set_special(def.special_index, *value);
        }
    }
}

/// [282] The pattern a new instance starts with (kit, steps, p-locks,
/// fusions, sounds). Also seeded as an ordinary user preset ([280]).
const STARTING_PATTERN: &str =
    include_str!("../../assets/presets/default-user/patterns/Tommy01.fdpat.json");

/// [282] Applies [`STARTING_PATTERN`] to a new instance, with its kit. Called
/// once, at the first editor frame of an instance whose `instance_seeded`
/// marker is still false.
pub fn apply_starting_pattern(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    setter: &ParamSetter,
) {
    let Ok(preset) = serde_json::from_str::<presets::PatternPreset>(STARTING_PATTERN) else {
        return;
    };
    apply_pattern(params, pattern, sound_settings, state, setter, &preset);
    state.mark_pattern_dirty();
}

/// Apply a pattern preset; optionally installs the captured lane kit first.
fn apply_pattern(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    setter: &ParamSetter,
    preset: &presets::PatternPreset,
) {
    let with_kit = state
        .preset_browser
        .as_ref()
        .map_or(true, |b| b.load_with_kit);
    if with_kit {
        let mut layout = TrackLayoutState::empty_layout();
        for (i, k) in preset.kit.iter().enumerate() {
            if *k >= 0 {
                if let Some(kind) = TrackInstrumentKind::from_index(*k as usize) {
                    // [286] Lanes 13 and 14 stay reserved for effects.
                    if crate::track::kind_allowed_on_slot(kind, i, None) {
                        layout.slots[i] = TrackSlot::active_with_kind(kind);
                    }
                }
            }
        }
        apply_lane_layout_preset(
            setter,
            params,
            sound_settings,
            pattern,
            state,
            layout,
            false,
        );
    }

    let Ok(plock_bytes) = presets::hex_decode(&preset.plock_hex) else {
        return;
    };
    let Ok(seq_plock_bytes) = presets::hex_decode(&preset.seq_plock_hex) else {
        return;
    };
    let Ok(fusion_bytes) = presets::hex_decode(&preset.fusion_hex) else {
        return;
    };
    restore_from_buffers(
        &preset.step_masks,
        &plock_bytes,
        &seq_plock_bytes,
        &fusion_bytes,
        pattern,
        &params.plock_state.state,
        &params.seq_plock_state.state,
    );
    crate::ui::controls::set_int_param_if_changed(
        setter,
        &params.pattern_length,
        preset.pattern_length.clamp(1, 64) as i32,
    );

    // Restore each captured lane sound onto the slot that still holds its kind.
    // (After a kit install the kinds match; when loading without the kit we skip
    // mismatched lanes so a different instrument is never clobbered.) Presets
    // saved before sounds existed carry an empty list → nothing is touched.
    for snd in &preset.sounds {
        let slot = snd.slot as usize;
        if slot >= crate::track::MAX_TRACKS {
            continue;
        }
        let Some(kind) = TrackInstrumentKind::from_index(snd.kind) else {
            continue;
        };
        if params.track_layout.state.kind_for_slot(slot) != Some(kind) {
            continue;
        }
        write_slot_sound(
            setter,
            params,
            sound_settings,
            slot,
            kind,
            &snd.standards,
            snd.algo,
            &snd.specials,
            snd.user_texture.as_deref(),
        );
    }
    if !preset.sounds.is_empty() {
        sound_settings.bump_version();
    }
}

/// Debug-only: export the current state as a factory preset JSON into the
/// `_factory` staging directory.
#[cfg(debug_assertions)]
fn export_factory(
    params: &DrumFlashParams,
    pattern: &SharedPattern,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
    kind: PresetKind,
    name: String,
) {
    let json = match kind {
        PresetKind::Instrument => {
            let slot = state.selected_instrument.min(crate::track::MAX_TRACKS - 1);
            let Some(slot_kind) = params.track_layout.state.kind_for_slot(slot) else {
                return;
            };
            let algo = params.algos()[slot].value();
            serde_json::to_string_pretty(&presets::capture_instrument(
                name.clone(),
                slot_kind,
                &sound_settings.instruments[slot],
                algo,
                presets::user_texture_path(slot_kind, slot, &params.user_textures),
            ))
        }
        PresetKind::Pattern => {
            let layout =
                PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
            let mut algos = [0i32; crate::track::MAX_TRACKS];
            for (i, a) in algos.iter_mut().enumerate() {
                *a = params.algos()[i].value();
            }
            serde_json::to_string_pretty(&presets::capture_pattern(
                name.clone(),
                &layout,
                pattern,
                &params.plock_state.state,
                &params.seq_plock_state.state,
                params.pattern_length.value().clamp(1, 64) as u8,
                sound_settings,
                &algos,
                &params.user_textures,
            ))
        }
        PresetKind::Song => {
            let Ok(bank) = params.pattern_bank.bank.lock() else {
                return;
            };
            serde_json::to_string_pretty(&presets::capture_song(name.clone(), bank.song))
        }
        PresetKind::Grid => {
            let layout =
                PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
            serde_json::to_string_pretty(&presets::capture_grid(name.clone(), &layout))
        }
    };
    if let Ok(json) = json {
        let _ = presets::export_factory(kind, &name, &json);
    }
}

#[cfg(test)]
mod starting_pattern_tests {
    use super::*;

    /// [282] The embedded starting pattern must parse, keep its 4-lane kit
    /// and carry its sounds: a broken file would silently leave new
    /// instances on the old default.
    #[test]
    fn starting_pattern_parses_with_its_kit_and_sounds() {
        let preset: presets::PatternPreset =
            serde_json::from_str(STARTING_PATTERN).expect("Tommy01 parses");
        assert_eq!(preset.kit.iter().filter(|k| **k >= 0).count(), 4);
        assert_eq!(preset.sounds.len(), 4);
        assert!(preset.step_masks.iter().any(|m| *m != 0));
    }
}
