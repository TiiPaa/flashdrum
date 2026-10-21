//! Editor UI state: clipboard types, popup structs and `EditorUIState` with its
//! lane copy/paste/clear/randomize operations.

use crate::{
    config::GlobalConfig,
    plock::PlockState,
    sequencer::{pattern::STEP_COUNT, FusedGroup, SharedPattern},
    sound_settings::{SoundSettings, SoundSettingsState},
    track::{TrackInstrumentKind, TrackLayoutState, TrackSlot},
    ui::controls::{set_float_param_if_changed, set_int_param_if_changed},
    DrumFlashParams,
};
use nih_plug::{params::persist::PersistentField, prelude::*};
use nih_plug_egui::egui;
use std::sync::atomic::Ordering;

/// Data stored in the clipboard for lane copy/paste.
#[derive(Debug, Clone)]
pub struct LaneClipboardData {
    /// The instrument kind (Kick, Snare, HiHat, etc.).
    pub kind: TrackInstrumentKind,
    /// The sound settings for this lane (all parameters).
    pub settings: SoundSettings,
    /// Per-slot synthesis algorithm.
    pub algo: i32,
    /// The step sequence for this lane.
    pub steps: [bool; STEP_COUNT],
    /// Step Fusion groups for this lane.
    pub fusions: Vec<FusedGroup>,
    /// Sound plocks for this lane.
    pub sound_plocks: LaneSoundPlocks,
    /// Sequencer plocks for this lane.
    pub seq_plocks: LaneSeqPlocks,
    /// Lane humanize value.
    pub humanize: f32,
    /// Lane push/pull value in ms.
    pub push_pull: f32,
    /// Lane length parameter.
    pub length: i32,
    /// Whether the lane has an individual length lock.
    pub length_locked: bool,
    /// [228] The lane's custom texture file, if any (Rift). Pasting loads it
    /// on the target lane - shared with the source through the decode cache,
    /// so it costs no second copy.
    pub texture_path: Option<std::path::PathBuf>,
    /// [286] The lane's sends to Fx1 / Fx2.
    pub fx_send: [f32; crate::track::FX_SLOT_COUNT],
    /// [300] The lane's send mutes.
    pub fx_send_mute: [bool; crate::track::FX_SLOT_COUNT],
    /// [303] The lane's inserts.
    pub inserts: crate::synthesis::inserts::InsertSettings,
}

#[derive(Debug, Clone)]
pub struct LaneSoundPlocks {
    pub mask: u64,
    pub field_masks: [u64; crate::plock::STEP_COUNT],
    pub values: [[f32; crate::plock::FIELD_COUNT]; crate::plock::STEP_COUNT],
}

