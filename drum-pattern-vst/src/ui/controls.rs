//! Small param-bound controls and keyboard helpers shared across panels.

use crate::ui::theme::*;
use crate::ui::widgets::{keycap_tex, select_fit_width, styled_select, KeycapState, ToggleLED};
use crate::DrumFlashParams;
use nih_plug::prelude::*;
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};

pub fn set_bool_param_if_changed(setter: &ParamSetter, param: &BoolParam, value: bool) {
    if param.value() != value {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
}

pub fn set_float_param_if_changed(setter: &ParamSetter, param: &FloatParam, value: f32) {
    if (param.value() - value).abs() > f32::EPSILON {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
}

pub fn set_int_param_if_changed(setter: &ParamSetter, param: &IntParam, value: i32) {
    if param.value() != value {
        setter.begin_set_parameter(param);
        setter.set_parameter(param, value);
        setter.end_set_parameter(param);
    }
}

/// Reset every lane's mute/solo to off. Used by the destructive clears
/// (header Clear All, lane layout presets, style presets, lane delete) so no
/// invisible mute/solo survives on a deactivated lane and keeps muting the
/// rest of the kit.
pub fn clear_all_mutes_solos(setter: &ParamSetter, params: &DrumFlashParams) {
    for i in 0..crate::track::MAX_TRACKS {
        set_bool_param_if_changed(setter, params.mutes()[i], false);
        set_bool_param_if_changed(setter, params.solos()[i], false);
    }
}

pub fn draw_track_length_control(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    params: &DrumFlashParams,
    length_param: &IntParam,
    instrument: usize,
    master_length: usize,
) -> bool {
    let locked = params.lane_length_locks.is_locked(instrument);
    let raw = length_param.value() as usize;
    let mut length_value = if locked {
        raw as i32
    } else {
        master_length as i32
    };

    let response = ui.add_sized(
        Vec2::new(35.0, 20.0),
        egui::DragValue::new(&mut length_value)
            .speed(1.0)
            .range(1..=64),
    );
    let changed = response.changed();
    let response = crate::ui::help::HelpExt::help_or(
        response,
        crate::ui::help::LANE_LEN,
        if locked {
            "Locked lane length. Right-click to follow pattern length."
        } else {
            "Follows pattern length. Drag to lock this lane."
        },
    );
    let interacted =
        changed || response.clicked() || response.dragged() || response.secondary_clicked();

    response.context_menu(|ui| {
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.set_min_width(150.0);
        ui.set_max_width(150.0);
        if locked {
            if crate::ui::menus::context_menu_button(ui, "Follow pattern length", INK(), true)
                .clicked()
            {
                params.lane_length_locks.set_locked(instrument, false);
                setter.set_parameter(length_param, master_length as i32);
                ui.close_menu();
            }
        } else {
            ui.label(
                RichText::new("Already follows pattern length")
                    .font(f_sans_med(10.5))
                    .color(INK3()),
            );
        }
    });

    if changed {
        params.lane_length_locks.set_locked(instrument, true);
        setter.set_parameter(length_param, length_value.clamp(1, 64));
    }

    interacted
}

pub fn fusion_modifier_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| i.modifiers.shift) || platform_shift_pressed()
}

/// [282] Ctrl (Cmd on macOS) + click on a grid cell: point the Lane Editor at
/// that cell without turning it on or off and without opening its menu. Same
/// platform fallback as the other modifiers (hosts that swallow keys).
pub fn select_cell_modifier_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| i.modifiers.command) || platform_ctrl_pressed()
}

/// Fine-tune modifier for sliders ([181]). Accepts **Shift or Alt** — the
/// gesture is documented as Alt but Shift was what the plock slider used, so
/// both work rather than making the user guess.
///
/// Like [`fusion_modifier_pressed`], it falls back to a platform-level key check:
/// hosts that swallow keyboard events (Studio One, REAPER) never let egui see the
/// modifier state, which is exactly how the fine-tune silently stopped working.
pub fn fine_tune_modifier_pressed(ui: &egui::Ui) -> bool {
    ui.input(|i| i.modifiers.shift || i.modifiers.alt)
        || platform_shift_pressed()
        || platform_alt_pressed()
}

