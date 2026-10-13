//! UI theme: color tokens, skins and font/size helpers.
//!
//! Colors are runtime values so the user can switch skins without rebuilding.
//! Access them through the same-name accessor functions (`BG()`, `BLUE()`…).
//! The active skin is persisted in `GlobalConfig.skin`.

#![allow(non_snake_case)]

use nih_plug_egui::egui::{Color32, FontFamily, FontId};
use std::sync::RwLock;

// ============================================================
// Theme struct
// ============================================================
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    // Surfaces
    pub bg: Color32,
    pub panel: Color32,
    pub panel2: Color32,
    pub p_hover: Color32,
    pub p_active: Color32,
    pub line: Color32,
    pub line2: Color32,
    // Accents
    pub blue: Color32,
    pub green: Color32,
    pub red: Color32,
    pub amber: Color32,
    // P-locks
    pub pl_link: Color32,
    pub pl_link_dim: Color32,
    pub pl_snap_dim: Color32,
    pub seqpl: Color32,
    pub seqpl_dim: Color32,
    // Texte
    pub ink: Color32,
    pub ink2: Color32,
    pub ink3: Color32,
    pub faint: Color32,
    // Grid cells
    pub cell_empty_beat: Color32,
    pub cell_empty_off: Color32,
    pub cell_current: Color32,
    pub cell_disabled: Color32,
    pub fusion_fill: Color32,
    pub cell_seqpl_off: Color32,
    pub cell_pl_snap_off: Color32,
    pub cell_pl_link_off: Color32,
    pub song_empty: Color32,
    // Feedback
    pub danger: Color32,
    pub drag_target: Color32,
    pub solo_fill: Color32,
}

// ============================================================
// Built-in skins
// ============================================================
pub const SKIN_DARK: Theme = Theme {
    bg: Color32::from_rgb(10, 10, 15),
    panel: Color32::from_rgb(20, 20, 25),
    panel2: Color32::from_rgb(28, 28, 36),
    p_hover: Color32::from_rgb(36, 36, 48),
    p_active: Color32::from_rgb(42, 42, 56),
    line: Color32::from_rgb(42, 42, 53),
    line2: Color32::from_rgb(58, 58, 72),
    blue: Color32::from_rgb(74, 158, 255),
    green: Color32::from_rgb(74, 222, 128),
    red: Color32::from_rgb(248, 113, 113),
    amber: Color32::from_rgb(251, 191, 36),
    pl_link: Color32::from_rgb(181, 255, 225),
    pl_link_dim: Color32::from_rgb(111, 174, 149),
    pl_snap_dim: Color32::from_rgb(160, 30, 30),
    seqpl: Color32::from_rgb(168, 85, 247),
    seqpl_dim: Color32::from_rgb(120, 60, 180),
    ink: Color32::from_rgb(232, 232, 240),
    ink2: Color32::from_rgb(204, 208, 216),
    ink3: Color32::from_rgb(160, 166, 178),
    faint: Color32::from_rgb(112, 120, 134),
    cell_empty_beat: Color32::from_rgb(35, 35, 44),
    cell_empty_off: Color32::from_rgb(27, 27, 34),
    cell_current: Color32::from_rgb(48, 48, 60),
    cell_disabled: Color32::from_rgb(10, 10, 14),
    fusion_fill: Color32::from_rgb(20, 34, 58),
    cell_seqpl_off: Color32::from_rgb(28, 18, 48),
    cell_pl_snap_off: Color32::from_rgb(36, 16, 16),
    cell_pl_link_off: Color32::from_rgb(19, 40, 33),
    song_empty: Color32::from_rgb(18, 18, 24),
    danger: Color32::from_rgb(255, 80, 80),
    drag_target: Color32::from_rgb(255, 200, 80),
    solo_fill: Color32::from_rgb(6, 32, 15),
};