#[derive(Debug, Clone)]
pub struct LaneSeqPlocks {
    pub mask: u64,
    pub solo_mask: u64,
    pub probabilities: [u32; crate::plock::STEP_COUNT],
    pub stutters: [u32; crate::plock::STEP_COUNT],
    pub conditions: [u32; crate::plock::STEP_COUNT],
    pub microtimings: [u32; crate::plock::STEP_COUNT],
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PlockClipboardEntry {
    pub instrument: usize,
    pub step: usize, // 0-15 within the page
    pub field_mask: u64,
    pub values: Vec<f32>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct FusionClipboardEntry {
    pub instrument: usize,
    pub start_step: usize, // 0-15 within the page
    pub end_step: usize,   // 0-15 within the page
    pub step_count: u8,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct PageClipboard {
    pub triggers: [u16; 16],
    pub plocks: Vec<PlockClipboardEntry>,
    #[serde(default)]
    pub fusions: Vec<FusionClipboardEntry>,
    /// [231] Sequencer p-locks of the page (probability, stutter, condition,
    /// microtiming, solo), step relative to the page.
    #[serde(default)]
    pub seq_plocks: Vec<SeqPlockClipboardEntry>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SeqPlockClipboardEntry {
    pub instrument: usize,
    pub step: usize,
    pub snapshot: crate::plock::SeqStepSnapshot,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SinglePlockClipboard {
    pub instrument: usize,
    pub field_mask: u64,
    pub values: Vec<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SoundEditorTab {
    #[default]
    Sound,
    /// [303] Sends (instrument lanes) and the Sequencer switches (effect lanes); the
    /// lane inserts come here next.
    Effects,
    Track,
}

#[derive(Clone, Copy, Debug)]
pub struct DragStepState {
    pub slot: usize,
    pub source_step: usize,
    pub source_rect: egui::Rect,
    pub start_time: f64,
    pub active: bool,
    pub current_target: usize,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
pub struct EditorUIState {
    pub selected_instrument: usize,
    #[serde(default)]
    pub selected_track_slot: usize,
    /// Cached instrument-preset list for the Track-tab loader + the (slot, kind)
    /// key it was built for. Runtime-only (rebuilt from disk), never persisted.
    #[serde(skip)]
    pub track_preset_entries: Vec<crate::presets::InstrumentPresetEntry>,
    #[serde(skip)]
    pub track_preset_cache_key: Option<(usize, usize)>,
    #[serde(default)]
    pub sound_editor_tab: SoundEditorTab,
    /// [303] Which inserts show their knobs in the Effects tab (folded by default).
    #[serde(default)]
    pub insert_unfolded: [bool; crate::synthesis::inserts::INSERT_COUNT],
    /// [303] The insert being dragged by its handle (to change the chain order).
    #[serde(skip)]
    pub insert_drag: Option<usize>,
    #[serde(default)]
    pub bottom_panel_tab: usize,
    pub selected_pattern_slot: usize,
    pub last_midi_export_path: Option<String>,
    pub last_midi_export_error: Option<String>,
    /// [271] MIDI export confirmation modal: the exported file, shown with an
    /// "Open folder" button (the exports directory is otherwise invisible).
    /// Runtime-only.
    #[serde(skip)]
    pub midi_export_modal: Option<std::path::PathBuf>,
    /// Grid-shift confirmation: a fusion straddles the wrap in this direction
    /// (delta of the pending shift, ±1); the modal asks before breaking it.
    #[serde(skip)]
    pub shift_grid_confirm: Option<isize>,
    pub dump_name_input: String,
    pub current_page: usize, // 0-3 (displaying steps current_page*16 .. current_page*16+15)
    pub follow_mode: bool,   // if true, page follows the playhead
    pub page_clipboard: Option<PageClipboard>, // copied page data for paste
    pub plock_clipboard: Option<SinglePlockClipboard>, // copied single plock for paste
    pub sequencer_mode: bool, // if true, right-click opens sequencer params instead of sound plocks
    // Pattern bank state
    /// The slot last loaded into the current pattern (None = fresh/preset).
    pub last_loaded_slot: Option<usize>,
    /// True when the user clicked "Save" and is now selecting a target slot.
    pub save_mode_active: bool,
    /// Copied pattern slot for copy/paste between slots.
    pub pattern_clipboard: Option<crate::pattern_bank::PatternSlot>,
    /// True when the user clicked "Clr" once and needs to confirm.
    pub clear_confirm_mode: bool,
    // Step Fusion state
    /// Start cell of a fusion selection (Shift+click), per track slot.
    pub fusion_selection_start: [Option<usize>; crate::track::MAX_TRACKS],
    /// Currently editing fusion group: (instrument, group_index).
    pub fusion_editing: Option<(usize, usize)>,
    /// [283] The lane the fusion edit box was opened from: a linked lane
    /// edits the group of the lane it follows but shows its own morph.
    #[serde(skip)]
    pub fusion_editing_lane: usize,
    /// [282] Window size last requested for the Zoom setting (asked once per
    /// change, so a host that refuses it is not asked again every frame).
    #[serde(skip)]
    pub zoom_requested_size: Option<(u32, u32)>,
    /// True when a fusion group just entered edit mode this frame; requests focus on the step-count field.
    pub fusion_edit_focus_request: bool,
    /// Current value of the track-name field in the Track tab.
    #[serde(skip)]
    pub track_name_input: String,
    /// Slot that `track_name_input` corresponds to; `None` means it needs to be
    /// initialized from the current slot.
    #[serde(skip)]
    pub track_name_input_slot: Option<usize>,
    /// When true, request focus on the Track tab Name field the next time it is drawn.
    #[serde(skip)]
    pub track_name_focus_request: bool,
    /// When set, the current pattern has been edited by the user and should be saved
    /// back to the given pattern-bank slot at the end of the UI frame. Used to keep
    /// edits made while Song Mode is active from being lost when the song advances.
    #[serde(skip)]
    pub pattern_dirty_slot: Option<usize>,
    /// Current value being edited in the Step Fusion step-count field.
    #[serde(skip)]
    pub fusion_edit_steps: u8,
    // Right-click plock popup state (replaces egui context_menu to control frame chrome).
    pub plock_popup: Option<PlockPopup>,
    /// Cell whose p-lock the Lane Editor edits ([184]). `None` = the lane global.
    #[serde(default)]
    pub sound_edit_target: Option<SelectedCell>,
    /// Slot waiting for a second click on the Lane Editor's Default button
    /// before its sound settings are reset to the instrument's defaults [200].
    /// `(slot, armed_at)` — auto-disarms after a few seconds or on any click
    /// elsewhere, so an armed "Sure?" is never a dead end.
    #[serde(skip)]
    pub sound_default_confirm: Option<(usize, f64)>,
    /// Per-slot A/B scratchpad for the Lane Editor's Store/Restore buttons
    /// [201]: a quick snapshot of what the panel edits — lane, step p-lock,
    /// or morph group, following the edit scope ([201b]). Not persisted —
    /// editing aid only.
    #[serde(skip)]
    pub sound_scratchpad: [Option<SoundScratch>; crate::track::MAX_TRACKS],
    /// Which of a fused group's three stores the panel edits ([184] ph. 3-4).
    #[serde(skip)]
    pub fusion_tab: FusionTab,
    /// When true, a click occurred while the p-lock popup was open; suppress step-cell
    /// toggles this frame so the popup (not the cell underneath) handles the click.
    #[serde(skip)]
    pub suppress_step_cell_click: bool,
    // Right-click page popup state (Copy/Paste/Clear page).
    pub page_popup: Option<PagePopup>,
    /// [228] A file dialog in flight for a user-texture slot. The dialog runs
    /// on its own thread: run inside the egui frame it re-entered baseview's
    /// message loop, egui panicked on the re-entry and, with `panic = abort`,
    /// took Studio One down. Not persisted.
    #[serde(skip)]
    pub texture_pick: Option<TexturePick>,
    /// Pending pattern-bank slot load (click on P1-P16) while the current
    /// pattern has unsaved changes. Requires explicit confirmation.
    #[serde(skip)]
    pub pattern_load_confirm: Option<usize>,
    /// Source slot while dragging a lane reorder handle.
    #[serde(default)]
    pub lane_drag_source: Option<usize>,
    /// Selected step in the Song Editor (0..15).
    #[serde(default)]
    pub song_selected_step: usize,
    /// Clipboard for a song step: (slot, repeat).
    #[serde(default)]
    pub song_clipboard: Option<(i8, u8)>,
    /// Song "Clear All" armed (time of the first click): the next click
    /// empties the song. Disarms after 3 s or on a click elsewhere ([283]).
    #[serde(skip)]
    pub song_clear_confirm: Option<f64>,
    /// Last song sequence published to the audio-thread controller, used to avoid
    /// pushing redundant snapshots every frame.
    #[serde(skip)]
    pub last_published_song: Option<crate::pattern_bank::SongSequence>,
    /// Clipboard for lane copy/paste (instrument + settings + sequence).
    #[serde(skip)]
    pub lane_clipboard: Option<LaneClipboardData>,
    /// Slot waiting for a second click before clearing its grid.
    #[serde(skip)]
    pub lane_clear_grid_confirm: Option<usize>,
    /// Slot waiting for a second click before being deleted/deactivated.
    #[serde(skip)]
    pub lane_delete_confirm: Option<usize>,
    /// Visual flash timer for the Test (T) button when triggered by external MIDI.
    #[serde(skip)]
    pub slot_flash_until: [f64; crate::track::MAX_TRACKS],
    /// The panel's notice strip was dismissed by hand ([184] ph. 3). Re-armed as
    /// soon as the situation it describes clears, so a dismissal hides the strip
    /// for this occurrence and not for good.
    #[serde(skip)]
    pub scope_notice_dismissed: bool,
    /// Long-press drag state for a step cell (move step with its plocks).
    #[serde(skip)]
    pub step_drag: Option<DragStepState>,
    /// Global user preferences loaded from `Documents/Flash Drum/config.json`.
    #[serde(skip)]
    pub global_config: GlobalConfig,
    /// True when the global settings popup is open.
    #[serde(skip)]
    pub settings_open: bool,
    /// [242] True when the Macros assignment modal is open.
    #[serde(skip)]
    pub macros_open: bool,
    /// Preset browser modal (instruments / patterns / songs).
    #[serde(skip)]
    pub preset_browser: Option<PresetBrowserState>,
    /// Density of Randomize Lane (fraction of steps turned on).
    #[serde(default = "default_randomize_density")]
    pub randomize_density: f32,
    /// Header "Clear All" armed (time of the first click): the next click
    /// wipes the whole program (grid + pattern bank + song + lanes). Disarms
    /// after 3 s or on a click elsewhere ([283]).
    #[serde(skip)]
    pub clear_program_confirm: Option<f64>,
}

/// Randomize Lane density of a fresh state and of a double-click reset (user
/// request: 80 %).
pub const DEFAULT_RANDOMIZE_DENSITY: f32 = 0.8;

fn default_randomize_density() -> f32 {
    DEFAULT_RANDOMIZE_DENSITY
}

/// State of the Presets modal ([150]).
pub struct PresetBrowserState {
    /// Active tab.
    pub kind: crate::presets::PresetKind,
    /// Name of the preset being saved.
    pub name_input: String,
    /// Patterns tab: also install the preset's lane kit on load.
    pub load_with_kit: bool,
    /// User preset waiting for a second click before deletion.
    pub confirm_delete: Option<std::path::PathBuf>,
    /// Grid tab: the built-in "Clear All" layout waits for a second click.
    pub confirm_clear_all_grid: bool,
    /// User preset being renamed inline ([237]) — the row shows a text field.
    pub renaming: Option<std::path::PathBuf>,
    /// New name being typed for `renaming`.
    pub rename_input: String,
    /// Focus the rename field on the frame it appears.
    pub rename_focus_request: bool,
    /// [261] Last failed save/rename/delete, shown in the modal until the next
    /// success (the old code swallowed errors and cleared the name field as
    /// if the save had worked).
    pub last_error: Option<String>,
    /// [262] Cached preset list for the active tab — re-read from disk only on
    /// open, tab change, or after a successful save/rename/delete, never per
    /// frame (the old code re-read and re-parsed every JSON at 60 fps).
    pub files: Option<Vec<crate::presets::PresetFileInfo>>,
    /// Which tab `files` was cached for.
    pub files_kind: Option<crate::presets::PresetKind>,
}

impl PresetBrowserState {
    /// [262] The preset list of the active tab, refreshed only when stale.
    /// Returns an owned snapshot so the render loop can keep mutating `self`.
    pub fn cached_files(&mut self) -> Vec<crate::presets::PresetFileInfo> {
        if self.files.is_none() || self.files_kind != Some(self.kind) {
            self.files = Some(crate::presets::list_presets(self.kind));
            self.files_kind = Some(self.kind);
        }
        self.files.clone().unwrap_or_default()
    }

    /// Drop the cache after a mutation (save/rename/delete).
    pub fn invalidate_files(&mut self) {
        self.files = None;
    }
}

impl Default for PresetBrowserState {
    fn default() -> Self {
        Self {
            kind: crate::presets::PresetKind::Pattern,
            name_input: String::new(),
            load_with_kit: true,
            confirm_delete: None,
            confirm_clear_all_grid: false,
            renaming: None,
            rename_input: String::new(),
            rename_focus_request: false,
            last_error: None,
            files: None,
            files_kind: None,
        }
    }
}

pub fn select_legacy_track(state: &mut EditorUIState, slot_idx: usize) {
    let slot_idx = slot_idx.min(crate::track::MAX_TRACKS - 1);
    // [184] Selecting a different lane abandons the cell selection: the panel
    // would otherwise show one lane's name and another lane's p-lock.
    if state.selected_track_slot != slot_idx {
        state.sound_edit_target = None;
    }
    state.selected_track_slot = slot_idx;
    // `selected_instrument` holds the SLOT index (0..MAX_TRACKS); the voice
    // schema for registry lookups is derived from the slot's kind.
    state.selected_instrument = slot_idx;
}

pub fn voice_idx_for_slot(params: &DrumFlashParams, slot_idx: usize) -> Option<usize> {
    params
        .track_layout
        .state
        .kind_for_slot(slot_idx)
        .map(|k| k.drum_voice_index())
}

/// Voice index used for registry/schema lookups (INSTRUMENTS, special_param, ...)
/// for a slot. Falls back to Kick's schema for inactive/unknown slots so the UI
/// never indexes past `DrumVoice::COUNT`.
pub fn schema_voice_idx(params: &DrumFlashParams, slot_idx: usize) -> usize {
    voice_idx_for_slot(params, slot_idx).unwrap_or(0)
}

pub fn effective_lane_length_for_ui(
    params: &DrumFlashParams,
    slot_idx: usize,
    master_length: usize,
) -> usize {
    // Lane locks and length params are slot-indexed, matching the audio engine
    // (`raw_lengths` / `lane_length_locks.is_locked(slot)` in lib.rs).
    let master_length = master_length.clamp(1, 64);
    if params.lane_length_locks.is_locked(slot_idx) {
        params.lengths()[slot_idx].value().clamp(1, 64) as usize
    } else {
        master_length
    }
}

/// The grid cell whose sound the Lane Editor is editing ([184]).
///
/// Distinct from [`PlockPopup`]: the popup is a transient menu whose presence
/// also acts as a modal flag that deadens grid clicks, whereas this selection is
/// persistent and must never do that. The step is stored **raw**, as clicked, and
/// normalised to a fusion's start cell at read time — so deleting a fusion
/// restores the exact cell the user picked.
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize)]
pub struct SelectedCell {
    pub slot: usize,
    pub step: usize,
}

/// Which store the panel edits on a FUSED cell ([184] phase 4).
///
/// A fused group has three: the start cell's own p-lock, and the two ends of the
/// morph. Phase 3 exposed only the two ends, which left no way to give a fused
/// step a flat override — the context menu was the only route, and phase 4
/// removes it. `Step` is the default because creating a ramp should be a
/// deliberate act, not what happens the first time you touch a slider.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum FusionTab {
    #[default]
    Step,
    Start,
    End,
}

/// What the Lane Editor's rows read and write this frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditScope {
    /// The lane's global sound: no cell selected, or editing is not allowed here.
    LaneGlobal,
    /// The selected cell's sound p-lock. Also the "Start" end of a fused group's
    /// morph, since that endpoint IS the start cell's p-lock.
    StepPlock { step: usize },
    /// A fused group's morph, with the endpoint the user asked for ([184] ph. 3).
    Morph {
        step: usize,
        fusion_index: usize,
        end: crate::ui::param_source::MorphEnd,
    },
}

/// Outcome of resolving the selection against the current pattern.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ResolvedScope {
    pub scope: EditScope,
    /// Cell to ring in the grid, already normalised to a fusion's start.
    pub highlight_step: Option<usize>,
    /// The selection no longer points at anything editable: the caller drops it.
    pub drop_selection: bool,
    /// The Start/End switch is meaningful here: the cell is fused AND its group
    /// emits more than one pulse (with a single pulse the engine disables
    /// morphing outright, so offering the switch would promise nothing).
    pub morph_available: bool,
}

/// Resolve the selected cell into an edit scope. Pure, so it is unit-testable
/// without an egui context.
///
/// - no selection, or the selection belongs to another lane than the one the
///   panel shows → the lane's global sound;
/// - p-lock editing disabled (Song mode, grid Follow) → global, but the
///   selection is **kept** so it comes back when the user leaves that mode;
/// - the lane is inactive, or the step is past the lane's length → global, and
///   the selection is dropped;
/// - a step covered by a fusion → normalised to the fusion's start cell, which
///   is the only cell that carries sound for the whole group.
pub fn resolve_edit_scope(
    selected: Option<SelectedCell>,
    panel_slot: usize,
    lane_active: bool,
    lane_length: usize,
    // `fusion_for_step(step)` -> the group covering it, as
    // `(start cell, fusion index, pulses)`.
    fusion_for_step: impl Fn(usize) -> Option<(usize, usize, u8)>,
    plock_editing_allowed: bool,
    tab: FusionTab,
) -> ResolvedScope {
    let global = ResolvedScope {
        scope: EditScope::LaneGlobal,
        highlight_step: None,
        drop_selection: false,
        morph_available: false,
    };
    let Some(cell) = selected else {
        return global;
    };
    if cell.slot != panel_slot {
        // Another lane is shown: edit that lane's global sound, but do not throw
        // the selection away — switching back must restore it.
        return global;
    }
    if !lane_active || cell.step >= crate::plock::STEP_COUNT || cell.step >= lane_length {
        return ResolvedScope {
            drop_selection: true,
            ..global
        };
    }
    if !plock_editing_allowed {
        return global;
    }
    let fusion = fusion_for_step(cell.step);
    let step = fusion.map(|(start, _, _)| start).unwrap_or(cell.step);
    // A single-pulse group cannot morph: `lib.rs` skips the interpolation when
    // `pulse_count == 1`, so the switch must not pretend otherwise.
    let morph_available = fusion.is_some_and(|(_, _, pulses)| pulses > 1);
    let scope = match (fusion, tab) {
        // The two morph ends. `Start` also goes through the morph source: an
        // inherited `Source` target stores ITS value in the group, so the routing
        // is per field, not per tab.
        (Some((_, fusion_index, _)), FusionTab::End) if morph_available => EditScope::Morph {
            step,
            fusion_index,
            end: crate::ui::param_source::MorphEnd::End,
        },
        (Some((_, fusion_index, _)), FusionTab::Start) if morph_available => EditScope::Morph {
            step,
            fusion_index,
            end: crate::ui::param_source::MorphEnd::Start,
        },
        // `Step`, and every non-morphable case: the start cell's own p-lock.
        _ => EditScope::StepPlock { step },
    };
    ResolvedScope {
        scope,
        highlight_step: Some(step),
        drop_selection: false,
        morph_available,
    }
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PlockPopup {
    pub instrument: usize,
    pub step: usize,
    /// Trigger state at the moment the popup was opened. P-lock menu clicks must
    /// never toggle the underlying sequencer step.
    #[serde(default)]
    pub step_was_active: bool,
    #[serde(with = "serde_pos2")]
    pub screen_pos: egui::Pos2,
    /// [218] True on the frame the popup opened. The right-click that opens the
    /// menu is still "clicked" when the menu draws in that same frame, so
    /// without this the close-on-right-click swallowed its own opening gesture
    /// and the menu never appeared.
    #[serde(default)]
    pub just_opened: bool,
    /// [213] Which p-lock this popup is editing - `false` = Sound,
    /// `true` = Sequencer. Seeded from the grid's P-Lock Mode when the popup
    /// opens, then switchable inside the menu: choosing the type no longer
    /// means flipping a grid-wide mode first.
    #[serde(default)]
    pub sequencer: bool,
}

/// [228] One native file dialog running on a helper thread, and where its
/// answer lands. `None` inside = still open; `Some(None)` = cancelled.
pub struct TexturePick {
    pub slot: usize,
    pub result: std::sync::Arc<std::sync::Mutex<Option<Option<std::path::PathBuf>>>>,
}

#[derive(Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PageMenuAction {
    Paste,
    Clear,
}

#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PagePopup {
    pub page: usize,
    #[serde(with = "serde_pos2")]
    pub screen_pos: egui::Pos2,
    pub confirm_action: Option<PageMenuAction>,
}

/// [201b] What the Store/Restore scratchpad holds — follows the panel's edit
/// scope: the lane's sound, one step's p-lock, or a fusion's morph group.
#[derive(Clone)]
pub enum SoundScratch {
    /// Lane scope: full sound settings + algo.
    Lane(crate::sound_settings::SoundSettings, u8),
    /// Step p-lock scope: the step's p-lock as-is (active bit, field mask,
    /// field values).
    Plock {
        step: usize,
        active: bool,
        field_mask: u64,
        values: [f32; crate::plock::FIELD_COUNT],
    },
    /// Morph scope: the fused group verbatim.
    Morph {
        fusion_index: usize,
        group: crate::sequencer::pattern::FusedGroup,
    },
}

impl EditorUIState {
    /// Randomize Lane density; 0.0 means "unset" (fresh/legacy state) → the
    /// 80 % default.
    pub fn randomize_density(&self) -> f32 {
        if self.randomize_density <= 0.0 {
            DEFAULT_RANDOMIZE_DENSITY
        } else {
            self.randomize_density.clamp(0.05, 1.0)
        }
    }

