//! [283] Help mode: the header "Help" keycap turns on explanatory tooltips
//! over the features. The texts live here, in one place; a call site wraps a
//! block of widgets with [`zone`]. Off, a zone costs one flag read.

use nih_plug_egui::egui;

mod params;
pub use params::{special as special_param, standard as std_param};

fn flag_id() -> egui::Id {
    egui::Id::new("flash_drum_help_mode")
}

/// Whether help mode is on for this editor (set once per frame from the
/// persisted `GlobalConfig::help_mode`, and by the header keycap).
pub fn is_on(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(flag_id()).unwrap_or(false))
}

pub fn set(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(flag_id(), on));
}

/// Lays out `add` as usual; in help mode, hovering the block shows `text`.
/// The hover zone is registered after the contents with a hover-only sense:
/// it sits on top, so egui marks it hovered, but it never takes a click or a
/// drag from the widgets underneath.
pub fn zone<R>(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    text: &str,
    add: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let inner = ui.scope(add);
    rect(ui, inner.response.rect, id_salt, text);
    inner.inner
}

/// Help for an area already drawn (painter-drawn widgets, a whole lane...):
/// call it AFTER the widgets of `area`. Zones nest: egui hovers every
/// hover-only zone on top of the pointer, so the first one registered this
/// frame (the innermost, since a block's zone comes after its contents) claims
/// the tooltip and the enclosing ones stay silent.
pub fn rect(ui: &mut egui::Ui, area: egui::Rect, id_salt: impl std::hash::Hash, text: &str) {
    if !is_on(ui.ctx()) {
        return;
    }
    if text.is_empty() {
        return;
    }
    let response = ui.interact(
        area,
        ui.id().with(("help_zone", id_salt)),
        egui::Sense::hover(),
    );
    if response.hovered() && claim(ui.ctx()) {
        if response.enabled() {
            // Next to the pointer, never over it: on a large area a tooltip
            // anchored below/above the rect could cover what is being clicked.
            response.on_hover_text_at_pointer(text);
        } else {
            // A greyed row (inside `add_enabled_ui(false)`) still explains
            // itself; egui has no at-pointer variant for disabled widgets.
            response.on_disabled_hover_text(text);
        }
    }
}

/// Help on a widget's own response, chained like `on_hover_text`.
pub trait HelpExt {
    /// In help mode, hovering the widget shows `text` (disabled widgets too).
    /// Claims like a zone, so an enclosing zone stays silent over it.
    fn help(self, text: &str) -> Self;
    /// For a widget that already has a short tooltip: `short` normally, the
    /// full `text` in help mode (one tooltip, never both).
    fn help_or(self, text: &str, short: impl Into<egui::WidgetText>) -> Self;
}

impl HelpExt for egui::Response {
    fn help(self, text: &str) -> Self {
        if !text.is_empty() && is_on(&self.ctx) && self.hovered() && claim(&self.ctx) {
            if self.enabled() {
                return self.on_hover_text(text);
            }
            return self.on_disabled_hover_text(text);
        }
        self
    }

    fn help_or(self, text: &str, short: impl Into<egui::WidgetText>) -> Self {
        if is_on(&self.ctx) {
            self.help(text)
        } else {
            self.on_hover_text(short)
        }
    }
}

/// True for the first zone that asks during this frame.
fn claim(ctx: &egui::Context) -> bool {
    let frame = ctx.cumulative_pass_nr();
    let id = egui::Id::new("flash_drum_help_claimed");
    ctx.data_mut(|d| {
        if d.get_temp::<u64>(id) == Some(frame) {
            false
        } else {
            d.insert_temp(id, frame);
            true
        }
    })
}

/// Tooltip of the header keycap itself, shown whether help mode is on or off.
pub const HELP_KEYCAP: &str =
    "Help mode: hover a control to see what it does. Click again to turn it off.";

// --- Generator (bottom panel) -------------------------------------------

pub const GEN_TYPE: &str = "Generator type: how GENERATE builds the pattern. All four follow style A: its key hits (the kick on every beat in House, the backbeat snare...) are always there, and Density sets how many hits come on top.\n\
- Probabilistic: every other place the style allows gets a hit or not by chance, with the style's odds. Natural, varied grooves; the 4 bars differ. The only type that blends style A with style B (Mix A/B).\n\
- Markov: a hit tends to bring the next one, a rest the next rest, so hits come in short runs separated by gaps. A rolling, flowing feel; the 4 bars differ.\n\
- Euclidean: each instrument's hits are spread as evenly as possible over the 64 steps, with a few random changes. Steady, mechanical, hypnotic (techno, minimal).\n\
- Classic: starts from a famous groove picked at random (Amen break, Funky Drummer, bossa nova, motorik, shuffle), whatever the style, then Variation changes some of its steps (0 = the groove as is). Kick, snare and hi-hat, plus the style's key hits.";

