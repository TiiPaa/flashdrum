//! Menu chrome: frames, headers and rows shared by the plock, page, settings
//! and add-module popups.

use crate::ui::theme::*;
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};

pub fn plock_menu_frame(ui: &mut egui::Ui, accent: Color32, content: impl FnOnce(&mut egui::Ui)) {
    let _ = accent; // the accent now lives in the header (title + underline), not a top bar
    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    ui.visuals_mut().widgets.noninteractive.corner_radius = egui::CornerRadius::same(0);

    // Reserve a slot for the skeuo plate, painted BEHIND the content once its
    // final rect is known (relief + border + liseré + soft shadow).
    let bg = ui.painter().add(egui::Shape::Noop);
    let resp = egui::Frame::NONE
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(280.0);
            ui.set_max_width(350.0);
            content(ui);
        });
    ui.painter().set(
        bg,
        crate::ui::skeuo::plate_shape(resp.response.rect, RADIUS_PANEL as f32),
    );
}

pub fn page_menu_frame(ui: &mut egui::Ui, accent: Color32, content: impl FnOnce(&mut egui::Ui)) {
    let _ = accent;
    ui.visuals_mut().widgets.noninteractive.bg_stroke = egui::Stroke::NONE;
    ui.visuals_mut().widgets.noninteractive.corner_radius = egui::CornerRadius::same(0);

    let bg = ui.painter().add(egui::Shape::Noop);
    let resp = egui::Frame::NONE
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_min_width(130.0);
            ui.set_max_width(160.0);
            content(ui);
        });
    ui.painter().set(
        bg,
        crate::ui::skeuo::plate_shape(resp.response.rect, RADIUS_PANEL as f32),
    );
}

pub fn plock_menu_header(ui: &mut egui::Ui, title: &str, _step: usize, accent: Color32) -> bool {
    let mut close_clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).font(f_sans_sb(11.0)).color(accent));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // Discreet painted ✓ ("done" — plock changes are applied live, so this
            // is NOT a cancel; a × would wrongly suggest discarding).
            let (rect, response) =
                ui.allocate_exact_size(egui::Vec2::new(18.0, 18.0), egui::Sense::click());
            let response =
                crate::ui::help::HelpExt::help(response, crate::ui::help::STEP_MENU_DONE);
            let col = if response.hovered() { accent } else { INK2() };
            let c = rect.center();
            let s = 5.0;
            let st = egui::Stroke::new(2.0, col);
            ui.painter().line_segment(
                [
                    egui::pos2(c.x - s, c.y + s * 0.1),
                    egui::pos2(c.x - s * 0.25, c.y + s * 0.8),
                ],
                st,
            );
            ui.painter().line_segment(
                [
                    egui::pos2(c.x - s * 0.25, c.y + s * 0.8),
                    egui::pos2(c.x + s, c.y - s * 0.8),
                ],
                st,
            );
            if response.clicked() {
                close_clicked = true;
            }
        });
    });
    ui.add_space(3.0);
    // Thin accent underline.
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(
        r,
        0.0,
        Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), 90),
    );
    ui.add_space(6.0);
    close_clicked
}

pub fn page_menu_header(ui: &mut egui::Ui, title: &str, accent: Color32) {
    ui.label(RichText::new(title).font(f_sans_sb(11.0)).color(accent));
    ui.add_space(4.0);
}

pub fn plock_menu_row(
    ui: &mut egui::Ui,
    label: &str,
    accent: Color32,
    overridden: bool,
    value_text: Option<&str>,
    content: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.set_height(22.0);
        let label_color = if overridden { accent } else { INK3() };
        ui.label(
            RichText::new(label)
                .font(f_sans_med(10.5))
                .color(label_color),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let response = content(ui);
            if let Some(text) = value_text {
                ui.add_space(8.0);
                ui.allocate_ui_with_layout(
                    Vec2::new(48.0, 22.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        ui.label(RichText::new(text).font(f_mono_med(10.0)).color(INK2()));
                    },
                );
            }
            response
        })
        .inner
    })
    .inner
}

