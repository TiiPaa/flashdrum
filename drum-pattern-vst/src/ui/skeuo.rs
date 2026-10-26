//! # Skeuo UI elements — the single home for every graphical element's look.
//!
//! Each function here renders exactly ONE UI element (keycap, pad, slider track,
//! recessed well, …). **Every call site in the UI goes through these functions**,
//! so changing an element's appearance is a one-place edit that applies
//! everywhere — no more hunting the same widget across `header.rs`, `slider.rs`,
//! `grid.rs`, etc.
//!
//! ## Vector *or* bitmap — same entry point
//! Every element takes a `ui: &egui::Ui` and a `rect`. The bodies below draw with
//! vector shapes (`ui.painter()` + the primitives in [`crate::ui::widgets`]), but
//! you can swap ANY element to a **baked bitmap** without touching a single call
//! site: replace that one function's body with, e.g.
//!
//! ```ignore
//! egui::Image::new(egui::include_image!("../../assets/keycaps/keycap-rest.png"))
//!     .paint_at(ui, rect);            // (3-slice for variable widths if needed)
//! ```
//!
//! The PNG loader (`egui_extras::install_image_loaders`) is installed once at
//! editor startup, so `include_image!` works anywhere here.
//!
//! ## Where things live
//! - **This file**: the elements (what a thing *is*): `keycap`, `pad`,
//!   `slider_track`, `well_recess`, …
//! - **`widgets.rs`**: low-level primitives (what a thing is *made of*): smooth
//!   gradients, soft shadows, radial fills, fade lines. Reused across elements.
//! - **`theme.rs`**: the palette and geometry constants.

use crate::ui::theme::*;
use crate::ui::widgets::{
    fade_line, grad3, inner_side_shadows, inner_top_shadow, lerp_c, soft_shadow, KeycapState,
};
use nih_plug_egui::egui::{self, Color32};

#[inline]
const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// ============================================================
// Keycap — every button/tab/page/slot/segment/dropdown background.
// (rest grey / pressed blue / pressed amber). r5, soft shadow + smooth 3-stop
// gradient + dark border + subtle relief liseré.
// ============================================================
pub fn keycap(ui: &egui::Ui, rect: egui::Rect, state: KeycapState) {
    keycap_body(ui.painter(), rect, state, true);
}

/// Keycap body. `shadow` draws the soft drop shadow — turn it OFF when the keycap
/// sits inside a recessed well (the recess already provides the depth, and the
/// shadow would bleed square-ish past the well's rounded corners).
fn keycap_body(p: &egui::Painter, rect: egui::Rect, state: KeycapState, shadow: bool) {
    let radius = 5.0_f32;
    let (ctop, cmid, cbot, border) = match state {
        KeycapState::Rest => (
            rgb(74, 75, 82),
            rgb(56, 57, 63),
            rgb(51, 52, 57),
            rgb(23, 23, 27),
        ),
        KeycapState::PressedBlue => (
            rgb(47, 134, 196),
            rgb(30, 95, 146),
            rgb(26, 84, 128),
            rgb(13, 44, 68),
        ),
        KeycapState::PressedAmber => (
            rgb(201, 122, 30),
            rgb(168, 100, 24),
            rgb(138, 78, 12),
            rgb(70, 40, 8),
        ),
    };
    if shadow {
        soft_shadow(p, rect, radius, 1.5, 2.5, 38.0);
    }
    p.rect_filled(rect, radius, cmid);
    // gradient mesh inset ≥ r×0.42 (=2.1) so its square corners never poke past
    // the rounded border.
    grad3(p, rect.shrink(2.2), ctop, cmid, cbot, 0.55);
    p.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let pressed = matches!(state, KeycapState::PressedBlue | KeycapState::PressedAmber);
    let inset = radius + 1.0;
    let w = (rect.width() - 2.0 * inset).max(0.0);
    fade_line(
        p,
        egui::Rect::from_min_size(rect.min + egui::vec2(inset, 1.5), egui::vec2(w, 1.5)),
        (255, 255, 255),
        if pressed { 22 } else { 16 },
    );
    fade_line(
        p,
        egui::Rect::from_min_size(
            egui::pos2(rect.left() + inset, rect.bottom() - 3.0),
            egui::vec2(w, 2.0),
        ),
        (0, 0, 0),
        44,
    );
}