pub const GEN_STYLE_A: &str =
    "Style A: the main style (Rock, Funk, Techno...). Every generator type follows it.";

pub const GEN_MIX: &str = "Mix A/B: blends style A with style B. Probabilistic type only: greyed with the other types, which follow style A alone.\n\
Far left: style A only. Far right: style B only. In between: the odds of both styles are mixed.";

pub const GEN_STYLE_B: &str =
    "Style B: the second style, blended in with the Mix A/B slider. Probabilistic type only: greyed with the other types, which follow style A alone.";

/// Short tooltip of B and Mix A/B when greyed, outside help mode.
pub const GEN_PROBABILISTIC_ONLY: &str = "Used by the Probabilistic type only (Type, on the left).";

pub const GEN_DENSITY: &str = "Density: how many hits, with every type.\n\
- Fully left: sparse, the style's key hits and a few more (with Classic, mostly the groove's hits on the beats).\n\
- Default (70 %): a usual groove.\n\
- Fully right: very busy.";

pub const GEN_VARIATION: &str = "Variation:\n\
- Classic type: how far the pattern moves away from the original.\n\
- Every type: how differently several lanes of the same instrument (two kicks, two snares...) play.";

pub const GEN_GENERATE: &str = "Writes a new pattern over the whole grid with the settings above. Each click gives a different result.\n\
Replaces the current steps, fused cells and parameter locks. The pattern bank is not touched. If the pattern is not saved: first click arms it (Sure?), second click generates.";

// --- Grid: page bar ---------------------------------------------------------

pub const PAGE_KEYCAP: &str = "Pages: the pattern has up to 64 steps, shown 16 at a time.\n\
- Click: show this page.\n\
- Double-click: loop this page alone (the playhead stays in it, keeping its phase). Double-click again to release it.\n\
- Right-click: loop / copy / paste / clear this page.\n\
Red light = the page being played. Amber ring = this page loops (grey in Song mode, where the loop is ignored).";

pub const PAGE_FOLLOW: &str =
    "Follow ON: the grid turns the pages with the playhead.\nFollow OFF: the grid stays on the page you chose.";

pub const PATTERN_LEN: &str = "Pattern length, 1 to 64 steps. Drag the slider or use 16 / 32 / 48 / 64. Double-click the slider to reset to 16.\n\
Lanes whose own length (Len column) is not locked follow it.";

pub const PATTERN_X2: &str = "Doubles the pattern: copies steps, fusions and p-locks of every lane after the current end, then doubles the length.\n\
Only up to 32 steps (64 max).";

// --- Grid: column headings ---------------------------------------------------

pub const GRID_SHIFT: &str = "Shifts the whole grid one step left (‹) or right (›): steps, fusions and p-locks of every lane, wrapping inside the pattern length.\n\
If a fusion sits on the edge, you are asked before it is broken. Saved patterns are not touched.";

pub const GRID_STEP_NUMBERS: &str = "Step numbers of the page shown. Blue = the step being played. Brighter numbers = the beats (1, 5, 9, 13).";

// --- Grid: lane row ----------------------------------------------------------

pub const LANE_GRIP: &str = "Drag to move this lane up or down.\n\
It moves with everything it owns: steps, fusions, sound, p-locks, M/S, Humanize/Push/Len and its place in every saved pattern. Linked lanes move together.";

pub const LANE_GRIP_FOLLOWER: &str = "A linked lane follows the one above it, so it cannot be moved on its own. Move that lane instead: it takes its linked lanes with it.";

pub const LANE_LINK_ARROW: &str = "Linked lane: it plays and edits the steps and fusions of the lane above (layering two sounds on one rhythm).\n\
Its sound, p-locks, morphing, M/S, routing and length stay its own; a small dot shows a p-lock of the other linked lane on the same step. Right-click the lane name > Unlink steps to separate them.";

pub const LANE_NAME: &str = "Lane: the instrument, or the name you gave it. It flashes when the lane receives a MIDI note.\n\
- Click: select the lane and show its sound in the editor on the right.\n\
- Double-click: rename it (Track tab).\n\
- Right-click: lane menu (instrument, copy / paste, link, clear, delete, randomize).";

