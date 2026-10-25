//! Sound Editor: tabbed Sound/Track panel, editor rows, lane layout presets.

use crate::instrument_registry::StandardField;
use crate::param_id::ParamId;
use crate::sequencer::SharedPattern;
use crate::sound_settings::SoundSettingsState;
use crate::synthesis::{self, DrumVoice, VoiceSettings};
use crate::track::TrackLayoutState;
use crate::ui::controls::algo_combo;
use crate::ui::editor_state::{
    schema_voice_idx, select_legacy_track, EditorUIState, SoundEditorTab,
};
use crate::ui::envelope_viz::{
    draw_amp_envelope, draw_buzz_filter_envelope, draw_buzz_gate_graph, draw_duck_graph, draw_filter_envelope,
    draw_oneshot_amp_graph, draw_oneshot_filter_graph, draw_oneshot_graph,
    draw_oneshot_pitch_graph, draw_pitch_envelope, draw_sample_amp_graph, draw_sample_filter_graph,
    draw_texture_graph,
};
use crate::ui::fmt::{freq_to_note, note_name, note_to_freq};
use crate::ui::help::{self, HelpExt};
use crate::ui::param_source::{GlobalSource, ParamSource, PlockSource};
use crate::ui::pattern_bank::load_pattern_for_ui;
use crate::ui::slider;
use crate::ui::theme::*;
use crate::ui::widgets::{styled_select, ToggleSwitch};
use crate::{preset_dumps, DrumFlashParams};
use nih_plug::{params::persist::PersistentField, prelude::*};
use nih_plug_egui::egui::{self, Color32, RichText, Vec2};
use std::sync::atomic::Ordering;

pub const EDITOR_LABEL_W: f32 = 138.0;
pub const EDITOR_PARAMS_W: f32 = 340.0;
/// [282] Wide enough for the longest readout ("12000 Hz") inside its field.
pub const EDITOR_VALUE_W: f32 = 64.0;

pub fn format_editor_value(value: f32, suffix: Option<&str>) -> String {
    let number = if value.abs() >= 100.0 {
        format!("{:.0}", value)
    } else if value.abs() >= 10.0 {
        format!("{:.1}", value)
    } else {
        format!("{:.2}", value)
    };
    match suffix {
        Some(s) => format!("{}{}", number, s),
        None => number,
    }
}

/// Left-aligned label that occupies EXACTLY the fixed column width, so every
/// row's control starts at the same x. `allocate_ui_with_layout` shrinks to the
/// text (ragged left edges) — we allocate the exact box and paint the label.
pub fn editor_label(ui: &mut egui::Ui, text: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(EDITOR_LABEL_W, 22.0), egui::Sense::hover());
    // [233] The saturation's sub-parameters step their label in; the sliders
    // stay aligned.
    ui.painter().text(
        egui::pos2(rect.left() + sub_indent(), rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        f_sans_med(11.5),
        INK2(),
    );
}

// [233] Indent applied to the next labels drawn: the saturation sub-rows set
// it before drawing and clear it after. UI thread only.
thread_local! {
    static SUB_INDENT: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}
const SUB_INDENT_PX: f32 = 14.0;

fn set_sub_indent(px: f32) {
    SUB_INDENT.with(|c| c.set(px));
}

fn sub_indent() -> f32 {
    SUB_INDENT.with(|c| c.get())
}

/// Section header (separator + title) shared by the Sound and Track tabs.
///
/// [282] Accent title at the labels' size, with more room above the rule
/// than below: muted and smaller, it read as one more parameter row and the
/// panel was one undivided block.
pub fn editor_section_header(ui: &mut egui::Ui, title: &str) {
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(6.0);
    ui.label(RichText::new(title).font(f_sans_sb(11.5)).color(BLUE()));
    ui.add_space(6.0);
}

/// [282] A row's value, in its recessed readout at the end of the row: every
/// readout of the Sound tab lines up in one column.
fn editor_value_readout(ui: &mut egui::Ui, text: &str) {
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(EDITOR_VALUE_W, 22.0), egui::Sense::hover());
    crate::ui::skeuo::value_readout(ui, rect, text, INK());
}

/// [282] How far a Sound-tab module reaches past the content column on each
/// side: its title then sits on the labels' x, and the p-lock / morph marker
/// in the gutter lands on the module's inner edge.
const MODULE_INSET: f32 = crate::ui::skeuo::MODULE_TITLE_X - ROW_GUTTER_W;

/// [282] A Sound-tab section being laid out inside its module. The module is
/// painted by [`module_end`] into the slot reserved here, under the rows.
struct ModuleSlot {
    idx: egui::layers::ShapeIdx,
    top: f32,
    title: Option<&'static str>,
    /// How far right the title sits (room for a control before it).
    title_dx: f32,
}

fn module_begin(ui: &mut egui::Ui, title: Option<&'static str>) -> ModuleSlot {
    let idx = ui.painter().add(egui::Shape::Noop);
    let top = ui.cursor().top();
    ui.add_space(if title.is_some() {
        crate::ui::skeuo::MODULE_STRIP_H + 8.0
    } else {
        6.0
    });
    ModuleSlot { idx, top, title, title_dx: 0.0 }
}

fn module_end(ui: &mut egui::Ui, module: ModuleSlot) {
    ui.add_space(if module.title.is_some() { 10.0 } else { 6.0 });
    let content = ui.max_rect();
    let rect = egui::Rect::from_min_max(
        egui::pos2(content.left() - MODULE_INSET, module.top),
        egui::pos2(content.right() + MODULE_INSET, ui.min_rect().bottom()),
    );
    ui.painter().set(
        module.idx,
        crate::ui::skeuo::editor_module_indented(ui.ctx(), rect, module.title, module.title_dx),
    );
    // Gap to the next module (plus the item spacing): wide enough that two
    // modules' edges read as two frames, not as a bundle of lines.
    ui.add_space(12.0);
}

pub fn draw_editor_slider_track(
    ui: &mut egui::Ui,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    track_w: f32,
) -> egui::Response {
    slider::draw_track(
        ui,
        value,
        min,
        max,
        default,
        logarithmic,
        track_w.max(60.0),
        slider::TrackStyle::editor(),
    )
}

/// Same track with a response curve ([189]): the value moves more slowly at
/// the low end of the range, so a parameter whose audible effect saturates
/// early gets usable resolution where it matters.
fn draw_editor_slider_track_curved(
    ui: &mut egui::Ui,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    track_w: f32,
    curve: f32,
) -> egui::Response {
    slider::draw_track(
        ui,
        value,
        min,
        max,
        default,
        logarithmic,
        track_w.max(60.0),
        slider::TrackStyle::editor().with_curve(curve),
    )
}

/// Same track with a quantisation step (e.g. 1.0 for integer semitones).
fn draw_editor_slider_track_stepped(
    ui: &mut egui::Ui,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    track_w: f32,
    step: f32,
) -> egui::Response {
    slider::draw_track(
        ui,
        value,
        min,
        max,
        default,
        logarithmic,
        track_w.max(60.0),
        slider::TrackStyle::editor().with_step(step),
    )
}

/// [282] "Mode" row under a bass drum's Frequency: Hz / Note, flush right
/// like the menus. Returns the new mode (true = notes) when clicked.
fn draw_freq_mode_row(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    notes_active: bool,
) -> Option<bool> {
    let selected = if notes_active { 1 } else { 0 };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        segmented_row(ui, "Mode", id_salt, &["Hz", "Note"], selected)
    })
    .inner
    .map(|new| new == 1)
}

fn draw_note_step_button(ui: &mut egui::Ui, left: bool) -> egui::Response {
    // Keycap with a PAINTED triangle (◂ / ▸) — the glyphs are missing from Plex.
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(24.0, 22.0), egui::Sense::click());
    crate::ui::skeuo::keycap(ui, rect, crate::ui::widgets::KeycapState::Rest);
    if resp.is_pointer_button_down_on() {
        ui.painter()
            .rect_filled(rect, 5.0, Color32::from_black_alpha(60));
    }
    let c = rect.center();
    let s = 4.0;
    let tri = if left {
        vec![
            egui::pos2(c.x + s * 0.4, c.y - s),
            egui::pos2(c.x + s * 0.4, c.y + s),
            egui::pos2(c.x - s * 0.6, c.y),
        ]
    } else {
        vec![
            egui::pos2(c.x - s * 0.4, c.y - s),
            egui::pos2(c.x - s * 0.4, c.y + s),
            egui::pos2(c.x + s * 0.6, c.y),
        ]
    };
    ui.painter()
        .add(egui::Shape::convex_polygon(tri, INK(), egui::Stroke::NONE));
    resp
}

pub struct EditorFrequencyRowResult {
    pub response: egui::Response,
    pub value_changed: bool,
}

/// Frequency row of the voices that can show it as a note. [282] The Hz /
/// Note switch has its own "Mode" row under it ([`draw_freq_mode_row`]): in
/// this row it pushed the params column 70 px wider than every other
/// section's. Here: label, then the Hz slider + readout or the note stepper,
/// in the same width as any slider row.
fn draw_editor_frequency_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    ratio: f32,
    notes_active: bool,
) -> EditorFrequencyRowResult {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        editor_label(ui, label);

        let mut value_changed = false;
        // [282] No zero-size placeholder response here: as an item it took
        // one item spacing (8 px), so this track started later and ended
        // shorter than every other row's.
        let row_response;
        // Both modes take the SAME width: the Hz slider + value. Narrower than
        // ◂ note ▸ in a tight column, it let the Notes row overflow by 2 px,
        // which shifted every right-aligned row below it.
        let track_w = (ui.available_width() - EDITOR_VALUE_W - 8.0).max(60.0);
        let group_w = track_w + 8.0 + EDITOR_VALUE_W;

        if notes_active {
            let note_val = freq_to_note(*value * ratio).round();
            row_response = ui.allocate_ui_with_layout(
                Vec2::new(group_w, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    if draw_note_step_button(ui, true).clicked() {
                        let new_note = (note_val - 1.0).max(0.0);
                        *value = note_to_freq(new_note) / ratio;
                        value_changed = true;
                    }
                    // What the two step buttons and their gaps leave.
                    let note_w = (group_w - 2.0 * 24.0 - 2.0 * 8.0).max(40.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(note_w, 22.0),
                        egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            ui.label(
                                RichText::new(note_name(note_val))
                                    .font(f_mono_sb(13.0))
                                    .color(INK()),
                            );
                        },
                    );
                    if draw_note_step_button(ui, false).clicked() {
                        let new_note = (note_val + 1.0).min(127.0);
                        *value = note_to_freq(new_note) / ratio;
                        value_changed = true;
                    }
                },
            )
            .response;
        } else {
            row_response =
                draw_editor_slider_track(ui, value, min, max, default, logarithmic, track_w);
            value_changed = row_response.changed();
            editor_value_readout(ui, &format_editor_value(*value, Some("Hz")));
        }

        EditorFrequencyRowResult {
            response: row_response,
            value_changed,
        }
    })
    .inner
}

pub fn draw_editor_slider_row(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    suffix: Option<&str>,
) -> egui::Response {
    draw_editor_slider_row_curved(
        ui,
        label,
        value,
        min,
        max,
        default,
        logarithmic,
        suffix,
        1.0,
    )
}

/// Editor slider row whose track carries a response curve ([189]).
/// `curve == 1.0` is the plain linear row.
#[allow(clippy::too_many_arguments)]
pub fn draw_editor_slider_row_curved(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    suffix: Option<&str>,
    curve: f32,
) -> egui::Response {
    draw_editor_slider_row_inner(
        ui,
        label,
        value,
        min,
        max,
        default,
        logarithmic,
        suffix,
        0.0,
        curve,
    )
}

/// Full editor slider row with an optional quantisation step (0 = continuous).
pub fn draw_editor_slider_row_full(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    suffix: Option<&str>,
    step: f32,
) -> egui::Response {
    draw_editor_slider_row_inner(
        ui,
        label,
        value,
        min,
        max,
        default,
        logarithmic,
        suffix,
        step,
        1.0,
    )
}

/// Shared body of every editor slider row. A row is either stepped or curved,
/// never both: quantisation belongs to discrete-ish values, the curve to
/// continuous amounts.
#[allow(clippy::too_many_arguments)]
fn draw_editor_slider_row_inner(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    logarithmic: bool,
    suffix: Option<&str>,
    step: f32,
    curve: f32,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        editor_label(ui, label);

        // Track flexes to fill the fixed-width params column.
        let track_w = (ui.available_width() - EDITOR_VALUE_W - 8.0).max(60.0);
        let response = if step > 0.0 {
            draw_editor_slider_track_stepped(
                ui,
                value,
                min,
                max,
                default,
                logarithmic,
                track_w,
                step,
            )
        } else if curve != 1.0 {
            draw_editor_slider_track_curved(
                ui,
                value,
                min,
                max,
                default,
                logarithmic,
                track_w,
                curve,
            )
        } else {
            draw_editor_slider_track(ui, value, min, max, default, logarithmic, track_w)
        };

        editor_value_readout(ui, &format_editor_value(*value, suffix));
        response
    })
    .inner
}

/// Width of the Sound panel's scroll bar ([190]).
const SCROLLBAR_W: f32 = 10.0;

/// Give the Sound panel's scroll bar the same recessed-groove-and-pill look as
/// every slider ([190]).
///
/// egui's default scroll style is `floating` with a **dormant opacity of zero**,
/// so at rest the bar was simply not drawn — there was nothing to see until the
/// pointer entered the panel. It stays floating (`floating_allocated_width` 0),
/// so making it permanent costs no layout: the content inset below already
/// keeps the rows clear of it, and no row shifts by a pixel.
///
/// Every opacity is pinned to 1.0 because they would otherwise make the handle
/// *dimmer* on hover than at rest; the hover feedback is a colour change
/// instead, like the rest of the panel.
fn style_scroll_bar(ui: &mut egui::Ui) {
    let style = ui.style_mut();
    style.spacing.scroll = egui::style::ScrollStyle {
        floating: true,
        floating_allocated_width: 0.0,
        bar_width: SCROLLBAR_W,
        floating_width: SCROLLBAR_W,
        handle_min_length: 24.0,
        // false = the handle takes its colour from the widget fills below.
        foreground_color: false,
        dormant_background_opacity: 1.0,
        active_background_opacity: 1.0,
        interact_background_opacity: 1.0,
        dormant_handle_opacity: 1.0,
        active_handle_opacity: 1.0,
        interact_handle_opacity: 1.0,
        ..egui::style::ScrollStyle::solid()
    };
    // The groove: the same channel colour as a slider track.
    style.visuals.extreme_bg_color = crate::ui::skeuo::groove_fill();
    let pill = |w: &mut egui::style::WidgetVisuals, fill: egui::Color32| {
        w.bg_fill = fill;
        w.corner_radius = egui::CornerRadius::same((SCROLLBAR_W * 0.5) as u8);
    };
    // Quiet at rest so it never competes with the blue value fills, brighter
    // under the pointer, brightest while dragging.
    pill(&mut style.visuals.widgets.inactive, FAINT());
    pill(&mut style.visuals.widgets.hovered, INK3());
    pill(&mut style.visuals.widgets.active, INK2());
}

/// [184] Width reserved at the left of every Lane Editor row, in **every** scope.
///
/// Always reserved, never conditional: if the gutter only existed in p-lock
/// scope, selecting a cell would shift every slider by 14 px ("UI zones
/// stables" — reserve the space, do not move things).
const ROW_GUTTER_W: f32 = 14.0;

/// Draws one row's override marker and returns `true` when the user asked to
/// hand the parameter back to the lane.
///
/// In lane scope this is empty space. In p-lock scope an overridden row gets an
/// accent bar, clickable to revert — the bar IS the affordance, which keeps the
/// label ASCII-only (a glyph like a revert arrow is exactly what produced the
/// mojibake of task [164]).
fn row_gutter(ui: &mut egui::Ui, overridden: bool) -> bool {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ROW_GUTTER_W, 22.0),
        if overridden {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    if !overridden {
        return false;
    }
    let hovered = response.hovered();
    // 18 px: the top stays where the label's cap-height starts, the bottom runs
    // 4 px lower so the mark reads as belonging to the whole row.
    let bar = egui::Rect::from_min_size(
        egui::pos2(rect.left() + 3.0, rect.center().y - 7.0),
        Vec2::new(3.0, 18.0),
    );
    ui.painter()
        .rect_filled(bar, 1.0, if hovered { INK() } else { PL_LINK() });
    response
        .help_or(
            help::EDITOR_GUTTER,
            "Overrides the lane - click to follow the lane again",
        )
        .clicked()
}

/// [221] Right-click on a slider row: write that parameter on all 16 cells of
/// the page the grid is showing. **Spread** lays `min -> max` (or back) across
/// the page, **Scatter** draws a random value per cell, **Clear** drops those
/// locks. Any lockable continuous parameter of any instrument gets it: this is
/// what "offset proportional to the step" became, and it is worth as much on a
/// filter or a pitch as on Rift's Offset. Cells beyond the lane's length or
/// inside a fused block take the value too; it simply waits there.
#[allow(clippy::too_many_arguments)]
fn page_lock_menu(
    response: &egui::Response,
    plock: &crate::plock::PlockState,
    slot: usize,
    page: usize,
    id: ParamId,
    min: f32,
    max: f32,
    logarithmic: bool,
) {
    let Some(field) = id.plock_field() else {
        return;
    };
    response.context_menu(|ui| {
        use crate::ui::menus::{context_menu_button, context_menu_separator};
        ui.spacing_mut().item_spacing.y = 4.0;
        ui.set_min_width(176.0);
        ui.set_max_width(176.0);
        let page_no = page + 1;
        let mut done = false;
        if context_menu_button(ui, &format!("Spread up on page {page_no}"), INK(), true).clicked() {
            plock.fill_page(
                slot,
                page,
                field,
                &crate::plock::spread_values(min, max, logarithmic, false),
            );
            done = true;
        }
        if context_menu_button(ui, &format!("Spread down on page {page_no}"), INK(), true).clicked()
        {
            plock.fill_page(
                slot,
                page,
                field,
                &crate::plock::spread_values(min, max, logarithmic, true),
            );
            done = true;
        }
        if context_menu_button(ui, &format!("Scatter on page {page_no}"), INK(), true).clicked() {
            // Seeded from the clock: every click is a new draw, and the
            // values then live in the p-locks, frozen and editable.
            let seed = (ui.input(|i| i.time) * 1_000_003.0) as u64 as u32 | 1;
            plock.fill_page(
                slot,
                page,
                field,
                &crate::plock::scatter_values(min, max, logarithmic, seed),
            );
            done = true;
        }
        context_menu_separator(ui);
        if context_menu_button(ui, &format!("Clear locks on page {page_no}"), INK(), true).clicked()
        {
            plock.clear_field_on_page(slot, page, field);
            done = true;
        }
        if done {
            ui.close_menu();
        }
    });
}

/// [227] One continuous special parameter as a plain slider row - gutter,
/// scope greying and the page menu included - for the rows the panel places
/// under a standard row rather than in the specials loop.
fn draw_plain_special_row(
    ui: &mut egui::Ui,
    src: &mut dyn ParamSource,
    plock: &crate::plock::PlockState,
    slot: usize,
    page: usize,
    def: &crate::instrument_registry::SpecialParamDef,
) {
    let id = ParamId::Special(def.special_index);
    let mut value = src.get(id);
    let inherited = src.inherited(id);
    let logarithmic = def.min > 0.0 && def.max / def.min >= 20.0;
    let help_text = row_help(
        &*src,
        id,
        format!("{}{}", help::special_param(def.name), help::SLIDER_GESTURES),
    );
    let (reverted, edited) = help::zone(ui, ("plain_special", def.name), &help_text, |ui| {
        row_scoped(ui, src.is_overridden(id), src.supports(id).reason(), |ui| {
            let row = draw_editor_slider_row_curved(
                ui,
                def.label,
                &mut value,
                def.min,
                def.max,
                inherited,
                logarithmic,
                def.unit,
                def.curve,
            );
            page_lock_menu(&row, plock, slot, page, id, def.min, def.max, logarithmic);
            row.changed()
        })
    });
    if reverted {
        src.clear(id);
    } else if edited {
        src.set(id, value);
    }
}

/// [228] Labels of Rift's Texture menu for one lane: the embedded textures
/// by name, then the lane's own file - its name when loaded, "name (missing)"
/// when the file is gone, "Custom (empty)" when the lane has none.
fn texture_menu_labels(options: &[&str], params: &DrumFlashParams, lane: usize) -> Vec<String> {
    use crate::synthesis::sample_bank::CUSTOM_TEXTURE_INDEX;
    use crate::user_textures::SlotStatus;
    options
        .iter()
        .enumerate()
        .map(|(i, name)| {
            if i != CUSTOM_TEXTURE_INDEX {
                return name.to_string();
            }
            let textures = &params.user_textures;
            match textures.status(lane) {
                SlotStatus::Empty => format!("{name} (empty)"),
                SlotStatus::Loaded => textures.file_name(lane).unwrap_or_default(),
                SlotStatus::Missing => {
                    format!("{} (missing)", textures.file_name(lane).unwrap_or_default())
                }
            }
        })
        .collect()
}