// ============================================================
// LED — backlit indicator dot (radial glass + specular highlight). NO glow halo.
// Blue when on, dark/faint when off. Used by the header toggle pills.
// ============================================================
pub fn led(p: &egui::Painter, center: egui::Pos2, radius: f32, on: bool) {
    let (cc, cm, ce) = if on {
        (rgb(205, 240, 255), rgb(74, 182, 255), rgb(30, 110, 160))
    } else {
        (rgb(104, 105, 113), rgb(74, 75, 82), rgb(50, 51, 56))
    };
    p.circle_filled(center, radius, ce); // AA silhouette under the (unAA) mesh
    radial_circle(p, center, radius * 0.98, cc, cm, ce);
    let spec = if on {
        Color32::from_rgba_unmultiplied(255, 255, 255, 210)
    } else {
        Color32::from_rgba_unmultiplied(150, 151, 158, 110)
    };
    p.circle_filled(
        center - egui::vec2(radius * 0.28, radius * 0.32),
        radius * 0.30,
        spec,
    );
}

/// Red play-position LED, incrusted in the top-right corner of the page button /
/// song block that is currently playing. Same backlit glass as `led`, but red,
/// no halo.
pub fn play_led(p: &egui::Painter, center: egui::Pos2, radius: f32) {
    let (cc, cm, ce) = (rgb(255, 210, 205), rgb(248, 96, 88), rgb(150, 34, 30));
    p.circle_filled(center, radius, ce);
    radial_circle(p, center, radius * 0.98, cc, cm, ce);
    p.circle_filled(
        center - egui::vec2(radius * 0.26, radius * 0.30),
        radius * 0.30,
        Color32::from_rgba_unmultiplied(255, 255, 255, 205),
    );
}

/// Smooth radial gradient on a circle: fan mesh (centre → mid@45% → edge).
fn radial_circle(
    p: &egui::Painter,
    c: egui::Pos2,
    radius: f32,
    cc: Color32,
    cm: Color32,
    ce: Color32,
) {
    let n = 28usize;
    let mut m = egui::epaint::Mesh::default();
    m.colored_vertex(c, cc);
    let ang = |i: usize| std::f32::consts::TAU * i as f32 / n as f32;
    for i in 0..n {
        let a = ang(i);
        m.colored_vertex(
            egui::pos2(c.x + a.cos() * radius * 0.45, c.y + a.sin() * radius * 0.45),
            cm,
        );
    }
    for i in 0..n {
        let a = ang(i);
        m.colored_vertex(
            egui::pos2(c.x + a.cos() * radius, c.y + a.sin() * radius),
            ce,
        );
    }
    let mid = |i: usize| 1 + i as u32;
    let out = |i: usize| 1 + n as u32 + i as u32;
    for i in 0..n {
        let j = (i + 1) % n;
        m.add_triangle(0, mid(i), mid(j));
        m.add_triangle(mid(i), out(i), out(j));
        m.add_triangle(mid(i), out(j), mid(j));
    }
    p.add(egui::Shape::mesh(m));
}

// ============================================================
// Segmented — the ONE renderer for every 2+ option switch (Seq Mode, Generator/
// Song, Hz/Note, P-Lock mode). Recessed well track + the active option as a
// pressed-blue keycap; inactive options are muted, clickable text. Segments size
// to their label. Returns the (possibly newly-clicked) selected index.
// ============================================================
pub fn segmented(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    labels: &[&str],
    selected: usize,
) -> usize {
    segmented_impl(ui, id_salt, labels, selected, false)
}

/// Like [`segmented`] but every segment gets the SAME width (widest label) →
/// symmetric halves (used by the header Seq switch).
pub fn segmented_equal(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    labels: &[&str],
    selected: usize,
) -> usize {
    segmented_impl(ui, id_salt, labels, selected, true)
}

fn segmented_impl(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    labels: &[&str],
    selected: usize,
    equal: bool,
) -> usize {
    let font = f_sans_sb(11.0);
    let h = CTL_HEIGHT;
    let pad = 14.0;
    let mut widths: Vec<f32> = labels
        .iter()
        .map(|l| {
            let tw = ui.fonts(|f| {
                f.layout_no_wrap((*l).to_string(), font.clone(), Color32::WHITE)
                    .size()
                    .x
            });
            (tw + 2.0 * pad).max(52.0)
        })
        .collect();
    if equal {
        let m = widths.iter().cloned().fold(0.0_f32, f32::max);
        for w in widths.iter_mut() {
            *w = m;
        }
    }
    let total: f32 = widths.iter().sum();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(total + 4.0, h), egui::Sense::hover());

    // Recessed well track (dark rect + inner top shadow + dark border).
    {
        let p = ui.painter_at(rect);
        p.rect_filled(rect, RADIUS_CTL, rgb(20, 21, 25));
        inner_top_shadow(&p, rect, RADIUS_CTL, 4.0, 120);
        p.rect_stroke(
            rect,
            RADIUS_CTL,
            egui::Stroke::new(1.0, rgb(11, 11, 14)),
            egui::StrokeKind::Inside,
        );
    }

    let mut result = selected.min(labels.len().saturating_sub(1));
    let mut cx = rect.left() + 2.0;
    for (i, label) in labels.iter().enumerate() {
        let seg = egui::Rect::from_min_size(
            egui::pos2(cx, rect.top() + 2.0),
            egui::vec2(widths[i], h - 4.0),
        );
        let resp = ui.interact(
            seg,
            ui.make_persistent_id((&id_salt, "seg", i)),
            egui::Sense::click(),
        );
        let active = i == result;
        if active {
            // No drop shadow: the keycap sits inside the recessed well.
            keycap_body(ui.painter(), seg, KeycapState::PressedBlue, false);
        }
        let color = if active {
            rgb(234, 246, 255)
        } else if resp.hovered() {
            INK()
        } else {
            INK2()
        };
        ui.painter().text(
            seg.center(),
            egui::Align2::CENTER_CENTER,
            *label,
            font.clone(),
            color,
        );
        if resp.clicked() {
            result = i;
        }
        cx += widths[i];
    }
    result
}