pub const LANE_VOL: &str = "Lane volume, 0 to 2x (1x by default). Drag; hold Shift or Alt for fine moves; double-click to reset.";

pub const LANE_MUTE: &str = "M = Mute: silences this lane. Automatable from the DAW.";

pub const LANE_SOLO: &str = "S = Solo: hear only the soloed lanes. As soon as one lane is soloed, all the others are muted. Automatable from the DAW.";

pub const LANE_STEPS: &str = "Steps of this lane.\n\
- Click: turn a step on or off.\n\
- Ctrl+click (Cmd+click on Mac): edit this step's sound in the Lane Editor, without turning it on or off.\n\
- Shift+click two cells of the same page: fuse them into one long block that plays a number of evenly spaced hits (tuplets). Escape cancels.\n\
- Hold a step half a second, then drag: move it, with its p-locks, within the page.\n\
- Right-click: this step's p-lock menu (sound or sequencer locks, fusion settings).\n\
Colours: blue = on, green = sound p-lock, violet = sequencer p-lock, small dot in a corner = the other kind of p-lock, or a p-lock of a linked lane on that step. Dark dashed cells are beyond this lane's length.";

pub const TRACK_OWN_SWING: &str = "On: this lane plays with its own Swing and Groove (below) instead of the header's. Off: it follows the header. Kept with the lane: copies, presets and the session.";
pub const TRACK_SWING: &str = "This lane's swing, -50% to +50%: delays (or, below 0, advances) every second 16th note of this lane only. Needs Own Swing; no effect with Straight. Double-click to reset.";
pub const TRACK_GROOVE: &str = "How this lane's Swing is applied (same choices as the header's groove). Needs Own Swing.\n\
- Straight: no swing.\n\
- Swing 16th: classic swing; +50% gives a triplet feel.\n\
- Shuffle: a stronger swing.\n\
- MPC Style: swing that bites harder in the middle of its range.";
pub const LANE_HUM: &str = "Humanize: random velocity on every hit. 0% = all hits equal, 100% = from soft to full. The timing is never changed.";

pub const LANE_PUSH: &str = "Push = Push/Pull: moves the whole lane in time, -50 to +50 ms. Positive = later (laid back), negative = earlier (rushing).";

pub const LANE_LEN: &str = "Len = this lane's own length (polymeter). Unlocked, it follows the pattern length.\n\
Drag or type a value to lock it: the lane then loops on its own length, the other lanes keep theirs. Right-click > Follow pattern length to unlock.";

pub const EMPTY_FX_SLOT: &str = "Effect bus (Fx1 = lane 13, Fx2 = lane 14). Click to add an effect here (Reverb, Delay or Rumble); the other lanes then send to it from their Effects tab. Right-click: Paste Lane.";

pub const FX_LOOP_START: &str = "Loop restart ON (green arrow): every time the pattern loops, this effect goes back to the lane's own values (its sliders). Click to switch it off. To lock this step, right-click it.";

pub const FX_LOOP_HOLD: &str = "Loop restart OFF (grey arrow): nothing resets when the pattern loops, the last lock carries on into the first of the next loop. Click to switch the restart back on.";

pub const LANE_FX_MUTE: &str = "M = Mute: bypasses this effect. Nothing enters it and no tail comes out. Automatable from the DAW.";

pub const LANE_FX_STEPS: &str = "Cells of this effect lane. Click a cell to select it, then change the effect's parameters in the Lane Editor: each one you change becomes a lock on that cell. A lock holds until the next one, or until the pattern loops, then the sliders are back. With Glide on, the value moves smoothly from the sliders to the first lock and between locks. Right-click: copy, paste or clear the cell's locks.";

pub const EMPTY_SLOT: &str = "Empty slot. Click to choose an instrument and create a lane here. Right-click: Paste Lane.\n\
You can also drop a WAV file from your computer on the grid: it creates a One-Shot lane playing that file. Dropped on a One-Shot lane, it replaces that lane's file.";

// --- Grid: lane right-click menu ---------------------------------------------

pub const MENU_INSTRUMENT: &str = "Changes this lane's instrument. Its sound goes back to the new instrument's default settings; steps and p-locks are kept.";

pub const MENU_COPY_LANE: &str = "Copies the whole lane: instrument, sound, steps, fusions, p-locks, Humanize, Push, Len and its WAV file.";