/// [228] The custom texture belongs to a lane only while that lane hosts an
/// instrument with a Texture menu: changing the lane's kind, or removing the
/// lane, drops the file (user decision, 2026-09-21). Once per frame.
fn reconcile_lane_textures(params: &DrumFlashParams) {
    for lane in 0..crate::track::MAX_TRACKS {
        if params.user_textures.path(lane).is_none() {
            continue;
        }
        let has_texture_menu = params
            .track_layout
            .state
            .kind_for_slot(lane)
            .map(|kind| {
                kind.instrument_def()
                    .special_params
                    .iter()
                    .any(|d| d.name.ends_with("_texture"))
            })
            .unwrap_or(false);
        if !has_texture_menu {
            params.user_textures.clear(lane);
        }
    }
}

/// [242] A macro assignment lives only while its lane offers the target:
/// changing a lane's kind (or removing the lane) drops the assignment whose
/// special index / standard field means something else on the new kind — the
/// same rule as the lane textures above. Once per frame.
fn reconcile_lane_macros(params: &DrumFlashParams) {
    params
        .macro_map_state
        .state
        .prune_invalid_targets(&params.track_layout.state);
}

/// [228] Collect the answer of a file dialog opened by `draw_user_texture_row`.
/// Once per frame, before the rows: the dialog thread writes its result into
/// the shared cell, the UI thread decodes the file here.
fn poll_texture_pick(
    ctx: &egui::Context,
    params: &DrumFlashParams,
    state: &mut EditorUIState,
    sound_settings: &SoundSettingsState,
) {
    let Some(pick) = state.texture_pick.as_ref() else {
        return;
    };
    let answer = pick.result.lock().ok().and_then(|r| r.clone());
    match answer {
        None => {
            // Still open: keep the frames coming so the answer is seen.
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        Some(picked) => {
            let lane = pick.slot;
            state.texture_pick = None;
            if let Some(path) = picked {
                if params.user_textures.load(lane, &path).is_ok() {
                    // The file was asked for: the Texture menu jumps to it.
                    let texture_def =
                        params
                            .track_layout
                            .state
                            .kind_for_slot(lane)
                            .and_then(|kind| {
                                kind.instrument_def()
                                    .special_params
                                    .iter()
                                    .find(|d| d.name.ends_with("_texture"))
                            });
                    if let (Some(def), Some(inst)) =
                        (texture_def, sound_settings.instruments.get(lane))
                    {
                        inst.set_special(
                            def.special_index,
                            crate::synthesis::sample_bank::CUSTOM_TEXTURE_INDEX as f32,
                        );
                        // A stereo file comes up in stereo; the switch is
                        // there to fold it to mono, not the other way round.
                        if params.user_textures.is_stereo(lane) {
                            store_field(inst, StandardField::Stereo, 1.0);
                        }
                        sound_settings.bump_version();
                    }
                }
            }
        }
    }
}

/// [228] The row under Rift's Texture menu: the lane's file (or why nothing
/// plays from it) and the Load / Clear buttons. The native dialog runs on its
/// own thread - blocking inside the egui frame re-entered baseview's message
/// loop and crashed the host; here the frame returns at once and
/// `poll_texture_pick` picks the answer up. (macOS would want the panel on the
/// main thread - to revisit when packaging for it.)
fn draw_user_texture_row(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    state: &mut EditorUIState,
    slot: usize,
) {
    use crate::user_textures::SlotStatus;
    let textures = &params.user_textures;
    let status = textures.status(slot);
    let picking = state.texture_pick.is_some();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        editor_label(ui, "File");
        let (text, color) = match status {
            SlotStatus::Empty => ("no file - Custom plays Noise".to_string(), INK3()),
            SlotStatus::Loaded => (textures.file_name(slot).unwrap_or_default(), INK()),
            SlotStatus::Missing => (
                format!("missing: {}", textures.file_name(slot).unwrap_or_default()),
                RED(),
            ),
        };
        let avail = ui.available_width();
        // Buttons on the right, the name takes what is left and is clipped.
        const BUTTONS_W: f32 = 52.0 + 52.0 + 8.0;
        let name_w = (avail - BUTTONS_W - 8.0).max(40.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(name_w, 22.0), egui::Sense::hover());
        ui.painter().with_clip_rect(rect).text(
            egui::pos2(rect.left(), rect.center().y),
            egui::Align2::LEFT_CENTER,
            &text,
            f_sans_med(10.5),
            color,
        );
        if let Some(path) = textures.path(slot) {
            let mut hover = path.to_string_lossy().into_owned();
            if let Some(err) = textures.error(slot) {
                hover.push_str("\n");
                hover.push_str(&err);
            }
            resp.on_hover_text(hover);
        }
        if crate::ui::controls::keycap_button(
            ui,
            if picking { "..." } else { "Load..." },
            52.0,
            crate::ui::widgets::KeycapState::Rest,
            !picking,
            f_sans_med(9.5),
        )
        .on_hover_text(
            "Load a WAV file as this lane's custom texture (any bit depth, stereo kept, 60 s at most) and select it in the Texture menu. The path is saved with the session and follows the lane in presets and copies.",
        )
        .clicked()
            && !picking
        {
            let result = std::sync::Arc::new(std::sync::Mutex::new(None));
            let start_dir = textures
                .path(slot)
                .and_then(|p| p.parent().map(|d| d.to_path_buf()));
            let cell = result.clone();
            std::thread::Builder::new()
                .name("flash-drum-file-dialog".into())
                .spawn(move || {
                    let mut dialog = rfd::FileDialog::new()
                        .set_title("Load a texture")
                        .add_filter("WAV audio", &["wav", "WAV"]);
                    if let Some(dir) = start_dir {
                        dialog = dialog.set_directory(dir);
                    }
                    let picked = dialog.pick_file();
                    if let Ok(mut r) = cell.lock() {
                        *r = Some(picked);
                    }
                })
                .ok();
            state.texture_pick = Some(crate::ui::editor_state::TexturePick { slot, result });
        }
        if crate::ui::controls::keycap_button(
            ui,
            "Clear",
            52.0,
            crate::ui::widgets::KeycapState::Rest,
            status != SlotStatus::Empty,
            f_sans_med(9.5),
        )
        .on_hover_text("Forget this lane's file: Custom plays Noise again.")
        .clicked()
            && status != SlotStatus::Empty
        {
            textures.clear(slot);
        }
    });
}

/// [227] Hover text of the Advance switch.
const ADVANCE_HELP: &str = "Advance: each hit of this lane (or each cell, see the options) moves the offset one step further into the texture. Off, the offset stays where the slider puts it.";

/// [227] The folded options under the Advance switch: three lane-level
/// switches stored as bits of the slot's `advance_mode` parameter.
fn draw_advance_options(
    ui: &mut egui::Ui,
    setter: &ParamSetter,
    param: &IntParam,
    slot: usize,
    advance_on: bool,
) {
    use crate::advance_mode::*;
    let mode = param.value();
    egui::CollapsingHeader::new(
        RichText::new("Advance options").font(f_sans_med(11.5)).color(INK2()),
    )
    .id_salt(("advance_options", slot))
    .default_open(false)
    // Switch off = options shut; back on, the user opens them again.
    .open(if advance_on { None } else { Some(false) })
    .show(ui, |ui| {
        let mut next = mode;
        for (bit, label, help) in [
            (
                RESET_PATTERN,
                "Reset on pattern loop",
                "The count restarts each time the pattern wraps around. Excludes Reset on page loop.",
            ),
            (
                RESET_PAGE,
                "Reset on page loop",
                "The count restarts each time a page comes round: the playhead enters another page of 16 steps, or the pattern wraps. Excludes Reset on pattern loop.",
            ),
            (
                EVERY_CELL,
                "Count every cell",
                "Advance on every cell the playhead passes, active or not, instead of on every hit of this lane.",
            ),
        ] {
            let mut value = if mode & bit != 0 { 1.0 } else { 0.0 };
            if draw_editor_switch_row(ui, label, &mut value)
                .on_hover_text(help)
                .changed()
            {
                next ^= bit;
                // The two resets exclude each other: turning one on turns
                // the other off.
                if next & bit != 0 {
                    match bit {
                        RESET_PATTERN => next &= !RESET_PAGE,
                        RESET_PAGE => next &= !RESET_PATTERN,
                        _ => {}
                    }
                }
            }
        }
        if next != mode {
            set_int_param(setter, param, next);
        }
    });
}

fn set_int_param(setter: &ParamSetter, param: &IntParam, value: i32) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

/// One row with BOTH its gutter and its greying.
///
/// `supports()` used to be consulted only by the special-param rows, so a
/// standard row like Filter accepted a drag that the scope then refused in
/// silence. Every row now goes through here, which is what makes "greyed with a
/// reason" a rule rather than a special case.
fn row_scoped<R>(
    ui: &mut egui::Ui,
    overridden: bool,
    unsupported: Option<&'static str>,
    row: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let inner = ui.add_enabled_ui(unsupported.is_none(), |ui| with_gutter(ui, overridden, row));
    if let Some(reason) = unsupported {
        // [283] Disabled: only the disabled variant ever shows.
        inner.response.on_disabled_hover_text(reason);
    }
    inner.inner
}

/// [283] The help of a row, or nothing when the row is greyed with a reason
/// of its own (`row_scoped` shows it; two tooltips would stack).
fn row_help(src: &dyn ParamSource, id: ParamId, text: String) -> String {
    if src.supports(id).reason().is_some() {
        String::new()
    } else {
        text
    }
}

/// Wraps one row so it carries its gutter, without touching the row helpers'
/// signatures. Spacing is zeroed so the row lands exactly `ROW_GUTTER_W` to the
/// right instead of that plus egui's default item spacing.
fn with_gutter<R>(
    ui: &mut egui::Ui,
    overridden: bool,
    row: impl FnOnce(&mut egui::Ui) -> R,
) -> (bool, R) {
    let mut reverted = false;
    let inner = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            reverted = row_gutter(ui, overridden);
            row(ui)
        })
        .inner;
    (reverted, inner)
}

/// [184] Bridges the panel's algo `IntParam` into the value-source layer, which
/// must not know about `ParamSetter` (it needs a live `GuiContext`, which is
/// what would make the layer untestable headless).
struct PanelAlgo<'a> {
    param: &'a nih_plug::prelude::IntParam,
    setter: &'a ParamSetter<'a>,
}

impl crate::ui::param_source::AlgoSink for PanelAlgo<'_> {
    fn get(&self) -> u8 {
        self.param.value().max(0) as u8
    }

    fn set(&self, value: u8) {
        crate::ui::controls::set_int_param_if_changed(self.setter, self.param, value as i32);
    }
}

/// Label + right-aligned segmented selector, for parameters that are a CHOICE
/// between two named modes rather than an on/off state ([181] — SDrex's
/// Modulation row used to be an anonymous switch labelled "Filter Mod").
fn segmented_row(
    ui: &mut egui::Ui,
    label: &str,
    id_salt: impl std::hash::Hash,
    options: &[&str],
    selected: usize,
) -> Option<usize> {
    editor_label(ui, label);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let new = crate::ui::skeuo::segmented(ui, id_salt, options, selected);
        if new != selected {
            Some(new)
        } else {
            None
        }
    })
    .inner
}

/// Skeuo dropdown flush to the params column's right edge (matches the mockup:
/// dropdowns line up with slider values and toggles, not left after the label).
/// The caller draws `editor_label` first, then calls this in the same row.
fn right_aligned_select(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    selected: usize,
    options: &[&str],
) -> Option<usize> {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        styled_select(ui, id_salt, selected, options, 146.0).1
    })
    .inner
}

pub fn draw_editor_switch_row(ui: &mut egui::Ui, label: &str, value: &mut f32) -> egui::Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        editor_label(ui, label);
        // Push the switch to the row's right edge (.ctl--switch: space-between).
        let avail = ui.available_width();
        ui.add_space((avail - 34.0).max(0.0));
        let checked = *value >= 0.5;
        let mut response = ui.add(ToggleSwitch::new(checked));
        if response.clicked() {
            *value = if checked { 0.0 } else { 1.0 };
            response.mark_changed();
        }
        response
    })
    .inner
}

fn draw_track_tab(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    setter: &ParamSetter,
    state: &mut EditorUIState,
) {
    let slot_idx = state.selected_track_slot;
    let layout_state =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    let slot = &layout_state.slots[slot_idx];

    if !slot.active {
        ui.add_space(12.0);
        ui.label(
            RichText::new("Empty slot - click a +N chip in the grid to add a module.")
                .font(f_sans_med(11.0))
                .color(INK3()),
        );
        return;
    }

    let mut new_state = layout_state.clone();
    let mut changed = false;

    // Keep the name buffer in sync with the selected slot.
    if state.track_name_input_slot != Some(slot_idx) {
        state.track_name_input = slot.name.clone();
        state.track_name_input_slot = Some(slot_idx);
    }

    ui.add_space(2.0);

    // ---- Instrument ----
    editor_section_header(ui, "Instrument");

    // Name — same 146×26 keycap box as the Type/Aux Out dropdowns so the right
    // edges line up.
    help::zone(ui, "track_name", help::TRACK_NAME, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            editor_label(ui, "Name");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(146.0, CTL_HEIGHT), egui::Sense::hover());
                crate::ui::widgets::keycap_tex(ui, rect, crate::ui::widgets::KeycapState::Rest);
                let response = ui
                    .allocate_new_ui(
                        egui::UiBuilder::new()
                            .max_rect(rect.shrink2(Vec2::new(8.0, 0.0)))
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut state.track_name_input)
                                    .id(egui::Id::new(("track_name", slot_idx)))
                                    .char_limit(6)
                                    .desired_width(f32::INFINITY)
                                    .font(f_sans_med(12.0))
                                    .text_color(Color32::WHITE)
                                    .frame(false),
                            )
                        },
                    )
                    .inner;
                if state.track_name_focus_request {
                    state.track_name_focus_request = false;
                    response.request_focus();
                }
                if response.changed() {
                    new_state.slots[slot_idx].name = state.track_name_input.clone();
                    changed = true;
                }
            });
        })
    });

    // Type (instrument kind) — shared category▸kind cascade, identical to the
    // empty-lane Add-Module popup and the lane right-click menu.
    let current_kind = slot.kind;
    help::zone(ui, "track_type", help::TRACK_TYPE, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            editor_label(ui, "Type");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Same 146×CTL_HEIGHT field as the Aux Out / Choke selectors, but it
                // opens the SHARED category▸kind cascade (identical to the empty-lane
                // +N chip and the lane right-click).
                let type_btn = egui::Button::new(
                    RichText::new(format!("{}", current_kind.default_name()))
                        .font(f_mono_med(11.0))
                        .color(INK()),
                )
                .fill(P_ACTIVE())
                .stroke(egui::Stroke::new(1.0, LINE2()))
                .corner_radius(RADIUS_CTL)
                .min_size(Vec2::new(146.0, CTL_HEIGHT));
                let picker = egui::menu::menu_custom_button(ui, type_btn, |ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    ui.set_min_width(130.0);
                    crate::ui::menus::instrument_category_menu(ui, Some(current_kind), slot_idx)
                });
                if let Some(kind) = picker.inner.flatten() {
                    if kind != current_kind {
                        let label = kind.default_name();
                        new_state.slots[slot_idx].kind = kind;
                        new_state.slots[slot_idx].name = label.to_string();
                        // [304] The LFO syncs belonged to the old instrument's LFOs.
                        new_state.slots[slot_idx].lfo_sync = [0; crate::track::LFO_SYNC_SLOTS];
                        // [285] A new instrument: its type's Randomize density.
                        new_state.slots[slot_idx].randomize_density =
                            state.global_config.randomize_density_for(kind);
                        new_state.slots[slot_idx].midi_note = kind.default_midi_note();
                        state.track_name_input = label.to_string();
                        changed = true;
                        // New kind → new voice: seed the slot's settings from the new
                        // instrument's defaults (the audio thread reinitializes it).
                        sound_settings.reset_slot_to_defaults(
                            slot_idx,
                            kind,
                            state.global_config.default_analog,
                        );
                        state.selected_instrument = slot_idx;
                    }
                }
            });
        })
    });

    // ---- Preset ----
    // Quick loader for saved instrument presets matching this lane's kind.
    editor_section_header(ui, "Preset");
    let preset_kind = new_state.slots[slot_idx].kind;
    let preset_cache_key = (slot_idx, preset_kind.index());
    if state.track_preset_cache_key != Some(preset_cache_key) {
        state.track_preset_entries = crate::presets::list_instrument_presets(preset_kind.index());
        state.track_preset_cache_key = Some(preset_cache_key);
    }
    help::zone(ui, "track_preset", help::TRACK_PRESET, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            editor_label(ui, "Load");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if state.track_preset_entries.is_empty() {
                    ui.label(
                        RichText::new("No presets")
                            .font(f_sans_med(11.0))
                            .color(INK3()),
                    );
                } else {
                    // Action dropdown: index 0 is a placeholder, 1.. are the presets.
                    let mut pending: Option<crate::presets::InstrumentPreset> = None;
                    {
                        let mut labels: Vec<&str> =
                            Vec::with_capacity(state.track_preset_entries.len() + 1);
                        labels.push("Load preset…");
                        for e in &state.track_preset_entries {
                            labels.push(e.name.as_str());
                        }
                        if let (_, Some(i)) = crate::ui::widgets::styled_select(
                            ui,
                            "track_inst_preset",
                            0,
                            &labels,
                            146.0,
                        ) {
                            if i >= 1 {
                                pending = state
                                    .track_preset_entries
                                    .get(i - 1)
                                    .and_then(crate::presets::load_instrument_preset);
                            }
                        }
                    }
                    if let Some(preset) = pending {
                        crate::ui::preset_browser::apply_instrument_preset_to_slot(
                            setter,
                            params,
                            sound_settings,
                            state,
                            slot_idx,
                            &preset,
                        );
                    }
                }
            });
        })
    });

    // ---- Routing ----
    editor_section_header(ui, "Routing");

    // Main Mix (toggle)
    help::zone(ui, "track_main_mix", help::TRACK_MAIN_MIX, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            editor_label(ui, "Main Mix");
            let avail = ui.available_width();
            ui.add_space((avail - 34.0).max(0.0));
            let checked = slot.routing.main_on;
            if ui.add(ToggleSwitch::new(checked)).clicked() {
                new_state.slots[slot_idx].routing.main_on = !checked;
                changed = true;
            }
        })
    });

    // Aux Out (dropdown)
    let current_out = slot.routing.out_select.index();
    // [194] An output can be shared, so the picker says how many lanes each one
    // already carries - sharing should be a deliberate choice, not an accidental
    // sum. The count includes every lane, so the label describes the OUTPUT and
    // not the reader: "Out 3 - 2 lanes" is true seen from anywhere.
    let out_labels: Vec<String> = (0..=crate::track::MAX_TRACKS)
        .map(|i| {
            if i == 0 {
                return "No Aux".to_string();
            }
            match layout_state.lanes_on_output(i as u8) {
                0 => format!("Out {}", i),
                1 => format!("Out {} - 1 lane", i),
                n => format!("Out {} - {} lanes", i, n),
            }
        })
        .collect();
    let out_refs: Vec<&str> = out_labels.iter().map(|s| s.as_str()).collect();
    help::zone(ui, "track_aux", help::TRACK_AUX, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            editor_label(ui, "Aux Out");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let (_, Some(i)) = crate::ui::widgets::styled_select(
                    ui,
                    "track_out",
                    current_out as usize,
                    &out_refs,
                    146.0,
                ) {
                    if i as u8 != current_out {
                        new_state.assign_slot_output(
                            slot_idx,
                            crate::track::TrackAudioOut::from_index(i as u8),
                        );
                        changed = true;
                    }
                }
            });
        })
    });

    // [286] An effect lane has no choke group, no MIDI note, no humanize and
    // no sends of its own: only its name, type, preset and routing apply.
    if !slot.kind.is_effect() {
        // Choke group (dropdown) — when this slot triggers, every other active
        // slot in the same group is silenced (classic HH→OH, generalized).
        let current_choke = slot
            .routing
            .choke_group
            .min(crate::track::CHOKE_GROUP_COUNT) as usize;
        let choke_labels = ["None", "1", "2", "3", "4"];
        help::zone(ui, "track_choke", help::TRACK_CHOKE, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                editor_label(ui, "Choke");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let (_, Some(i)) = crate::ui::widgets::styled_select(
                        ui,
                        "track_choke",
                        current_choke,
                        &choke_labels,
                        146.0,
                    ) {
                        if i != current_choke {
                            new_state.slots[slot_idx].routing.choke_group = i as u8;
                            changed = true;
                        }
                    }
                });
            })
        });

        // ---- MIDI ----
        editor_section_header(ui, "MIDI");

        // Channel (global, read-only)
        help::zone(ui, "track_channel", help::TRACK_CHANNEL, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                editor_label(ui, "Channel");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!("{}", layout_state.global_midi_channel))
                            .font(f_mono_med(12.0))
                            .color(Color32::WHITE),
                    );
                });
            })
        });

        // Note
        help::zone(ui, "track_note", help::TRACK_NOTE, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                editor_label(ui, "Note");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut note = slot.midi_note as i32;
                    if ui
                        .add(egui::DragValue::new(&mut note).range(0..=127).speed(1.0))
                        .changed()
                    {
                        new_state.slots[slot_idx].midi_note = note.clamp(0, 127) as u8;
                        changed = true;
                        state.midi_learn_slot = None;
                    }
                    // [241] Learn: the next note from a MIDI controller becomes the
                    // lane's note. A second click stops waiting.
                    let learning = state.midi_learn_slot == Some(slot_idx);
                    let learn = help::zone(ui, "track_note_learn", help::TRACK_NOTE_LEARN, |ui| {
                        crate::ui::controls::keycap_button(
                            ui,
                            if learning { "Play a note" } else { "Learn" },
                            56.0,
                            if learning {
                                crate::ui::widgets::KeycapState::PressedAmber
                            } else {
                                crate::ui::widgets::KeycapState::Rest
                            },
                            true,
                            f_sans_sb(10.0),
                        )
                    });
                    if learn.clicked() {
                        state.midi_learn_slot = if learning { None } else { Some(slot_idx) };
                    }
                });
            })
        });

        // ---- Sequencing ----
        editor_section_header(ui, "Sequencing");
        // [289] Humanize (random velocity) lives here now, not in the grid row.
        help::zone(ui, "track_humanize", help::LANE_HUM, |ui| {
            let hum_param = params.humanizes()[slot_idx];
            let mut hum_pct = hum_param.value() * 100.0;
            let response = draw_editor_slider_row(
                ui,
                "Humanize",
                &mut hum_pct,
                0.0,
                100.0,
                hum_param.default_plain_value() * 100.0,
                false,
                Some("%"),
            );
            if response.changed() {
                setter.begin_set_parameter(hum_param);
                setter.set_parameter(hum_param, (hum_pct / 100.0).clamp(0.0, 1.0));
                setter.end_set_parameter(hum_param);
            }
        });

        // [275] The lane's own swing, instead of the header's Swing and Groove.
        // Off, its two controls stay in place, greyed.
        let own = slot.own_swing;
        help::zone(ui, "track_own_swing", help::TRACK_OWN_SWING, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                editor_label(ui, "Own Swing");
                let avail = ui.available_width();
                ui.add_space((avail - 34.0).max(0.0));
                if ui.add(ToggleSwitch::new(own)).clicked() {
                    new_state.slots[slot_idx].own_swing = !own;
                    changed = true;
                }
            })
        });
        help::zone(ui, "track_swing", help::TRACK_SWING, |ui| {
            ui.add_enabled_ui(own, |ui| {
                let mut pct = slot.swing * 100.0;
                if draw_editor_slider_row(ui, "Swing", &mut pct, -50.0, 50.0, 0.0, false, Some("%"))
                    .changed()
                {
                    new_state.slots[slot_idx].swing = (pct / 100.0).clamp(-0.5, 0.5);
                    changed = true;
                }
            });
        });
        help::zone(ui, "track_groove", help::TRACK_GROOVE, |ui| {
            ui.add_enabled_ui(own, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    editor_label(ui, "Groove");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let names = crate::groove::GrooveType::variants();
                        let current = (slot.swing_groove as usize).min(names.len() - 1);
                        if let (_, Some(i)) = crate::ui::widgets::styled_select(
                            ui,
                            "track_groove",
                            current,
                            names,
                            146.0,
                        ) {
                            if i != current {
                                new_state.slots[slot_idx].swing_groove = i as u8;
                                changed = true;
                            }
                        }
                    });
                })
            });
        });
    }

    if changed {
        PersistentField::<TrackLayoutState>::set(&params.track_layout, new_state);
    }
}