// ============================================================
// Slider track — the ONE renderer for every horizontal slider (header Len,
// track Vol/Hum/Push, sound-editor ENV). Recessed groove (dark pill + inner top
// shadow) + a pill value fill. `handle` = Some(radius) draws the round knob.
// ============================================================
/// The recessed channel every control is cut into: slider tracks, and the
/// Sound panel's scroll bar ([190]). One definition so they cannot drift apart.
pub fn groove_fill() -> Color32 {
    rgb(16, 17, 21)
}

/// The darker lip around a groove.
pub fn groove_border() -> Color32 {
    rgb(9, 9, 12)
}

pub fn slider_track(ui: &egui::Ui, track: egui::Rect, norm: f32, fill: Color32, cap: bool) {
    let p = ui.painter();
    let rr = track.height() * 0.5;
    // recessed groove: dark pill + a dark border + thin inner top shadow
    p.rect_filled(track, rr, groove_fill());
    p.rect_stroke(
        track,
        rr,
        egui::Stroke::new(1.0, groove_border()),
        egui::StrokeKind::Inside,
    );
    let mx = track.left() + rr;
    let gw = (track.width() - 2.0 * rr).max(0.0);
    p.rect_filled(
        egui::Rect::from_min_size(egui::pos2(mx, track.top() + 0.5), egui::vec2(gw, 1.5)),
        egui::epaint::CornerRadius::ZERO,
        Color32::from_black_alpha(130),
    );
    // value fill = a light bar inside the channel with rounded ends
    let x_val = track.left() + track.width() * norm;
    if norm > 0.0 {
        let ih = (track.height() - 2.0).max(2.0);
        let y = track.center().y;
        let x0 = track.left() + 1.0;
        if x_val - x0 >= ih {
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(x0, y - ih * 0.5),
                    egui::pos2(x_val, y + ih * 0.5),
                ),
                ih * 0.5,
                fill,
            );
        }
    }
    // striped fader cap at the value position — only for full sliders (`cap`),
    // never for the tiny Vol/Hum/Push mini-sliders (which stay a plain fill bar).
    if cap {
        fader_cap(
            p,
            x_val,
            track.center().y,
            (track.height() * 2.6).clamp(13.0, 20.0),
        );
    }
}

/// Striped fader cap knob (mechanical grip): 3 hard horizontal bands, rounded
/// silhouette, soft drop shadow. `h` = cap height; width follows proportionally.
fn fader_cap(p: &egui::Painter, cx: f32, cy: f32, h: f32) {
    let w = (h * 0.62).max(8.0);
    let knob = egui::Rect::from_center_size(egui::pos2(cx, cy), egui::vec2(w, h));
    soft_shadow(p, knob, 3.0, 1.5, 2.0, 55.0);
    p.rect_filled(knob, 3.0, rgb(16, 16, 20)); // silhouette / border
    let k = knob.shrink(1.0);
    let (y0, hh) = (k.top(), k.height());
    let r_top = egui::epaint::CornerRadius {
        nw: 2,
        ne: 2,
        sw: 0,
        se: 0,
    };
    let r_bot = egui::epaint::CornerRadius {
        nw: 0,
        ne: 0,
        sw: 2,
        se: 2,
    };
    p.rect_filled(
        egui::Rect::from_min_max(k.min, egui::pos2(k.right(), y0 + hh * 0.40)),
        r_top,
        rgb(96, 97, 105),
    );
    p.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(k.left(), y0 + hh * 0.40),
            egui::pos2(k.right(), y0 + hh * 0.60),
        ),
        egui::epaint::CornerRadius::ZERO,
        rgb(50, 51, 57),
    );
    p.rect_filled(
        egui::Rect::from_min_max(egui::pos2(k.left(), y0 + hh * 0.60), k.max),
        r_bot,
        rgb(76, 77, 84),
    );
}