#[cfg(target_os = "windows")]
fn platform_shift_pressed() -> bool {
    const VK_SHIFT: i32 = 0x10;
    const VK_LSHIFT: i32 = 0xA0;
    const VK_RSHIFT: i32 = 0xA1;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    unsafe {
        [VK_SHIFT, VK_LSHIFT, VK_RSHIFT]
            .iter()
            .any(|&key| (GetAsyncKeyState(key) as u16 & 0x8000) != 0)
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_shift_pressed() -> bool {
    false
}

#[cfg(target_os = "windows")]
fn platform_ctrl_pressed() -> bool {
    const VK_CONTROL: i32 = 0x11;
    const VK_LCONTROL: i32 = 0xA2;
    const VK_RCONTROL: i32 = 0xA3;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    unsafe {
        [VK_CONTROL, VK_LCONTROL, VK_RCONTROL]
            .iter()
            .any(|&key| (GetAsyncKeyState(key) as u16 & 0x8000) != 0)
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_ctrl_pressed() -> bool {
    false
}

#[cfg(target_os = "windows")]
fn platform_alt_pressed() -> bool {
    const VK_MENU: i32 = 0x12;
    const VK_LMENU: i32 = 0xA4;
    const VK_RMENU: i32 = 0xA5;

    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetAsyncKeyState(vkey: i32) -> i16;
    }

    unsafe {
        [VK_MENU, VK_LMENU, VK_RMENU]
            .iter()
            .any(|&key| (GetAsyncKeyState(key) as u16 & 0x8000) != 0)
    }
}

#[cfg(not(target_os = "windows"))]
fn platform_alt_pressed() -> bool {
    false
}

pub fn toggle_led_param(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &BoolParam,
    label: &str,
    enabled: bool,
) {
    let value = param.value();
    if ui
        .add(ToggleLED::new_enabled(label, value, enabled))
        .clicked()
    {
        let new_value = !value;
        setter.begin_set_parameter(param);
        setter.set_parameter(param, new_value);
        setter.end_set_parameter(param);
    }
}

pub fn enum_combo<E: nih_plug::prelude::Enum + PartialEq + 'static>(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &EnumParam<E>,
    label: &str,
) {
    let current = param.value();
    let current_idx = current.to_index();
    let variants = E::variants();
    if !label.is_empty() {
        ui.label(RichText::new(label).font(f_sans_sb(10.5)).color(INK3()));
    }
    if let (_, Some(i)) = styled_select(ui, ("enum_combo", label), current_idx, variants, 116.0) {
        if i != current_idx {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, E::from_index(i));
            setter.end_set_parameter(param);
        }
    }
}

pub fn enum_combo_compact<E: nih_plug::prelude::Enum + PartialEq + 'static>(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &EnumParam<E>,
    id: &'static str,
    width: f32,
) {
    let current = param.value();
    let current_idx = current.to_index();
    let variants = E::variants();
    let width = select_fit_width(ui, variants, width);
    if let (_, Some(i)) = styled_select(ui, id, current_idx, variants, width) {
        if i != current_idx {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, E::from_index(i));
            setter.end_set_parameter(param);
        }
    }
}

/// Hover/press feedback overlay for custom-painted keycaps (egui::Button gives
/// this for free; our hand-painted keycaps need it explicitly).
fn keycap_feedback(painter: &egui::Painter, rect: egui::Rect, resp: &egui::Response) {
    // Pressed-in look only (no hover highlight, per user preference).
    if resp.is_pointer_button_down_on() {
        painter.rect_filled(rect, RADIUS_CTL, Color32::from_black_alpha(60));
    }
}

/// Unified keycap button: baked texture background (rest/blue/amber) + centred
/// label, sized like an egui `Button::min_size` (grows with the label). Used for
/// pages, length/x2, pattern slots, tabs, GENERATE — everything that should read
/// as a hardware key. Disabled buttons dim and stop sensing clicks.
pub fn keycap_button(
    ui: &mut egui::Ui,
    label: &str,
    min_w: f32,
    state: KeycapState,
    enabled: bool,
    font: egui::FontId,
) -> egui::Response {
    let tw = ui.fonts(|f| {
        f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    });
    let w = (tw + 18.0).max(min_w);
    keycap_button_sized(ui, label, Vec2::new(w, CTL_HEIGHT), state, enabled, font)
}