/// [241] Once per frame: MIDI learn of a lane's note. While a lane waits, the audio
/// thread keeps the next incoming note for it; the note becomes the lane's note and
/// the lane flashes. Stops when the lane is removed or becomes an effect bus.
pub fn poll_midi_learn(
    ctx: &egui::Context,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    state: &mut EditorUIState,
) {
    let Some(slot) = state.midi_learn_slot else {
        // A wait left from a closed editor would swallow a note.
        if sound_settings.midi_learn_pending() {
            sound_settings.set_midi_learn(false);
        }
        return;
    };
    let layout = &params.track_layout.state;
    if !layout.is_active(slot) || layout.kind_for_slot(slot).is_some_and(|k| k.is_effect()) {
        state.midi_learn_slot = None;
        sound_settings.set_midi_learn(false);
        return;
    }
    if let Some(note) = sound_settings.take_learned_note() {
        let mut new_state =
            PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
        new_state.slots[slot].midi_note = note;
        PersistentField::<TrackLayoutState>::set(&params.track_layout, new_state);
        state.midi_learn_slot = None;
        state.slot_flash_until[slot] = ctx.input(|i| i.time) + 0.5;
        ctx.request_repaint();
        return;
    }
    if !sound_settings.midi_learn_pending() {
        sound_settings.set_midi_learn(true);
    }
    // The note comes from the audio thread: look for it without waiting for input.
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
}

pub fn apply_lane_layout_preset(
    setter: &ParamSetter,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    pattern: &SharedPattern,
    state: &mut EditorUIState,
    mut layout: TrackLayoutState,
    clear_pattern_data: bool,
) {
    // [283] A preset replaces the lanes, not the global MIDI channel set in
    // Settings (it used to come back as 10).
    let current = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
    layout.keep_globals_of(&current);
    if clear_pattern_data {
        load_pattern_for_ui(pattern, &crate::sequencer::pattern::Pattern::empty());
        params.plock_state.state.clear_all();
        params.seq_plock_state.state.clear_all();
        crate::ui::grid::clear_all_fusions(pattern);
    }

    // The new layout can deactivate lanes: drop every mute/solo so none keeps
    // acting while invisible.
    crate::ui::controls::clear_all_mutes_solos(setter, params);

    for (slot_idx, slot) in layout.slots.iter().enumerate() {
        if slot.active {
            sound_settings.reset_slot_to_defaults(
                slot_idx,
                slot.kind,
                state.global_config.default_analog,
            );
        }
    }

    let selected_slot = layout.active_slot_indices().next().unwrap_or(0);
    PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
    select_legacy_track(state, selected_slot);
}

// Sound Panel (always visible, tabbed by instrument)
/// Special indices of an A-H-D filter envelope — attack, hold, attack curve,
/// decay curve — for instruments that declare one ([221]).
///
/// Buzz, SDrex and Rift all carry the same envelope but numbered their
/// parameters differently, and SDrex spells its decay curve
/// `_filter_dec_curve` where Buzz says `_filter_curve`. Looking them up by name
/// keeps the panel out of the business of remembering index tables.
fn ahd_filter_env_params(
    instrument: &crate::instrument_registry::InstrumentDef,
) -> Option<(usize, usize, usize, usize)> {
    let find = |suffix: &str| {
        instrument
            .special_params
            .iter()
            .find(|d| d.name.ends_with(suffix))
            .map(|d| d.special_index)
    };
    Some((
        // [243] One-Shot spells its fraction-of-sample stages `_atk`/`_hld`
        // (the `_attack`/`_hold` suffixes are reserved to physical times).
        find("_filter_attack").or_else(|| find("_filter_atk"))?,
        find("_filter_hold").or_else(|| find("_filter_hld"))?,
        find("_filter_atk_curve")?,
        find("_filter_dec_curve").or_else(|| find("_filter_curve"))?,
    ))
}

/// [243] Special indices of an A-H-D PITCH envelope — attack, hold, decay,
/// attack curve, decay curve. One-Shot has one; Rift's decay-only envelope
/// (`_pitch_env_time`) keeps the legacy graph.
fn ahd_pitch_env_params(
    instrument: &crate::instrument_registry::InstrumentDef,
) -> Option<(usize, usize, usize, usize, usize)> {
    let find = |suffix: &str| {
        instrument
            .special_params
            .iter()
            .find(|d| d.name.ends_with(suffix))
            .map(|d| d.special_index)
    };
    Some((
        find("_pitch_env_attack").or_else(|| find("_pitch_env_atk"))?,
        find("_pitch_env_hold").or_else(|| find("_pitch_env_hld"))?,
        find("_pitch_env_decay")?,
        find("_pitch_env_atk_curve")?,
        find("_pitch_env_dec_curve")?,
    ))
}

/// [243] The One-Shot draws its envelopes OVER the lane file's waveform, on
/// the played region's time axis — the discriminator is its `_offset`
/// special without Rift's `_wander`.
fn is_oneshot(instrument: &crate::instrument_registry::InstrumentDef) -> bool {
    instrument
        .special_params
        .iter()
        .any(|d| d.name == "oneshot_offset")
}