// ============================================================
// Well recess — the shadow of a recessed panel (grid well, groove containers).
// Wall shadows on top + left + right that fade inward and hug the rounded
// corners; the floor (bottom) stays lit. Draw this AFTER the well's content.
// ============================================================
/// Pip, top-right of a step cell: this cell also carries the OTHER kind of
/// p-lock ([219], generalised by [220]).
///
/// A cell is coloured by the p-lock of the mode currently displayed, so the
/// other one is invisible - and since [213] either can be placed without ever
/// leaving the mode one is in. The pip says "there is something else here", in
/// the other kind's own colour: violet for a sequencer p-lock seen from Sound
/// mode, light green for a sound p-lock seen from Sequencer mode. The black
/// ring keeps it legible on a lit pad.
///
/// Several pips on one cell, laid from the top-right corner leftwards
/// ([283]: a linked lane also shows its partner lanes' p-locks).
pub fn plock_pips(p: &egui::Painter, cell: egui::Rect, colors: &[Color32]) {
    for (i, color) in colors.iter().enumerate() {
        let c = egui::pos2(cell.right() - 6.5 - i as f32 * 6.5, cell.top() + 6.0);
        p.circle_filled(c, 2.8, Color32::from_black_alpha(150));
        p.circle_filled(c, 2.2, *color);
    }
}

/// MIDI-activity flash colours, shared by the lane-name plate flash ([238]):
/// the whole plate goes WHITE, so the text flips dark to stay readable.
const FLASH_FILL: Color32 = rgb(236, 240, 246);
const FLASH_INK: Color32 = rgb(28, 28, 32);

/// Opening bracket marking the FIRST cell of a fusion being created ([210]).
///
/// A fusion is a span, so its pending start is drawn as the `[` that opens one:
/// the shape says "the span starts here, waiting for its end" instead of just
/// tinting the cell. Static on purpose - the pulsing outline already means
/// "this fusion is being edited", and two states must not share one signal.
pub fn fusion_start_bracket(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let w = 2.0;
    let arm = (rect.width() * 0.28).clamp(4.0, 6.0);
    // Clear of the cell's own 2 px outline, or the two shapes merge into a blob.
    let inset = 4.0;
    let left = rect.left() + inset;
    let top = rect.top() + inset;
    let bottom = rect.bottom() - inset;
    // Spine.
    p.rect_filled(
        egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(left + w, bottom)),
        w * 0.5,
        color,
    );
    // Arms, top and bottom.
    for y in [top, bottom - w] {
        p.rect_filled(
            egui::Rect::from_min_max(egui::pos2(left, y), egui::pos2(left + arm, y + w)),
            w * 0.5,
            color,
        );
    }
}

/// Grid-link marker for a lane that plays the lane above ([191]).
///
/// A return arrow: it rises out of the row above, turns, and points into this
/// lane - "my grid comes from up there". Drawn only on the **follower**; the
/// lane it borrows from carries no mark, so the arrow alone means "linked".
///
/// It replaces a 2 px stripe on the row's left edge that was almost invisible
/// and said nothing about direction. `rect` is the free space between the drag
/// grip and the name plate; the glyph is centred in it and never wider, so no
/// zone moves.
pub fn link_arrow(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let stroke = (rect.width() * 0.22).clamp(1.5, 2.5);
    let head = (rect.width() * 0.34).clamp(2.5, 4.0);
    // The corner sits low-left; the stem rises from it toward the row above.
    let x = rect.left() + stroke * 0.5;
    let y = rect.center().y + stroke * 0.5;
    let top = rect.top() + stroke;
    // Stem, drawn past the corner so the turn has no notch.
    p.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(x - stroke * 0.5, top),
            egui::pos2(x + stroke * 0.5, y + stroke * 0.5),
        ),
        stroke * 0.5,
        color,
    );
    // Elbow running right, stopping where the head begins.
    let tip = rect.right() - stroke * 0.5;
    p.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(x - stroke * 0.5, y - stroke * 0.5),
            egui::pos2(tip - head, y + stroke * 0.5),
        ),
        stroke * 0.5,
        color,
    );
    // Head, pointing into the lane.
    p.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(tip, y),
            egui::pos2(tip - head, y - head * 0.72),
            egui::pos2(tip - head, y + head * 0.72),
        ],
        color,
        egui::Stroke::NONE,
    ));
}

/// The small tracked capitals that title a group of controls (the Generator's
/// PATTERN / STYLE / CREATE), left-centred on `pos`.
pub fn group_title(p: &egui::Painter, pos: egui::Pos2, text: &str) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: f_mono_sb(9.0),
            color: INK3(),
            extra_letter_spacing: 1.2,
            ..Default::default()
        },
    );
    let galley = p.ctx().fonts(|f| f.layout_job(job));
    let r = egui::Align2::LEFT_CENTER.anchor_size(pos, galley.size());
    p.galley(r.min, galley, INK3());
}