    /// Mark the current pattern as dirty so it will be auto-saved back to the
    /// currently loaded pattern-bank slot at the end of the frame when Song Mode
    /// is active. This prevents edits from being lost when the song advances.
    pub fn mark_pattern_dirty(&mut self) {
        self.pattern_dirty_slot = self.last_loaded_slot;
    }

    /// Copy the current lane (instrument + settings + sequence) to the clipboard.
    pub fn copy_lane(
        &mut self,
        params: &DrumFlashParams,
        slot: usize,
        settings_state: &SoundSettingsState,
        pattern: &SharedPattern,
        plock: &PlockState,
    ) {
        let Some(kind) = params.track_layout.state.kind_for_slot(slot) else {
            return;
        };

        let settings = settings_state.get_settings_for_slot(slot);
        let algo = params.algos()[slot].value();
        // [283] A linked lane copies the steps it shows (the lane above's).
        let grid = lane_grid_slot(params, slot);
        let steps = std::array::from_fn(|step| pattern.is_active(step, grid));
        let fusions = pattern.load_fusions(grid);
        let sound_plocks = copy_lane_sound_plocks(plock, slot);
        let seq_plocks = copy_lane_seq_plocks(&params.seq_plock_state.state, slot);
        let humanize = params.humanizes()[slot].value();
        let push_pull = params.pushes()[slot].value();
        let length = params.lengths()[slot].value();
        let length_locked = params.lane_length_locks.is_locked(slot);
        let texture_path = params.user_textures.path(slot);
        let fx_send = std::array::from_fn(|bus| params.track_layout.state.fx_send(slot, bus));
        let inserts = params.track_layout.state.inserts(slot);
        let fx_send_mute = std::array::from_fn(|bus| params.track_layout.state.fx_send_muted(slot, bus));

        self.lane_clipboard = Some(LaneClipboardData {
            kind,
            settings,
            algo,
            steps,
            fusions,
            sound_plocks,
            seq_plocks,
            humanize,
            push_pull,
            length,
            length_locked,
            texture_path,
            fx_send,
            fx_send_mute,
            inserts,
        });
    }