pub fn draw_sound_panel(
    ui: &mut egui::Ui,
    sound_settings: &SoundSettingsState,
    params: &DrumFlashParams,
    setter: &ParamSetter,
    state: &mut EditorUIState,
    plock: &crate::plock::PlockState,
    pattern: &SharedPattern,
) {
    state.selected_instrument = state.selected_instrument.min(crate::track::MAX_TRACKS - 1);
    state.selected_track_slot = state.selected_instrument;
    reconcile_lane_textures(params);
    reconcile_lane_macros(params);
    poll_texture_pick(ui.ctx(), params, state, sound_settings);
    // Slot index drives per-slot state (sound_settings, algos, mutes, ...);
    // the voice index drives registry/schema lookups (INSTRUMENTS, special_param).
    let voice_idx = schema_voice_idx(params, state.selected_instrument);
    let layout_snapshot =
        PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());

    ui.set_width(ui.available_width());

    let panel_algo = PanelAlgo {
        param: params.algos()[state.selected_instrument],
        setter,
    };

    // [184] Which store do the rows edit this frame? A cell selected with a
    // right-click puts the panel on that step's p-lock; anything else falls back
    // to the lane's global sound. `resolve_edit_scope` is pure and unit-tested,
    // so the rules (another lane, Song/Follow mode, dead lane, fused cell) live
    // in one place instead of being re-derived here.
    let slot = state.selected_instrument;
    // [221] The page the grid is showing: what Spread / Scatter write onto.
    let page = state.current_page.min(3);
    let lane_master_length = params.pattern_length.value().clamp(1, 64) as usize;
    let lane_length =
        crate::ui::editor_state::effective_lane_length_for_ui(params, slot, lane_master_length);
    // [283] A linked lane plays the fused groups of the lane it follows, with
    // its own morph.
    let grid = params.track_layout.state.grid_slot(slot);
    let fusions = pattern.load_lane_fusions(slot, grid);
    let resolved = crate::ui::editor_state::resolve_edit_scope(
        state.sound_edit_target,
        slot,
        params.track_layout.state.is_active(slot),
        lane_length,
        |step| {
            fusions
                .iter()
                .enumerate()
                .find(|(_, group)| {
                    step >= group.start_cell as usize && step <= group.end_cell as usize
                })
                .map(|(index, group)| (group.start_cell as usize, index, group.step_count))
        },
        // Follow ON is fine ([184]): the selection targets a fixed (lane, step),
        // whatever page the grid happens to be showing. Song mode is not: the
        // pattern itself changes underneath.
        !params.song_mode.value(),
        state.fusion_tab,
    );
    if resolved.drop_selection {
        state.sound_edit_target = None;
    }
    let mut src: Box<dyn ParamSource + '_> = match resolved.scope {
        crate::ui::editor_state::EditScope::LaneGlobal => Box::new(
            GlobalSource::new(sound_settings, slot, voice_idx, &panel_algo)
                .with_layout(&params.track_layout),
        ),
        crate::ui::editor_state::EditScope::StepPlock { step } => Box::new(PlockSource::new(
            plock,
            step,
            GlobalSource::new(sound_settings, slot, voice_idx, &panel_algo)
                .with_layout(&params.track_layout),
        )),
        crate::ui::editor_state::EditScope::Morph {
            step,
            fusion_index,
            end,
        } => Box::new(
            crate::ui::param_source::MorphSource::new(
                pattern,
                fusion_index,
                fusions[fusion_index],
                end,
                PlockSource::new(
                    plock,
                    step,
                    GlobalSource::new(sound_settings, slot, voice_idx, &panel_algo)
                .with_layout(&params.track_layout),
                ),
            )
            .following(grid),
        ),
    };
    // Badge content for the header below: `Some(step)` = editing that step's
    // p-lock, `None` = editing the lane.
    let scope_badge = match resolved.scope {
        crate::ui::editor_state::EditScope::StepPlock { step } => Some(step),
        crate::ui::editor_state::EditScope::Morph { step, .. } => Some(step),
        crate::ui::editor_state::EditScope::LaneGlobal => None,
    };
    // The Start/End switch is only meaningful on a fused, multi-pulse group.
    let morph_available = resolved.morph_available;

    let inst = &sound_settings.instruments[state.selected_instrument];
    // [184] Every row reads and writes through this source. Today it is always
    // the lane's global sound; swapping it for a `PlockSource` is what will make
    // the same rows edit one step's p-lock.
    // [184] Discriminates this scope's widget ids from the popup's (both are live
    // at once during the migration) and from the other scope's, so a dropdown's
    // open/closed state can never leak between them.
    let row_salt = src.salt();

    // [269] What the sections below read, gathered once.
    let cx = PanelCtx {
        params,
        setter,
        sound_settings,
        plock,
        pattern,
        layout_snapshot: &layout_snapshot,
        resolved: &resolved,
        fusions: &fusions,
        inst,
        instrument: &crate::instrument_registry::INSTRUMENTS[voice_idx],
        slot,
        voice_idx,
        page,
        row_salt,
        morph_available,
    };

    draw_panel_header(ui, &cx, state, scope_badge);
    // [303] An effect lane has no sends and no inserts: no Effects tab.
    let show_effects = !layout_snapshot.slots[slot].kind.is_effect();
    if !show_effects && state.sound_editor_tab == SoundEditorTab::Effects {
        state.sound_editor_tab = SoundEditorTab::Sound;
    }
    draw_panel_tabs(ui, state, scope_badge, show_effects);
    draw_action_strip(ui, &cx, state, &mut src);

    let mut freq = src.get(ParamId::Std(StandardField::Freq));
    let mut decay = src.get(ParamId::Std(StandardField::Decay));
    let mut vol = src.get(ParamId::Std(StandardField::Volume));
    let mut filt = src.get(ParamId::Std(StandardField::FilterFreq));
    let mut attack = src.get(ParamId::Std(StandardField::Attack));
    let mut release = src.get(ParamId::Std(StandardField::Release));
    // [146] A legacy stored 4.0 plays (and so shows) as 1.
    let mut decay_curve = crate::synthesis::decay_curve_amount(src.get(ParamId::Std(StandardField::DecayCurve)));
    let mut release_curve = src.get(ParamId::Std(StandardField::ReleaseCurve));
    let mut hold = src.get(ParamId::Std(StandardField::Hold));
    let mut filter_env_amount = src.get(ParamId::Std(StandardField::FilterEnvAmount));
    let mut filter_env_decay = src.get(ParamId::Std(StandardField::FilterEnvDecay));
    let mut analog = src.get(ParamId::Std(StandardField::Analog));
    let mut stereo = src.get(ParamId::Std(StandardField::Stereo));
    let mut changed = false;

    // One-shot migration for sampler builds that persisted pitch in Hz.
    // The voice also understands the legacy marker, so audio is correct even
    // before the Sound tab is opened; opening it commits the semitone value.
    if crate::instrument_registry::is_sampler(voice_idx) && inst.special_value(10) < 0.5 {
        // Frozen legacy table: the roots those builds stored Hz against.
        let legacy_root = match DrumVoice::from_index(voice_idx) {
            Some(DrumVoice::Bd606) => 60.0,
            Some(DrumVoice::Sd606) => 200.0,
            _ => 8000.0, // the hats (Ch606, Oh606)
        };
        freq = if freq > 0.0 {
            (12.0 * (freq / legacy_root).log2()).clamp(-24.0, 24.0)
        } else {
            0.0
        };
        inst.frequency.store(freq.to_bits(), Ordering::Relaxed);
        inst.set_special(10, 1.0);
        // The old blob has no End parameter (reads 0) — restore full length.
        if inst.special_value(11) <= 0.0 {
            inst.set_special(11, 1.0);
        }
        changed = true;
    }

    // [282] Volume stays fixed above the scrolling sections (Sound tab), in a
    // module without a title. Its row keeps the params column like every
    // other row, so its readout lines up with the rest. The frame's margins
    // are the scroll content's (14 px, plus the 20 px kept for the bar on the
    // right), so the module lines up with the ones below. Top margin 2: the
    // Store / Restore / Default strip above already ends with its own space,
    // and 8 here left it 9 px under the tabs but 17 px over Volume (2 px
    // more than above, which the keys' drop shadow takes back).
    // [303] The Effects tab keeps the lane's Volume in view as well.
    if matches!(state.sound_editor_tab, SoundEditorTab::Sound | SoundEditorTab::Effects) {
        // (No Store / Restore strip above it in Effects: the tabs' edge needs the room.)
        let top = if state.sound_editor_tab == SoundEditorTab::Effects { 10 } else { 2 };
        egui::Frame::new()
            .inner_margin(egui::Margin {
                left: 14,
                right: 14 + 20,
                top,
                bottom: 0,
            })
            .show(ui, |ui| {
                let volume_module = module_begin(ui, None);
                let vol_id = ParamId::Std(StandardField::Volume);
                let vol_help = row_help(&*src, vol_id, help::std_param(voice_idx, StandardField::Volume));
                let (vol_reverted, vol_changed) = help::zone(ui, "vol_row", &vol_help, |ui| {
                    ui.allocate_ui(Vec2::new(EDITOR_PARAMS_W, 22.0), |ui| {
                        row_scoped(
                            ui,
                            src.is_overridden(vol_id),
                            src.supports(vol_id).reason(),
                            |ui| {
                                // [288] Displayed 0..1; the stored gain keeps its 0..2 range.
                                let mut vol_disp = vol * 0.5;
                                let vol_changed = draw_editor_slider_row(
                                    ui,
                                    "Volume",
                                    &mut vol_disp,
                                    0.0,
                                    1.0,
                                    VoiceSettings::default().volume * 0.5,
                                    false,
                                    Some(""),
                                )
                                .changed();
                                if vol_changed {
                                    vol = vol_disp * 2.0;
                                }
                                vol_changed
                            },
                        )
                    })
                    .inner
                });
                if vol_reverted {
                    src.clear(vol_id);
                    changed = true;
                } else if vol_changed {
                    src.set(vol_id, vol);
                    changed = true;
                }
                module_end(ui, volume_module);
            });
    }

    // [184] Kept for the floating notice below, which must sit at the panel's
    // bottom edge rather than scroll away with the content.
    let panel_rect = ui.max_rect();
    let scroll_height = ui.available_height().max(120.0);
    let content_style = ui.style().clone();
    style_scroll_bar(ui);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(scroll_height)
        .show(ui, |ui| {
            // The scroll styling above is for the BAR only; the rows keep the
            // plain style so nothing inside picks up the handle's fills.
            ui.set_style(content_style);
            ui.set_width(ui.available_width() - 20.0);
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: 14,
                    right: 14,
                    top: 6,
                    bottom: 14,
                })
                .show(ui, |ui| {
                    match state.sound_editor_tab {
                        SoundEditorTab::Sound => {

            draw_dev_preset_dumps(ui, &cx, state, RowValues { freq, decay, vol, filt, attack, release, decay_curve, release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo });
            ui.add_space(4.0);


            // Data-driven grouped Sound Panel (schema follows the slot's kind)
            let instrument = &crate::instrument_registry::INSTRUMENTS[voice_idx];
            let standard_defs = instrument.standard_params;
            let special_defs = instrument.special_params;

            for family in [
                crate::instrument_registry::ParamFamily::Osc,
                crate::instrument_registry::ParamFamily::Pitch,
                crate::instrument_registry::ParamFamily::Env,
                crate::instrument_registry::ParamFamily::Analog,
                crate::instrument_registry::ParamFamily::Filter,
                crate::instrument_registry::ParamFamily::Modulation,
                crate::instrument_registry::ParamFamily::Saturation,
                crate::instrument_registry::ParamFamily::Output,
            ] {
                // Skip a section that has no parameters for this instrument
                // (e.g. Saturation on instruments without a saturation pack).
                let fam_has_std = standard_defs.iter().any(|d| {
                    d.family == family
                        && d.field != crate::instrument_registry::StandardField::Volume
                });
                let fam_has_special = special_defs.iter().any(|d| d.family == family);
                if !fam_has_std && !fam_has_special {
                    continue;
                }
                let section_title = if let Some(title) =
                    crate::instrument_registry::rumble_section_title(voice_idx, family)
                {
                    title
                } else {
                    match family {
                    crate::instrument_registry::ParamFamily::Osc => {
                        crate::instrument_registry::source_section_title(voice_idx)
                    }
                    crate::instrument_registry::ParamFamily::Pitch => "Pitch",
                    crate::instrument_registry::ParamFamily::Env => "Amp",
                    crate::instrument_registry::ParamFamily::Analog => "Analog",
                    crate::instrument_registry::ParamFamily::Filter => "Filter",
                    crate::instrument_registry::ParamFamily::Modulation => "Modulation",
                    crate::instrument_registry::ParamFamily::Saturation => "Distortion",
                    crate::instrument_registry::ParamFamily::Output => "Output",
                    }
                };
                // [282] Each section is a module with its title strip.
                let section_module = module_begin(ui, Some(section_title));

            let has_filter_env = standard_defs
                .iter()
                .any(|d| d.field == crate::instrument_registry::StandardField::FilterEnvAmount);
            // [225] An instrument reading a long texture shows it in its
            // Oscillator section: a 0..1 Offset slider over twelve seconds of
            // material says nothing on its own.
            let texture_param = instrument
                .special_params
                .iter()
                .find(|d| d.name.ends_with("_texture"));
            let has_graph = family == crate::instrument_registry::ParamFamily::Env
                || (family == crate::instrument_registry::ParamFamily::Pitch
                    && special_defs.iter().any(|d| d.name.ends_with("_pitch_env")))
                || (family == crate::instrument_registry::ParamFamily::Filter && has_filter_env)
                // [243] The texture/file graph only makes sense with an Offset
                // to point at (Rift's texture, One-Shot's lane file).
                || (family == crate::instrument_registry::ParamFamily::Osc
                    && texture_param.is_some()
                    && special_defs.iter().any(|d| d.name.ends_with("_offset")))
                // [286] The Rumble's duck curve.
                || (family == crate::instrument_registry::ParamFamily::Saturation
                    && special_defs.iter().any(|d| d.name == "rumble_duck"));
            // [225] EVERY section keeps the same params column, graph or not.
            // Graph-less sections used to spread their sliders across the whole
            // panel, so the same instrument showed two slider lengths depending
            // on the section — and two instruments showed different lengths for
            // the same parameter. One width, everywhere.
            let params_w = EDITOR_PARAMS_W;
            // Top-aligned: a centred horizontal put the graph halfway down a
            // tall section (the texture graph sat at the Advance rows instead
            // of at Texture).
            ui.horizontal_top(|ui| {
                // Left column: params (width-constrained so the graph keeps its space)
                ui.vertical(|ui| {
                    ui.set_max_width(params_w);
                    ui.set_width(params_w);
                    ui.spacing_mut().item_spacing.y = 9.0;
                    if draw_filter_type_row(ui, &cx, family, &mut src) {
                        changed = true;
                    }
                    let (row_values, hoisted, rows_changed) = draw_standard_rows(
                        ui,
                        &cx,
                        family,
                        texture_param,
                        &mut src,
                        RowValues { freq, decay, vol, filt, attack, release, decay_curve, release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo },
                    );
                    RowValues { freq, decay, vol, filt, attack, release, decay_curve, release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo } = row_values;
                    changed |= rows_changed;
                    let (new_stereo, specials_changed) =
                        draw_special_rows(ui, &cx, family, &hoisted, &mut src, state, stereo);
                    stereo = new_stereo;
                    changed |= specials_changed;
                    draw_algorithm_row(ui, &cx, family, state);
                });

                draw_section_graphs(
                    ui,
                    &cx,
                    family,
                    has_graph,
                    texture_param,
                    &mut src,
                    RowValues { freq, decay, vol, filt, attack, release, decay_curve, release_curve, hold, filter_env_amount, filter_env_decay, analog, stereo },
                );
            });

            draw_gate_row(ui, &cx, family, params_w, &mut src, state);
            module_end(ui, section_module);
            }

            // [301] An effect lane's Sequencer (in its Sound tab: an effect lane has no Effects tab): how its p-locks behave (Glide) and
            // whether it goes back to its sliders when the pattern loops (Loop Restart,
            // also the arrow on the lane's first cell). Lane settings, never locks.
            if crate::track::is_fx_slot(slot) && layout_snapshot.slots[slot].kind.is_effect() {
                let seq_module = module_begin(ui, Some("Sequencer"));
                let instrument = &crate::instrument_registry::INSTRUMENTS[voice_idx];
                for def in instrument
                    .special_params
                    .iter()
                    .filter(|d| is_fx_sequencer_param(d.name))
                {
                    let id = ParamId::Special(def.special_index);
                    // Loop Restart is stored inverted (older lanes hold 0 = restart).
                    let inverted = def.name.ends_with("_norestart");
                    let stored = src.get(id);
                    let mut on = if inverted { 1.0 - stored } else { stored };
                    let row_help_text = help::special_param(def.name).to_string();
                    let toggled = help::zone(ui, ("fx_seq_row", def.name), &row_help_text, |ui| {
                        ui.allocate_ui(Vec2::new(EDITOR_PARAMS_W, 22.0), |ui| {
                            draw_editor_switch_row(ui, def.label, &mut on).changed()
                        })
                        .inner
                    });
                    if toggled {
                        src.set(id, if inverted { 1.0 - on } else { on });
                        changed = true;
                    }
                }
                module_end(ui, seq_module);
            }
            // [286] On an effect lane: what the reverb is using RIGHT NOW. While the
            // transport plays, the p-locks of the lane move its knobs; this shows where
            // they actually are (a value that differs from the lane's own is lit).
            if crate::track::is_fx_slot(slot) && layout_snapshot.slots[slot].kind.is_effect() {
                let bus = slot - crate::track::FX_FIRST_SLOT;
                let live_module = module_begin(ui, Some("Playing now"));
                match sound_settings.fx_live(bus) {
                    None => {
                        ui.label(
                            RichText::new("Stopped: the effect uses the sliders above.")
                                .font(f_sans_med(10.5))
                                .color(INK3()),
                        );
                    }
                    Some(values) => {
                        ui.ctx().request_repaint();
                        let lane = &sound_settings.instruments[slot];
                        // Rows and the lane's own values follow the effect on this lane.
                        let volume = lane.standard(StandardField::Volume);
                        let (rows, own, glide_on): (Vec<(&str, &str)>, Vec<f32>, bool) =
                            if layout_snapshot.slots[slot].kind
                                == crate::track::TrackInstrumentKind::Rumble
                            {
                                use crate::synthesis::rumble_sp as sp;
                                (
                                    vec![
                                        ("Decay", " s"),
                                        ("High Pass", " Hz"),
                                        ("Low Pass", " Hz"),
                                        ("Duck", ""),
                                        ("Duck Length", " ms"),
                                        ("Drive", ""),
                                        ("Width", ""),
                                        ("Echo", ""),
                                        ("Volume", ""),
                                        // 1 = untouched; it drops at every kick the duck hears.
                                        ("Duck now", ""),
                                    ],
                                    vec![
                                        lane.special_value(sp::DECAY),
                                        lane.special_value(sp::HIGH_PASS),
                                        lane.special_value(sp::LOW_PASS),
                                        lane.special_value(sp::DUCK),
                                        lane.special_value(sp::RELEASE),
                                        lane.special_value(sp::DRIVE),
                                        lane.special_value(sp::WIDTH),
                                        lane.special_value(sp::ECHO_MIX),
                                        volume,
                                        1.0,
                                    ],
                                    lane.special_value(sp::GLIDE) >= 0.5,
                                )
                            } else if layout_snapshot.slots[slot].kind
                                == crate::track::TrackInstrumentKind::Delay
                            {
                                use crate::synthesis::delay_sp as sp;
                                (
                                    vec![
                                        ("Time", " ms"),
                                        ("Feedback", ""),
                                        ("Damping", ""),
                                        ("Low Cut", " Hz"),
                                        ("High Cut", " Hz"),
                                        ("Ping-pong", ""),
                                        ("Width", ""),
                                        ("Volume", ""),
                                    ],
                                    vec![
                                        lane.special_value(sp::TIME),
                                        lane.special_value(sp::FEEDBACK),
                                        lane.special_value(sp::DAMPING),
                                        lane.special_value(sp::LOW_CUT),
                                        lane.special_value(sp::HIGH_CUT),
                                        lane.special_value(sp::PINGPONG),
                                        lane.special_value(sp::WIDTH),
                                        volume,
                                    ],
                                    lane.special_value(sp::GLIDE) >= 0.5,
                                )
                            } else {
                                use crate::synthesis::reverb_sp as sp;
                                (
                                    vec![
                                        ("Size", ""),
                                        ("Decay", " s"),
                                        ("Damping", ""),
                                        ("Pre-delay", " ms"),
                                        ("Low Cut", " Hz"),
                                        ("High Cut", " Hz"),
                                        ("Width", ""),
                                        ("Volume", ""),
                                        ("Gate Time", " ms"),
                                        ("Gate Release", " ms"),
                                    ],
                                    vec![
                                        lane.special_value(sp::SIZE),
                                        lane.special_value(sp::DECAY),
                                        lane.special_value(sp::DAMPING),
                                        lane.special_value(sp::PREDELAY),
                                        lane.special_value(sp::LOW_CUT),
                                        lane.special_value(sp::HIGH_CUT),
                                        lane.special_value(sp::WIDTH),
                                        volume,
                                        lane.special_value(sp::GATE),
                                        lane.special_value(sp::GATE_RELEASE),
                                    ],
                                    lane.special_value(sp::GLIDE) >= 0.5,
                                )
                            };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Mode").font(f_sans_med(11.0)).color(INK3()));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(
                                    RichText::new(if glide_on { "Glide" } else { "Step" })
                                        .font(f_mono_med(11.0))
                                        .color(INK()),
                                );
                            });
                        });
                        for (i, (label, unit)) in rows.iter().enumerate() {
                            let differs = (values[i] - own[i]).abs() > 1.0e-3 * own[i].abs().max(1.0);
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(*label).font(f_sans_med(11.0)).color(INK3()));
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    ui.label(
                                        RichText::new(format!("{}{}", format_editor_value(values[i], None), unit))
                                            .font(f_mono_med(11.0))
                                            .color(if differs { PL_LINK() } else { INK() }),
                                    );
                                });
                            });
                        }
                    }
                }
                module_end(ui, live_module);
            }
                    }
                        SoundEditorTab::Effects => {
            // [286] Sends to the effect buses. On the lane they are its own send levels;
            // on a selected cell they are that step's locks (the hit keeps them).
            if !layout_snapshot.slots[slot].kind.is_effect() {
                // [303] With a cell selected the sliders are THAT STEP's sends (a lock), not
                // the lane's: the title says so, or a lane send seems to do nothing.
                let step_scope = src.salt().0 != 0;
                let sends_module = module_begin(
                    ui,
                    Some(if step_scope { "Sends - this step only" } else { "Sends" }),
                );
                for bus in 0..crate::track::FX_SLOT_COUNT {
                    let id = ParamId::FxSend(bus);
                    let bus_slot = &layout_snapshot.slots[crate::track::FX_FIRST_SLOT + bus];
                    let bus_on = bus_slot.active && bus_slot.kind.is_effect();
                    // [298] The bus is named after its effect: the row says where it sends.
                    let send_label = if bus_on {
                        let name = if bus_slot.name.is_empty() {
                            bus_slot.kind.default_name().to_string()
                        } else {
                            bus_slot.name.clone()
                        };
                        format!("> {}", name.chars().take(10).collect::<String>())
                    } else {
                        format!("Fx{}", bus + 1)
                    };
                    let send_help = row_help(&*src, id, help::TRACK_FX_SEND.to_string());
                    let current = src.get(id) * 100.0;
                    let default = src.inherited(id) * 100.0;
                    let send_muted = params.track_layout.state.fx_send_muted(slot, bus);
                    let mut mute_clicked = false;
                    let mut edit_clicked = false;
                    let (reverted, edited) = ui.horizontal(|ui| {
                    let row = help::zone(ui, ("fx_send_row", bus), &send_help, |ui| {
                        ui.allocate_ui(Vec2::new(EDITOR_PARAMS_W, 22.0), |ui| {
                            ui.add_enabled_ui(bus_on, |ui| {
                                row_scoped(
                                    ui,
                                    src.is_overridden(id),
                                    src.supports(id).reason(),
                                    |ui| {
                                        let mut pct = current;
                                        let row = draw_editor_slider_row(
                                            ui,
                                            &send_label,
                                            &mut pct,
                                            0.0,
                                            100.0,
                                            default,
                                            false,
                                            Some("%"),
                                        );
                                        (row.changed(), pct)
                                    },
                                )
                            })
                            .inner
                        })
                        .inner
                    });
                    // [300] M: switch this send off without losing its level. [Edit]: go to
                    // the effect lane it feeds.
                    ui.add_space(6.0);
                    let mute = egui::Button::new(
                        RichText::new("M")
                            .font(f_mono_sb(10.5))
                            .color(if send_muted { Color32::from_rgb(20, 20, 24) } else { INK2() }),
                    )
                    .fill(if send_muted { AMBER() } else { P_ACTIVE() })
                    .stroke(egui::Stroke::new(1.0, LINE2()))
                    .min_size(Vec2::new(22.0, 20.0));
                    mute_clicked = ui
                        .add_enabled(bus_on, mute)
                        .on_hover_text("Mute this send (its level is kept)")
                        .clicked();
                    let edit = egui::Button::new(RichText::new("Edit").font(f_sans_sb(10.5)).color(INK2()))
                        .fill(P_ACTIVE())
                        .stroke(egui::Stroke::new(1.0, LINE2()))
                        .min_size(Vec2::new(34.0, 20.0));
                    edit_clicked = ui
                        .add_enabled(bus_on, edit)
                        .on_hover_text("Open the effect this send feeds")
                        .clicked();
                    row
                    })
                    .inner;
                    if mute_clicked {
                        let mut layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
                        layout.slots[slot].fx_send_mute[bus] = !send_muted;
                        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
                    }
                    if edit_clicked {
                        crate::ui::editor_state::select_legacy_track(state, crate::track::FX_FIRST_SLOT + bus);
                        state.sound_editor_tab = SoundEditorTab::Sound;
                    }
                    let (row_changed, pct) = edited;
                    if reverted {
                        src.clear(id);
                        changed = true;
                    } else if row_changed {
                        src.set(id, (pct / 100.0).clamp(0.0, 1.0));
                        changed = true;
                    }
                }
                module_end(ui, sends_module);
            }

            // [303] The lane's inserts, in chain order (instrument lanes only).
            if layout_snapshot.slots[slot].active && !layout_snapshot.slots[slot].kind.is_effect() {
                draw_lane_inserts(ui, params, sound_settings, slot, state);
            }
                        }
                        SoundEditorTab::Track => {
                            draw_track_tab(ui, params, sound_settings, setter, state);
                        }
                    }
                });
        });

    draw_scope_notice(ui, state, panel_rect, &*src);

    // [184] One flush per frame, whatever the scope wrote. The source tracks its
    // own dirty flag, which is stricter than the old `changed` bool: it also
    // covers the special-param rows, which used to bump the version each on their
    // own. `changed` is kept only as a local record that a row was edited.
    src.commit();
    let _ = changed;
}

/// [269] The 13 standard values the Sound panel edits this frame: local
/// mirrors of the scope's store, read once at the top of the panel. Each
/// section takes them by value and unpacks them into locals named as before.
#[derive(Clone, Copy)]
struct RowValues {
    freq: f32,
    decay: f32,
    vol: f32,
    filt: f32,
    attack: f32,
    release: f32,
    decay_curve: f32,
    release_curve: f32,
    hold: f32,
    filter_env_amount: f32,
    filter_env_decay: f32,
    analog: f32,
    stereo: f32,
}

/// [269] What the Sound panel's sections read, gathered once per frame so a
/// section takes one argument instead of a dozen. Each section unpacks the
/// fields it needs into locals named as before, so its body reads unchanged.
#[derive(Clone, Copy)]
struct PanelCtx<'a> {
    params: &'a DrumFlashParams,
    setter: &'a ParamSetter<'a>,
    sound_settings: &'a SoundSettingsState,
    plock: &'a crate::plock::PlockState,
    pattern: &'a SharedPattern,
    layout_snapshot: &'a TrackLayoutState,
    resolved: &'a crate::ui::editor_state::ResolvedScope,
    fusions: &'a [crate::sequencer::pattern::FusedGroup],
    inst: &'a crate::sound_settings::InstrumentSettingsState,
    instrument: &'static crate::instrument_registry::InstrumentDef,
    slot: usize,
    voice_idx: usize,
    page: usize,
    row_salt: (u8, usize, usize),
    morph_available: bool,
}

/// [269] Header band: "Lane Editor", the lane being edited, and the scope
/// badge (lane sound vs one step's p-lock or morph end).
fn draw_panel_header(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    state: &mut EditorUIState,
    scope_badge: Option<usize>,
) {
    let PanelCtx {
        layout_snapshot,
        resolved,
        voice_idx,
        ..
    } = *cx;
    let header_rect = ui
        .allocate_exact_size(Vec2::new(ui.available_width(), 42.0), egui::Sense::hover())
        .0;
    // [282] No rule under the header: the tab band below continues its
    // colour, the plate's edge under the tabs is the only seam.
    ui.painter()
        .rect_filled(header_rect, 0.0, PANEL_SKEUO_HEADER);

    ui.allocate_new_ui(
        egui::UiBuilder::new()
            .max_rect(header_rect.shrink2(Vec2::new(14.0, 0.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.horizontal_centered(|ui| {
                ui.set_height(header_rect.height());
                ui.label(
                    RichText::new("Lane Editor")
                        .font(f_sans_bold(13.0))
                        .color(Color32::WHITE),
                );
                ui.add_space(2.0);
                // Always show which lane is being edited (lanes are selected by
                // clicking them in the grid, there are no per-instrument tabs).
                let header_name = {
                    let slot_name = &layout_snapshot.slots[state.selected_instrument].name;
                    let name = if slot_name.is_empty() {
                        crate::instrument_registry::INSTRUMENTS[voice_idx].name
                    } else {
                        slot_name.as_str()
                    };
                    format!("Slot {} - {}", state.selected_instrument + 1, name)
                };
                ui.label(RichText::new(header_name).font(f_mono(11.0)).color(INK3()));
                // (Engine selector belongs to the future modular phase — omitted for now.)

                // [184] Scope badge, flush right: says whether the rows below edit
                // the lane's sound or one step's p-lock, and offers the way back.
                // Always drawn, so the header never changes height.
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        match scope_badge {
                        Some(step) => {
                            let cleared = ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("x").font(f_mono_sb(11.0)).color(INK()),
                                    )
                                    .min_size(Vec2::new(20.0, 20.0))
                                    .fill(PANEL2())
                                    .stroke(egui::Stroke::new(1.0, LINE2())),
                                )
                                .on_hover_text("Back to the lane's own sound")
                                .clicked();
                            if cleared {
                                state.sound_edit_target = None;
                            }
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(format!("Step {}", step + 1))
                                    .font(f_mono_sb(11.0))
                                    .color(PL_LINK()),
                            );
                            ui.add_space(4.0);
                            let badge = match resolved.scope {
                                crate::ui::editor_state::EditScope::Morph { .. } => "Morph",
                                _ => "P-Lock",
                            };
                            ui.label(
                                RichText::new(badge).font(f_sans_sb(11.0)).color(PL_LINK()),
                            )
                            .on_hover_text(if badge == "Morph" {
                                "Editing one end of this fused group's morph. Rows in accent colour are morph targets or step overrides; the others follow the lane."
                            } else {
                                "Editing this step's sound p-lock. Rows in accent colour are morph targets or step overrides; the others follow the lane."
                            });
                            // [200e] The Step/Start/End switch moved to the
                            // action strip under the tabs — it only concerns
                            // the Sound tab, like Store/Restore/Default.
                        }
                        None => {
                            ui.label(RichText::new("Lane").font(f_sans_sb(11.0)).color(INK3()))
                                .on_hover_text(
                                    "Editing the lane's own sound. Ctrl+click (Cmd+click on Mac) or right-click a step in the grid to edit that step's p-lock instead.",
                                );
                        }
                    }
                },
            );
        });
    },
    );
}

/// [269] The Sound / Track tabs, with the accent rule in p-lock scope.
///
/// [282] Folder tabs (`skeuo::folder_tab`) on the header's band: the active
/// one is cut from the plate and opens onto it, the other sits back. The
/// flat blue half-width tabs were the one control of the panel without the
/// skeuomorphic look.
fn draw_panel_tabs(ui: &mut egui::Ui, state: &mut EditorUIState, scope_badge: Option<usize>, show_effects: bool) {
    const BAND_H: f32 = 36.0;
    const MARGIN: f32 = 14.0;
    const GAP: f32 = 6.0;
    let band = ui
        .allocate_exact_size(Vec2::new(ui.available_width(), BAND_H), egui::Sense::hover())
        .0;
    let painter = ui.painter();
    painter.rect_filled(band, 0.0, PANEL_SKEUO_HEADER);
    // The plate's top edge, on a half pixel so the 1 px line stays crisp.
    let base = band.bottom() - 0.5;
    let tab_count = if show_effects { 3.0 } else { 2.0 };
    let tab_w = ((band.width() - 2.0 * MARGIN - (tab_count - 1.0) * GAP) / tab_count).round();
    let edge = egui::Stroke::new(1.0, egui::Color32::from_rgb(20, 20, 24));

    let mut active_span = None;
    for (idx, (tab, label, hover_txt)) in [
        (
            SoundEditorTab::Sound,
            "Sound",
            "Synthesis settings of the selected lane",
        ),
        (
            SoundEditorTab::Effects,
            "Effects",
            "Sends to the effect lanes, and the lane's inserts",
        ),
        (
            SoundEditorTab::Track,
            "Track",
            "Instrument type, MIDI note, routing, length",
        ),
    ]
    .into_iter()
    .filter(|(tab, _, _)| show_effects || *tab != SoundEditorTab::Effects)
    .enumerate()
    {
        let selected = state.sound_editor_tab == tab;
        let x0 = band.left() + MARGIN + idx as f32 * (tab_w + GAP) + 0.5;
        // The inactive tab stands 3 px lower: it sits behind the active one.
        let height = if selected { 30.0 } else { 27.0 };
        let rect = egui::Rect::from_min_max(
            egui::pos2(x0, base - height),
            egui::pos2(x0 + tab_w - 1.0, base),
        );
        let resp = ui
            .interact(
                rect,
                ui.make_persistent_id(("lane_editor_tab", idx)),
                egui::Sense::click(),
            )
            .on_hover_text(hover_txt);
        crate::ui::skeuo::folder_tab(ui.painter(), rect, selected, resp.hovered());
        let color = if selected {
            INK()
        } else if resp.hovered() {
            INK2()
        } else {
            INK3()
        };
        ui.painter().text(
            egui::pos2(rect.center().x, rect.center().y + 0.5),
            egui::Align2::CENTER_CENTER,
            label,
            f_sans_sb(11.5),
            color,
        );
        if selected {
            active_span = Some((rect.left(), rect.right()));
        }
        if resp.clicked() {
            state.sound_editor_tab = tab;
        }
    }

    // The plate's edge, broken under the active tab (it opens onto the plate).
    let painter = ui.painter();
    match active_span {
        Some((l, r)) => {
            painter.hline(band.left()..=l, base, edge);
            painter.hline(r..=band.right(), base, edge);
        }
        None => {
            painter.hline(band.x_range(), base, edge);
        }
    }

    // [184] In p-lock scope, an accent rule along the plate's edge. The rows
    // below edit ONE step, not the lane, and that has to be unmistakable at a
    // glance — the header badge alone is easy to miss. Drawn inside the band,
    // so it costs no height and cannot shift the scroll area.
    if scope_badge.is_some() {
        let rule = egui::Rect::from_min_max(
            egui::pos2(band.left(), band.bottom() - 2.0),
            band.right_bottom(),
        );
        painter.rect_filled(rule, 0.0, PL_LINK());
    }
}

