use crate::ui::theme::*;
use nih_plug_egui::egui::{self, Color32, Response, Sense, StrokeKind, Ui, Vec2, Widget};
use std::hash::Hash;

pub fn hover_t(ctx: &egui::Context, id: egui::Id, hovered: bool) -> f32 {
    ctx.animate_value_with_time(id, if hovered { 1.0 } else { 0.0 }, 0.14)
}

/// Vertical gradient approximated by stacked horizontal bands — egui has no
/// native gradient. Recipe from the designer's `skeuo_widgets.rs::vgrad`:
/// a rounded base rect in the bottom colour, then N inner bands top→bottom
/// (inset 1px so the rounded corners stay clean). `stops` = (position 0..1, colour).
pub fn vgrad(painter: &egui::Painter, rect: egui::Rect, stops: &[(f32, Color32)], radius: f32) {
    let base = stops.last().map(|s| s.1).unwrap_or(Color32::BLACK);
    painter.rect_filled(rect, egui::epaint::CornerRadius::same(radius as u8), base);
    let inner = rect.shrink(1.0);
    let r = radius.max(0.0);
    let n = 12;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let c = vgrad_sample(stops, t);
        let y0 = inner.top() + inner.height() * t;
        let y1 = inner.top() + inner.height() * (i as f32 + 1.0) / n as f32;
        // Follow the rounded corners so the square bands don't poke out past the
        // rounded base (which showed as light pixels at the corners).
        let d = (y0 - inner.top()).min(inner.bottom() - y1).max(0.0);
        let inset = if d < r {
            r - (r * r - (r - d) * (r - d)).sqrt()
        } else {
            0.0
        };
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(inner.left() + inset, y0),
                egui::pos2(inner.right() - inset, y1),
            ),
            egui::epaint::CornerRadius::ZERO,
            c,
        );
    }
}

/// Keycap visual state → vector skeuo keycap (rest grey / pressed blue / amber).
#[derive(Clone, Copy)]
pub enum KeycapState {
    Rest,
    PressedBlue,
    PressedAmber,
}

// ============================================================
// Skeuo vector primitives (validated in the egui lab, rendered by the same
// egui pipeline the plugin uses — smooth gradients, soft shadows, no banding).
// ============================================================

/// Smooth 3-stop vertical gradient via a colour-per-vertex mesh (no banding).
/// `mid_t` is the middle stop position (0..1).
pub fn grad3(
    p: &egui::Painter,
    rect: egui::Rect,
    ctop: Color32,
    cmid: Color32,
    cbot: Color32,
    mid_t: f32,
) {
    let ym = rect.top() + rect.height() * mid_t;
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let mut m = egui::epaint::Mesh::default();
    m.colored_vertex(egui::pos2(l, t), ctop);
    m.colored_vertex(egui::pos2(r, t), ctop);
    m.colored_vertex(egui::pos2(l, ym), cmid);
    m.colored_vertex(egui::pos2(r, ym), cmid);
    m.colored_vertex(egui::pos2(l, b), cbot);
    m.colored_vertex(egui::pos2(r, b), cbot);
    m.add_triangle(0, 1, 3);
    m.add_triangle(0, 3, 2);
    m.add_triangle(2, 3, 5);
    m.add_triangle(2, 5, 4);
    p.add(egui::Shape::mesh(m));
}

/// Soft drop shadow: stacked rounded rects with a quadratic alpha falloff (a real
/// gradient halo, unlike egui's coarse single-ramp `Shadow`). `dy` = downward
/// offset, `reach` = px spread, `peak` = inner alpha.
pub fn soft_shadow(
    p: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    dy: f32,
    reach: f32,
    peak: f32,
) {
    let steps = 10;
    for i in (1..=steps).rev() {
        let t = i as f32 / steps as f32;
        let grow = reach * t;
        let a = peak * (1.0 - t) * (1.0 - t);
        let rr = rect.translate(egui::vec2(0.0, dy)).expand(grow);
        p.rect_filled(
            rr,
            radius + grow,
            Color32::from_black_alpha(a.round().clamp(0.0, 255.0) as u8),
        );
    }
}

