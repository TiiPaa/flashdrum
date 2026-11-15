//! Fixed-height top rows of the Lane Editor. Each row is allocated once;
//! independent child UIs cannot move the following row or their neighbours.

use nih_plug_egui::egui::{self, Align, Layout, Rect, Sense, Ui, UiBuilder, Vec2};

use super::theme::CTL_HEIGHT;

const SIDE_INSET: f32 = 14.0;
const ROW_SPACING: f32 = 3.0;
/// Space kept free beside the scrollable modules, also respected by the banner
/// and by Volume's readout so all three use the same inner right boundary.
pub(crate) const SCROLLBAR_RESERVE: f32 = 20.0;

fn allocate_row(ui: &mut Ui) -> Rect {
    ui.add_space(ROW_SPACING);
    let rect = ui
        .allocate_exact_size(Vec2::new(ui.available_width(), CTL_HEIGHT), Sense::hover())
        .0;
    ui.add_space(ROW_SPACING);
    rect
}

/// Actions are anchored on the parameter labels' x. The morph selector is
/// right-aligned independently and never participates in Volume's layout.
pub(crate) fn action_row(ui: &mut Ui, actions_width: f32, gutter_width: f32) -> (Ui, Ui) {
    let row = allocate_row(ui);
    let left = row.left() + SIDE_INSET + gutter_width;
    let actions = Rect::from_min_size(egui::pos2(left, row.top()), Vec2::new(actions_width, row.height()));
    let morph = Rect::from_min_max(
        egui::pos2((actions.right() + 12.0).min(row.right() - SIDE_INSET), row.top()),
        egui::pos2(row.right() - SIDE_INSET, row.bottom()),
    );
    // The existing action code emits Default, Restore, Store. RTL preserves
    // their visible Store / Restore / Default order, anchored to the left.
    let mut actions_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("lane_editor_actions")
            .max_rect(actions)
            .layout(Layout::right_to_left(Align::Center)),
    );
    actions_ui.spacing_mut().item_spacing.x = 8.0;
    let morph_ui = ui.new_child(
        UiBuilder::new()
            .id_salt("lane_editor_morph")
            .max_rect(morph)
            .layout(Layout::right_to_left(Align::Center)),
    );
    (actions_ui, morph_ui)
}

/// Full-width parameter row: the regular gutter/label/readout sizes are kept
/// by the caller; only the slider gains the space freed by the upper row.
pub(crate) fn volume_row(ui: &mut Ui) -> Ui {
    let row = allocate_row(ui);
    let content = Rect::from_min_max(
        row.left_top() + Vec2::new(SIDE_INSET, 2.0),
        row.right_bottom() - Vec2::new(SIDE_INSET + SCROLLBAR_RESERVE, 2.0),
    );
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt("lane_editor_volume")
            .max_rect(content)
            .layout(Layout::top_down(Align::Min)),
    );
    // Nested horizontal() rows otherwise start at 18 px and shift the 22 px
    // controls vertically relative to the upper row's 26 px keycaps.
    child.spacing_mut().interact_size.y = 22.0;
    child
}