/// A die seen from the front, showing five: the Generator's **Full random** key.
/// `rect` is the die's square; the outline and the pips take `color`.
pub fn dice_icon(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let side = rect.width().min(rect.height());
    let die = egui::Rect::from_center_size(rect.center(), egui::vec2(side, side));
    let stroke = (side * 0.11).clamp(1.2, 2.0);
    p.rect_stroke(
        die.shrink(stroke * 0.5),
        side * 0.22,
        egui::Stroke::new(stroke, color),
        egui::StrokeKind::Middle,
    );
    let pip = side * 0.095;
    let off = side * 0.24;
    let c = die.center();
    for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (0.0, 0.0), (-1.0, 1.0), (1.0, 1.0)] {
        p.circle_filled(egui::pos2(c.x + dx * off, c.y + dy * off), pip, color);
    }
}

pub fn well_recess(ui: &egui::Ui, rect: egui::Rect, radius: f32) {
    let p = ui.painter_at(rect);
    inner_top_shadow(&p, rect, radius, 5.0, 150);
    inner_side_shadows(&p, rect, radius, 4.0, 120);
}

// ============================================================
// LCD screen background — the recessed green CRT glass behind the ADSR graph.
// Symmetric green gradient (lit from the centre) + recessed top shadow +
// green-black border + subtle scanlines. Draw this, then the curve on top.
// ============================================================
pub fn lcd_bg(ui: &egui::Ui, rect: egui::Rect, radius: f32) {
    let p = ui.painter();
    // dark rounded base → clean AA corners under the (square) gradient mesh
    p.rect_filled(rect, radius, rgb(7, 13, 9));
    grad3(
        p,
        rect.shrink(2.0),
        rgb(9, 16, 11),
        rgb(16, 27, 20),
        rgb(9, 16, 11),
        0.5,
    );
    // recessed top shadow (inset from the corners so it doesn't poke)
    let ins = radius.max(1.0);
    p.rect_filled(
        egui::Rect::from_min_size(
            egui::pos2(rect.left() + ins, rect.top() + 1.0),
            egui::vec2((rect.width() - 2.0 * ins).max(0.0), 3.0),
        ),
        egui::epaint::CornerRadius::ZERO,
        Color32::from_black_alpha(80),
    );
    // green-black border (the frame is part of the screen)
    p.rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, rgb(5, 9, 6)),
        egui::StrokeKind::Inside,
    );
    // subtle CRT scanlines
    let inner = rect.shrink(2.0);
    let mut y = inner.top() + 1.0;
    while y < inner.bottom() {
        p.line_segment(
            [egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)],
            egui::Stroke::new(1.0, Color32::from_black_alpha(28)),
        );
        y += 3.0;
    }
}

// ============================================================
// Switch — on/off toggle. Recessed slot (dark → blue) + round metal cap that
// slides. `t` is the animated 0→1 state (0 = off/left, 1 = on/right). No harsh
// top line, no specular dot inside the cap.
// ============================================================
pub fn switch(ui: &egui::Ui, rect: egui::Rect, t: f32) {
    let p = ui.painter();
    let t = t.clamp(0.0, 1.0);
    let rr = rect.height() * 0.5;
    let track = lerp_c(rgb(19, 20, 24), rgb(38, 104, 156), t);
    let border = lerp_c(rgb(9, 9, 12), rgb(18, 60, 92), t);
    p.rect_filled(rect, rr, track);
    p.rect_stroke(
        rect,
        rr,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let kr = rect.height() * 0.5 - 2.0;
    let kx = (rect.left() + rr) + ((rect.right() - rr) - (rect.left() + rr)) * t;
    let kc = egui::pos2(kx, rect.center().y);
    p.circle_filled(kc + egui::vec2(0.0, 1.0), kr, Color32::from_black_alpha(90));
    p.circle_filled(kc, kr, rgb(206, 208, 214));
    radial_circle(
        p,
        kc,
        kr * 0.96,
        rgb(232, 234, 240),
        rgb(196, 198, 205),
        rgb(150, 152, 160),
    );
}

// ============================================================
// Tag — small M/S/T lane button (17×17, r3). Raised mini-keycap; when active it
// takes the accent colour, otherwise a neutral grey. Caller allocates + senses.
// ============================================================
pub fn tag(
    ui: &egui::Ui,
    rect: egui::Rect,
    letter: &str,
    active: bool,
    accent: Color32,
    ink_on: Color32,
) {
    let p = ui.painter();
    let (top, bot, border, ink) = if active {
        (
            lerp_c(accent, Color32::WHITE, 0.22),
            accent,
            lerp_c(accent, Color32::BLACK, 0.5),
            ink_on,
        )
    } else {
        (
            rgb(66, 67, 74),
            rgb(52, 53, 58),
            rgb(23, 23, 27),
            rgb(150, 151, 158),
        )
    };
    p.rect_filled(rect, 3.0, bot);
    grad3(p, rect.shrink(1.4), top, lerp_c(top, bot, 0.5), bot, 0.55);
    p.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        letter,
        f_mono_sb(9.0),
        ink,
    );
}