/// [269] Sound tab only: the Step / Start / End switch and Default / Restore
/// / Store.
fn draw_action_strip(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    state: &mut EditorUIState,
    src: &mut Box<dyn ParamSource + '_>,
) {
    let PanelCtx {
        params,
        setter,
        sound_settings,
        plock,
        pattern,
        layout_snapshot,
        resolved,
        fusions,
        slot,
        voice_idx,
        morph_available,
        ..
    } = *cx;
    // [200d] Action strip, Sound tab only: Default / Store / Restore live
    // here (right-aligned) instead of the header — the scope badge and the
    // Step/Start/End switch need the header's width. The strip is fixed
    // (never scrolls away) and only exists in the Sound tab.
    if state.sound_editor_tab == SoundEditorTab::Sound {
        ui.add_space(3.0);
        // The horizontal wrapper constrains the strip to its content height —
        // a bare `with_layout` would grab ALL the panel's remaining height
        // and squeeze the params scroll area to zero (seen 2026-09-03).
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            // [200e] Left section: the Step / Start / End switch ([184] ph. 4) —
            // a fused group's three stores. Always drawn so the strip never
            // changes width; the two morph ends are greyed when the cell is
            // not a fused, multi-pulse group (a single pulse cannot morph,
            // the engine skips the interpolation), leaving `Step` as the only
            // choice.
            use crate::ui::editor_state::FusionTab;
            let tabs = [FusionTab::Step, FusionTab::Start, FusionTab::End];
            let selected = tabs
                .iter()
                .position(|tab| *tab == state.fusion_tab)
                .unwrap_or(0);
            // [282] Shown only where it does something: on a cell of a fused,
            // multi-pulse group. Greyed everywhere else, its blue `Step` key
            // still read as an active choice. Hiding it moves nothing: the
            // strip keeps its height and Store / Restore / Default are
            // right-aligned.
            if morph_available {
                let picked = help::zone(ui, "fusion_tab", help::EDITOR_FUSION_TAB, |ui| {
                    crate::ui::skeuo::segmented(
                        ui,
                        ("fusion_tab", slot),
                        &["Step", "Start", "End"],
                        selected,
                    )
                });
                if picked != selected {
                    state.fusion_tab = tabs[picked.min(tabs.len() - 1)];
                }
            } else {
                // Keeps the strip's height without the switch.
                ui.allocate_space(Vec2::new(0.0, CTL_HEIGHT));
            }
            // Right section: Store / Restore / Default.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(14.0);
            // [200] Default: reset to the instrument's factory defaults,
            // scoped to what the panel edits [200c]:
            // - Lane: the lane's sound settings (step p-locks kept);
            // - Step p-lock: the step's LOCKED fields go back to factory
            //   defaults, mask untouched (a Link p-lock must not turn into
            //   a full snapshot);
            // - Morph: the group's existing morph targets go back to
            //   factory defaults (no new target created).
            // Accessible in every scope. Two clicks (like Clear All). An armed
            // "Sure?" disarms after 3 s or on a click anywhere else [200b].
            const CONFIRM_TIMEOUT_S: f64 = 3.0;
            let now = ui.input(|i| i.time);
            if let Some((_, armed_at)) = state.sound_default_confirm {
                if now - armed_at >= CONFIRM_TIMEOUT_S {
                    state.sound_default_confirm = None;
                }
            }
            let armed = matches!(
                state.sound_default_confirm,
                Some((s, _)) if s == state.selected_instrument
            );
            if armed {
                // So the label reverts to "Default" on its own.
                ui.ctx().request_repaint_after(
                    std::time::Duration::from_secs_f64(CONFIRM_TIMEOUT_S),
                );
            }
            let default_resp = crate::ui::controls::keycap_button(
                ui,
                if armed { "Sure?" } else { "Default" },
                52.0,
                crate::ui::widgets::KeycapState::Rest,
                true,
                f_sans_med(9.5),
            )
            .on_hover_text(match resolved.scope {
                crate::ui::editor_state::EditScope::LaneGlobal => {
                    "Reset this lane's sound settings to the instrument's factory defaults (two clicks; step p-locks are kept). Click elsewhere or wait 3 s to cancel."
                }
                crate::ui::editor_state::EditScope::StepPlock { .. } => {
                    "Reset this step's LOCKED parameters to the instrument's factory defaults (two clicks; which fields are locked is kept). Click elsewhere or wait 3 s to cancel."
                }
                crate::ui::editor_state::EditScope::Morph { .. } => {
                    "Reset this group's morph targets to the instrument's factory defaults (two clicks). Click elsewhere or wait 3 s to cancel."
                }
            });
            if default_resp.clicked() {
                if armed {
                    let slot = state.selected_instrument;
                    match resolved.scope {
                        crate::ui::editor_state::EditScope::LaneGlobal => {
                            let kind = layout_snapshot.slots[slot].kind;
                            sound_settings.reset_slot_to_defaults(
                                slot,
                                kind,
                                state.global_config.default_analog,
                            );
                        }
                        crate::ui::editor_state::EditScope::StepPlock { step } => {
                            // Write factory defaults into the LOCKED fields
                            // only, straight to the value store — NOT via
                            // `PlockSource::set`, which locks every field it
                            // writes.
                            for field in 0..crate::plock::FIELD_COUNT {
                                if !plock.field_masks.is_set(slot, step, field) {
                                    continue;
                                }
                                if let Some(id) =
                                    crate::param_id::ParamId::from_plock_field(field)
                                {
                                    plock.values.set(
                                        slot,
                                        step,
                                        field,
                                        crate::instrument_registry::param_default(
                                            voice_idx, id,
                                        ),
                                    );
                                }
                            }
                        }
                        crate::ui::editor_state::EditScope::Morph {
                            fusion_index,
                            ..
                        } => {
                            // Only the group's EXISTING targets, so no new
                            // morph target is created.
                            let group = fusions[fusion_index];
                            for i in 0..group.morph_count as usize {
                                let field = group.morph_targets[i].field as usize;
                                if let Some(id) =
                                    crate::param_id::ParamId::from_plock_field(field)
                                {
                                    src.set(
                                        id,
                                        crate::instrument_registry::param_default(
                                            voice_idx, id,
                                        ),
                                    );
                                }
                            }
                            src.commit();
                        }
                    }
                    state.sound_default_confirm = None;
                } else {
                    state.sound_default_confirm =
                        Some((state.selected_instrument, now));
                }
            } else if armed
                && ui.input(|i| i.pointer.any_pressed())
                && !default_resp.hovered()
            {
                // Clicked anywhere else while armed: cancel.
                state.sound_default_confirm = None;
            }
            ui.add_space(3.0);
            // [201] Store/Restore: quick A/B snapshot while tweaking (per
            // slot, editing aid, not persisted). [201b] The snapshot follows
            // the edit scope — lane, step p-lock, or morph group — like
            // Default [200c]. Restore applies the SNAPSHOT's scope, whatever
            // the panel shows now.
            let slot = state.selected_instrument;
            let stored = state.sound_scratchpad[slot].is_some();
            let scope_name = match resolved.scope {
                crate::ui::editor_state::EditScope::LaneGlobal => "lane's sound",
                crate::ui::editor_state::EditScope::StepPlock { .. } => "step's p-lock",
                crate::ui::editor_state::EditScope::Morph { .. } => "group's morph",
            };
            let restore_resp = ui
                .add_enabled_ui(stored, |ui| {
                    crate::ui::controls::keycap_button(
                        ui,
                        "Restore",
                        52.0,
                        crate::ui::widgets::KeycapState::Rest,
                        true,
                        f_sans_med(9.5),
                    )
                })
                .inner
                .on_hover_text("Restore the snapshot saved with Store (A/B while tweaking).")
                .on_disabled_hover_text("Nothing stored for this lane yet — use Store first.");
            if restore_resp.clicked() {
                if let Some(scratch) = &state.sound_scratchpad[slot] {
                    match scratch {
                        crate::ui::editor_state::SoundScratch::Lane(snap, algo) => {
                            sound_settings.set_settings_for_slot(slot, snap);
                            let algo_param = params.algos()[slot];
                            setter.set_parameter(algo_param, *algo as i32);
                        }
                        crate::ui::editor_state::SoundScratch::Plock {
                            step,
                            active,
                            field_mask,
                            values,
                        } => {
                            if *active {
                                plock.masks.set_active(slot, *step, true);
                                plock.field_masks.set_raw(slot, *step, *field_mask);
                                for (field, value) in values.iter().enumerate() {
                                    plock.values.set(slot, *step, field, *value);
                                }
                            } else {
                                // Stored while the step had NO p-lock: restore
                                // means back to none.
                                plock.clear(slot, *step);
                            }
                        }
                        crate::ui::editor_state::SoundScratch::Morph {
                            fusion_index,
                            group,
                        } => {
                            // Same publish path as MorphSource::commit.
                            pattern.store_lane_fusion(
                                slot,
                                params.track_layout.state.grid_slot(slot),
                                *fusion_index,
                                *group,
                            );
                        }
                    }
                }
            }
            ui.add_space(3.0);
            let store_resp = crate::ui::controls::keycap_button(
                ui,
                "Store",
                46.0,
                crate::ui::widgets::KeycapState::Rest,
                true,
                f_sans_med(9.5),
            )
            .on_hover_text(format!(
                "Save a quick snapshot of this {scope_name} (A/B while tweaking; not persisted)."
            ));
            if store_resp.clicked() {
                let scratch = match resolved.scope {
                    crate::ui::editor_state::EditScope::LaneGlobal => {
                        crate::ui::editor_state::SoundScratch::Lane(
                            sound_settings.get_settings_for_slot(slot),
                            params.algos()[slot].value() as u8,
                        )
                    }
                    crate::ui::editor_state::EditScope::StepPlock { step } => {
                        let mut values = [0.0f32; crate::plock::FIELD_COUNT];
                        for (field, v) in values.iter_mut().enumerate() {
                            *v = plock.values.get(slot, step, field);
                        }
                        crate::ui::editor_state::SoundScratch::Plock {
                            step,
                            active: plock.masks.is_active(slot, step),
                            field_mask: plock.field_masks.get_raw(slot, step),
                            values,
                        }
                    }
                    crate::ui::editor_state::EditScope::Morph {
                        fusion_index, ..
                    } => crate::ui::editor_state::SoundScratch::Morph {
                        fusion_index,
                        group: fusions[fusion_index],
                    },
                };
                state.sound_scratchpad[slot] = Some(scratch);
            }
        });
        });
        ui.add_space(3.0);
    }
}

/// [269] Debug builds only: dump / load / delete the lane's sound as a preset
/// dump (authoring aid).
fn draw_dev_preset_dumps(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    state: &mut EditorUIState,
    values: RowValues,
) {
    let PanelCtx {
        params,
        setter,
        sound_settings,
        inst,
        voice_idx,
        ..
    } = *cx;
    let RowValues {
        freq,
        decay,
        vol,
        filt,
        attack,
        release,
        decay_curve,
        release_curve,
        hold,
        filter_env_amount,
        filter_env_decay,
        analog,
        stereo,
    } = values;
    // ------ Dev Tools: Preset Dumps ------
    if cfg!(debug_assertions) {
        ui.collapsing("Dev: Preset Dumps", |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                ui.text_edit_singleline(&mut state.dump_name_input);
                if ui.button("Dump").clicked() {
                    let instrument = &crate::instrument_registry::INSTRUMENTS[voice_idx];
                    let mut specials = Vec::new();
                    for def in instrument.special_params {
                        specials.push(inst.special_value(def.special_index));
                    }
                    let algo = params.algos()[state.selected_instrument].value() as u8;
                    // Skip Analog for instruments that don't use it
                    let standards = if instrument.analog_fixed || instrument.is_sampler {
                        // "Analog fixé" voices + the samplers: no analog
                        // drift, so 0.0 is a placeholder
                        [
                            freq,
                            decay,
                            vol,
                            filt,
                            attack,
                            release,
                            decay_curve,
                            release_curve,
                            hold,
                            filter_env_amount,
                            filter_env_decay,
                            0.0,
                            stereo,
                        ]
                    } else {
                        [
                            freq,
                            decay,
                            vol,
                            filt,
                            attack,
                            release,
                            decay_curve,
                            release_curve,
                            hold,
                            filter_env_amount,
                            filter_env_decay,
                            analog,
                            stereo,
                        ]
                    };

                    let dump = preset_dumps::PresetDump {
                        name: state.dump_name_input.clone(),
                        instrument_idx: state.selected_instrument,
                        instrument_label: instrument.label.to_string(),
                        standards,
                        algo,
                        specials,
                    };
                    if let Err(e) = preset_dumps::dump_preset(&dump) {
                        eprintln!("Dump failed: {}", e);
                    }
                }
            });
            let dumps = preset_dumps::list_dumps();
            if !dumps.is_empty() {
                ui.label("Existing dumps:");
                for info in dumps {
                    ui.horizontal(|ui| {
                        ui.label(format!("{} - {}", info.instrument_label, info.name));
                        if ui.button("Load").clicked() {
                            if let Ok(dump) = preset_dumps::load_dump(&info.path) {
                                let dump_slot =
                                    dump.instrument_idx.min(crate::track::MAX_TRACKS - 1);
                                select_legacy_track(state, dump_slot);
                                let dump_voice = schema_voice_idx(params, dump_slot);
                                let target_inst = &sound_settings.instruments[dump_slot];
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Freq,
                                    dump.standards[0],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Decay,
                                    dump.standards[1],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Volume,
                                    dump.standards[2],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::FilterFreq,
                                    dump.standards[3],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Attack,
                                    dump.standards[4],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Release,
                                    dump.standards[5],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::DecayCurve,
                                    dump.standards[6],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::ReleaseCurve,
                                    dump.standards[7],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Hold,
                                    dump.standards[8],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::FilterEnvAmount,
                                    dump.standards[9],
                                );
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::FilterEnvDecay,
                                    dump.standards[10],
                                );
                                // Skip Analog for instruments that don't use it:
                                // "Analog fixé" voices + the samplers ([269]).
                                let dump_def = &crate::instrument_registry::INSTRUMENTS[dump_voice];
                                let is_analog_fixed = dump_def.analog_fixed || dump_def.is_sampler;
                                if !is_analog_fixed {
                                    store_field(
                                        target_inst,
                                        crate::instrument_registry::StandardField::Analog,
                                        dump.standards[11],
                                    );
                                }
                                store_field(
                                    target_inst,
                                    crate::instrument_registry::StandardField::Stereo,
                                    dump.standards[12],
                                );
                                let algo_param = params.algos()[dump_slot];
                                setter.set_parameter(algo_param, dump.algo as i32);
                                let inst_def = &crate::instrument_registry::INSTRUMENTS[dump_voice];
                                for (i, def) in inst_def.special_params.iter().enumerate() {
                                    if i < dump.specials.len() {
                                        target_inst
                                            .set_special(def.special_index, dump.specials[i]);
                                    }
                                }
                                sound_settings.bump_version();
                            }
                        }
                        if ui.button("Delete").clicked() {
                            let _ = preset_dumps::delete_dump(&info.path);
                        }
                    });
                }
            }
        });
    }
}

/// [269] [223] The filter TYPE row that heads the Filter section. Returns
/// whether it edited something.
fn draw_filter_type_row(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    family: crate::instrument_registry::ParamFamily,
    src: &mut Box<dyn ParamSource + '_>,
) -> bool {
    let PanelCtx {
        instrument,
        row_salt,
        ..
    } = *cx;
    let special_defs = instrument.special_params;
    let mut changed = false;
    // [223] The filter TYPE heads its section: it decides what
    // every row under it means. It is declared among the
    // specials, which render after the standard rows, so it
    // would otherwise land in the middle of them. Found by
    // name, so Buzz, SDrex and Rift are all ordered the same
    // way; the specials loop below skips it.
    if family == crate::instrument_registry::ParamFamily::Filter {
        if let Some(def) = special_defs
            .iter()
            .find(|d| d.name.ends_with("_filter_type"))
        {
            let id = ParamId::Special(def.special_index);
            let names: &[&str] = def.options.unwrap_or(&["LP", "HP", "BP"]);
            let current =
                (src.get(id).round().max(0.0) as usize).min(names.len().saturating_sub(1));
            let (reverted, picked) = help::zone(
                ui,
                ("filter_type", def.name),
                &row_help(&**src, id, help::special_param(def.name).to_string()),
                |ui| {
                    row_scoped(ui, src.is_overridden(id), src.supports(id).reason(), |ui| {
                        editor_label(ui, def.label);
                        right_aligned_select(ui, (def.name, row_salt), current, names)
                    })
                },
            );
            if reverted {
                src.clear(id);
                changed = true;
            } else if let Some(idx) = picked {
                src.set(id, idx as f32);
                changed = true;
            }
        }
    }
    changed
}