pub fn plock_menu_action_row(ui: &mut egui::Ui, label: &str, accent: Color32) -> egui::Response {
    // Full-width keycap with the accent-coloured label (red for destructive, etc.).
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), egui::Sense::click());
    crate::ui::skeuo::keycap(ui, rect, crate::ui::widgets::KeycapState::Rest);
    if resp.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, RADIUS_CTL, Color32::from_black_alpha(60));
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        f_sans_med(10.5),
        accent,
    );
    resp
}

/// Full-width keycap row for the egui-native context menus (lane name, empty
/// lane, lane length, song block). Returns the `Response` so callers can chain
/// `.on_hover_text(...)`. `enabled` dims the label and drops the hover/press
/// feedback; the caller still gates its action on the same condition.
pub fn context_menu_button(
    ui: &mut egui::Ui,
    label: &str,
    accent: Color32,
    enabled: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), egui::Sense::click());
    crate::ui::skeuo::keycap(ui, rect, crate::ui::widgets::KeycapState::Rest);
    // Hover highlight: the user must see which row they are about to pick.
    let hovered = enabled && resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect, RADIUS_CTL, P_HOVER());
    }
    if enabled && resp.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, RADIUS_CTL, Color32::from_black_alpha(60));
    }
    let col = if !enabled {
        INK3()
    } else if hovered {
        Color32::WHITE
    } else {
        accent
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        f_sans_med(10.5),
        col,
    );
    resp
}

/// Flat variant of `context_menu_button` (no 3D keycap) — used inside the
/// nested Instrument ▸ Category ▸ kind submenus where keycaps are too heavy.
/// Same interaction contract: hover highlight + label color, disabled rows
/// dim without feedback.
pub fn context_menu_row_plain(
    ui: &mut egui::Ui,
    label: &str,
    accent: Color32,
    enabled: bool,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::click());
    let hovered = enabled && resp.hovered();
    if hovered {
        ui.painter().rect_filled(rect, RADIUS_CTL, P_HOVER());
    }
    let col = if !enabled {
        INK3()
    } else if hovered {
        Color32::WHITE
    } else {
        accent
    };
    ui.painter().text(
        rect.left_center() + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        f_sans_med(10.5),
        col,
    );
    resp
}

/// Shared instrument picker used EVERYWHERE a lane's kind is chosen (the
/// empty-lane Add-Module popup, the lane right-click menu, and the Track-tab
/// Type field): one cascading submenu per category (BD/SD/HH/PERC/FX/OTHER),
/// each listing its kinds. The current kind (if any) is prefixed "> " and
/// highlighted. Returns the picked kind, or `None` if nothing was chosen this
/// frame. Keeping this in one place is what makes the three selectors identical.
pub fn instrument_category_menu(
    ui: &mut egui::Ui,
    current: Option<crate::track::TrackInstrumentKind>,
) -> Option<crate::track::TrackInstrumentKind> {
    use crate::track::{InstrumentCategory, TrackInstrumentKind};
    let mut picked = None;
    for cat in InstrumentCategory::ALL {
        ui.menu_button(
            RichText::new(cat.label())
                .font(f_sans_med(10.5))
                .color(INK()),
            |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.set_min_width(130.0);
                ui.set_max_width(130.0);
                for kind in TrackInstrumentKind::kinds_in(cat) {
                    let is_current = Some(kind) == current;
                    let label = if is_current {
                        format!("> {}", kind.default_name())
                    } else {
                        kind.default_name().to_string()
                    };
                    if context_menu_row_plain(
                        ui,
                        &label,
                        if is_current { BLUE() } else { INK() },
                        !is_current,
                    )
                    .clicked()
                        && !is_current
                    {
                        picked = Some(kind);
                        // Custom rows don't trigger egui's auto-close — do it here
                        // so every caller gets the same behaviour.
                        ui.close_menu();
                    }
                }
            },
        );
    }
    picked
}

/// Faint separator line used to group items inside a context menu.
pub fn context_menu_separator(ui: &mut egui::Ui) {
    ui.add_space(2.0);
    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, LINE());
    ui.add_space(2.0);
}