/// Thin line whose alpha fades to 0 at both ends (centre-peaked) — for the relief
/// liseré and the domed inner shadow.
pub fn fade_line(p: &egui::Painter, rect: egui::Rect, rgb: (u8, u8, u8), peak: u8) {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let cx = rect.center().x;
    let c0 = Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, 0);
    let cc = Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, peak);
    let mut m = egui::epaint::Mesh::default();
    m.colored_vertex(egui::pos2(l, t), c0);
    m.colored_vertex(egui::pos2(cx, t), cc);
    m.colored_vertex(egui::pos2(r, t), c0);
    m.colored_vertex(egui::pos2(l, b), c0);
    m.colored_vertex(egui::pos2(cx, b), cc);
    m.colored_vertex(egui::pos2(r, b), c0);
    m.add_triangle(0, 1, 4);
    m.add_triangle(0, 4, 3);
    m.add_triangle(1, 2, 5);
    m.add_triangle(1, 5, 4);
    p.add(egui::Shape::mesh(m));
}

/// Horizontal inset at depth `d` from an edge to stay inside a corner of radius
/// `r` (so square shadow bands don't poke past rounded corners).
pub fn arc_inset(r: f32, d: f32) -> f32 {
    if d < r {
        (r - (r * r - (r - d) * (r - d)).max(0.0).sqrt()).max(0.0)
    } else {
        0.0
    }
}

/// Recess wall shadow along the TOP edge: fades down, hugs the rounded corners.
pub fn inner_top_shadow(p: &egui::Painter, rect: egui::Rect, radius: f32, height: f32, peak: u8) {
    let n = 12;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let y0 = rect.top() + 1.0 + height * t;
        let y1 = rect.top() + 1.0 + height * (i as f32 + 1.0) / n as f32;
        let a = (peak as f32 * (1.0 - t)).round().max(0.0) as u8;
        if a == 0 {
            continue;
        }
        let ins = arc_inset(radius, (y0 - rect.top()).max(0.0)).max(1.0);
        let x0 = rect.left() + ins;
        let x1 = (rect.right() - ins).max(x0);
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(x0, y0), egui::pos2(x1, y1)),
            egui::epaint::CornerRadius::ZERO,
            Color32::from_black_alpha(a),
        );
    }
}

/// Recess wall shadows on the LEFT + RIGHT edges: fade inward, hug the corners.
pub fn inner_side_shadows(p: &egui::Painter, rect: egui::Rect, radius: f32, width: f32, peak: u8) {
    let n = 8;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let a = (peak as f32 * (1.0 - t)).round().max(0.0) as u8;
        if a == 0 {
            continue;
        }
        let d0 = width * t;
        let d1 = width * (i as f32 + 1.0) / n as f32;
        let ins = arc_inset(radius, d0.max(0.5)).max(1.0);
        let yt = rect.top() + ins;
        let yb = (rect.bottom() - ins).max(yt);
        let col = Color32::from_black_alpha(a);
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 1.0 + d0, yt),
                egui::pos2(rect.left() + 1.0 + d1, yb),
            ),
            egui::epaint::CornerRadius::ZERO,
            col,
        );
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.right() - 1.0 - d1, yt),
                egui::pos2(rect.right() - 1.0 - d0, yb),
            ),
            egui::epaint::CornerRadius::ZERO,
            col,
        );
    }
}

/// Linear RGB lerp between two colours.
pub fn lerp_c(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Even highlight/shadow line: full `peak` across the middle, fading to 0 only
/// near the ends (avoids the centre "point" a centre-peaked fade makes on pads).
pub fn plateau_line(p: &egui::Painter, rect: egui::Rect, rgb: (u8, u8, u8), peak: u8) {
    let e = (rect.width() * 0.22).min(6.0);
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let c0 = Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, 0);
    let cc = Color32::from_rgba_unmultiplied(rgb.0, rgb.1, rgb.2, peak);
    let mut m = egui::epaint::Mesh::default();
    for (x, c) in [(l, c0), (l + e, cc), (r - e, cc), (r, c0)] {
        m.colored_vertex(egui::pos2(x, t), c);
    }
    for (x, c) in [(l, c0), (l + e, cc), (r - e, cc), (r, c0)] {
        m.colored_vertex(egui::pos2(x, b), c);
    }
    for q in 0..3u32 {
        m.add_triangle(q, q + 1, q + 5);
        m.add_triangle(q, q + 5, q + 4);
    }
    p.add(egui::Shape::mesh(m));
}

/// Thin relay to the single keycap renderer, [`crate::ui::skeuo::keycap`]. Kept
/// for the existing call sites — all keycap visuals live in one place now.
pub fn keycap_tex(ui: &egui::Ui, rect: egui::Rect, state: KeycapState) {
    crate::ui::skeuo::keycap(ui, rect, state);
}