pub const SKIN_MIDNIGHT: Theme = Theme {
    bg: Color32::from_rgb(8, 10, 18),
    panel: Color32::from_rgb(14, 18, 30),
    panel2: Color32::from_rgb(20, 26, 40),
    p_hover: Color32::from_rgb(26, 34, 52),
    p_active: Color32::from_rgb(32, 42, 62),
    line: Color32::from_rgb(30, 38, 56),
    line2: Color32::from_rgb(40, 50, 72),
    blue: Color32::from_rgb(96, 165, 250),
    green: Color32::from_rgb(74, 222, 128),
    red: Color32::from_rgb(248, 113, 113),
    amber: Color32::from_rgb(251, 191, 36),
    pl_link: Color32::from_rgb(190, 255, 230),
    pl_link_dim: Color32::from_rgb(120, 175, 155),
    pl_snap_dim: Color32::from_rgb(170, 50, 60),
    seqpl: Color32::from_rgb(150, 110, 250),
    seqpl_dim: Color32::from_rgb(105, 80, 185),
    ink: Color32::from_rgb(226, 232, 240),
    ink2: Color32::from_rgb(198, 208, 222),
    ink3: Color32::from_rgb(152, 167, 188),
    faint: Color32::from_rgb(106, 120, 142),
    cell_empty_beat: Color32::from_rgb(28, 34, 48),
    cell_empty_off: Color32::from_rgb(22, 27, 40),
    cell_current: Color32::from_rgb(44, 54, 74),
    cell_disabled: Color32::from_rgb(8, 10, 16),
    fusion_fill: Color32::from_rgb(18, 32, 60),
    cell_seqpl_off: Color32::from_rgb(24, 20, 52),
    cell_pl_snap_off: Color32::from_rgb(40, 18, 22),
    cell_pl_link_off: Color32::from_rgb(18, 38, 34),
    song_empty: Color32::from_rgb(14, 17, 26),
    danger: Color32::from_rgb(255, 90, 90),
    drag_target: Color32::from_rgb(250, 204, 21),
    solo_fill: Color32::from_rgb(8, 34, 20),
};

pub const SKIN_EMBER: Theme = Theme {
    bg: Color32::from_rgb(16, 10, 8),
    panel: Color32::from_rgb(26, 16, 12),
    panel2: Color32::from_rgb(36, 22, 16),
    p_hover: Color32::from_rgb(48, 30, 20),
    p_active: Color32::from_rgb(58, 38, 26),
    line: Color32::from_rgb(52, 34, 24),
    line2: Color32::from_rgb(70, 46, 32),
    blue: Color32::from_rgb(251, 146, 60),
    green: Color32::from_rgb(134, 239, 172),
    red: Color32::from_rgb(248, 113, 113),
    amber: Color32::from_rgb(251, 191, 36),
    pl_link: Color32::from_rgb(185, 255, 220),
    pl_link_dim: Color32::from_rgb(118, 170, 145),
    pl_snap_dim: Color32::from_rgb(170, 40, 30),
    seqpl: Color32::from_rgb(217, 120, 239),
    seqpl_dim: Color32::from_rgb(150, 85, 170),
    ink: Color32::from_rgb(245, 235, 225),
    ink2: Color32::from_rgb(222, 208, 196),
    ink3: Color32::from_rgb(184, 164, 149),
    faint: Color32::from_rgb(140, 124, 111),
    cell_empty_beat: Color32::from_rgb(44, 32, 26),
    cell_empty_off: Color32::from_rgb(34, 26, 22),
    cell_current: Color32::from_rgb(70, 52, 40),
    cell_disabled: Color32::from_rgb(14, 10, 8),
    fusion_fill: Color32::from_rgb(50, 32, 20),
    cell_seqpl_off: Color32::from_rgb(46, 24, 44),
    cell_pl_snap_off: Color32::from_rgb(44, 18, 16),
    cell_pl_link_off: Color32::from_rgb(22, 44, 36),
    song_empty: Color32::from_rgb(26, 17, 14),
    danger: Color32::from_rgb(255, 90, 80),
    drag_target: Color32::from_rgb(251, 191, 36),
    solo_fill: Color32::from_rgb(10, 36, 18),
};

/// (name, theme) pairs shown in the Settings skin selector.
pub const SKINS: &[(&str, Theme)] = &[
    ("Dark", SKIN_DARK),
    ("Midnight", SKIN_MIDNIGHT),
    ("Ember", SKIN_EMBER),
];

pub const DEFAULT_SKIN_NAME: &str = "Dark";

// ============================================================
// Active skin state
// ============================================================
static ACTIVE: RwLock<&'static Theme> = RwLock::new(&SKIN_DARK);