    /// Paste the full clipboard data to the target lane.
    /// Returns true if the paste was successful.
    pub fn paste_lane(
        &mut self,
        setter: &ParamSetter,
        params: &DrumFlashParams,
        target_slot: usize,
        settings_state: &SoundSettingsState,
        pattern: &SharedPattern,
        plock: &PlockState,
    ) -> bool {
        if target_slot >= crate::track::MAX_TRACKS {
            return false;
        }

        let Some(clipboard) = &self.lane_clipboard else {
            return false;
        };

        let mut layout =
            PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.clone());
        // [286] An effect only goes on an effect slot, an instrument elsewhere.
        let existing = layout.slots[target_slot].active.then_some(layout.slots[target_slot].kind);
        if !crate::track::kind_allowed_on_slot(clipboard.kind, target_slot, existing) {
            return false;
        }
        let filled_an_empty_lane = !layout.slots[target_slot].active;
        if layout.slots[target_slot].active {
            layout.slots[target_slot].kind = clipboard.kind;
            layout.slots[target_slot].name = clipboard.kind.default_name().to_string();
            layout.slots[target_slot].midi_note = clipboard.kind.default_midi_note();
        } else {
            layout.slots[target_slot] = TrackSlot::active_with_kind(clipboard.kind);
        }
        layout.slots[target_slot].fx_send = clipboard.fx_send;
        layout.slots[target_slot].inserts = clipboard.inserts;
        layout.slots[target_slot].fx_send_mute = clipboard.fx_send_mute;
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);