fn vgrad_sample(stops: &[(f32, Color32)], t: f32) -> Color32 {
    let mut prev = stops[0];
    for &s in stops {
        if t <= s.0 {
            let span = (s.0 - prev.0).max(1e-4);
            // Gamma-correct interpolation (designer's `lerp_to_gamma`): linear
            // lerp muddied the mid-tones and read as "flat/ugly".
            return prev
                .1
                .lerp_to_gamma(s.1, ((t - prev.0) / span).clamp(0.0, 1.0));
        }
        prev = s;
    }
    stops.last().map(|s| s.1).unwrap_or(Color32::BLACK)
}

// ============================================================
// ToggleSwitch — 34×18 r10
// ============================================================
pub struct ToggleSwitch {
    on: bool,
}

impl ToggleSwitch {
    pub fn new(on: bool) -> Self {
        Self { on }
    }
}

impl Widget for ToggleSwitch {
    fn ui(self, ui: &mut Ui) -> Response {
        let desired_size = Vec2::new(34.0, 18.0);
        let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());

        if response.clicked() {
            ui.ctx().request_repaint();
        }

        // Animated on/off state (0.14s) — the skeuo switch slides + fades.
        let on_t = ui.ctx().animate_value_with_time(
            response.id.with("state"),
            if self.on { 1.0 } else { 0.0 },
            0.14,
        );
        crate::ui::skeuo::switch(ui, rect, on_t);

        response
    }
}

// ============================================================
// ToggleLED — pilule h26 r7, LED Ø7
// ============================================================
pub struct ToggleLED {
    label: String,
    on: bool,
    enabled: bool,
}

impl ToggleLED {
    pub fn new_enabled(label: &str, on: bool, enabled: bool) -> Self {
        Self {
            label: label.to_owned(),
            on,
            enabled,
        }
    }
}