#[inline]
fn current() -> &'static Theme {
    ACTIVE.read().map(|g| *g).unwrap_or(&SKIN_DARK)
}

/// Switch the active skin by name. Unknown names are ignored.
pub fn set_skin(name: &str) {
    if let Some((_, theme)) = SKINS.iter().find(|(n, _)| *n == name) {
        if let Ok(mut guard) = ACTIVE.write() {
            *guard = theme;
        }
    }
}

/// Name of the currently active skin.
pub fn skin_name() -> &'static str {
    let active = *current();
    SKINS
        .iter()
        .find(|(_, t)| *t == active)
        .map(|(n, _)| *n)
        .unwrap_or(DEFAULT_SKIN_NAME)
}

// ============================================================
// Token accessors (same names as the former constants)
// ============================================================
#[inline]
pub fn BG() -> Color32 {
    current().bg
}
#[inline]
pub fn PANEL() -> Color32 {
    current().panel
}
#[inline]
pub fn PANEL2() -> Color32 {
    current().panel2
}
#[inline]
pub fn P_HOVER() -> Color32 {
    current().p_hover
}
#[inline]
pub fn P_ACTIVE() -> Color32 {
    current().p_active
}
#[inline]
pub fn LINE() -> Color32 {
    current().line
}
#[inline]
pub fn LINE2() -> Color32 {
    current().line2
}
#[inline]
pub fn BLUE() -> Color32 {
    current().blue
}
#[inline]
pub fn GREEN() -> Color32 {
    current().green
}
#[inline]
pub fn RED() -> Color32 {
    current().red
}
#[inline]
pub fn AMBER() -> Color32 {
    current().amber
}
#[inline]
pub fn PL_LINK() -> Color32 {
    current().pl_link
}
#[inline]
pub fn PL_LINK_DIM() -> Color32 {
    current().pl_link_dim
}
#[inline]
pub fn PL_SNAP_DIM() -> Color32 {
    current().pl_snap_dim
}
#[inline]
pub fn SEQPL() -> Color32 {
    current().seqpl
}
#[inline]
pub fn SEQPL_DIM() -> Color32 {
    current().seqpl_dim
}
#[inline]
pub fn INK() -> Color32 {
    current().ink
}
#[inline]
pub fn INK2() -> Color32 {
    current().ink2
}
#[inline]
pub fn INK3() -> Color32 {
    current().ink3
}
#[inline]
pub fn FAINT() -> Color32 {
    current().faint
}
#[inline]
pub fn CELL_EMPTY_BEAT() -> Color32 {
    current().cell_empty_beat
}
#[inline]
pub fn CELL_EMPTY_OFF() -> Color32 {
    current().cell_empty_off
}
#[inline]
pub fn CELL_CURRENT() -> Color32 {
    current().cell_current
}
#[inline]
pub fn CELL_DISABLED() -> Color32 {
    current().cell_disabled
}
#[inline]
pub fn FUSION_FILL() -> Color32 {
    current().fusion_fill
}
#[inline]
pub fn CELL_SEQPL_OFF() -> Color32 {
    current().cell_seqpl_off
}
#[inline]
pub fn CELL_PL_SNAP_OFF() -> Color32 {
    current().cell_pl_snap_off
}
#[inline]
pub fn CELL_PL_LINK_OFF() -> Color32 {
    current().cell_pl_link_off
}
#[inline]
pub fn SONG_EMPTY() -> Color32 {
    current().song_empty
}
#[inline]
pub fn DANGER() -> Color32 {
    current().danger
}
#[inline]
pub fn DRAG_TARGET() -> Color32 {
    current().drag_target
}
#[inline]
pub fn SOLO_FILL() -> Color32 {
    current().solo_fill
}

// ============================================================
// Color helpers
// ============================================================
#[inline]
pub fn white_a(alpha: u8) -> Color32 {
    Color32::from_white_alpha(alpha)
}