        // Pasting onto an EMPTY lane also turns a slot into a new instrument,
        // so it owes the same guarantee as the instrument picker: the clipboard
        // fills the pattern on screen, and the lane comes up blank on the other
        // 15 instead of carrying the deleted lane's steps. The live pattern
        // needs no wiping here — the paste below rewrites all 64 steps, every
        // p-lock field and the fusions.
        if filled_an_empty_lane {
            params
                .pattern_bank
                .clear_lane_in_saved_patterns(target_slot);
        }

        settings_state.set_settings_for_slot(target_slot, &clipboard.settings);
        set_int_param_if_changed(setter, params.algos()[target_slot], clipboard.algo);
        // [228] The custom texture follows the lane (or its absence does).
        match &clipboard.texture_path {
            Some(path) => {
                let _ = params.user_textures.load(target_slot, path);
            }
            None => params.user_textures.clear(target_slot),
        }

        // [283] A linked lane's steps are the shared ones it shows.
        let grid = lane_grid_slot(params, target_slot);
        paste_lane_steps(pattern, grid, &clipboard.steps);
        pattern.store_fusions(grid, &clipboard.fusions);
        paste_lane_sound_plocks(plock, target_slot, &clipboard.sound_plocks);
        paste_lane_seq_plocks(
            &params.seq_plock_state.state,
            target_slot,
            &clipboard.seq_plocks,
        );
        set_float_param_if_changed(setter, params.humanizes()[target_slot], clipboard.humanize);
        set_float_param_if_changed(setter, params.pushes()[target_slot], clipboard.push_pull);
        set_int_param_if_changed(setter, params.lengths()[target_slot], clipboard.length);
        set_lane_length_lock_for_slot(params, target_slot, clipboard.length_locked);