impl Widget for ToggleLED {
    fn ui(self, ui: &mut Ui) -> Response {
        let on = self.on;
        let enabled = self.enabled;
        let font = f_sans_sb(11.0);
        let text_w = ui.fonts(|f| {
            f.layout_no_wrap(self.label.clone(), font.clone(), INK())
                .size()
                .x
        });
        // padding 12 + LED 7 + gap 7 + text + padding 12
        let w = 12.0 + 7.0 + 7.0 + text_w + 12.0;
        let (rect, response) = ui.allocate_exact_size(
            Vec2::new(w, CTL_HEIGHT),
            if enabled {
                Sense::click()
            } else {
                Sense::hover()
            },
        );

        if enabled && response.clicked() {
            ui.ctx().request_repaint();
        }

        // Keycap pill (grey key) — the background does NOT change with state; the
        // backlit LED is what signals on/off, like a hardware indicator button.
        crate::ui::skeuo::keycap(ui, rect, KeycapState::Rest);
        let painter = ui.painter_at(rect);
        if !enabled {
            painter.rect_filled(
                rect,
                RADIUS_CTL,
                Color32::from_rgba_unmultiplied(34, 35, 40, 150),
            );
        }

        // Backlit LED (no halo): blue when on, dark when off/disabled.
        let led_center = egui::pos2(rect.left() + 12.0 + 3.5, rect.center().y);
        crate::ui::skeuo::led(&painter, led_center, 3.5, on && enabled);

        let text_color = if !enabled {
            INK2()
        } else if on {
            INK()
        } else {
            INK2()
        };
        painter.text(
            egui::pos2(rect.left() + 12.0 + 7.0 + 7.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            &self.label,
            font,
            text_color,
        );

        response
    }
}

// ============================================================
// Styled select — maquette `.selbox` (h26, r6, mono current)
// ============================================================
/// [282] `width`, widened so the longest option fits beside the caret
/// whatever the text size (one width for all, so a pick never shifts layout).
pub fn select_fit_width(ui: &Ui, options: &[&str], width: f32) -> f32 {
    let longest = ui.fonts(|fonts| {
        options
            .iter()
            .map(|option| {
                fonts
                    .layout_no_wrap(option.to_string(), f_mono_med(11.0), INK())
                    .size()
                    .x
            })
            .fold(0.0, f32::max)
    });
    // Text inset (9) + caret zone (21).
    width.max((longest + 30.0).ceil())
}

pub fn styled_select(
    ui: &mut Ui,
    id_salt: impl Hash,
    selected: usize,
    options: &[&str],
    width: f32,
) -> (Response, Option<usize>) {
    styled_select_impl(ui, id_salt, selected, options, width, false)
}

/// Like [`styled_select`] but the current value is centred and there is NO caret
/// — for tight cells (song blocks) where just the code reads cleaner.
pub fn styled_select_centered(
    ui: &mut Ui,
    id_salt: impl Hash,
    selected: usize,
    options: &[&str],
    width: f32,
) -> (Response, Option<usize>) {
    styled_select_impl(ui, id_salt, selected, options, width, true)
}

fn styled_select_impl(
    ui: &mut Ui,
    id_salt: impl Hash,
    selected: usize,
    options: &[&str],
    width: f32,
    centered: bool,
) -> (Response, Option<usize>) {
    let width = width.max(48.0);
    let selected = selected.min(options.len().saturating_sub(1));
    let current = options.get(selected).copied().unwrap_or("?");
    let popup_id = ui.make_persistent_id(("styled_select", id_salt));
    let popup_open = ui.memory(|mem| mem.is_popup_open(popup_id));

    let (rect, mut response) = ui.allocate_exact_size(Vec2::new(width, CTL_HEIGHT), Sense::click());
    if response.clicked() {
        ui.memory_mut(|mem| mem.toggle_popup(popup_id));
        ui.ctx().request_repaint();
    }

    let hovered = response.hovered();
    let hover = hover_t(ui.ctx(), response.id, hovered || popup_open);
    keycap_tex(ui, rect, KeycapState::Rest);
    let painter = ui.painter_at(rect);
    if hover > 0.01 {
        painter.rect_stroke(
            rect,
            RADIUS_CTL,
            egui::Stroke::new(1.0, lerp_color(LINE2(), BLUE(), hover)),
            StrokeKind::Inside,
        );
    }

    if centered {
        // Just the code, centred (no caret) — song blocks.
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            current,
            f_mono_med(11.0),
            INK(),
        );
    } else {
        painter.text(
            egui::pos2(rect.left() + 9.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            current,
            f_mono_med(11.0),
            INK(),
        );
        // Down-pointing triangle caret
        let caret_size = 5.0;
        let caret_center = egui::pos2(rect.right() - 11.0, rect.center().y + 0.5);
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(
                    caret_center.x - caret_size,
                    caret_center.y - caret_size * 0.4,
                ),
                egui::pos2(
                    caret_center.x + caret_size,
                    caret_center.y - caret_size * 0.4,
                ),
                egui::pos2(caret_center.x, caret_center.y + caret_size * 0.9),
            ],
            INK3(),
            egui::Stroke::NONE,
        ));
    }

    let mut picked = None;
    if popup_open {
        // Open downward by default, but flip ABOVE the box when there isn't room
        // below (e.g. selectors near the bottom edge of the window).
        let popup_h = options.len() as f32 * 24.0 + 2.0;
        let screen = ui.ctx().screen_rect();
        let mut pos = if rect.bottom() + 4.0 + popup_h <= screen.bottom() {
            rect.left_bottom() + Vec2::new(0.0, 4.0)
        } else {
            rect.left_top() - Vec2::new(0.0, popup_h + 4.0)
        };
        if let Some(to_global) = ui.ctx().layer_transform_to_global(ui.layer_id()) {
            pos = to_global * pos;
        }

        let area_response = egui::Area::new(popup_id)
            .kind(egui::UiKind::Popup)
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(P_ACTIVE())
                    .stroke(egui::Stroke::new(1.0, LINE2()))
                    .corner_radius(RADIUS_CTL)
                    .inner_margin(0.0)
                    .show(ui, |ui| {
                        ui.set_min_width(width);
                        for (idx, option) in options.iter().enumerate() {
                            let (opt_rect, opt_response) =
                                ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::click());
                            let opt_hover =
                                hover_t(ui.ctx(), opt_response.id, opt_response.hovered());
                            if opt_hover > 0.01 {
                                ui.painter().rect_filled(
                                    opt_rect,
                                    0.0,
                                    lerp_color(Color32::TRANSPARENT, BLUE(), opt_hover),
                                );
                            } else if idx == selected {
                                ui.painter().rect_filled(opt_rect, 0.0, PANEL2());
                            }
                            ui.painter().text(
                                egui::pos2(opt_rect.left() + 10.0, opt_rect.center().y),
                                egui::Align2::LEFT_CENTER,
                                *option,
                                f_sans_med(11.0),
                                lerp_color(INK2(), Color32::WHITE, opt_hover),
                            );
                            if opt_response.clicked() {
                                picked = Some(idx);
                            }
                        }
                    });
            });

        let close_for_pick = picked.is_some();
        let close_for_escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
        let close_for_outside =
            response.clicked_elsewhere() && area_response.response.clicked_elsewhere();
        if close_for_pick || close_for_escape || close_for_outside {
            ui.memory_mut(|mem| mem.close_popup());
        }
    }

    if picked.is_some() {
        response.mark_changed();
    }

    (response, picked)
}

// (led_segmented removed — the header Seq Mode switch now uses skeuo::segmented,
// the single keycap-based renderer for all segmented controls.)