// ============================================================
// Lane name — the lane's title button. A keycap (grey at rest, pressed-blue when
// the lane is selected) with a left-aligned label. Caller allocates + senses.
// `flash` (0..=1) flashes the whole plate white on MIDI activity ([238] —
// replaces the small LED of [216]).
// ============================================================
/// `fx`: an effect lane (Fx1 / Fx2) wears amber instead of the instruments' blue-grey.
pub fn lane_name(ui: &egui::Ui, rect: egui::Rect, text: &str, selected: bool, flash: f32, fx: bool) {
    let p = ui.painter();
    let f = flash.clamp(0.0, 1.0);
    // FLAT: no gradient/border at rest (a raised look reads as a permanent hover);
    // only the selected lane is filled blue with a thin outline.
    let (sel_fill, sel_line, rest_fill, sel_ink, plain_ink) = if fx {
        (
            rgb(150, 104, 30),
            rgb(206, 154, 58),
            rgb(74, 58, 30),
            rgb(255, 244, 220),
            rgb(236, 198, 124),
        )
    } else {
        (
            rgb(42, 104, 156),
            rgb(60, 132, 186),
            rgb(52, 53, 59),
            rgb(234, 246, 255),
            rgb(201, 203, 211),
        )
    };
    if selected {
        p.rect_filled(rect, 5.0, lerp_c(sel_fill, FLASH_FILL, f));
        p.rect_stroke(
            rect,
            5.0,
            egui::Stroke::new(1.0, lerp_c(sel_line, FLASH_FILL, f)),
            egui::StrokeKind::Inside,
        );
    } else {
        p.rect_filled(rect, 5.0, lerp_c(rest_fill, FLASH_FILL, f));
    }
    let rest_ink = if selected { sel_ink } else { plain_ink };
    let tc = lerp_c(rest_ink, FLASH_INK, f);
    // Tight left padding so the button doesn't leave much empty space L/R.
    p.text(
        egui::pos2(rect.left() + 5.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        f_mono_sb(11.0),
        tc,
    );
}

// ============================================================
// Plate — raised panel as a single composite `Shape` (soft shadow + gradient +
// border + top liseré). Returned (not painted) so a popup can reserve a shape
// slot BEFORE its content and set the plate behind it. Used by the menu frames.
// ============================================================
pub fn plate_shape(rect: egui::Rect, radius: f32) -> egui::Shape {
    let mut v: Vec<egui::Shape> = Vec::new();
    // soft drop shadow — stacked rects, quadratic falloff
    let steps = 10;
    for i in (1..=steps).rev() {
        let t = i as f32 / steps as f32;
        let grow = 6.0 * t;
        let a = (42.0 * (1.0 - t) * (1.0 - t)).round().clamp(0.0, 255.0) as u8;
        v.push(egui::Shape::rect_filled(
            rect.translate(egui::vec2(0.0, 2.0)).expand(grow),
            radius + grow,
            Color32::from_black_alpha(a),
        ));
    }
    v.push(egui::Shape::rect_filled(rect, radius, rgb(37, 38, 43)));
    v.push(grad3_mesh_shape(
        rect.shrink(2.5),
        rgb(47, 48, 54),
        rgb(41, 42, 47),
        rgb(37, 38, 43),
        0.55,
    ));
    v.push(egui::Shape::rect_stroke(
        rect,
        radius,
        egui::Stroke::new(1.0, rgb(20, 20, 24)),
        egui::StrokeKind::Inside,
    ));
    // top liseré (center-peaked fade)
    let inset = 10.0;
    v.push(fade_line_mesh_shape(
        egui::Rect::from_min_size(
            egui::pos2(rect.left() + inset, rect.top() + 1.5),
            egui::vec2((rect.width() - 2.0 * inset).max(0.0), 1.5),
        ),
        (255, 255, 255),
        18,
    ));
    egui::Shape::Vec(v)
}

fn grad3_mesh_shape(
    rect: egui::Rect,
    ctop: Color32,
    cmid: Color32,
    cbot: Color32,
    mid_t: f32,
) -> egui::Shape {
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
    egui::Shape::mesh(m)
}

fn fade_line_mesh_shape(rect: egui::Rect, rgb3: (u8, u8, u8), peak: u8) -> egui::Shape {
    let (l, r, t, b) = (rect.left(), rect.right(), rect.top(), rect.bottom());
    let cx = rect.center().x;
    let c0 = Color32::from_rgba_unmultiplied(rgb3.0, rgb3.1, rgb3.2, 0);
    let cc = Color32::from_rgba_unmultiplied(rgb3.0, rgb3.1, rgb3.2, peak);
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
    egui::Shape::mesh(m)
}

// ============================================================
// [282] Lane Editor module — one Sound-tab section pressed into the plate:
// recessed body, optional title strip with a blue wash under the title
// (no vertical accent: a vertical bar at the left edge is the p-lock / morph
// marker's shape). Returned as shapes because the section's height is only
// known once its rows are laid out: the caller reserves a slot first.
// ============================================================
pub const MODULE_STRIP_H: f32 = 26.0;

/// [282] Colour of the blue washes (module strips, active Lane Editor tab):
/// the pressed-blue keycap's top colour, so a wash and the Step / Hz keys are
/// the same blue. The accent `BLUE()` laid thin over the grey plate came out
/// a desaturated slate next to the keys.
pub const WASH_BLUE: Color32 = rgb(47, 134, 196);
/// Opacity of a wash where it is strongest.
pub const WASH_ALPHA: f32 = 110.0;

/// [303] `title_dx` moves the title right (room for a control before it, like an insert's
/// drag handle); 0 for every other module.
pub fn editor_module_indented(
    ctx: &egui::Context,
    rect: egui::Rect,
    title: Option<&str>,
    title_dx: f32,
) -> egui::Shape {
    use egui::{pos2, vec2, CornerRadius, Rect, Shape, Stroke, StrokeKind};
    let mut shapes = Vec::with_capacity(16);
    // [282] One line per edge: no lit lip around the module, no highlight
    // or crease on the strip (its lighter fill is the separation). Stacked
    // modules otherwise showed four or five hairlines in a dozen pixels.
    shapes.push(Shape::rect_filled(rect, 5.0, rgb(30, 31, 35)));
    // recess wall: shadow fading down from the top edge
    for i in 0..4 {
        let a = 90 - i * 22;
        let y = rect.top() + 1.0 + i as f32;
        shapes.push(Shape::rect_filled(
            Rect::from_min_max(
                pos2(rect.left() + 4.0, y),
                pos2(rect.right() - 4.0, y + 1.0),
            ),
            0.0,
            Color32::from_black_alpha(a as u8),
        ));
    }
    if let Some(title) = title {
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), MODULE_STRIP_H));
        shapes.push(Shape::rect_filled(
            strip,
            CornerRadius {
                nw: 5,
                ne: 5,
                sw: 0,
                se: 0,
            },
            rgb(41, 42, 48),
        ));
        // blue wash, strongest under the title, gone past mid-width. Its
        // top-left corner follows the strip's rounding: a square corner
        // poked a blue pixel past the module's border.
        let b = WASH_BLUE;
        let wash = Rect::from_min_max(
            strip.min + vec2(1.0, 1.0),
            pos2(strip.left() + strip.width() * 0.55, strip.bottom() - 1.0),
        );
        // Alpha falls linearly with x, which the triangles interpolate exactly.
        let tint = |x: f32| {
            let t = ((x - wash.left()) / wash.width()).clamp(0.0, 1.0);
            Color32::from_rgba_unmultiplied(b.r(), b.g(), b.b(), (WASH_ALPHA * (1.0 - t)) as u8)
        };
        let r = 4.0; // the strip's 5 px corner, 1 px in
        let mut outline = vec![wash.left_bottom()];
        for i in 0..=6 {
            let a = std::f32::consts::FRAC_PI_2 * i as f32 / 6.0;
            outline.push(pos2(
                wash.left() + r - r * a.cos(),
                wash.top() + r - r * a.sin(),
            ));
        }
        outline.push(wash.right_top());
        outline.push(wash.right_bottom());
        // Convex outline: a fan from its first vertex covers it.
        let mut m = egui::epaint::Mesh::default();
        for p in &outline {
            m.colored_vertex(*p, tint(p.x));
        }
        for i in 1..outline.len() as u32 - 1 {
            m.add_triangle(0, i, i + 1);
        }
        shapes.push(Shape::mesh(m));
        // tracked caps title, on the labels' x
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &title.to_uppercase(),
            0.0,
            egui::TextFormat {
                font_id: f_sans_sb(10.0),
                color: INK(),
                extra_letter_spacing: 1.4,
                ..Default::default()
            },
        );
        let galley = ctx.fonts(|f| f.layout_job(job));
        let pos = pos2(
            rect.left() + MODULE_TITLE_X + title_dx,
            strip.center().y - galley.size().y * 0.5,
        );
        shapes.push(Shape::galley(pos, galley, INK()));
    }
    shapes.push(Shape::rect_stroke(
        rect,
        5.0,
        Stroke::new(1.0, rgb(20, 20, 24)),
        StrokeKind::Inside,
    ));
    Shape::Vec(shapes)
}