/// [269] The standard rows of one section (and the specials hoisted under
/// them). Returns the edited values, the hoisted specials (the specials loop
/// skips them) and whether a row was edited.
fn draw_standard_rows(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    family: crate::instrument_registry::ParamFamily,
    texture_param: Option<&'static crate::instrument_registry::SpecialParamDef>,
    src: &mut Box<dyn ParamSource + '_>,
    values: RowValues,
) -> (RowValues, Vec<&'static str>, bool) {
    let PanelCtx {
        plock,
        instrument,
        slot,
        voice_idx,
        page,
        ..
    } = *cx;
    let standard_defs = instrument.standard_params;
    let special_defs = instrument.special_params;
    let RowValues {
        mut freq,
        mut decay,
        mut vol,
        mut filt,
        mut attack,
        mut release,
        mut decay_curve,
        mut release_curve,
        mut hold,
        mut filter_env_amount,
        mut filter_env_decay,
        mut analog,
        mut stereo,
    } = values;
    let mut changed = false;
    // Standard params for this family
    // smp voices: One Shot bypasses the amp envelope — grey out
    // the Env sliders (the One Shot switch is a special param,
    // rendered below, and stays enabled).
    let env_disabled = family == crate::instrument_registry::ParamFamily::Env
        && crate::instrument_registry::is_sampler(voice_idx)
        && src.get(ParamId::Special(2)) > 0.5;
    // [227] Specials drawn right under a standard row instead of
    // at the tail of the section (see the hoist below); the
    // specials loop skips them.
    let mut hoisted: Vec<&'static str> = Vec::new();
    ui.add_enabled_ui(!env_disabled, |ui| {
        // [240] Canonical Amp row order at RENDER time: every
        // instrument reads its envelope the same way (a time, then
        // its curve), whatever order its registry table declares.
        // Stable sort: unranked fields keep their places.
        let mut fam_defs: Vec<_> = standard_defs
            .iter()
            .filter(|d| {
                d.family == family && d.field != crate::instrument_registry::StandardField::Volume
            })
            .collect();
        if family == crate::instrument_registry::ParamFamily::Env {
            fam_defs.sort_by_key(|d| crate::instrument_registry::env_row_rank(d.field));
        }
        for def in fam_defs {
            // smp voices: Stereo renders under the Sample
            // select (Osc family), not in Output ([168]).
            // [228] Texture voices: under the File row.
            if (crate::instrument_registry::is_sampler(voice_idx) || texture_param.is_some())
                && def.field == crate::instrument_registry::StandardField::Stereo
            {
                continue;
            }
            // [282] Bass drums: Hz / Note on its own "Mode" row, right above
            // Frequency (the choice comes before the value it formats).
            let note_row = instrument.freq_as_notes
                && def.field == crate::instrument_registry::StandardField::Freq;
            if note_row {
                let freq_id = ParamId::Std(def.field);
                let (_, mode_change) = row_scoped(
                    ui,
                    false,
                    src.supports(freq_id).reason(),
                    |ui| {
                        draw_freq_mode_row(
                            ui,
                            ("freq_mode", cx.row_salt),
                            src.get(ParamId::FreqMode) >= 0.5,
                        )
                    },
                );
                if let Some(notes) = mode_change {
                    src.set(ParamId::FreqMode, notes as u8 as f32);
                    if notes {
                        // Entering note mode snaps to the nearest note.
                        let ratio = instrument.freq_display_ratio;
                        freq = note_to_freq(freq_to_note(freq * ratio).round()) / ratio;
                        src.set(freq_id, freq);
                    }
                    changed = true;
                }
            }
            let row_rect = ui
                .horizontal(|ui| {
                    let label_text =
                        if def.field == crate::instrument_registry::StandardField::FilterFreq {
                            let ft = crate::instrument_registry::filter_type_label(voice_idx);
                            if ft.is_empty() {
                                def.label.to_string()
                            } else {
                                format!("{} ({})", def.label, ft)
                            }
                        } else {
                            def.label.to_string()
                        };

                    let is_bass_drum = instrument.freq_as_notes;
                    let freq_in_notes = is_bass_drum
                        && def.field == crate::instrument_registry::StandardField::Freq
                        && src.get(ParamId::FreqMode) >= 0.5;

                    match (&def.widget, def.field) {
                        (
                            crate::instrument_registry::ParamWidget::Slider {
                                min,
                                max,
                                logarithmic,
                                suffix,
                            },
                            field,
                        ) => {
                            // Bass drums can display frequency as Hz or musical notes.
                            if is_bass_drum
                                && field == crate::instrument_registry::StandardField::Freq
                            {
                                let ratio = instrument.freq_display_ratio;
                                let id = ParamId::Std(field);
                                let (reverted, row) = row_scoped(
                                    ui,
                                    src.is_overridden(id),
                                    src.supports(id).reason(),
                                    |ui| {
                                        draw_editor_frequency_row(
                                            ui,
                                            &label_text,
                                            &mut freq,
                                            *min,
                                            *max,
                                            VoiceSettings::default().frequency,
                                            *logarithmic,
                                            ratio,
                                            freq_in_notes,
                                        )
                                    },
                                );
                                if reverted {
                                    src.clear(id);
                                    changed = true;
                                } else if row.value_changed || row.response.changed() {
                                    src.set(id, freq);
                                    changed = true;
                                }
                            } else {
                                let value: &mut f32 = match field {
                                    crate::instrument_registry::StandardField::Freq => &mut freq,
                                    crate::instrument_registry::StandardField::Decay => &mut decay,
                                    crate::instrument_registry::StandardField::Volume => &mut vol,
                                    crate::instrument_registry::StandardField::FilterFreq => {
                                        &mut filt
                                    }
                                    crate::instrument_registry::StandardField::Attack => {
                                        &mut attack
                                    }
                                    crate::instrument_registry::StandardField::Release => {
                                        &mut release
                                    }
                                    crate::instrument_registry::StandardField::DecayCurve => {
                                        &mut decay_curve
                                    }
                                    crate::instrument_registry::StandardField::ReleaseCurve => {
                                        &mut release_curve
                                    }
                                    crate::instrument_registry::StandardField::Hold => &mut hold,
                                    crate::instrument_registry::StandardField::FilterEnvAmount => {
                                        &mut filter_env_amount
                                    }
                                    crate::instrument_registry::StandardField::FilterEnvDecay => {
                                        &mut filter_env_decay
                                    }
                                    crate::instrument_registry::StandardField::Analog => {
                                        &mut analog
                                    }
                                    crate::instrument_registry::StandardField::Stereo => {
                                        &mut stereo
                                    }
                                };
                                // [184] One definition of "the default":
                                // `param_default` keeps both branches
                                // (per-voice table for the samplers and
                                // SDrex, shared defaults otherwise).
                                let default_value = src.inherited(ParamId::Std(field));
                                // Relative-pitch voices: the Pitch slider
                                // steps by 1 semitone (Pitch Fine covers
                                // the cents).
                                let smp_pitch = field
                                    == crate::instrument_registry::StandardField::Freq
                                    && crate::instrument_registry::is_sampler(voice_idx);
                                let id = ParamId::Std(field);
                                // [292] TM6(AC): the Frequency range (and its reset
                                // value) follow the Model: each recorded tom has
                                // its own pitch.
                                let (min_v, max_v, default_value) = if voice_idx
                                    == crate::synthesis::DrumVoice::Tm6Ac as usize
                                    && field == crate::instrument_registry::StandardField::Freq
                                {
                                    let model = src.get(ParamId::Special(0));
                                    let (lo, hi) = crate::instrument_registry::tm6_freq_range(model);
                                    (lo, hi, crate::instrument_registry::tm6_recorded_hz(model))
                                } else {
                                    (*min, *max, default_value)
                                };
                                let (reverted, edited) = row_scoped(
                                    ui,
                                    src.is_overridden(id),
                                    src.supports(id).reason(),
                                    |ui| {
                                        let row = draw_editor_slider_row_full(
                                            ui,
                                            &label_text,
                                            value,
                                            min_v,
                                            max_v,
                                            default_value,
                                            *logarithmic,
                                            *suffix,
                                            if smp_pitch { 1.0 } else { 0.0 },
                                        );
                                        page_lock_menu(
                                            &row,
                                            plock,
                                            slot,
                                            page,
                                            id,
                                            min_v,
                                            max_v,
                                            *logarithmic,
                                        );
                                        row.changed()
                                    },
                                );
                                if reverted {
                                    src.clear(id);
                                    changed = true;
                                } else if edited {
                                    src.set(id, *value);
                                    changed = true;
                                }
                            }
                        }
                        (crate::instrument_registry::ParamWidget::Checkbox, field) => {
                            let value: &mut f32 = match field {
                                crate::instrument_registry::StandardField::Stereo => &mut stereo,
                                _ => &mut stereo,
                            };
                            let id = ParamId::Std(field);
                            let (reverted, edited) = row_scoped(
                                ui,
                                src.is_overridden(id),
                                src.supports(id).reason(),
                                |ui| draw_editor_switch_row(ui, &label_text, value).changed(),
                            );
                            if reverted {
                                src.clear(id);
                                changed = true;
                            } else if edited {
                                src.set(id, *value);
                                changed = true;
                            }
                        }
                    }
                })
                .response
                .rect;
            // [283] Every slider row but the Hz / Note frequency one carries
            // the page menu, so its help says so.
            let mut std_help = help::std_param(voice_idx, def.field);
            if matches!(
                def.widget,
                crate::instrument_registry::ParamWidget::Slider { .. }
            ) && !note_row
            {
                std_help.push_str(help::SLIDER_GESTURES);
            }
            // A row greyed with a reason shows that reason, not the help.
            if src.supports(ParamId::Std(def.field)).reason().is_none() {
                help::rect(ui, row_rect, ("std_row", def.label), &std_help);
            }
            // Multisample voices: Pitch Fine lives directly under
            // the Pitch slider (it tunes the same parameter).
            if def.field == crate::instrument_registry::StandardField::Freq
                && crate::instrument_registry::is_sampler(voice_idx)
            {
                let fine_id = ParamId::Special(9);
                let mut fine = src.get(fine_id);
                let (reverted, edited) = help::zone(
                    ui,
                    "pitch_fine",
                    &row_help(
                        &**src,
                        fine_id,
                        help::special_param("_fine_tune").to_string(),
                    ),
                    |ui| {
                        row_scoped(
                            ui,
                            src.is_overridden(fine_id),
                            src.supports(fine_id).reason(),
                            |ui| {
                                draw_editor_slider_row(
                                    ui,
                                    "Pitch Fine",
                                    &mut fine,
                                    -100.0,
                                    100.0,
                                    0.0,
                                    false,
                                    Some(" ct"),
                                )
                                .changed()
                            },
                        )
                    },
                );
                if reverted {
                    src.clear(fine_id);
                } else if edited {
                    src.set(fine_id, fine);
                }
            }
            // [227] Rows that belong right under a standard row,
            // not at the tail of the section: Resonance under the
            // cutoff, the filter envelope's stages under their
            // amount/decay rows so it reads A-H-D in order, curves
            // included ([240]: Attack, Atk Curve, Hold under Filter
            // Env ; Dec Curve under Filter Decay - Buzz's
            // `_filter_curve` IS its decay curve). By name suffix
            // and within the SAME family, so a voice whose
            // Resonance lives in its source section keeps it there.
            let hoist: &[&str] = match def.field {
                crate::instrument_registry::StandardField::FilterFreq => &["_resonance"],
                // Canonical stage order (a time, then its curve):
                // the `_atk`/`_hld` spellings cover One-Shot's
                // fraction-of-sample stages ([243]).
                crate::instrument_registry::StandardField::FilterEnvAmount => &[
                    "_filter_attack",
                    "_filter_atk",
                    "_filter_atk_curve",
                    "_filter_hold",
                    "_filter_hld",
                ],
                crate::instrument_registry::StandardField::FilterEnvDecay => {
                    &["_filter_dec_curve", "_filter_curve"]
                }
                _ => &[],
            };
            for suffix in hoist {
                if let Some(sdef) = special_defs
                    .iter()
                    .find(|d| d.family == family && d.name.ends_with(suffix))
                {
                    draw_plain_special_row(ui, src.as_mut(), plock, slot, page, sdef);
                    hoisted.push(sdef.name);
                }
            }
        }
    });
    (
        RowValues {
            freq,
            decay,
            vol,
            filt,
            attack,
            release,
            decay_curve,
            release_curve,
            hold,
            filter_env_amount,
            filter_env_decay,
            analog,
            stereo,
        },
        hoisted,
        changed,
    )
}

/// [269] The special rows of one section, stored PER SLOT. Returns the
/// edited Stereo switch (some specials draw it) and whether a row was edited.
#[allow(clippy::too_many_arguments)]
/// [303] The lane's inserts, in the lane's chain order. Each module's title strip carries
/// its buttons: move up, move down, fold. Folded (the default), only its On switch shows.
/// Lane settings (never locked per step, whatever cell is selected), written to the track
/// layout.
/// [303] Room left before an insert's name for its drag handle.
const GRIP_ROOM: f32 = 16.0;

fn draw_lane_inserts(
    ui: &mut egui::Ui,
    params: &DrumFlashParams,
    sound_settings: &SoundSettingsState,
    slot: usize,
    state: &mut EditorUIState,
) {
    use crate::synthesis::inserts::{value_index, INSERTS, INSERT_COUNT};
    let mut ins = params.track_layout.state.inserts(slot);
    let mut edited = false;
    // Each module's span (insert, top, bottom), for the drag-and-drop below.
    let mut spans: Vec<(usize, f32, f32)> = Vec::with_capacity(INSERT_COUNT);
    for pos in 0..INSERT_COUNT {
        let n = ins.order[pos] as usize;
        let def = &INSERTS[n];
        let mut module = module_begin(ui, Some(def.name));
        // The drag handle sits at the start of the strip, before the name.
        module.title_dx = GRIP_ROOM;
        // The strip's controls: the handle on the left; the fold triangle and the On switch
        // on the right.
        let content = ui.max_rect();
        let strip = egui::Rect::from_min_max(
            egui::pos2(content.left() - MODULE_INSET, module.top),
            egui::pos2(content.right() + MODULE_INSET, module.top + crate::ui::skeuo::MODULE_STRIP_H),
        );
        let unfolded = state.insert_unfolded[n];
        let size = 18.0;
        // Right to left: the On switch, then the fold triangle.
        let switch_rect = egui::Rect::from_min_size(
            egui::pos2(strip.right() - 12.0 - 34.0, strip.center().y - 9.0),
            Vec2::new(34.0, 18.0),
        );
        let button = |k: f32| {
            egui::Rect::from_center_size(
                egui::pos2(switch_rect.left() - 14.0 - k * (size + 6.0), strip.center().y),
                Vec2::splat(size),
            )
        };
        let fold_rect = button(0.0);
        let grip_rect = egui::Rect::from_center_size(
            egui::pos2(strip.left() + crate::ui::skeuo::MODULE_TITLE_X + 4.0, strip.center().y),
            Vec2::splat(size),
        );
        let switched = ui
            .put(switch_rect, crate::ui::widgets::ToggleSwitch::new(ins.on[n]))
            .on_hover_text(def.help)
            .clicked();
        if switched {
            ins.on[n] = !ins.on[n];
            // Switching an insert on opens it: its settings are what one wants next.
            if ins.on[n] {
                state.insert_unfolded[n] = true;
            }
            edited = true;
        }
        let fold = ui.interact(fold_rect, ui.make_persistent_id(("insert_fold", n)), egui::Sense::click());
        let grip = ui.interact(grip_rect, ui.make_persistent_id(("insert_grip", n)), egui::Sense::drag());
        let painter = ui.painter();
        // Fold triangle: pointing down when open, right when folded.
        {
            let c = fold_rect.center();
            let r = 4.5;
            let pts = if unfolded {
                vec![egui::pos2(c.x - r, c.y - r * 0.6), egui::pos2(c.x + r, c.y - r * 0.6), egui::pos2(c.x, c.y + r * 0.7)]
            } else {
                vec![egui::pos2(c.x - r * 0.6, c.y - r), egui::pos2(c.x - r * 0.6, c.y + r), egui::pos2(c.x + r * 0.7, c.y)]
            };
            let color = if fold.hovered() { INK() } else { INK2() };
            painter.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
        }
        // Drag handle: six dots.
        {
            let dragging = state.insert_drag == Some(n);
            let color = if dragging || grip.hovered() { INK() } else { INK3() };
            for row in 0..3 {
                for col in 0..2 {
                    let p = egui::pos2(
                        grip_rect.center().x - 2.5 + col as f32 * 5.0,
                        grip_rect.center().y - 5.0 + row as f32 * 5.0,
                    );
                    painter.circle_filled(p, 1.4, color);
                }
            }
        }
        if grip.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
        if grip.drag_started() {
            state.insert_drag = Some(n);
        }
        let _ = grip.on_hover_text("Drag to move this insert in the chain");
        if fold
            .on_hover_text(if unfolded { "Hide the settings" } else { "Show the settings" })
            .clicked()
        {
            state.insert_unfolded[n] = !unfolded;
        }

        // [303] The compressor's GR meter, under its title (also folded: it is what one
        // watches while playing).
        if n == crate::synthesis::inserts::COMPRESSOR && ins.on[n] {
            draw_gr_meter(ui, sound_settings, slot);
        }
        if state.insert_unfolded[n] {
            let enabled = ins.on[n];
            ui.add_enabled_ui(enabled, |ui| {
                for (row, p) in def.params.iter().enumerate() {
                    let k = value_index(row, p);
                    let value = &mut ins.values[n][k];
                    let changed = help::zone(ui, ("insert_param", slot, n, k), p.help, |ui| {
                        ui.allocate_ui(Vec2::new(EDITOR_PARAMS_W, 22.0), |ui| {
                            if let Some(options) = p.options {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 8.0;
                                    editor_label(ui, p.label);
                                    let last = options.len().saturating_sub(1) as f32;
                                    let current = value.round().clamp(0.0, last) as usize;
                                    match right_aligned_select(ui, ("insert_menu", slot, n, k), current, options) {
                                        Some(idx) if idx != current => {
                                            *value = idx as f32;
                                            true
                                        }
                                        _ => false,
                                    }
                                })
                                .inner
                            } else {
                                draw_editor_slider_row(ui, p.label, value, p.min, p.max, p.default, p.log, p.unit)
                                    .changed()
                            }
                        })
                        .inner
                    });
                    edited |= changed;
                }
            });
        }
        let module_top = module.top;
        module_end(ui, module);
        spans.push((n, module_top, ui.cursor().top() - 12.0));
    }
    // Drag-and-drop of an insert by its handle: a line shows where it will land; letting go
    // moves it there.
    if let Some(dragged) = state.insert_drag {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        let others: Vec<&(usize, f32, f32)> = spans.iter().filter(|s| s.0 != dragged).collect();
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let target = others.iter().filter(|s| 0.5 * (s.1 + s.2) < pointer.y).count();
            let y = if target == 0 {
                others.first().map(|s| s.1 - 6.0)
            } else {
                others.get(target - 1).map(|s| s.2 + 6.0)
            };
            if let Some(y) = y {
                let content = ui.max_rect();
                ui.painter().line_segment(
                    [egui::pos2(content.left() - MODULE_INSET, y), egui::pos2(content.right() + MODULE_INSET, y)],
                    egui::Stroke::new(2.0, AMBER()),
                );
            }
            if ui.input(|i| i.pointer.any_released()) {
                let mut order: Vec<u8> = ins.order.iter().copied().filter(|&x| x as usize != dragged).collect();
                order.insert(target.min(order.len()), dragged as u8);
                if let Ok(order) = <[u8; INSERT_COUNT]>::try_from(order) {
                    if order != ins.order {
                        ins.order = order;
                        edited = true;
                    }
                }
                state.insert_drag = None;
            }
        }
        if !ui.input(|i| i.pointer.any_down()) {
            state.insert_drag = None;
        }
    }
    if edited {
        let mut layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
        layout.slots[slot].inserts = ins;
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
    }
}

/// [303] Gain-reduction meter of a lane's compressor insert: a bar from 0 to 24 dB that
/// jumps to each new peak and falls back at 20 dB/s, with its value.
fn draw_gr_meter(ui: &mut egui::Ui, sound_settings: &SoundSettingsState, slot: usize) {
    const RANGE_DB: f32 = 24.0;
    const FALL_DB_PER_S: f32 = 20.0;
    let id = egui::Id::new(("insert_gr_meter", slot));
    let now = ui.ctx().input(|i| i.time);
    let (shown, last) = ui.ctx().data(|d| d.get_temp::<(f32, f64)>(id)).unwrap_or((0.0, now));
    let fallen = (shown - FALL_DB_PER_S * (now - last) as f32).max(0.0);
    let gr = sound_settings.take_insert_gr(slot).max(fallen);
    ui.ctx().data_mut(|d| d.insert_temp(id, (gr, now)));
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
    help::zone(
        ui,
        ("insert_gr", slot),
        "GR: how many dB the compressor is taking off right now. A bar that jumps on every hit and falls back between them is a compressor working on the hits; one that never falls back is a compressor holding the whole lane down.",
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                editor_label(ui, "GR");
                let w = (EDITOR_PARAMS_W - EDITOR_LABEL_W - 64.0).max(60.0);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 10.0), egui::Sense::hover());
                let painter = ui.painter();
                painter.rect_filled(rect, 2.0, Color32::from_rgb(18, 18, 22));
                let frac = (gr / RANGE_DB).clamp(0.0, 1.0);
                if frac > 0.0 {
                    let bar = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + rect.width() * frac, rect.bottom()));
                    painter.rect_filled(bar, 2.0, AMBER());
                }
                for db in [3.0, 6.0, 12.0, 18.0] {
                    let x = rect.left() + rect.width() * db / RANGE_DB;
                    painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())], egui::Stroke::new(1.0, Color32::from_black_alpha(140)));
                }
                ui.label(RichText::new(format!("-{gr:.1} dB")).font(f_mono_med(11.0)).color(INK()));
            });
        },
    );
}

/// [303] The effect lanes' Sequencer switches (in their Sound tab).
fn is_fx_sequencer_param(name: &str) -> bool {
    matches!(
        name,
        "reverb_glide" | "delay_glide" | "rumble_glide" | "reverb_norestart" | "delay_norestart" | "rumble_norestart"
    )
}