pub const MENU_PASTE_LANE: &str = "Replaces this lane with the copied one: instrument, sound, steps, fusions, p-locks, Humanize, Push and Len. Its routing and M/S stay.";

pub const MENU_PASTE_GRID: &str =
    "Pastes only the copied lane's steps (on/off). No fusions, no p-locks, no sound.";

pub const MENU_LINK: &str = "Links this lane to the lane above: it plays the same steps and fusions with its own sound (layering). Editing the steps on either lane edits both.";

pub const MENU_UNLINK: &str = "Stops following the lane above: this lane plays its own steps again (the ones it had before the link).";

pub const MENU_CLEAR_LANE: &str = "Clears this lane's steps, fusions and p-locks. The instrument, sound, routing and lane controls stay. Click twice to confirm.";

pub const MENU_DELETE_LANE: &str = "Removes this lane: the slot becomes empty (+N) and can get a new instrument later. Click twice to confirm.";

pub const MENU_DENSITY: &str =
    "How full Randomize Lane makes this lane: left = a few steps, right = almost every step. Each lane keeps its own; a new lane starts with its instrument's default (Settings > Randomize Lane). Double-click: back to that default.";

pub const MENU_RANDOMIZE: &str = "Fills this lane with random steps, at the Density above. The lane's fusions and p-locks are cleared.";

// --- Grid: strip under the grid ------------------------------------------------

pub const PLOCK_MODE: &str = "P-Lock Mode: which kind of per-step lock the grid shows and edits.\n\
- Sound: per-step sound changes (pitch, decay, filter...), shown in green.\n\
- Sequencer: per-step playing rules (probability, stutter, nudge, conditions, step solo), shown in violet.\n\
Right-click a step to edit its lock.";

pub const FUSION_BOX: &str = "Fusion: Shift+click two cells of a lane (same page) to fuse them into one block.\n\
A fused block plays a number of evenly spaced hits over its whole length: 3 cells with 4 hits make a tuplet.\n\
To change the number of hits, right-click the block > Edit Fusion Steps; the editor then appears here (Del = delete the fusion, x = close).";

// --- Step menu (right-click a step) ----------------------------------------------

pub const STEP_MENU_TYPE: &str = "Which lock to edit on this step:\n\
- Sound: change the sound of this step only (pitch, decay, filter...). The editor on the right then edits this step instead of the whole lane.\n\
- Sequencer: how this step plays (probability, stutter, nudge, conditions, solo).";

pub const STEP_MENU_DONE: &str =
    "Close this menu. Changes are already applied: nothing is cancelled.";

pub const PLOCK_CREATE: &str =
    "Creates a sound p-lock on this step and opens it in the editor on the right.\n\
Only the settings you then change are locked on this step; all the others keep following the lane.";

pub const PLOCK_PASTE: &str =
    "Pastes the copied p-lock on this step (only offered for a p-lock copied from this same lane).";

pub const PLOCK_COPY: &str = "Copies this step's p-lock. Paste it on another step of the same lane (right-click it > Paste Plock).";

pub const PLOCK_CLEAR: &str = "Removes this step's sound p-lock: the step plays the lane's sound again. The step and its sequencer lock stay.";

pub const FUSION_MORPH: &str = "Morphing: up to 4 sound settings glide from the start to the end of the block, hit after hit.\n\
Opens the End tab in the editor on the right, where you set the values to reach. Needs at least 2 hits in the block.\n\
On linked lanes the block is shared but each lane has its own morphing (none until you create one).";

pub const FUSION_EDIT_STEPS: &str = "Sets how many evenly spaced hits the block plays (1 to 64). The field appears in the Fusion box under the grid; press Enter to apply. On linked lanes, it changes the block on all of them.";

pub const FUSION_DELETE: &str = "Removes the fusion: only its first cell stays, as a normal step. On linked lanes, it removes the block from all of them.";

pub const SEQ_MODE: &str = "Active = this step has a sequencer lock; Inactive = it plays normally. Changing any setting below creates the lock.";

pub const SEQ_SOLO: &str = "Step solo: while this step plays (or its whole fused block), every lane that has no step solo here is muted. Separate from the lane's S button.";

pub const SEQ_PROBABILITY: &str =
    "Chance that this step plays, drawn again on every pass. 100% = always. Double-click to reset.";

pub const SEQ_STUTTER: &str = "Plays this step N times, evenly spaced inside the step (1x = normal). Not available on a fused block. Double-click to reset.";