/// x of a module's title from the module's left edge: the labels' x.
pub const MODULE_TITLE_X: f32 = 22.0;

// ============================================================
// [282] Value readout — the recessed field a Lane Editor value sits in.
// ============================================================
pub fn value_readout(ui: &egui::Ui, rect: egui::Rect, text: &str, color: Color32) {
    use egui::{pos2, vec2, Align2, Rect, Stroke, StrokeKind};
    let p = ui.painter();
    let r = rect.shrink2(vec2(0.0, 1.0));
    p.rect_filled(r, 3.0, rgb(24, 25, 29));
    p.rect_filled(
        Rect::from_min_size(r.min + vec2(2.0, 1.0), vec2(r.width() - 4.0, 2.0)),
        0.0,
        Color32::from_black_alpha(70),
    );
    p.rect_stroke(
        r,
        3.0,
        Stroke::new(1.0, rgb(19, 19, 22)),
        StrokeKind::Inside,
    );
    p.text(
        pos2(r.right() - 6.0, r.center().y),
        Align2::RIGHT_CENTER,
        text,
        f_mono(11.0),
        color,
    );
}

// ============================================================
// [282] Folder tab — the Lane Editor's Sound / Track tabs. One rounded-top
// shape, its outline following the corners (open at the bottom: the tab
// stands on the plate's edge). The active tab is cut from the plate (plate
// colour + the keycap-blue wash) and opens onto it; the other sits lower and
// darker. `rect.bottom()` is the plate edge.
// ============================================================
pub const FOLDER_TAB_RADIUS: f32 = 6.0;