fn draw_special_rows(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    family: crate::instrument_registry::ParamFamily,
    hoisted: &[&'static str],
    src: &mut Box<dyn ParamSource + '_>,
    state: &mut EditorUIState,
    mut stereo: f32,
) -> (f32, bool) {
    let PanelCtx {
        params,
        setter,
        plock,
        pattern,
        instrument,
        slot,
        voice_idx,
        ..
    } = *cx;
    let special_defs = instrument.special_params;
    let mut changed = false;
    // [304] Rows drawn so far in this section (an LFO group after other rows gets a divider).
    let mut rows_drawn = 0usize;
    // Special params for this family — stored PER SLOT so two
    // slots of the same kind stay independent.
    for def in special_defs.iter().filter(|d| d.family == family) {
        // Pitch Fine is rendered directly under the Pitch slider.
        if def.name.ends_with("_fine_tune") {
            continue;
        }
        // [227] Already drawn under its standard row.
        if hoisted.contains(&def.name) {
            continue;
        }
        // [286] Loop Restart is switched with the arrow on the effect lane's first
        // cell; [303] it and Glide are rows of the Effects tab's Sequencer module.
        if is_fx_sequencer_param(def.name) {
            continue;
        }
        // [286] A lane-wide setting (the Delay's Sync, the Reverb's Type and Gate switch, the
        // Rumble's triggers...) is never part of a step's lock, but it SHOWS with a cell
        // selected too: editing it there changes the lane (`PlockSource` passes it through),
        // and it says what the step's Time lock means (a synced Time is greyed).
        // [223] The filter type heads the section, above the
        // standard rows.
        if def.name.ends_with("_filter_type") {
            continue;
        }
        // [243] A one-entry Texture "menu" (One-Shot: the
        // lane's file only) is no choice at all — the File
        // row IS its UI, and the Stereo switch follows it
        // (greyed while the file has no right channel).
        if def.name.ends_with("_texture") && def.options.map_or(false, |o| o.len() <= 1) {
            row_scoped(ui, false, None, |ui| {
                draw_user_texture_row(ui, params, state, slot);
            });
            let is_stereo_file = params
                .user_textures
                .pool
                .get(slot)
                .map(|b| b.right.is_some())
                .unwrap_or(false);
            let stereo_id = ParamId::Std(StandardField::Stereo);
            let (reverted, edited) = row_scoped(
                ui,
                src.is_overridden(stereo_id),
                src.supports(stereo_id).reason(),
                |ui| {
                    ui.add_enabled_ui(is_stereo_file, |ui| {
                        draw_editor_switch_row(ui, "Stereo", &mut stereo)
                            .on_hover_text("Stereo: play the file's left and right channels as they are. Off, both are mixed to mono.")
                            .on_disabled_hover_text("Only bites on a stereo user file: this file has one channel.")
                            .changed()
                    })
                    .inner
                },
            );
            if reverted {
                src.clear(stereo_id);
                changed = true;
            } else if edited {
                src.set(stereo_id, stereo);
                changed = true;
            }
            continue;
        }
        // Buzz gate controls render in their own sub-row, with
        // the gate shape graph beside them (see below).
        if def
            .name
            .starts_with(crate::instrument_registry::GATE_ROW_PREFIX)
        {
            continue;
        }
        // Multisample voices (*606): the Sample list only makes
        // sense in fixed-sample mode — grey it out (don't hide
        // it, to keep the layout stable) while Analog Mode
        // (random multisample) is on.
        let sample_disabled = def.name.ends_with("_sample") && src.get(ParamId::Special(0)) > 0.5;
        // [269] Keyed on the names (Sdrex today): a
        // `*_filter_mod` switch turned on greys the flanger row.
        let filter_mod_active = instrument
            .special_params
            .iter()
            .find(|d| {
                d.name
                    .ends_with(crate::instrument_registry::FILTER_MOD_SUFFIX)
            })
            .map(|d| src.get(ParamId::Special(d.special_index)) > 0.5)
            .unwrap_or(false);
        // [181] Only Feedback is flanger-specific now: the
        // Fade-in applies to both modulation modes.
        let modulation_disabled = filter_mod_active
            && def
                .name
                .ends_with(crate::instrument_registry::FLANGER_ONLY_SUFFIX);
        // [221] Grain only bites while Loop is on: with Loop
        // off the slice is played once and its length is the
        // envelope's business. Greyed, not hidden. Keyed on the
        // names, so any instrument declaring a `*_loop` switch
        // beside `*_grain*` rows gets the same behaviour.
        let grain_disabled = def.name.contains("_grain")
            && instrument
                .special_params
                .iter()
                .find(|d| d.name.ends_with("_loop"))
                .map(|d| src.get(ParamId::Special(d.special_index)) < 0.5)
                .unwrap_or(false);
        // [233] The saturation's sub-parameters (Amount, Mix,
        // Output Gain) step in under Saturation Type and grey
        // out on None. Pre-Filter, Crush and Decimate are not
        // part of it. The only sub-parameter styling there is.
        let saturation_sub = def.name.ends_with("_saturation_amount")
            || def.name.ends_with("_saturation_mix")
            || def.name.ends_with("_saturation_output_gain");
        let saturation_off = saturation_sub
            && instrument
                .special_params
                .iter()
                .find(|d| d.name.ends_with("_saturation_type"))
                .map(|d| src.get(ParamId::Special(d.special_index)) < 0.5)
                .unwrap_or(false);
        // [286] The reverb's Gate Time and Gate Release step in under the Gate switch and
        // grey out while it is off (the same sub-parameter look as the saturation's).
        let gate_sub = def.name == "reverb_gate" || def.name == "reverb_gate_release";
        let gate_off = gate_sub
            && src.get(ParamId::Special(crate::synthesis::reverb_sp::GATE_ON)) < 0.5;
        // [286] The delay's Time knob is the sync menu's business once a note value is chosen.
        let sync_disabled = (def.name == "delay_time"
            && src.get(ParamId::Special(crate::synthesis::delay_sp::SYNC)) >= 0.5)
            || (def.name == "rumble_echo_time"
                && src.get(ParamId::Special(crate::synthesis::rumble_sp::ECHO_SYNC)) >= 0.5)
            // [304] An LFO rate synced to the tempo.
            || crate::instrument_registry::lfo_sync_slot(def.name)
                .is_some_and(|k| params.track_layout.state.lfo_sync(slot, k) > 0);
        let special_id = ParamId::Special(def.special_index);
        // [184] A parameter with no per-step slot of its own is
        // greyed WITH its reason rather than hidden: the special
        // whose field collides with Attack, for instance.
        let unsupported = src.supports(special_id).reason();
        // [227] Advance: a switch above its step slider and the
        // options folded under it. Switch and options are LANE
        // settings (the slot's `advance_mode` param): they say
        // how the sequencer counts, not how the voice sounds.
        let advance_row = def.name.ends_with("_advance");
        let advance_param = params.advance_modes()[slot];
        let advance_mode = advance_param.value();
        let advance_on = advance_mode & crate::advance_mode::ON != 0;
        if advance_row {
            let mut on = if advance_on { 1.0 } else { 0.0 };
            let (_, toggled) = row_scoped(ui, false, None, |ui| {
                draw_editor_switch_row(ui, "Advance", &mut on)
                    .on_hover_text(ADVANCE_HELP)
                    .changed()
            });
            if toggled {
                set_int_param(
                    setter,
                    advance_param,
                    advance_mode ^ crate::advance_mode::ON,
                );
            }
        }
        let advance_disabled = advance_row && !advance_on;
        if saturation_sub || gate_sub {
            set_sub_indent(SUB_INDENT_PX);
        }
        // [304] Each LFO (its Rate, Sync and Depth) reads as its own group: a hairline
        // between it and the rows above.
        if rows_drawn > 0 && crate::instrument_registry::lfo_sync_slot(def.name).is_some() {
            ui.add_space(2.0);
            let (line, _) = ui.allocate_exact_size(Vec2::new(EDITOR_PARAMS_W, 1.0), egui::Sense::hover());
            ui.painter().hline(line.x_range(), line.center().y, egui::Stroke::new(1.0, LINE()));
            ui.add_space(2.0);
        }
        rows_drawn += 1;
        let row_area = ui.add_enabled_ui(
            !(sample_disabled
                || modulation_disabled
                || grain_disabled
                || sync_disabled
                || advance_disabled
                || saturation_off
                || gate_off
                || unsupported.is_some()),
            |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    let reverted = row_gutter(ui, src.is_overridden(special_id));
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let current = src.get(special_id);
                    let new_value =
                        special_row_widget(ui, cx, def, special_id, current, stereo, &**src);
                    if reverted {
                        src.clear(special_id);
                    } else if let Some(value) = new_value {
                        src.set(special_id, value);
                        // [292] Switching the TM6 Model moves Frequency to that
                        // tom's recorded pitch (inside its own range).
                        if def.name == crate::instrument_registry::TM6_MODEL {
                            src.set(
                                ParamId::Std(crate::instrument_registry::StandardField::Freq),
                                crate::instrument_registry::tm6_recorded_hz(value),
                            );
                        }
                    }
                });
            },
        );
        set_sub_indent(0.0);
        // [304] Under an LFO rate: its tempo sync (a lane setting, never a lock).
        if let Some(k) = crate::instrument_registry::lfo_sync_slot(def.name) {
            let current = params.track_layout.state.lfo_sync(slot, k) as usize;
            let options = crate::instrument_registry::RUMBLE_ECHO_SYNC;
            // Named after its rate ("Pitch LFO Rate" -> "Pitch LFO Sync"): two LFOs, two
            // Syncs, no guessing which is which.
            let sync_label = match def.label.strip_suffix(" Rate") {
                Some(stem) => format!("{stem} Sync"),
                None => "Sync".to_string(),
            };
            let picked = help::zone(ui, ("lfo_sync", slot, k), help::LFO_SYNC, |ui| {
                ui.horizontal(|ui| {
                    ui.set_max_width(EDITOR_PARAMS_W);
                    // The same left gutter as every other row (its revert dot lives there):
                    // without it the label sat one gutter to the left.
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(ROW_GUTTER_W);
                    ui.spacing_mut().item_spacing.x = 8.0;
                    editor_label(ui, &sync_label);
                    right_aligned_select(ui, ("lfo_sync_menu", slot, k), current.min(options.len() - 1), options)
                })
                .inner
            });
            if let Some(entry) = picked.filter(|e| *e != current) {
                let mut layout = PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
                layout.slots[slot].lfo_sync[k] = entry as u8;
                PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
            }
        }
        if unsupported.is_none() {
            let mut row_help = help::special_param(def.name).to_string();
            if def.continuous && def.options.is_none() {
                row_help.push_str(help::SLIDER_GESTURES);
            }
            help::rect(
                ui,
                row_area.response.rect,
                ("special_row", def.name),
                &row_help,
            );
        }
        if let Some(reason) = unsupported {
            // The greyed row explains itself over its WHOLE area.
            // (An `ui.label("")` allocates nothing, so the hover
            // text it carried was unreachable.)
            row_area.response.on_disabled_hover_text(reason);
        }
        // [228] Under the Texture menu, always: the lane's own
        // file and Load / Clear. Loading selects "Custom".
        if def.name.ends_with("_texture") {
            row_scoped(ui, false, None, |ui| {
                draw_user_texture_row(ui, params, state, slot);
            });
        }
        // [228] Stereo, a permanent row under Stereo Spread:
        // a stereo file plays its two channels as they are
        // when on, mixed to mono when off. Greyed - not hidden
        // - when the current texture has no right channel
        // (embedded textures, mono files, empty slots).
        if def.name.ends_with("_stereo_spread") {
            let texture_index = instrument
                .special_params
                .iter()
                .find(|d| d.name.ends_with("_texture"))
                .map(|d| src.get(ParamId::Special(d.special_index)).round().max(0.0) as usize)
                .unwrap_or(0);
            let is_stereo_file = crate::synthesis::sample_bank::resolve_texture(
                texture_index,
                Some((&params.user_textures.pool, slot)),
            )
            .bank()
            .right
            .is_some();
            let stereo_id = ParamId::Std(StandardField::Stereo);
            let (reverted, edited) = row_scoped(
                ui,
                src.is_overridden(stereo_id),
                src.supports(stereo_id).reason(),
                |ui| {
                    ui.add_enabled_ui(is_stereo_file, |ui| {
                        draw_editor_switch_row(ui, "Stereo", &mut stereo)
                            .on_hover_text("Stereo: play the file's left and right channels as they are. Off, both are mixed to mono. Stereo Spread adds its offset on top.")
                            .on_disabled_hover_text("Only bites on a stereo user file: this texture has one channel. Use Stereo Spread for width.")
                            .changed()
                    })
                    .inner
                },
            );
            if reverted {
                src.clear(stereo_id);
                changed = true;
            } else if edited {
                src.set(stereo_id, stereo);
                changed = true;
            }
        }
        // [227] One click: a p-lock with a random Offset on
        // every ACTIVE cell of the lane, all pages. Each click
        // is a new draw; the values then live in the cells,
        // visible in the grid and editable one by one. Rift
        // only: on a One-Shot the file IS the sound, a random
        // start per cell makes no musical sense there.
        if def.name.ends_with("_offset")
            && instrument
                .special_params
                .iter()
                .any(|d| d.name.ends_with("_wander"))
        {
            let (_, clicked) = row_scoped(ui, false, None, |ui| {
                crate::ui::controls::keycap_button(
                    ui,
                    "Random Offset on active cells",
                    0.0,
                    crate::ui::widgets::KeycapState::Rest,
                    true,
                    f_sans_med(9.5),
                )
                .on_hover_text(
                    "Writes a p-lock with a random Offset on every active cell of this lane, on all pages. Each click is a new draw. Right-click the Offset slider for the page-wide Spread / Scatter / Clear actions.",
                )
                .clicked()
            });
            if clicked {
                if let Some(field) = special_id.plock_field() {
                    let seed = (ui.input(|i| i.time) * 1_000_003.0) as u64 as u32 | 1;
                    let mut values = [0.0f32; crate::plock::STEP_COUNT];
                    crate::plock::random_values(def.min, def.max, false, seed, &mut values);
                    let pairs: Vec<(usize, f32)> = (0..crate::plock::STEP_COUNT)
                        .filter(|&step| pattern.is_active(step, slot))
                        .map(|step| (step, values[step]))
                        .collect();
                    plock.fill_steps(slot, field, &pairs);
                }
            }
        }
        if advance_row {
            // Vertical inside the gutter row: a collapsing
            // header laid out horizontally would open its body
            // to the RIGHT of the header.
            row_scoped(ui, false, None, |ui| {
                ui.vertical(|ui| {
                    ui.add_enabled_ui(advance_on, |ui| {
                        draw_advance_options(ui, setter, advance_param, slot, advance_on);
                    });
                });
            });
        }
        // smp voices: the Stereo switch lives directly under the
        // Sample select ([168]) and works in BOTH modes — in
        // Analog Mode a random pair plays on every hit.
        if def.name.ends_with("_sample") && crate::instrument_registry::is_sampler(voice_idx) {
            let stereo_id = ParamId::Std(StandardField::Stereo);
            let (reverted, edited) = row_scoped(
                ui,
                src.is_overridden(stereo_id),
                src.supports(stereo_id).reason(),
                |ui| {
                    draw_editor_switch_row(ui, "Stereo", &mut stereo)
                        .on_hover_text(
                            "Stereo: plays two DIFFERENT samples per hit — the left \
                     channel plays the first of the pair, the right channel the \
                     second. The Sample list picks the pair (1+2, 3+4, 5+6, 7+8). \
                     With Analog Mode on, a random pair is played on every hit.",
                        )
                        .changed()
                },
            );
            if reverted {
                src.clear(stereo_id);
                changed = true;
            } else if edited {
                src.set(stereo_id, stereo);
                changed = true;
            }
        }
    }
    (stereo, changed)
}

/// [269] The widget of one special row, chosen from what the registry
/// declares (named options, a switch, the sample and type menus, or a
/// slider). Returns the value picked this frame, if any.
#[allow(clippy::too_many_arguments)]
fn special_row_widget(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    def: &'static crate::instrument_registry::SpecialParamDef,
    special_id: ParamId,
    current: f32,
    stereo: f32,
    src: &dyn ParamSource,
) -> Option<f32> {
    let PanelCtx {
        params,
        plock,
        slot,
        page,
        row_salt,
        ..
    } = *cx;
    let mut new_value = None;
    // [221] Named choices declared in the registry:
    // one generic dropdown instead of a new per-label
    // branch for every instrument. Tested first so a
    // declared list always wins over the older
    // recognise-by-wording branches below.
    if let Some(options) = def.options {
        let current_idx = (current.round().max(0.0) as usize).min(options.len() - 1);
        editor_label(ui, def.label);
        // [228] The Texture menu names the file each
        // user slot holds ("3: kick.wav"), "(missing)"
        // when it is gone, the plain slot when empty.
        let dynamic: Option<Vec<String>> = if def.name.ends_with("_texture") {
            Some(texture_menu_labels(options, params, slot))
        } else {
            None
        };
        let dynamic_refs: Option<Vec<&str>> = dynamic
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect());
        let shown: &[&str] = dynamic_refs.as_deref().unwrap_or(options);
        // Saved lanes store the menu index, so "Custom" keeps its index; the
        // Texture menu only DISPLAYS it last (the embedded textures first).
        let order: Vec<usize> = if def.name == crate::instrument_registry::TM6_MODEL {
            // [292] The retired Auto (0) is not offered; saved lanes keep 1 / 2.
            (1..shown.len()).collect()
        } else if def.name.ends_with("_texture") {
            let custom = crate::synthesis::sample_bank::CUSTOM_TEXTURE_INDEX;
            (0..shown.len())
                .filter(|&i| i != custom)
                .chain((custom < shown.len()).then_some(custom))
                .collect()
        } else {
            (0..shown.len()).collect()
        };
        let shown_in_order: Vec<&str> = order.iter().map(|&i| shown[i]).collect();
        let current_pos = order.iter().position(|&i| i == current_idx).unwrap_or(0);
        if let Some(pos) =
            right_aligned_select(ui, (def.name, row_salt), current_pos, &shown_in_order)
        {
            new_value = Some(order[pos] as f32);
        }
    // Boolean mode switches, including SDrex's modulation
    // target and free-running LFO phase.
    } else if def.name.ends_with("_filter_mod") {
        // [181] Two named destinations for the one LFO —
        // an on/off switch labelled "Filter Mod" did not
        // say what it was choosing between.
        let selected = if current > 0.5 { 1 } else { 0 };
        if let Some(idx) = segmented_row(
            ui,
            def.label,
            (def.name, row_salt),
            &["Flanger", "Filter LFO"],
            selected,
        ) {
            new_value = Some(idx as f32);
        }
    } else if def.name.ends_with("_analog_mode")
|| def.name.ends_with("_one_shot")
|| def.name.ends_with("_free_phase")
// [227] Any 0/1 discrete parameter without a
// named list IS a switch. Loop and Reverse
// were sliders for want of being listed here.
|| (!def.continuous
    && def.options.is_none()
    && def.min == 0.0
    && def.max == 1.0)
    {
        // [286] "Loop Restart" is shown as an ON switch but stored inverted.
        let inverted = def.name.ends_with("_norestart");
        let mut value = if inverted { 1.0 - current } else { current };
        if draw_editor_switch_row(ui, def.label, &mut value).changed() {
            new_value = Some(if inverted { 1.0 - value } else { value });
        }
    // Multisample voices: Sample select 1..8 (stored 1-based)
    } else if def.name.ends_with("_sample") {
        editor_label(ui, def.label);
        // [284] Analog Mode plays a different recording on every hit: say so where the
        // number would be, so the variation does not read as a setting that moves.
        if src.get(ParamId::Special(0)) > 0.5 {
            let random = if stereo > 0.5 { "Random pair / hit" } else { "Random each hit" };
            right_aligned_select(ui, (def.name, row_salt), 0, &[random]);
        } else if stereo > 0.5 {
            // [168] Stereo on: the list picks the PAIR.
            let pair_names = ["1+2", "3+4", "5+6", "7+8"];
            let current_idx = ((current.round() as usize).clamp(1, 8) - 1) / 2;
            if let Some(idx) =
                right_aligned_select(ui, (def.name, row_salt), current_idx, &pair_names)
            {
                new_value = Some((idx * 2 + 1) as f32);
            }
        } else {
            let sample_names = ["1", "2", "3", "4", "5", "6", "7", "8"];
            let current_idx = (current.round() as usize).clamp(1, 8) - 1;
            if let Some(idx) =
                right_aligned_select(ui, (def.name, row_salt), current_idx, &sample_names)
            {
                new_value = Some(idx as f32 + 1.0);
            }
        }
    // Boolean toggle for on/off switches (min=0, max=1)
    } else if def.min == 0.0 && def.max == 1.0 && def.label.to_lowercase().contains("pre-filter") {
        let mut value = current;
        if draw_editor_switch_row(ui, def.label, &mut value).changed() {
            new_value = Some(value);
        }
    // Saturation Type: show select with names instead of number slider
    } else if def.label.to_lowercase().contains("saturation type") {
        let type_names = [
            "None",
            "SoftClip",
            "Valve",
            "Transistor",
            "HardClip",
            "Tape",
        ];
        let current_idx = (current as usize).min(type_names.len().saturating_sub(1));
        editor_label(ui, def.label);
        if let Some(idx) = right_aligned_select(ui, (def.name, row_salt), current_idx, &type_names)
        {
            new_value = Some(idx as f32);
        }
    // Cymbal Noise Type: show select with names
    } else if def.label.to_lowercase().contains("noise type") {
        let type_names = ["White", "Pink", "Brown", "Blue"];
        let current_idx = (current as usize).min(type_names.len().saturating_sub(1));
        editor_label(ui, def.label);
        if let Some(idx) = right_aligned_select(ui, (def.name, row_salt), current_idx, &type_names)
        {
            new_value = Some(idx as f32);
        }
    // Kick Click Type: show select with names
    } else if def.label.to_lowercase().contains("click type") {
        let type_names = ["Soft", "Medium", "Hard"];
        let current_idx = (current as usize).min(type_names.len().saturating_sub(1));
        editor_label(ui, def.label);
        if let Some(idx) = right_aligned_select(ui, (def.name, row_salt), current_idx, &type_names)
        {
            new_value = Some(idx as f32);
        }
    // Buzz oscillator waveform: show select with names
    } else if def.name.ends_with("_wave") {
        let type_names = ["Sine", "Square", "Saw"];
        let current_idx = (current as usize).min(type_names.len().saturating_sub(1));
        editor_label(ui, def.label);
        if let Some(idx) = right_aligned_select(ui, (def.name, row_salt), current_idx, &type_names)
        {
            new_value = Some(idx as f32);
        }
    // Buzz filter type: LP / HP / BP select
    } else if def.name.ends_with("_filter_type") {
        let type_names = ["LP", "HP", "BP"];
        let current_idx = (current as usize).min(type_names.len().saturating_sub(1));
        editor_label(ui, def.label);
        if let Some(idx) = right_aligned_select(ui, (def.name, row_salt), current_idx, &type_names)
        {
            new_value = Some(idx as f32);
        }
    } else {
        let mut value = current;
        let logarithmic = def.min > 0.0 && def.max / def.min >= 20.0;
        let row = draw_editor_slider_row_curved(
            ui,
            def.label,
            &mut value,
            def.min,
            def.max,
            src.inherited(ParamId::Special(def.special_index)),
            logarithmic,
            def.unit,  // [182] specials carry a unit too
            def.curve, // [189] response curve from the registry
        );
        page_lock_menu(
            &row,
            plock,
            slot,
            page,
            special_id,
            def.min,
            def.max,
            logarithmic,
        );
        if row.changed() {
            new_value = Some(value);
        }
    }
    new_value
}