pub const SEQ_NUDGE: &str = "Moves this step earlier (-) or later (+), up to 100 ms, with its stutter or fusion hits. Double-click to reset.";

pub const SEQ_CONDITION: &str = "Condition: on which passes of the pattern the step plays.\n\
- Always: every pass.\n\
- 1st loop only: only the first pass, counted from the start of the DAW timeline.\n\
- a/b: on pass a of every b passes. 1/2 = every other pass, 4/4 = the 4th of every 4.\n\
A pass is one loop of the pattern, or of the looped page.";

pub const SEQ_NOT: &str = "Not: inverts the condition. The step plays on every pass EXCEPT the ones it selects (not available on Always).";

pub const SEQ_AND: &str = "And: adds a second condition. The step plays only on the passes where both hold: 1/2 And 1/3 = once every 6 passes.";

pub const SEQ_CREATE: &str =
    "Creates a neutral sequencer lock on this step (100%, 1x, 0 ms, Always), ready to edit.";

pub const SEQ_CLEAR: &str = "Removes this step's sequencer lock: probability, stutter, nudge, conditions and solo go back to normal.";

// --- Page menu (right-click a page key) --------------------------------------------

pub const PAGE_MENU_LOOP: &str = "Loops this page alone: the playhead stays in it, keeping its phase. Same as double-clicking the page key. Not available in Song mode.";

pub const PAGE_MENU_COPY: &str =
    "Copies this page (its 16 steps on every lane, with p-locks and the fusions inside it).";

pub const PAGE_MENU_PASTE: &str = "Pastes the copied page here, replacing it (asks first). The pattern gets longer if needed so the page plays.";

pub const PAGE_MENU_CLEAR: &str = "Clears this page on every lane: steps, p-locks and fusions (asks first). The pattern length stays.";

// --- Header -----------------------------------------------------------------------------

pub const HDR_VERSION: &str =
    "Flash Drum version and build number. Give them when you report a problem.";

pub const HDR_MASTER: &str = "Master volume of the plugin, on the main mix and every aux output (0 dB by default). Click or drag; double-click to reset. Automatable.";

pub const HDR_SWING: &str = "Swing amount, -50% to +50%: delays (or, below 0, advances) every second 16th note. Works with the groove chosen on its right; no effect with Straight. Double-click to reset. Automatable.";

pub const HDR_GROOVE: &str = "Groove: how Swing bends the timing.\n\
- Straight: no swing.\n\
- Swing 16th: classic swing; +50% gives a triplet feel.\n\
- Shuffle: a stronger swing.\n\
- MPC Style: swing that bites harder in the middle of its range.";

pub const HDR_SEQ: &str = "Who plays the lanes.\n\
- Internal: the grid plays, following the DAW transport.\n\
- Ext MIDI: the grid is silent; each MIDI note received plays the lanes set to that note (Track tab > MIDI Note). Use it to play Flash Drum from a keyboard or a MIDI clip.";

pub const HDR_MIDI_PAT: &str = "MIDI Pat: incoming MIDI notes 60 to 75 switch between the saved patterns P1 to P16, at once (note 60 = C3 in Studio One, C4 in most other DAWs). Those notes make no sound.\n\
Internal mode only; ignored in Song mode. Before switching, unsaved grid changes are saved into the slot they came from.";

pub const HDR_PRESETS: &str =
    "Opens the preset browser: save and load instruments, patterns, lane layouts (Grid) and songs.";

pub const HDR_CLEAR_ALL: &str = "Clears the whole program: grid, the 16 saved patterns, the song AND all the lanes. Cannot be undone.\n\
First click arms it (Sure?), second click clears.";

pub const HDR_SETTINGS: &str =
    "Opens the settings: outputs, default Analog, MIDI channel and notes, macros, Auto-Edit, skin.";

// --- Settings popup ------------------------------------------------------------------

pub const SET_CLOSE: &str = "Close the settings (clicking outside also closes them).";

pub const SET_OUTPUTS: &str = "Routes every lane to the aux output of its own number, once: lane 1 to Out 1, lane 2 to Out 2... Each routed lane leaves the main mix; switch Main Mix back on in its Track tab if you want both.\n\
Each lane's routing can be changed afterwards in its Track tab.";

pub const SET_DEFAULT_ANALOG: &str = "Analog value given to new lanes (and to lanes reset by a preset or an instrument change). Existing lanes are not changed. Saved for all your projects.";