#[inline]
pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(
        (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
        (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
        (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
        (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
    )
}

// ============================================================
// Primitives (rayons, gaps, strokes)
// ============================================================
// Radius scheme (Skeuo spec — RADIUS.md / SPEC-COMPUTED.md):
// 3 (tags, sliders) -> 4 (pads, lane name, LCD) -> 5 (keycaps, wells, blocks) -> 7 (plates, popups).
pub const RADIUS_PAD: f32 = 4.0;

// Skeuo surfaces (SPEC-COMPUTED.md / skeuo_theme.rs) — skin-independent metal look.
pub const WINDOW_BG_TOP: Color32 = Color32::from_rgb(42, 43, 48);
pub const WINDOW_BG_BOT: Color32 = Color32::from_rgb(30, 31, 35);
pub const HEADER_TOP: Color32 = Color32::from_rgb(61, 62, 68);
pub const HEADER_MID: Color32 = Color32::from_rgb(43, 44, 49);
pub const HEADER_BOT: Color32 = Color32::from_rgb(38, 39, 43);
/// Recessed grid well (seqwrap) — darker inset behind the pads.
pub const WELL_FILL: Color32 = Color32::from_rgb(29, 30, 34);
/// Lane Editor panel surface — skeuo chassis grey (replaces the old near-black
/// `PANEL()` that made the whole right column read as a black rectangle).
pub const PANEL_SKEUO: Color32 = Color32::from_rgb(37, 38, 43);
/// Header/tab band of the Lane Editor — slightly lighter than the panel body.
pub const PANEL_SKEUO_HEADER: Color32 = Color32::from_rgb(44, 45, 51);
/// Hover tint for the Lane Editor tabs.
pub const PANEL_SKEUO_HOVER: Color32 = Color32::from_rgb(52, 53, 60);
/// Dark border of plates/wells (#121215).
pub const PANEL_BORDER: Color32 = Color32::from_rgb(18, 18, 21);
/// Keycap (bouton biseauté) : texte au repos.
pub const INK_KEYCAP: Color32 = Color32::from_rgb(224, 226, 232);
pub const RADIUS_CTL: f32 = 5.0;
pub const RADIUS_PANEL: f32 = 7.0;
pub const GAP_TIGHT: f32 = 3.0;

// Hauteurs / tailles standard
pub const CTL_HEIGHT: f32 = 26.0;
pub const HEADER_H: f32 = 44.0;
pub const LANE_H: f32 = 24.0;
pub const STEP_H: f32 = 21.0;
pub const TAG_SIZE: f32 = 21.0;

// ============================================================
// Font helpers — weighted IBM Plex families (registered in install_egui_fonts).
// Convention: "chiffre = mono, mot = sans" → mono_* for numbers/values/codes.
// ============================================================
/// [282] Every text size of the UI grows by this much (Iso: text too small
/// to read for long). Applied by the font helpers below and by `font_size`.
pub const FONT_BUMP: f32 = 1.0;

/// A raw text size from the UI code, with the [282] bump.
#[inline]
pub fn font_size(size: f32) -> f32 {
    size + FONT_BUMP
}

#[inline]
pub fn f_sans(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Proportional)
}
#[inline]
pub fn f_sans_med(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Name("sans_med".into()))
}
#[inline]
pub fn f_sans_sb(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Name("sans_sb".into()))
}
#[inline]
pub fn f_sans_bold(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Name("sans_bold".into()))
}
#[inline]
pub fn f_mono(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Monospace)
}
#[inline]
pub fn f_mono_med(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Name("mono_med".into()))
}
#[inline]
pub fn f_mono_sb(size: f32) -> FontId {
    FontId::new(font_size(size), FontFamily::Name("mono_sb".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Single test because the active skin is process-global state and tests
    // run in parallel: sequential checks must not interleave.
    #[test]
    fn skin_switching_updates_tokens() {
        set_skin(DEFAULT_SKIN_NAME);
        assert_eq!(skin_name(), "Dark");
        assert_eq!(BG(), SKIN_DARK.bg);
        assert_eq!(BLUE(), SKIN_DARK.blue);

        set_skin("Midnight");
        assert_eq!(skin_name(), "Midnight");
        assert_eq!(BG(), SKIN_MIDNIGHT.bg);
        assert_eq!(BLUE(), SKIN_MIDNIGHT.blue);

        set_skin("Ember");
        assert_eq!(skin_name(), "Ember");
        assert_eq!(BG(), SKIN_EMBER.bg);

        set_skin("DoesNotExist");
        assert_eq!(skin_name(), "Ember");

        set_skin(DEFAULT_SKIN_NAME);
        assert_eq!(skin_name(), "Dark");
        assert_eq!(BG(), SKIN_DARK.bg);
    }
}