/// [269] The Algorithm selector, in the Oscillator section.
fn draw_algorithm_row(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    family: crate::instrument_registry::ParamFamily,
    state: &EditorUIState,
) {
    let PanelCtx {
        params,
        setter,
        voice_idx,
        ..
    } = *cx;
    // Algorithm selector inside OSC family
    if family == crate::instrument_registry::ParamFamily::Osc {
        if let Some(voice) = DrumVoice::from_index(voice_idx) {
            let algos = synthesis::algos_for(voice);
            // [269] No voice-index exception: a shared list
            // (OpenHiHat uses HiHat's) that disagrees with a
            // voice's algo_count fails the registry test
            // `algo_count_matches_the_named_algorithms`.
            if algos.len() > 1 {
                let algo_param = params.algos()[state.selected_instrument];
                help::zone(ui, "algorithm", help::EDITOR_ALGORITHM, |ui| {
                    // [282] Same gutter as the other rows (never overridden:
                    // the algorithm is not p-lockable here), so its label
                    // lines up instead of reading as a section title.
                    with_gutter(ui, false, |ui| {
                        editor_label(ui, "Algorithm");
                        let algo_names: Vec<&str> = algos.iter().map(|a| a.name).collect();
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            algo_combo(ui, setter, algo_param, &algo_names)
                        });
                    })
                });
            }
        }
    }
}

/// [269] The right-hand column of one section: its graph (envelope, filter
/// envelope, pitch, sample waveform, texture or lane file).
#[allow(clippy::too_many_arguments)]
fn draw_section_graphs(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    family: crate::instrument_registry::ParamFamily,
    has_graph: bool,
    texture_param: Option<&'static crate::instrument_registry::SpecialParamDef>,
    src: &mut Box<dyn ParamSource + '_>,
    values: RowValues,
) {
    let PanelCtx {
        params,
        instrument,
        slot,
        voice_idx,
        ..
    } = *cx;
    let standard_defs = instrument.standard_params;
    let RowValues {
        decay,
        filt,
        attack,
        decay_curve,
        release_curve,
        hold,
        filter_env_amount,
        filter_env_decay,
        ..
    } = values;
    // Right column: graphs (gap so they aren't cramped against the params)
    if has_graph {
        ui.add_space(16.0);
    }
    let sample_graph = if crate::instrument_registry::is_sampler(voice_idx) {
        // [247] Shared with the DSP — Oh6smp (24) plays oh606(),
        // not ch606(): its slices are twice as long.
        let bank = crate::synthesis::sample_bank::sampler_bank(voice_idx)
            .expect("is_sampler voices always have a bank");
        let hit_idx = (src.get(ParamId::Special(1)).round() as usize).clamp(1, 8) - 1;
        // Legacy sessions (pitch marker unset) predate End: full length.
        let end = if src.get(ParamId::Special(10)) < 0.5 {
            1.0
        } else {
            src.get(ParamId::Special(11))
        };
        Some((&bank.hits[hit_idx][..], src.get(ParamId::Special(3)), end))
    } else {
        None
    };

    // [243] One-Shot: the lane file's peaks + the region markers,
    // so the envelope graphs can draw OVER the waveform on the
    // region's time axis. The Arc keeps the bank alive while the
    // graphs borrow its peaks.
    let oneshot_wave = if is_oneshot(instrument) {
        let pick = |suffix: &str| {
            instrument
                .special_params
                .iter()
                .find(|d| d.name.ends_with(suffix))
                .map(|d| src.get(ParamId::Special(d.special_index)))
                .unwrap_or(0.0)
        };
        Some((
            params.user_textures.pool.get(slot),
            pick("_offset"),
            pick("_reverse") > 0.5,
        ))
    } else {
        None
    };
    let oneshot_peaks: &[(f32, f32)] = oneshot_wave
        .as_ref()
        .and_then(|(bank, _, _)| bank.as_ref().map(|b| &b.peaks[..]))
        .unwrap_or(&[]);

    // [286] The Rumble's Duck section: its depth, its curve and how long the return takes.
    if family == crate::instrument_registry::ParamFamily::Saturation
        && instrument.special_params.iter().any(|d| d.name == "rumble_duck")
    {
        let pick = |name: &str| {
            instrument
                .special_params
                .iter()
                .find(|d| d.name == name)
                .map(|d| src.get(ParamId::Special(d.special_index)))
                .unwrap_or(0.0)
        };
        draw_duck_graph(ui, pick("rumble_duck"), pick("rumble_duck_curve"), pick("rumble_release"));
    }

    match family {
        crate::instrument_registry::ParamFamily::Osc => {
            // The texture, the read position, the band Wander can
            // throw it into, and where Advance lands next. Rift
            // only ([243]: the gate is the `_offset` special).
            if let Some(def) = texture_param.filter(|_| {
                instrument
                    .special_params
                    .iter()
                    .any(|d| d.name.ends_with("_offset"))
            }) {
                let pick = |suffix: &str| {
                    instrument
                        .special_params
                        .iter()
                        .find(|d| d.name.ends_with(suffix))
                        .map(|d| src.get(ParamId::Special(d.special_index)))
                        .unwrap_or(0.0)
                };
                if instrument
                    .special_params
                    .iter()
                    .any(|d| d.name.ends_with("_wander"))
                {
                    let index = src
                        .get(ParamId::Special(def.special_index))
                        .round()
                        .max(0.0) as usize;
                    // [228] Embedded or user texture, the same
                    // resolution the voice applies (an empty user
                    // slot shows the fallback it plays).
                    let source = crate::synthesis::sample_bank::resolve_texture(
                        index,
                        Some((&params.user_textures.pool, slot)),
                    );
                    let bank = source.bank();
                    // The grain window is drawn with the SAME mapping
                    // the voice uses, or the picture would lie.
                    let texture_secs = bank.data.len() as f32 / bank.source_rate.max(1.0);
                    let grain_frac = if texture_secs > 0.0 {
                        crate::synthesis::rift_grain_seconds(pick("_grain")) / texture_secs
                    } else {
                        0.0
                    };
                    draw_texture_graph(
                        ui,
                        &bank.peaks,
                        pick("_offset"),
                        pick("_wander"),
                        if params.advance_modes()[slot].value() & crate::advance_mode::ON != 0 {
                            pick("_advance")
                        } else {
                            0.0
                        },
                        grain_frac,
                        pick("_loop") > 0.5,
                        pick("_reverse") > 0.5,
                    );
                } else {
                    // [243] One-Shot: the lane's own file, with the
                    // Offset marker and the played region. No
                    // embedded fallback — no file, no waveform.
                    let bank = params.user_textures.pool.get(slot);
                    draw_oneshot_graph(
                        ui,
                        bank.as_ref().map(|b| &b.peaks[..]).unwrap_or(&[]),
                        pick("_offset"),
                        pick("_reverse") > 0.5,
                    );
                }
            }
        }
        // (Only a section that HAS a pitch envelope draws its graph: the Rumble borrows this
        // family for its Reverb section and must not get a pitch envelope there.)
        crate::instrument_registry::ParamFamily::Pitch if has_graph => {
            let pick = |suffix: &str| {
                instrument
                    .special_params
                    .iter()
                    .find(|d| d.name.ends_with(suffix))
                    .map(|d| src.get(ParamId::Special(d.special_index)))
                    .unwrap_or(0.0)
            };
            if let Some((atk, hold, dec, atk_c, dec_c)) = ahd_pitch_env_params(instrument) {
                // [243] A-H-D pitch envelope over the region's
                // waveform: depth in semitones, shaped ramps, on
                // the region's time axis.
                let (offset, reverse) = oneshot_wave
                    .as_ref()
                    .map(|(_, o, r)| (*o, *r))
                    .unwrap_or((0.0, false));
                draw_oneshot_pitch_graph(
                    ui,
                    oneshot_peaks,
                    offset,
                    reverse,
                    pick("_pitch_env"),
                    src.get(ParamId::Special(atk)),
                    src.get(ParamId::Special(hold)),
                    src.get(ParamId::Special(dec)),
                    src.get(ParamId::Special(atk_c)),
                    src.get(ParamId::Special(dec_c)),
                );
            } else {
                // [227] Same law as the voice: depth in semitones,
                // exponential fall over the time slider.
                draw_pitch_envelope(
                    ui,
                    pick("_pitch_env"),
                    pick("_pitch_env_time"),
                    crate::synthesis::RIFT_PITCH_ENV_CURVE,
                );
            }
        }
        crate::instrument_registry::ParamFamily::Env => {
            if let Some((_, offset, reverse)) = &oneshot_wave {
                // [243] Amp A-H-D over the region's waveform, on
                // the region's time axis (`release_curve` is the
                // repurposed bipolar attack curve).
                draw_oneshot_amp_graph(
                    ui,
                    oneshot_peaks,
                    *offset,
                    *reverse,
                    attack,
                    release_curve,
                    hold,
                    decay,
                    decay_curve,
                );
            } else if let Some((hit, start, end)) = sample_graph {
                draw_sample_amp_graph(
                    ui,
                    hit,
                    start,
                    end,
                    attack,
                    decay,
                    release_curve, // repurposed as the bipolar attack curve
                    decay_curve,
                    src.get(ParamId::Special(2)) > 0.5,
                );
            } else {
                // A-H-D bipolar: `release_curve` is repurposed as the
                // attack curve; `decay_curve` shapes the decay.
                draw_amp_envelope(ui, attack, release_curve, hold, decay, decay_curve);
            }
        }
        crate::instrument_registry::ParamFamily::Filter => {
            let has_filter_env = standard_defs
                .iter()
                .any(|d| d.field == crate::instrument_registry::StandardField::FilterEnvAmount);
            if has_filter_env {
                let filter_curve = crate::synthesis::DrumVoice::from_index(voice_idx)
                    .and_then(|v| v.filter_env_curve())
                    .unwrap_or(decay_curve);
                if let Some((hit, start, end)) = sample_graph {
                    draw_sample_filter_graph(
                        ui,
                        hit,
                        start,
                        end,
                        filt,
                        filter_env_amount,
                        filter_env_decay,
                        filter_curve,
                    );
                } else if let Some((atk, hold, atk_c, dec_c)) = ahd_filter_env_params(instrument) {
                    if let Some((_, offset, reverse)) = &oneshot_wave {
                        // [243] One-Shot: the A-H-D cutoff sweep
                        // over the region's waveform, on the
                        // region's time axis.
                        draw_oneshot_filter_graph(
                            ui,
                            oneshot_peaks,
                            *offset,
                            *reverse,
                            filt,
                            filter_env_amount,
                            src.get(ParamId::Special(atk)),
                            src.get(ParamId::Special(hold)),
                            filter_env_decay,
                            src.get(ParamId::Special(atk_c)),
                            src.get(ParamId::Special(dec_c)),
                        );
                    } else {
                        // A-H-D filter envelope (attack/hold/decay, each
                        // ramp with its own bipolar curve) sweeping the
                        // cutoff: Buzz, SDrex and Rift. Found by
                        // parameter NAME rather than by instrument
                        // index, which used to mean one hand-written
                        // branch — and one table of index literals — per
                        // voice.
                        draw_buzz_filter_envelope(
                            ui,
                            filt,
                            filter_env_amount,
                            src.get(ParamId::Special(atk)),
                            src.get(ParamId::Special(hold)),
                            filter_env_decay,
                            src.get(ParamId::Special(atk_c)),
                            src.get(ParamId::Special(dec_c)),
                        );
                    }
                } else {
                    draw_filter_envelope(
                        ui,
                        filter_curve,
                        filter_env_decay,
                        filt,
                        filter_env_amount,
                    );
                }
            }
        }
        _ => {}
    }
}

/// [269] The gate sub-row of the Env section (Buzz), beside its graph.
fn draw_gate_row(
    ui: &mut egui::Ui,
    cx: &PanelCtx,
    family: crate::instrument_registry::ParamFamily,
    params_w: f32,
    src: &mut Box<dyn ParamSource + '_>,
    state: &EditorUIState,
) {
    let PanelCtx {
        params, instrument, ..
    } = *cx;
    let special_defs = instrument.special_params;
    // Buzz, Env family: the gate controls get their own row with the
    // gate shape graph BESIDE the sliders (not stacked under the amp
    // graph — stacking grew the section and shifted the layout).
    // [269] Keyed on the gate params' names, not on Buzz's index.
    let is_gate_row = |d: &&crate::instrument_registry::SpecialParamDef| {
        d.family == family
            && d.name
                .starts_with(crate::instrument_registry::GATE_ROW_PREFIX)
    };
    if family == crate::instrument_registry::ParamFamily::Env
        && special_defs.iter().any(|d| is_gate_row(&d))
    {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_max_width(params_w);
                ui.set_width(params_w);
                ui.spacing_mut().item_spacing.y = 9.0;
                for def in special_defs.iter().filter(is_gate_row) {
                    let gate_id = ParamId::Special(def.special_index);
                    let mut value = src.get(gate_id);
                    let logarithmic = def.min > 0.0 && def.max / def.min >= 20.0;
                    let (reverted, edited) = help::zone(
                        ui,
                        ("gate_row", def.name),
                        &row_help(&**src, gate_id, help::special_param(def.name).to_string()),
                        |ui| {
                            row_scoped(
                                ui,
                                src.is_overridden(gate_id),
                                src.supports(gate_id).reason(),
                                |ui| {
                                    draw_editor_slider_row(
                                        ui,
                                        def.label,
                                        &mut value,
                                        def.min,
                                        def.max,
                                        def.default,
                                        logarithmic,
                                        def.unit,
                                    )
                                    .changed()
                                },
                            )
                        },
                    );
                    if reverted {
                        src.clear(gate_id);
                    } else if edited {
                        src.set(gate_id, value);
                    }
                }
            });
            ui.add_space(16.0);
            draw_buzz_gate_graph(
                ui,
                params.algos()[state.selected_instrument].value() == 1,
                src.get(ParamId::Special(0)),
                src.get(ParamId::Special(1)),
                src.get(ParamId::Special(2)),
            );
        });
    }
}

/// [269] [184] The scope notice, pinned to the panel's bottom edge.
fn draw_scope_notice(
    ui: &mut egui::Ui,
    state: &mut EditorUIState,
    panel_rect: egui::Rect,
    src: &dyn ParamSource,
) {
    // [184] ph. 3 — scope notice, pinned to the panel's bottom edge.
    //
    // It used to be the first row of the scroll body, where it scrolled out of
    // sight exactly when it mattered. A modal was the other candidate, but
    // "this group is full" is a STATE, not an event: a modal would either
    // reappear on every frame or need dismissing for something the user must be
    // able to re-read. So: painted after the scroll area, therefore above it, and
    // outside its layout — no shift, no click capture.
    // The notice stands as long as its situation does, and re-arms once that
    // situation clears — so dismissing it does not hide it forever.
    let scope_notice = src.notice();
    if scope_notice.is_none() {
        state.scope_notice_dismissed = false;
    }
    if let Some(notice) = scope_notice.filter(|_| !state.scope_notice_dismissed) {
        let pad = 10.0;
        let strip_w = panel_rect.width() - 2.0 * pad;
        // Laid out as a WRAPPED galley rather than painted as a single line:
        // `painter.text` does not wrap, so a message longer than the strip simply
        // ran off its right edge. The strip's height follows the text instead.
        let text_inset = 10.0;
        // Room kept for the "click to dismiss" hint so the two never overlap.
        let hint_w = 78.0;
        let text_max_w = (strip_w - 2.0 * text_inset - hint_w).max(80.0);
        let galley = ui.fonts(|fonts| {
            fonts.layout(notice.to_owned(), f_sans_med(10.5), PL_LINK(), text_max_w)
        });
        let height = (galley.size().y + 12.0).max(26.0);
        let strip = egui::Rect::from_min_size(
            egui::pos2(panel_rect.left() + pad, panel_rect.bottom() - height - pad),
            Vec2::new(strip_w, height),
        );
        // Clickable so it can be dismissed before its timeout. The trade-off is
        // deliberate: while it shows, this strip swallows clicks meant for the row
        // underneath. Acceptable because it is transient AND dismissing it is
        // exactly what a click there means.
        let response = ui.interact(strip, ui.id().with("scope_refusal"), egui::Sense::click());
        let hovered = response.hovered();
        let painter = ui.painter();
        painter.rect_filled(strip, 6.0, if hovered { PANEL() } else { PANEL2() });
        painter.rect_stroke(
            strip,
            6.0,
            egui::Stroke::new(1.0, PL_LINK()),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            egui::pos2(
                strip.left() + text_inset,
                strip.center().y - galley.size().y * 0.5,
            ),
            galley,
            PL_LINK(),
        );
        if hovered {
            // The dismiss hint replaces nothing: it sits at the right edge, which
            // the message never reaches.
            painter.text(
                egui::pos2(strip.right() - 10.0, strip.center().y),
                egui::Align2::RIGHT_CENTER,
                "click to dismiss",
                f_sans_med(9.5),
                INK3(),
            );
        }
        if response.clicked() {
            state.scope_notice_dismissed = true;
        }
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
}

pub fn store_field(
    inst: &crate::sound_settings::InstrumentSettingsState,
    field: crate::instrument_registry::StandardField,
    value: f32,
) {
    use crate::instrument_registry::StandardField;
    match field {
        StandardField::Freq => inst.frequency.store(value.to_bits(), Ordering::Relaxed),
        StandardField::Decay => inst.decay.store(value.to_bits(), Ordering::Relaxed),
        StandardField::Volume => inst.volume.store(value.to_bits(), Ordering::Relaxed),
        StandardField::FilterFreq => inst.filter_freq.store(value.to_bits(), Ordering::Relaxed),
        StandardField::Attack => inst.attack.store(value.to_bits(), Ordering::Relaxed),
        StandardField::Release => inst.release.store(value.to_bits(), Ordering::Relaxed),
        StandardField::DecayCurve => inst.decay_curve.store(value.to_bits(), Ordering::Relaxed),
        StandardField::ReleaseCurve => inst.release_curve.store(value.to_bits(), Ordering::Relaxed),
        StandardField::Hold => inst.hold.store(value.to_bits(), Ordering::Relaxed),
        StandardField::FilterEnvAmount => inst
            .filter_env_amount
            .store(value.to_bits(), Ordering::Relaxed),
        StandardField::FilterEnvDecay => inst
            .filter_env_decay
            .store(value.to_bits(), Ordering::Relaxed),
        StandardField::Analog => inst.analog.store(value.to_bits(), Ordering::Relaxed),
        StandardField::Stereo => inst.stereo.store(value.to_bits(), Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Footprint (height, width of the params column) of a Frequency row in the
    /// Sound panel, laid out headless with the plugin's own fonts and nested as
    /// `draw_standard_rows` nests it.
    fn frequency_row_footprint(notes_active: bool) -> (f32, f32) {
        let ctx = egui::Context::default();
        crate::ui::install_egui_fonts(&ctx);
        let mut footprint = (0.0, 0.0);
        // A few passes: egui settles fonts and sizes over the first frames.
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(900.0, 700.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical(|ui| {
                        ui.set_width(EDITOR_PARAMS_W);
                        ui.spacing_mut().item_spacing.y = 9.0;
                        let top = ui.cursor().top();
                        let mut value = 60.0;
                        ui.horizontal(|ui| {
                            row_scoped(ui, false, None, |ui| {
                                draw_editor_frequency_row(
                                    ui,
                                    "Frequency",
                                    &mut value,
                                    20.0,
                                    400.0,
                                    60.0,
                                    true,
                                    0.3,
                                    notes_active,
                                )
                            });
                        });
                        footprint = (ui.cursor().top() - top, ui.min_rect().width());
                    });
                });
            });
        }
        footprint
    }

    /// Switching a kick's Frequency between Hz and Notes must not move the
    /// rows below it: same height, and same width — a Notes row 2 px wider
    /// than the column widened it and shifted every right-aligned row under it.
    /// [282] And it must fit the params column: with the Hz / Note switch in
    /// the row it overflowed by ~70 px, which widened the whole section.
    #[test]
    fn frequency_row_keeps_its_footprint_between_hz_and_notes() {
        let (hz_h, hz_w) = frequency_row_footprint(false);
        let (notes_h, notes_w) = frequency_row_footprint(true);
        assert!(hz_h > 0.0 && hz_w > 0.0);
        assert!(
            hz_w <= EDITOR_PARAMS_W && notes_w <= EDITOR_PARAMS_W,
            "column {EDITOR_PARAMS_W} px: Hz {hz_w} px, Notes {notes_w} px"
        );
        assert_eq!(hz_h, notes_h, "height: Hz {hz_h} px, Notes {notes_h} px");
        assert_eq!(
            hz_w, notes_w,
            "column width: Hz {hz_w} px, Notes {notes_w} px"
        );
    }
}