        select_legacy_track(self, target_slot);
        self.mark_pattern_dirty();
        true
    }

    /// Paste only the visible step grid to an already active target lane.
    /// Instrument, sound settings, plocks, fusions, routing and lane controls stay untouched.
    pub fn paste_grid(
        &mut self,
        params: &DrumFlashParams,
        target_slot: usize,
        pattern: &SharedPattern,
    ) -> bool {
        if target_slot >= crate::track::MAX_TRACKS {
            return false;
        }
        // [286] An effect lane has no steps to paste onto.
        match params.track_layout.state.kind_for_slot(target_slot) {
            None => return false,
            Some(kind) if kind.is_effect() => return false,
            Some(_) => {}
        }

        let Some(clipboard) = &self.lane_clipboard else {
            return false;
        };

        paste_lane_steps(
            pattern,
            lane_grid_slot(params, target_slot),
            &clipboard.steps,
        );
        select_legacy_track(self, target_slot);
        self.mark_pattern_dirty();
        true
    }

    /// Clear the musical grid for an active lane, keeping the module and settings intact.
    pub fn clear_lane(
        &mut self,
        params: &DrumFlashParams,
        slot: usize,
        pattern: &SharedPattern,
        plock: &PlockState,
    ) -> bool {
        if slot >= crate::track::MAX_TRACKS || !params.track_layout.state.is_active(slot) {
            return false;
        }

        // [283] A linked lane clears the steps it shows (shared with the lane
        // above); its p-locks are its own.
        let grid = lane_grid_slot(params, slot);
        clear_grid_steps(pattern, grid);
        pattern.store_fusions(grid, &[]);
        clear_grid_sound_plocks(plock, slot);
        clear_grid_seq_plocks(&params.seq_plock_state.state, slot);

        self.fusion_selection_start[slot] = None;
        self.fusion_selection_start[grid] = None;
        self.fusion_editing = self
            .fusion_editing
            .filter(|(idx, _)| *idx != slot && *idx != grid);
        self.plock_popup = self.plock_popup.filter(|popup| popup.instrument != slot);
        // [184] The lane's p-locks were just wiped: stop pointing at one.
        self.sound_edit_target = self.sound_edit_target.filter(|cell| cell.slot != slot);
        self.lane_clear_grid_confirm = None;
        self.lane_delete_confirm = None;
        self.mark_pattern_dirty();
        true
    }

    /// Randomize the musical grid for an active lane, keeping the module and settings intact.
    pub fn randomize_lane(
        &mut self,
        params: &DrumFlashParams,
        slot: usize,
        pattern: &SharedPattern,
        plock: &PlockState,
    ) -> bool {
        if slot >= crate::track::MAX_TRACKS || !params.track_layout.state.is_active(slot) {
            return false;
        }

        // [283] A linked lane randomizes the steps it shows (shared with the
        // lane above); its p-locks are its own.
        let grid = lane_grid_slot(params, slot);
        clear_grid_steps(pattern, grid);
        pattern.store_fusions(grid, &[]);
        clear_grid_sound_plocks(plock, slot);
        clear_grid_seq_plocks(&params.seq_plock_state.state, slot);
        randomize_lane_steps(pattern, grid, self.randomize_density());

        self.fusion_selection_start[slot] = None;
        self.fusion_selection_start[grid] = None;
        self.fusion_editing = self
            .fusion_editing
            .filter(|(idx, _)| *idx != slot && *idx != grid);
        self.plock_popup = self.plock_popup.filter(|popup| popup.instrument != slot);
        // [184] The lane's p-locks were just wiped: stop pointing at one.
        self.sound_edit_target = self.sound_edit_target.filter(|cell| cell.slot != slot);
        self.lane_clear_grid_confirm = None;
        self.lane_delete_confirm = None;
        self.mark_pattern_dirty();
        true
    }
}

/// [283] The lane whose steps and fusions `slot` shows: itself, or for a
/// lane linked to the one above, the head of its chain (as the grid draws it).
fn lane_grid_slot(params: &DrumFlashParams, slot: usize) -> usize {
    PersistentField::<TrackLayoutState>::map(&params.track_layout, |s| s.grid_slot(slot))
}

pub fn copy_lane_sound_plocks(plock: &PlockState, slot: usize) -> LaneSoundPlocks {
    LaneSoundPlocks {
        mask: plock.masks.masks[slot].load(Ordering::Relaxed),
        field_masks: std::array::from_fn(|step| plock.field_masks.get_raw(slot, step)),
        values: std::array::from_fn(|step| {
            std::array::from_fn(|field| plock.values.get(slot, step, field))
        }),
    }
}

pub fn paste_lane_sound_plocks(plock: &PlockState, target_slot: usize, data: &LaneSoundPlocks) {
    plock.masks.masks[target_slot].store(data.mask, Ordering::Relaxed);
    for step in 0..crate::plock::STEP_COUNT {
        plock
            .field_masks
            .set_raw(target_slot, step, data.field_masks[step]);
        for field in 0..crate::plock::FIELD_COUNT {
            plock
                .values
                .set(target_slot, step, field, data.values[step][field]);
        }
    }
}

pub fn clear_grid_sound_plocks(plock: &PlockState, slot: usize) {
    if slot >= crate::track::MAX_TRACKS {
        return;
    }
    plock.masks.masks[slot].store(0, Ordering::Relaxed);
    for step in 0..crate::plock::STEP_COUNT {
        plock.field_masks.set_raw(slot, step, 0);
        for field in 0..crate::plock::FIELD_COUNT {
            plock.values.set(slot, step, field, 0.0);
        }
    }
}

pub fn copy_lane_seq_plocks(seq: &crate::plock::SequencerPlockState, slot: usize) -> LaneSeqPlocks {
    LaneSeqPlocks {
        mask: seq.masks[slot].load(Ordering::Relaxed),
        solo_mask: seq.solo_masks[slot].load(Ordering::Relaxed),
        probabilities: std::array::from_fn(|step| {
            seq.probabilities[slot][step].load(Ordering::Relaxed)
        }),
        stutters: std::array::from_fn(|step| seq.stutters[slot][step].load(Ordering::Relaxed)),
        conditions: std::array::from_fn(|step| seq.conditions[slot][step].load(Ordering::Relaxed)),
        microtimings: std::array::from_fn(|step| {
            seq.microtimings[slot][step].load(Ordering::Relaxed)
        }),
    }
}

pub fn paste_lane_seq_plocks(
    seq: &crate::plock::SequencerPlockState,
    target_slot: usize,
    data: &LaneSeqPlocks,
) {
    seq.masks[target_slot].store(data.mask, Ordering::Relaxed);
    seq.solo_masks[target_slot].store(data.solo_mask, Ordering::Relaxed);
    for step in 0..crate::plock::STEP_COUNT {
        seq.probabilities[target_slot][step].store(data.probabilities[step], Ordering::Relaxed);
        seq.stutters[target_slot][step].store(data.stutters[step], Ordering::Relaxed);
        seq.conditions[target_slot][step].store(data.conditions[step], Ordering::Relaxed);
        seq.microtimings[target_slot][step].store(data.microtimings[step], Ordering::Relaxed);
    }
}