pub const SET_MIDI_CHANNEL: &str = "MIDI channel of the notes Flash Drum sends (the lanes' notes, and the notes forwarded in Ext MIDI mode). Incoming MIDI is accepted on any channel.\n\
The MIDI file export always uses channel 10.";

pub const SET_ROOT_NOTE: &str = "First note used by Auto-assign below: lane 1 gets this note, the next lanes the following notes. 36 = C1 (the usual kick note).";

pub const SET_AUTO_NOTES: &str = "Gives every lane its MIDI note, once, in lane order: lane 1 gets the root note, lane 2 root+1, and so on. Each lane's note can be changed afterwards in its Track tab.";

pub const SET_MACROS: &str = "Opens the macros: 16 knobs your DAW can automate or MIDI-learn, each driving one sound setting of one lane.";

pub const SET_AUTO_EDIT: &str = "Auto-Edit on: clicking a step or a lane's M / S also selects that lane in the editor on the right.\n\
Off: only clicking the lane name selects it.";

pub const SET_SKIN: &str = "Colour theme of the plugin. Saved for all your projects.";

pub const SET_RANDOMIZE_KIND: &str = "The instrument whose Randomize Lane density you set below.";

pub const SET_RANDOMIZE_DENSITY: &str = "The Randomize Lane density a new lane of this instrument starts with: left = a few steps, right = almost every step. Existing lanes keep theirs (right-click a lane name to change it). Saved for all your projects.";

pub const SET_ZOOM: &str = "Size of the whole plugin window: text and controls grow with it, and Settings closes. Saved for all your projects.\n\
If the window keeps its size, your host refused the new size.";

// --- Pattern bank ------------------------------------------------------------------

pub const BANK_SAVE: &str = "Save: click it, then click a slot (P1 to P16) to store the current pattern there. An occupied slot is overwritten.\n\
A pattern holds the steps, fusions, p-locks and pattern length; not the lanes or their sounds.";

pub const BANK_SLOT: &str = "Saved patterns P1 to P16 (raised = holds a pattern, flat = empty, blue = the one being edited, * = unsaved changes).\n\
- Click: load it (an empty slot gives an empty grid, ready to save there). You are warned first if your changes are not saved.\n\
- Right-click a full slot: copy it. Right-click an empty slot: paste the copy there.\n\
Patterns are saved with your DAW project. In Song mode, grid edits are saved into the slot being played.";

pub const BANK_CLEAR: &str = "Clear: then choose Grid (clear the grid only, the saved pattern stays) or Slot (clear the grid AND empty its slot), or X to cancel.";

pub const BANK_CLEAR_GRID: &str = "Clears the steps, fusions and p-locks of the grid. The saved pattern in the slot is kept, so you can reload it.";

pub const BANK_CLEAR_SLOT: &str =
    "Clears the grid AND empties the slot it was loaded from. Cannot be undone.";

pub const BANK_CLEAR_CANCEL: &str = "Cancel: nothing is cleared.";

pub const BANK_DRAG: &str = "Opens a small MIDI DRAG window next to the mouse: press it and drag into a DAW track to drop the current pattern as a MIDI clip (Windows only).\n\
Same file as Export, also saved in Documents\\Flash Drum\\exports.";

pub const BANK_EXPORT: &str = "Writes the current pattern as a MIDI file in Documents\\Flash Drum\\exports.\n\
Channel 10, one note per lane (its MIDI note), the pattern length, with swing, nudges, stutters and fusion hits. Probability, conditions and mutes are not applied: every step is written.";

pub const LOAD_WARN_SAVE: &str =
    "Saves the current grid into the slot it came from (overwriting it), then switches.";

pub const LOAD_WARN_DISCARD: &str = "Switches without saving: the changes on the grid are lost.";

pub const LOAD_WARN_CANCEL: &str = "Stays on the current pattern, nothing changes.";

pub const EXPORT_OPEN_FOLDER: &str = "Opens the folder where the MIDI file was written.";

// --- Bottom panel: tabs and presets chips ----------------------------------------------

pub const BOTTOM_TABS: &str =
    "Generator: create patterns automatically.\nSong: chain the saved patterns into a song.\n\
This only changes the view: Song playback is turned on by the Song Mode box in the Song tab.";

pub const PRESET_RANDOM: &str = "Full random: fills the grid with random steps (about 1 in 3), ignoring the generator settings. Lanes and sounds stay; p-locks and fusions are cleared. If the pattern is not saved: first click arms it (Sure?), second click fills the grid.";