/// [`keycap_button`] at an exact size (the Generator's tall GENERATE key).
pub fn keycap_button_sized(
    ui: &mut egui::Ui,
    label: &str,
    size: Vec2,
    state: KeycapState,
    enabled: bool,
    font: egui::FontId,
) -> egui::Response {
    let sense = if enabled {
        egui::Sense::click()
    } else {
        egui::Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(size, sense);
    keycap_tex(ui, rect, state);
    if enabled {
        keycap_feedback(ui.painter(), rect, &resp);
    }
    let base = match state {
        KeycapState::Rest => INK_KEYCAP,
        KeycapState::PressedBlue => Color32::from_rgb(234, 246, 255),
        KeycapState::PressedAmber => Color32::from_rgb(255, 240, 214),
    };
    let text_color = if enabled {
        base
    } else {
        // dim the whole key and grey the label
        ui.painter()
            .rect_filled(rect, RADIUS_CTL, Color32::from_black_alpha(80));
        Color32::from_rgb(118, 119, 126)
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        text_color,
    );
    resp
}

/// The Generator's **Full random** key: a die, then the label, in amber, centred
/// together on a rest keycap of `size`.
pub fn full_random_button(ui: &mut egui::Ui, size: Vec2) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    keycap_tex(ui, rect, KeycapState::Rest);
    keycap_feedback(ui.painter(), rect, &resp);
    let font = f_sans_sb(11.0);
    let label = "Full random";
    let tw = ui.fonts(|f| f.layout_no_wrap(label.to_string(), font.clone(), AMBER()).size().x);
    let (icon, gap) = (13.0, 6.0);
    let x = rect.center().x - (icon + gap + tw) * 0.5;
    crate::ui::skeuo::dice_icon(
        ui.painter(),
        egui::Rect::from_min_size(egui::pos2(x, rect.center().y - icon * 0.5), Vec2::splat(icon)),
        AMBER(),
    );
    ui.painter().text(
        egui::pos2(x + icon + gap, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        AMBER(),
    );
    resp
}

pub fn chip_button(
    ui: &mut egui::Ui,
    label: &str,
    accent: bool,
    color: Color32,
    sense: egui::Sense,
) -> egui::Response {
    // Keycap look: momentary action buttons stay at "rest"; the accent (if any)
    // only tints the label (e.g. orange for Random).
    let font = f_sans_sb(11.0);
    let tw = ui.fonts(|f| {
        f.layout_no_wrap(label.to_string(), font.clone(), Color32::WHITE)
            .size()
            .x
    });
    let w = tw + 20.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, CTL_HEIGHT), sense);
    keycap_tex(ui, rect, KeycapState::Rest);
    keycap_feedback(ui.painter(), rect, &resp);
    let text_color = if accent { color } else { INK_KEYCAP };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        font,
        text_color,
    );
    resp
}

pub fn algo_combo(ui: &mut egui::Ui, setter: &ParamSetter, param: &IntParam, algo_names: &[&str]) {
    let current = param.value() as usize;
    let current_clamped = current.min(algo_names.len().saturating_sub(1));
    if let (_, Some(i)) = styled_select(ui, "algo_combo", current_clamped, algo_names, 146.0) {
        if i != current_clamped {
            setter.begin_set_parameter(param);
            setter.set_parameter(param, i as i32);
            setter.end_set_parameter(param);
        }
    }
}

pub fn p_lock_mode_segmented(ui: &mut egui::Ui, selected: usize) -> usize {
    crate::ui::skeuo::segmented(ui, "plock_mode", &["Sound", "Sequencer"], selected)
}

pub fn generator_song_segmented(ui: &mut egui::Ui, selected: usize) -> usize {
    crate::ui::skeuo::segmented(ui, "gen_song_mode", &["Generator", "Song"], selected)
}

/// [283] How long a two-click "Sure?" button stays armed (same as the Lane
/// Editor's Default).
pub const CONFIRM_TIMEOUT_S: f64 = 3.0;

/// Two-click confirmations: whether the button armed at `armed_at` is still
/// armed. Call before drawing it; past [`CONFIRM_TIMEOUT_S`] it disarms, and
/// while armed a repaint is scheduled so its label reverts on its own.
pub fn confirm_armed(ui: &egui::Ui, armed_at: &mut Option<f64>) -> bool {
    let now = ui.input(|i| i.time);
    if let Some(t) = *armed_at {
        let left = CONFIRM_TIMEOUT_S - (now - t);
        if left <= 0.0 {
            *armed_at = None;
        } else {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
    }
    armed_at.is_some()
}

/// Call after drawing an armed button that was not clicked: a press anywhere
/// else disarms it.
pub fn confirm_disarm_elsewhere(
    ui: &egui::Ui,
    armed_at: &mut Option<f64>,
    button: &egui::Response,
) {
    if armed_at.is_some() && !button.hovered() && ui.input(|i| i.pointer.any_pressed()) {
        *armed_at = None;
    }
}

#[cfg(test)]
mod confirm_tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
        armed: &mut Option<f64>,
    ) -> bool {
        let input = egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(400.0, 300.0),
            )),
            events,
            ..Default::default()
        };
        let mut still = false;
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                still = confirm_armed(ui, armed);
                let button = ui.add_sized(egui::vec2(80.0, 30.0), egui::Button::new("Sure?"));
                if !button.clicked() {
                    confirm_disarm_elsewhere(ui, armed, &button);
                }
            });
        });
        still
    }

    #[test]
    fn an_armed_confirm_disarms_after_the_timeout() {
        let ctx = egui::Context::default();
        let mut armed = Some(10.0);
        assert!(frame(&ctx, 12.0, vec![], &mut armed));
        assert!(!frame(&ctx, 13.1, vec![], &mut armed));
        assert_eq!(armed, None);
    }

    #[test]
    fn an_armed_confirm_disarms_on_a_click_elsewhere() {
        let ctx = egui::Context::default();
        let mut armed = Some(10.0);
        let far = egui::pos2(300.0, 250.0);
        frame(&ctx, 10.1, vec![egui::Event::PointerMoved(far)], &mut armed);
        let press = egui::Event::PointerButton {
            pos: far,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, 10.2, vec![press], &mut armed);
        assert_eq!(armed, None);
    }
}