pub fn clear_grid_seq_plocks(seq: &crate::plock::SequencerPlockState, slot: usize) {
    if slot >= crate::track::MAX_TRACKS {
        return;
    }
    seq.masks[slot].store(0, Ordering::Relaxed);
    seq.solo_masks[slot].store(0, Ordering::Relaxed);
    for step in 0..crate::plock::STEP_COUNT {
        seq.probabilities[slot][step].store(f32::to_bits(1.0), Ordering::Relaxed);
        seq.stutters[slot][step].store(f32::to_bits(1.0), Ordering::Relaxed);
        seq.conditions[slot][step].store(0, Ordering::Relaxed);
        seq.microtimings[slot][step].store(0, Ordering::Relaxed);
    }
}

pub fn set_lane_length_lock_for_slot(params: &DrumFlashParams, slot: usize, locked: bool) {
    if slot >= crate::track::MAX_TRACKS {
        return;
    }
    let old_mask = PersistentField::<u16>::map(&params.lane_length_locks, |mask| *mask);
    let bit = 1u16 << slot;
    let new_mask = if locked {
        old_mask | bit
    } else {
        old_mask & !bit
    };
    if new_mask != old_mask {
        PersistentField::<u16>::set(&params.lane_length_locks, new_mask);
    }
}

pub fn paste_lane_steps(pattern: &SharedPattern, target_slot: usize, steps: &[bool; STEP_COUNT]) {
    if target_slot >= crate::track::MAX_TRACKS {
        return;
    }
    let bit = 1u16 << target_slot;
    for (step, active) in steps.iter().copied().enumerate() {
        let mask = pattern.load_step_mask(step);
        let next = if active { mask | bit } else { mask & !bit };
        if next != mask {
            pattern.set_step_mask(step, next);
        }
    }
}

pub fn clear_grid_steps(pattern: &SharedPattern, target_slot: usize) {
    if target_slot >= crate::track::MAX_TRACKS {
        return;
    }
    let bit = 1u16 << target_slot;
    for step in 0..STEP_COUNT {
        let mask = pattern.load_step_mask(step);
        let next = mask & !bit;
        if next != mask {
            pattern.set_step_mask(step, next);
        }
    }
}

pub fn randomize_lane_steps(pattern: &SharedPattern, target_slot: usize, density: f32) {
    if target_slot >= crate::track::MAX_TRACKS {
        return;
    }
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let mut rng = seed;
    let bit = 1u16 << target_slot;
    for step in 0..STEP_COUNT {
        let mask = pattern.load_step_mask(step);
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
        let active = (rng % 1000) < (density * 1000.0) as u64;
        let next = if active { mask | bit } else { mask & !bit };
        if next != mask {
            pattern.set_step_mask(step, next);
        }
    }
}