// --- Song tab ----------------------------------------------------------------------

pub const SONG_MODE: &str = "Song Mode: plays the chain of blocks below instead of the grid alone. Starts from block 1 when the DAW starts playing.\n\
The first empty block (--) sends playback back to block 1. Automatable.";

pub const SONG_CLEAR: &str = "Empties the 16 blocks of the song (asks first: click Confirm?). The saved patterns are not touched.";

pub const SONG_BLOCK: &str = "Song block: the top choice picks the saved pattern (P1 to P16) it plays, the bottom number how many times it repeats (drag, or click and type).\n\
Right-click the block's edge: copy, paste, duplicate into the next block, clear. Red light = the block being played.";

// --- Macros ------------------------------------------------------------------------

pub const MACRO_KNOB: &str = "Macro knob: drag to set it. Your DAW sees it as an automatable parameter (and Studio One's MIDI learn).\n\
It writes the assigned sound setting like a hand move; p-locks still win on their steps. When any macro moves, every assigned macro writes its value again.";

pub const MACRO_LANE: &str =
    "Lane this macro drives (- = none). Picking a lane starts on its first setting.";

pub const MACRO_PARAM: &str = "Sound setting of that lane the macro drives. The knob's 0-100% covers the setting's whole range.";

pub const MACRO_CLEAR: &str = "Removes this macro's assignment.";

// --- Presets browser ------------------------------------------------------------------

pub const PRESET_TAB_INSTRUMENT: &str = "Instruments: the sound of one lane (instrument and all its settings). Loading applies it to the selected lane, changing its instrument if needed.";

pub const PRESET_TAB_PATTERN: &str = "Patterns: steps, fusions, p-locks and length, with the lanes and their sounds. Loading replaces the grid (the saved patterns P1-P16 are not touched).";

pub const PRESET_TAB_GRID: &str = "Grid: which instrument sits on each lane. Loading replaces the lanes, with their default sounds; the steps stay where they are.";

pub const PRESET_TAB_SONG: &str = "Songs: the chain of the Song tab (which pattern slot each block plays, and its repeats). The patterns themselves are not included.";

pub const PRESET_SAVE: &str = "Saves the current state of this tab under the name typed on the left, in Documents\\Flash Drum\\presets. A preset with the same name is replaced.";

pub const PRESET_LOAD_LANES: &str =
    "On: loading a pattern also installs its lanes and their sounds.\n\
Off: your lanes stay; the saved sounds only go to lanes holding the same instrument.";

pub const PRESET_GRID_FACTORY: &str = "Built-in lane layouts. Clear All removes every lane and the grid (two clicks). The others replace the lanes with default sounds and keep the steps.";

pub const PRESET_LOAD: &str = "Loads this preset (see the tab's help for what it replaces).";

pub const PRESET_RENAME: &str =
    "Renames this preset: type the new name, Enter to confirm, Escape to cancel.";

pub const PRESET_DELETE: &str = "Deletes this preset file. Click twice (Sure?).";

// --- Lane Editor (right panel) -------------------------------------------------------

/// Added to the help of every slider row that has the page menu.
pub const SLIDER_GESTURES: &str = "\nDrag; Shift or Alt for fine moves; double-click for the default. Right-click: spread, scatter or clear this setting's p-locks over the 16 steps of the page shown.";

pub const EDITOR_FUSION_TAB: &str = "On a fused block with several hits:\n\
- Step: the block's sound p-lock.\n\
- Start / End: the values the morph glides between, from the first hit to the last.\n\
Greyed when the step is not such a block.";

pub const EDITOR_ALGORITHM: &str = "Algorithm: the synthesis method of this instrument; each gives a different character. Always applies to the whole lane, even while editing a step.";

pub const EDITOR_GUTTER: &str = "This setting is locked on this step (or is a morph target). Click the bar to remove the lock: the setting follows the lane again.";

pub const TRACK_NAME: &str = "Lane name shown in the grid (6 characters max).";

pub const TRACK_TYPE: &str = "Instrument of this lane. Changing it resets the name, the MIDI note and the sound to the new instrument's defaults; steps and p-locks stay.";

pub const TRACK_PRESET: &str =
    "Loads an instrument preset saved for this instrument (Presets > Instruments) onto this lane.";

pub const TRACK_MAIN_MIX: &str = "Main Mix: the lane plays in the plugin's main stereo output. Off: it is heard only through its Aux Out.";