fn folder_tab_outline(r: egui::Rect) -> Vec<egui::Pos2> {
    let rad = FOLDER_TAB_RADIUS;
    let n = 8;
    let mut pts = Vec::with_capacity(2 * n + 4);
    pts.push(r.left_bottom());
    for i in 0..=n {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / n as f32;
        pts.push(egui::pos2(
            r.left() + rad - rad * a.cos(),
            r.top() + rad - rad * a.sin(),
        ));
    }
    for i in 0..=n {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / n as f32;
        pts.push(egui::pos2(
            r.right() - rad + rad * a.sin(),
            r.top() + rad - rad * a.cos(),
        ));
    }
    pts.push(r.right_bottom());
    pts
}

/// Fan fill of a convex outline with a vertical ramp, alpha included
/// (premultiplied lerp — `lerp_c` would make a translucent wash opaque).
fn vertical_ramp_fill(
    pts: &[egui::Pos2],
    top: Color32,
    bottom: Color32,
    y0: f32,
    y1: f32,
) -> egui::Shape {
    let mut m = egui::epaint::Mesh::default();
    for p in pts {
        let t = ((p.y - y0) / (y1 - y0)).clamp(0.0, 1.0);
        let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        m.colored_vertex(
            *p,
            Color32::from_rgba_premultiplied(
                l(top.r(), bottom.r()),
                l(top.g(), bottom.g()),
                l(top.b(), bottom.b()),
                l(top.a(), bottom.a()),
            ),
        );
    }
    for i in 1..pts.len() as u32 - 1 {
        m.add_triangle(0, i, i + 1);
    }
    egui::Shape::mesh(m)
}

pub fn folder_tab(p: &egui::Painter, rect: egui::Rect, active: bool, hovered: bool) {
    let pts = folder_tab_outline(rect);
    if active {
        p.add(vertical_ramp_fill(
            &pts,
            PANEL_SKEUO,
            PANEL_SKEUO,
            rect.top(),
            rect.bottom(),
        ));
        let b = WASH_BLUE;
        p.add(vertical_ramp_fill(
            &pts,
            Color32::from_rgba_unmultiplied(b.r(), b.g(), b.b(), WASH_ALPHA as u8),
            Color32::TRANSPARENT,
            rect.top(),
            rect.bottom(),
        ));
        // Open onto the plate: cover the plate edge under the tab.
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 0.5, rect.bottom() - 0.5),
                egui::pos2(rect.right() - 0.5, rect.bottom() + 1.0),
            ),
            0.0,
            PANEL_SKEUO,
        );
        crate::ui::widgets::plateau_line(
            p,
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 3.0, rect.top() + 0.5),
                egui::pos2(rect.right() - 3.0, rect.top() + 1.5),
            ),
            (255, 255, 255),
            16,
        );
    } else {
        let (top, bottom) = if hovered {
            (rgb(48, 49, 55), rgb(41, 42, 47))
        } else {
            (rgb(40, 41, 47), rgb(34, 35, 40))
        };
        p.add(vertical_ramp_fill(
            &pts,
            top,
            bottom,
            rect.top(),
            rect.bottom(),
        ));
    }
    p.add(egui::Shape::line(
        pts,
        egui::Stroke::new(1.0, rgb(20, 20, 24)),
    ));
}