pub mod serde_pos2 {
    use nih_plug_egui::egui::Pos2;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(pos: &Pos2, s: S) -> Result<S::Ok, S::Error> {
        [pos.x, pos.y].serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Pos2, D::Error> {
        let [x, y] = <[f32; 2]>::deserialize(d)?;
        Ok(Pos2::new(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [283] A lane linked to the one above shows and plays that lane's
    /// steps: its lane-menu actions act on those shared steps, not on the
    /// hidden ones it had before the link.
    #[test]
    fn lane_actions_on_a_linked_lane_act_on_the_steps_it_shows() {
        let params = DrumFlashParams::default();
        let mut layout = TrackLayoutState::modular_default_layout();
        layout.slots[1].linked_up = true;
        PersistentField::<TrackLayoutState>::set(&params.track_layout, layout);
        let pattern = params.pattern_state.shared();
        let plock = &params.plock_state.state;
        let settings = params.sound_settings.state.clone();
        let mut state = EditorUIState::default();
        let lane = |lane: usize| {
            (0..STEP_COUNT)
                .filter(|&s| pattern.is_active(s, lane))
                .count()
        };

        pattern.load_step_masks(&[0; STEP_COUNT]);
        // Hidden steps of lane 2 (index 1), shown steps of lane 1 (index 0).
        pattern.set_step_mask(3, 0b10);
        pattern.set_step_mask(0, 0b01);

        state.copy_lane(&params, 1, &settings, &pattern, plock);
        let copied = state.lane_clipboard.as_ref().unwrap().steps;
        assert!(copied[0] && !copied[3], "copies the shown steps");

        state.clear_lane(&params, 1, &pattern, plock);
        assert_eq!(lane(0), 0, "clears the shown steps");
        assert_eq!(lane(1), 1, "the hidden steps are left alone");

        state.randomize_density = 1.0;
        state.randomize_lane(&params, 1, &pattern, plock);
        assert_eq!(lane(0), STEP_COUNT, "randomizes the shown steps");
        assert_eq!(lane(1), 1);

        state.paste_grid(&params, 1, &pattern);
        assert_eq!(lane(0), 1, "pastes onto the shown steps");
        assert!(pattern.is_active(0, 0));
    }

    const PANEL_SLOT: usize = 3;
    const LANE_LEN: usize = 16;

    fn cell(slot: usize, step: usize) -> Option<SelectedCell> {
        Some(SelectedCell { slot, step })
    }

    /// No fusion anywhere.
    fn no_fusion(_step: usize) -> Option<(usize, usize, u8)> {
        None
    }

    /// Cells 4..=7 form fusion 0, emitting `pulses` pulses.
    fn fusion_4_to_7(pulses: u8) -> impl Fn(usize) -> Option<(usize, usize, u8)> {
        move |step| {
            if (4..=7).contains(&step) {
                Some((4, 0, pulses))
            } else {
                None
            }
        }
    }

    #[test]
    fn no_selection_edits_the_lane_global() {
        let r = resolve_edit_scope(
            None,
            PANEL_SLOT,
            true,
            LANE_LEN,
            no_fusion,
            true,
            FusionTab::Step,
        );
        assert_eq!(r.scope, EditScope::LaneGlobal);
        assert_eq!(r.highlight_step, None);
        assert!(!r.drop_selection);
    }

    #[test]
    fn a_plain_cell_edits_its_plock() {
        let r = resolve_edit_scope(
            cell(PANEL_SLOT, 5),
            PANEL_SLOT,
            true,
            LANE_LEN,
            no_fusion,
            true,
            FusionTab::Step,
        );
        assert_eq!(r.scope, EditScope::StepPlock { step: 5 });
        assert_eq!(r.highlight_step, Some(5));
        assert!(!r.drop_selection);
    }

    /// Showing another lane must not edit that lane's step, but the selection is
    /// kept so coming back restores it.
    #[test]
    fn a_selection_on_another_lane_is_kept_but_inactive() {
        let r = resolve_edit_scope(
            cell(7, 5),
            PANEL_SLOT,
            true,
            LANE_LEN,
            no_fusion,
            true,
            FusionTab::Step,
        );
        assert_eq!(r.scope, EditScope::LaneGlobal);
        assert!(
            !r.drop_selection,
            "switching lanes must not lose the selection"
        );
    }

    /// Song mode / grid Follow: p-lock editing is disabled, but leaving the mode
    /// must bring the selection back — so it is kept, not dropped.
    #[test]
    fn disabled_editing_falls_back_to_global_without_losing_the_selection() {
        let r = resolve_edit_scope(
            cell(PANEL_SLOT, 5),
            PANEL_SLOT,
            true,
            LANE_LEN,
            no_fusion,
            false,
            FusionTab::Step,
        );
        assert_eq!(r.scope, EditScope::LaneGlobal);
        assert_eq!(r.highlight_step, None);
        assert!(!r.drop_selection);
    }

    #[test]
    fn an_inactive_lane_or_an_out_of_range_step_drops_the_selection() {
        let inactive = resolve_edit_scope(
            cell(PANEL_SLOT, 5),
            PANEL_SLOT,
            false,
            LANE_LEN,
            no_fusion,
            true,
            FusionTab::Step,
        );
        assert!(inactive.drop_selection);
        assert_eq!(inactive.scope, EditScope::LaneGlobal);

        let past_len = resolve_edit_scope(
            cell(PANEL_SLOT, 20),
            PANEL_SLOT,
            true,
            LANE_LEN,
            no_fusion,
            true,
            FusionTab::Step,
        );
        assert!(
            past_len.drop_selection,
            "a step past the lane length is unreachable"
        );

        let past_grid = resolve_edit_scope(
            cell(PANEL_SLOT, 99),
            PANEL_SLOT,
            true,
            64,
            no_fusion,
            true,
            FusionTab::Step,
        );
        assert!(past_grid.drop_selection);
    }

    /// A multi-pulse fused group offers both endpoints, always anchored on the
    /// start cell — which is the only cell that carries the group's sound.
    #[test]
    fn a_multi_pulse_fusion_offers_both_morph_endpoints() {
        let group = fusion_4_to_7(4);
        for step in 4..=7 {
            let start = resolve_edit_scope(
                cell(PANEL_SLOT, step),
                PANEL_SLOT,
                true,
                LANE_LEN,
                &group,
                true,
                FusionTab::Start,
            );
            assert_eq!(
                start.scope,
                EditScope::Morph {
                    step: 4,
                    fusion_index: 0,
                    end: crate::ui::param_source::MorphEnd::Start
                },
                "step {step}"
            );
            assert!(start.morph_available);

            let end = resolve_edit_scope(
                cell(PANEL_SLOT, step),
                PANEL_SLOT,
                true,
                LANE_LEN,
                &group,
                true,
                FusionTab::End,
            );
            assert_eq!(
                end.scope,
                EditScope::Morph {
                    step: 4,
                    fusion_index: 0,
                    end: crate::ui::param_source::MorphEnd::End
                }
            );
        }
    }

    /// [184] ph. 4 — the `Step` tab gives a fused cell its own p-lock back, which
    /// is what the context menu used to be the only route to.
    #[test]
    fn the_step_tab_edits_a_fused_cells_own_plock() {
        let group = fusion_4_to_7(4);
        let r = resolve_edit_scope(
            cell(PANEL_SLOT, 6),
            PANEL_SLOT,
            true,
            LANE_LEN,
            &group,
            true,
            FusionTab::Step,
        );
        assert_eq!(
            r.scope,
            EditScope::StepPlock { step: 4 },
            "Step edits the start cell's p-lock, not the morph"
        );
        assert!(
            r.morph_available,
            "the two morph tabs stay offered - Step is a choice, not a fallback"
        );
    }

    /// Asking for the End endpoint on a cell that is not fused must fall back to
    /// the plain p-lock rather than pretend a morph exists.
    #[test]
    fn the_end_endpoint_is_ignored_without_a_fusion() {
        let r = resolve_edit_scope(
            cell(PANEL_SLOT, 5),
            PANEL_SLOT,
            true,
            LANE_LEN,
            no_fusion,
            true,
            FusionTab::End,
        );
        assert_eq!(r.scope, EditScope::StepPlock { step: 5 });
        assert!(!r.morph_available);
    }

    /// A fused group carries sound only on its start cell, so any covered cell
    /// resolves to that start — the raw click is what gets stored, so deleting the
    /// fusion later restores the exact cell.
    #[test]
    fn a_covered_cell_resolves_to_the_fusion_start() {
        // A single-pulse group cannot morph, so Start resolves to a plain p-lock
        // on the start cell.
        let single = fusion_4_to_7(1);
        for step in 4..=7 {
            let r = resolve_edit_scope(
                cell(PANEL_SLOT, step),
                PANEL_SLOT,
                true,
                LANE_LEN,
                &single,
                true,
                FusionTab::Step,
            );
            assert_eq!(r.scope, EditScope::StepPlock { step: 4 }, "step {step}");
            assert_eq!(r.highlight_step, Some(4));
            assert!(!r.morph_available, "one pulse cannot morph");
        }
        // Outside the group, nothing is normalised.
        let r = resolve_edit_scope(
            cell(PANEL_SLOT, 8),
            PANEL_SLOT,
            true,
            LANE_LEN,
            &single,
            true,
            FusionTab::Step,
        );
        assert_eq!(r.scope, EditScope::StepPlock { step: 8 });
    }
}