pub const TRACK_AUX: &str = "Aux Out: sends the lane to one of the 14 extra stereo outputs, to mix it on its own track in the DAW (activate the plugin's outputs in the DAW). Choosing an output turns Main Mix off.\n\
Several lanes can share an output; the list shows how many already use each one.";

pub const TRACK_FX_SEND: &str = "Send: how much of this lane goes into the effect on lane 13 (Fx1) or lane 14 (Fx2). 0 = dry. Taken after the lane Volume, whatever the Main Mix switch. On a selected cell it is that step's lock: the hit thrown from that step goes to the effect by that amount. Greyed until that lane holds an effect.";

pub const LFO_SYNC: &str = "Sync: Free = the rate knob above, in Hz. A note value (1/16, 1/8., 1/4...) locks this LFO to the host tempo, one cycle per note value, and greys the rate knob. A lane setting: it is never locked per step.";

pub const TRACK_CHOKE: &str = "Choke group: when this lane plays, it cuts the other lanes of the same group (a closed hi-hat cutting the open one). None = never cut.";

pub const TRACK_CHANNEL: &str = "MIDI channel of the notes sent. It is shared by all lanes: change it in Settings > Global MIDI Channel.";

pub const TRACK_NOTE: &str = "MIDI note of this lane: the note sent when it plays, written in MIDI exports, and the note that plays it in Ext MIDI mode.";
pub const TRACK_NOTE_LEARN: &str = "Click, then play a note on your MIDI controller: it becomes this lane's note (that note does not play). Click again to stop waiting.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame with a button wrapped in a help zone, laid out in a row
    /// like the generator's controls. Returns (button clicked, tooltip shown).
    fn frame(ctx: &egui::Context, events: Vec<egui::Event>, help_on: bool) -> (bool, bool) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(800.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let mut clicked = false;
        let _ = ctx.run(input, |ctx| {
            set(ctx, help_on);
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.horizontal(|ui| {
                    clicked = zone(ui, "test", "help text", |ui| {
                        ui.add_sized(egui::Vec2::new(100.0, 30.0), egui::Button::new("B"))
                    })
                    .clicked();
                });
            });
        });
        let tooltip = ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .iter()
                .any(|l| l.order == egui::Order::Tooltip)
        });
        (clicked, tooltip)
    }

    /// In help mode the explanation shows on hover, and the hover zone (on
    /// top of the widgets) never eats their clicks, help mode on or off.
    #[test]
    fn a_help_zone_explains_on_hover_and_never_takes_the_click() {
        for help_on in [false, true] {
            let ctx = egui::Context::default();
            let pos = egui::pos2(58.0, 23.0); // inside the button (panel margin 8)
            frame(&ctx, vec![], help_on);
            let (_, tooltip) = frame(&ctx, vec![egui::Event::PointerMoved(pos)], help_on);
            assert_eq!(tooltip, help_on, "tooltip with help mode {help_on}");
            let press = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            frame(&ctx, vec![press(true)], help_on);
            let (clicked, _) = frame(&ctx, vec![press(false)], help_on);
            assert!(clicked, "click lost with help mode {help_on}");
        }
    }

    /// Nested zones (a lane inside the grid...): one tooltip, the inner one's.
    #[test]
    fn nested_help_zones_show_one_tooltip() {
        let ctx = egui::Context::default();
        let run = |events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::Vec2::new(800.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                set(ctx, true);
                egui::CentralPanel::default().show(ctx, |ui| {
                    zone(ui, "outer", "outer text", |ui| {
                        zone(ui, "inner", "inner text", |ui| {
                            ui.add_sized(egui::Vec2::new(100.0, 30.0), egui::Button::new("B"))
                        });
                        ui.label("more");
                    });
                });
            });
        };
        run(vec![]);
        run(vec![egui::Event::PointerMoved(egui::pos2(58.0, 23.0))]);
        let tooltips = ctx.memory(|m| {
            m.areas()
                .visible_layer_ids()
                .iter()
                .filter(|l| l.order == egui::Order::Tooltip)
                .count()
        });
        assert_eq!(tooltips, 1);
    }

    #[test]
    fn help_mode_flag_follows_set() {
        let ctx = egui::Context::default();
        assert!(!is_on(&ctx));
        set(&ctx, true);
        assert!(is_on(&ctx));
        set(&ctx, false);
        assert!(!is_on(&ctx));
    }
}
