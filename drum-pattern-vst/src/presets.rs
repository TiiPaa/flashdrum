//! User & factory presets: instrument sounds, full patterns, songs.
//!
//! User presets live as versioned JSON files under
//! `Documents/Flash Drum/presets/{instruments,patterns,songs}/`.
//! Factory presets are embedded in the binary (see `factory_presets.rs`) and
//! are read-only in the UI.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::pattern_bank::{PatternSlot, SongSequence};
use crate::sequencer::SharedPattern;
use crate::sound_settings::{InstrumentSettingsState, SoundSettingsState};
use crate::track::{TrackInstrumentKind, MAX_TRACKS};

/// Bumped when a preset JSON layout changes; older files stay loadable only
/// via explicit migration (none needed at v1).
pub const PRESET_FORMAT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Preset kinds
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PresetKind {
    Instrument,
    Pattern,
    Grid,
    Song,
}

impl PresetKind {
    pub const ALL: [Self; 4] = [Self::Instrument, Self::Pattern, Self::Grid, Self::Song];

    pub fn label(self) -> &'static str {
        match self {
            Self::Instrument => "Instruments",
            Self::Pattern => "Patterns",
            Self::Grid => "Grid",
            Self::Song => "Songs",
        }
    }

    fn subdir(self) -> &'static str {
        match self {
            Self::Instrument => "instruments",
            Self::Pattern => "patterns",
            Self::Grid => "grids",
            Self::Song => "songs",
        }
    }

    /// Files end with e.g. `.fdpat.json`.
    fn extension(self) -> &'static str {
        match self {
            Self::Instrument => "fdinst.json",
            Self::Pattern => "fdpat.json",
            Self::Grid => "fdgrid.json",
            Self::Song => "fdsong.json",
        }
    }
}

// ---------------------------------------------------------------------------
// Preset payloads
// ---------------------------------------------------------------------------

/// One instrument's sound (a slot's standards + specials + algo).
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct InstrumentPreset {
    pub version: u32,
    pub name: String,
    /// `TrackInstrumentKind` index — a preset only applies to a lane of the
    /// same kind (the UI offers to switch the lane's kind on load).
    pub kind: usize,
    /// 13 standard fields in registry order: freq, decay, volume, filter_freq,
    /// attack, release, decay_curve, release_curve, hold, filter_env_amount,
    /// filter_env_decay, analog, stereo.
    pub standards: [f32; 13],
    pub algo: u8,
    /// Special values in the registry's `special_params` order for the kind.
    pub specials: Vec<f32>,
    /// [228] Path of the user texture the lane's Texture menu points at, when
    /// it points at a user slot: the preset brings its file along. Absent
    /// from older presets (`default`), and for every other instrument.
    #[serde(default)]
    pub user_texture: Option<String>,
}

/// One active lane's sound (standards + specials + algo) captured with a
/// pattern, keyed by slot so it can be re-applied to the right lane on load.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct PatternSlotSound {
    /// Slot index (0..MAX_TRACKS) this sound belongs to.
    pub slot: u8,
    /// `TrackInstrumentKind` index at capture time — the sound is only
    /// re-applied to a slot that still holds this kind.
    pub kind: usize,
    /// 13 standard fields in registry order (see `InstrumentPreset`).
    pub standards: [f32; 13],
    pub algo: u8,
    /// Special values in the kind's `special_params` order.
    pub specials: Vec<f32>,
    /// [228] See `InstrumentPreset::user_texture`.
    #[serde(default)]
    pub user_texture: Option<String>,
}

/// A full pattern: grid + fusions + sound plocks + seq plocks, plus the lane
/// kit it was captured with (applied only if the user asks for it on load) and
/// each active lane's instrument sound.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PatternPreset {
    pub version: u32,
    pub name: String,
    /// Lane kinds at capture time: `TrackInstrumentKind` index, or -1 for an
    /// inactive slot.
    pub kit: [i8; MAX_TRACKS],
    #[serde(with = "serde_arrays")]
    pub step_masks: [u16; 64],
    /// Hex-encoded binary blobs (same layout as the pattern bank slots, so
    /// legacy-length tolerance applies on load).
    pub plock_hex: String,
    pub seq_plock_hex: String,
    pub fusion_hex: String,
    pub pattern_length: u8,
    /// Per-lane instrument sounds captured with the pattern. `#[serde(default)]`
    /// so presets saved before this field load as an empty list (grid-only,
    /// sounds left untouched on apply).
    #[serde(default)]
    pub sounds: Vec<PatternSlotSound>,
}

/// A song sequence (16 blocks + repeats).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SongPreset {
    pub version: u32,
    pub name: String,
    pub song: SongSequence,
}

/// A lane kit only (which instrument sits on which slot) — the "Grid" preset
/// type, generalising the old fixed lane presets (4/12 lanes).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GridPreset {
    pub version: u32,
    pub name: String,
    /// `TrackInstrumentKind` index per slot, or -1 for an inactive slot.
    pub kit: [i8; MAX_TRACKS],
}

// ---------------------------------------------------------------------------
// Capture helpers (live state -> preset)
// ---------------------------------------------------------------------------

/// [228] The custom texture file of a lane, when its kind has a Texture menu
/// (a `*_texture` special, Rift's) and a file is loaded on that lane.
pub fn user_texture_path(
    kind: TrackInstrumentKind,
    lane: usize,
    textures: &crate::user_textures::UserTextures,
) -> Option<String> {
    kind.instrument_def()
        .special_params
        .iter()
        .any(|d| d.name.ends_with("_texture"))
        .then(|| textures.path(lane))
        .flatten()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Capture a slot's sound. `algo` is the slot's algorithm index;
/// `user_texture` is what [`user_texture_path`] found for the lane.
pub fn capture_instrument(
    name: String,
    kind: TrackInstrumentKind,
    inst: &InstrumentSettingsState,
    algo: i32,
    user_texture: Option<String>,
) -> InstrumentPreset {
    use std::sync::atomic::Ordering;
    let g = |a: &std::sync::atomic::AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
    let standards = [
        g(&inst.frequency),
        g(&inst.decay),
        g(&inst.volume),
        g(&inst.filter_freq),
        g(&inst.attack),
        g(&inst.release),
        g(&inst.decay_curve),
        g(&inst.release_curve),
        g(&inst.hold),
        g(&inst.filter_env_amount),
        g(&inst.filter_env_decay),
        g(&inst.analog),
        g(&inst.stereo),
    ];
    let inst_def = kind.instrument_def();
    let specials = inst_def
        .special_params
        .iter()
        .map(|def| inst.special_value(def.special_index))
        .collect();
    InstrumentPreset {
        version: PRESET_FORMAT_VERSION,
        name,
        kind: kind.index(),
        standards,
        algo: algo.clamp(0, 255) as u8,
        specials,
        user_texture,
    }
}

/// Capture the current pattern (grid + fusions + plocks + seq plocks), the
/// current lane kit, and each active lane's instrument sound. `algos[i]` is the
/// algorithm index of slot `i`.
#[allow(clippy::too_many_arguments)]
pub fn capture_pattern(
    name: String,
    layout: &crate::track::TrackLayoutState,
    pattern: &SharedPattern,
    plock_state: &crate::plock::PlockState,
    seq_plock_state: &crate::plock::SequencerPlockState,
    pattern_length: u8,
    sound_settings: &SoundSettingsState,
    algos: &[i32],
    textures: &crate::user_textures::UserTextures,
) -> PatternPreset {
    let mut slot = PatternSlot::default();
    slot.capture(pattern, plock_state, seq_plock_state, pattern_length);
    let mut kit = [-1i8; MAX_TRACKS];
    let mut sounds = Vec::new();
    for (i, s) in layout.slots.iter().enumerate() {
        if s.active {
            kit[i] = s.kind.index() as i8;
            // Reuse the instrument-sound extraction so the two paths stay in sync.
            let algo = algos.get(i).copied().unwrap_or(0);
            let user_texture = user_texture_path(s.kind, i, textures);
            let ip = capture_instrument(
                String::new(),
                s.kind,
                &sound_settings.instruments[i],
                algo,
                user_texture,
            );
            sounds.push(PatternSlotSound {
                slot: i as u8,
                kind: ip.kind,
                standards: ip.standards,
                algo: ip.algo,
                specials: ip.specials,
                user_texture: ip.user_texture,
            });
        }
    }
    PatternPreset {
        version: PRESET_FORMAT_VERSION,
        name,
        kit,
        step_masks: slot.step_masks,
        plock_hex: hex_encode(&slot.plock_bytes),
        seq_plock_hex: hex_encode(&slot.seq_plock_bytes),
        fusion_hex: hex_encode(&slot.fusion_bytes),
        pattern_length,
        sounds,
    }
}

pub fn capture_song(name: String, song: SongSequence) -> SongPreset {
    SongPreset {
        version: PRESET_FORMAT_VERSION,
        name,
        song,
    }
}

/// Capture the current lane kit as a Grid preset.
pub fn capture_grid(name: String, layout: &crate::track::TrackLayoutState) -> GridPreset {
    let mut kit = [-1i8; MAX_TRACKS];
    for (i, s) in layout.slots.iter().enumerate() {
        if s.active {
            kit[i] = s.kind.index() as i8;
        }
    }
    GridPreset {
        version: PRESET_FORMAT_VERSION,
        name,
        kit,
    }
}

/// Build a layout from a kit array (`-1` = inactive slot).
pub fn layout_from_kit(kit: &[i8; MAX_TRACKS]) -> crate::track::TrackLayoutState {
    let mut layout = crate::track::TrackLayoutState::empty_layout();
    for (i, k) in kit.iter().enumerate() {
        if *k >= 0 {
            if let Some(kind) = TrackInstrumentKind::from_index(*k as usize) {
                // [286] Lanes 13 and 14 are reserved for effects: a kit never puts an
                // instrument there, nor an effect anywhere else.
                if crate::track::kind_allowed_on_slot(kind, i, None) {
                    layout.slots[i] = crate::track::TrackSlot::active_with_kind(kind);
                }
            }
        }
    }
    layout
}

// ---------------------------------------------------------------------------
// File storage
// ---------------------------------------------------------------------------

pub(crate) fn presets_root() -> PathBuf {
    crate::paths::flash_drum_dir().join("presets")
}

pub fn presets_dir(kind: PresetKind) -> PathBuf {
    presets_root().join(kind.subdir())
}

/// `Documents/Flash Drum/presets/_factory/<subdir>/` — the staging area the
/// debug "Export factory" button writes to; files are then committed under
/// `assets/presets/` to be embedded in the binary.
#[cfg(debug_assertions)]
pub fn factory_staging_dir(kind: PresetKind) -> PathBuf {
    presets_root().join("_factory").join(kind.subdir())
}

fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let s = s.trim_matches('_').to_string();
    let mut s = if s.is_empty() {
        "preset".to_string()
    } else {
        s
    };
    // [261] Windows device names are reserved whatever the extension:
    // "aux.fdpat.json" would fail to write on some Windows versions.
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&s)) {
        s.push('_');
    }
    s
}

/// [261] Write via a temp file + rename: a crash mid-write never leaves a
/// truncated file that the next load would silently replace with defaults.
fn write_atomic(path: &Path, contents: &str) -> Result<(), String> {
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, contents).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// Info about a preset file on disk (name from content, path for load/delete).
#[derive(Debug, Clone)]
pub struct PresetFileInfo {
    pub name: String,
    pub path: PathBuf,
    /// File mtime, display only — the JSON format is untouched ([236]).
    pub modified: Option<std::time::SystemTime>,
}

fn save_json(dir: PathBuf, name: &str, kind: PresetKind, json: &str) -> Result<PathBuf, String> {
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.{}", sanitize_name(name), kind.extension()));
    write_atomic(&path, json)?;
    Ok(path)
}

fn load_json(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

pub fn delete_file(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// Rename a user preset ([237]): the display name lives INSIDE the JSON and
/// the file name derives from it, so both are updated together. Returns the
/// new path (unchanged when the sanitized name already matches).
/// [246] The kind is passed by the caller: re-deriving it from
/// `path.extension()` only yields `json` and the renamed file would lose its
/// `.fdinst/.fdpat/...` double extension, becoming invisible to `list_dir`.
pub fn rename_preset(path: &Path, new_name: &str, kind: PresetKind) -> Result<PathBuf, String> {
    let content = load_json(path)?;
    let mut value: serde_json::Value = serde_json::from_str(&content).map_err(|e| e.to_string())?;
    value["name"] = serde_json::Value::String(new_name.to_string());
    let dir = path
        .parent()
        .ok_or("preset has no parent dir")?
        .to_path_buf();
    let new_path = dir.join(format!("{}.{}", sanitize_name(new_name), kind.extension()));
    let json = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    write_atomic(&new_path, &json)?;
    if new_path != path {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(new_path)
}

/// `YYYY-MM-DD` (UTC) from a file mtime — no date crate, civil-from-days.
pub fn format_date(t: std::time::SystemTime) -> String {
    let secs = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}

// -- Typed save/load/list ----------------------------------------------------

pub fn save_instrument(p: &InstrumentPreset) -> Result<PathBuf, String> {
    save_json(
        presets_dir(PresetKind::Instrument),
        &p.name,
        PresetKind::Instrument,
        &serde_json::to_string_pretty(p).map_err(|e| e.to_string())?,
    )
}

pub fn save_pattern(p: &PatternPreset) -> Result<PathBuf, String> {
    save_json(
        presets_dir(PresetKind::Pattern),
        &p.name,
        PresetKind::Pattern,
        &serde_json::to_string_pretty(p).map_err(|e| e.to_string())?,
    )
}

pub fn save_song(p: &SongPreset) -> Result<PathBuf, String> {
    save_json(
        presets_dir(PresetKind::Song),
        &p.name,
        PresetKind::Song,
        &serde_json::to_string_pretty(p).map_err(|e| e.to_string())?,
    )
}

pub fn save_grid(p: &GridPreset) -> Result<PathBuf, String> {
    save_json(
        presets_dir(PresetKind::Grid),
        &p.name,
        PresetKind::Grid,
        &serde_json::to_string_pretty(p).map_err(|e| e.to_string())?,
    )
}

/// List user presets of a kind, sorted by name.
pub fn list_presets(kind: PresetKind) -> Vec<PresetFileInfo> {
    let dir = presets_dir(kind);
    list_dir(&dir, kind)
}

fn list_dir(dir: &Path, kind: PresetKind) -> Vec<PresetFileInfo> {
    let suffix = format!(".{}", kind.extension());
    let mut infos = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let is_preset = path
                .file_name()
                .and_then(|s| s.to_str())
                .map_or(false, |s| s.ends_with(&suffix));
            if !is_preset {
                continue;
            }
            // Read the display name from the file content (fall back to the
            // file stem if unparseable).
            let name = load_json(&path)
                .ok()
                .and_then(|content| {
                    serde_json::from_str::<NameOnly>(&content)
                        .ok()
                        .map(|n| n.name)
                })
                .unwrap_or_else(|| {
                    path.file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("preset")
                        .trim_end_matches(match kind {
                            PresetKind::Instrument => ".fdinst",
                            PresetKind::Pattern => ".fdpat",
                            PresetKind::Grid => ".fdgrid",
                            PresetKind::Song => ".fdsong",
                        })
                        .to_string()
                });
            let modified = entry.metadata().ok().and_then(|m| m.modified().ok());
            infos.push(PresetFileInfo {
                name,
                path,
                modified,
            });
        }
    }
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    infos
}

#[derive(Deserialize)]
struct NameOnly {
    name: String,
}

// ---------------------------------------------------------------------------
// Factory presets (embedded) + debug export
// ---------------------------------------------------------------------------

/// Parse the embedded factory presets of a kind (name + payload JSON).
pub fn factory_presets(kind: PresetKind) -> Vec<(String, &'static str)> {
    let raw: &[&str] = match kind {
        PresetKind::Instrument => crate::factory_presets::INSTRUMENTS,
        PresetKind::Pattern => crate::factory_presets::PATTERNS,
        PresetKind::Grid => crate::factory_presets::GRIDS,
        PresetKind::Song => crate::factory_presets::SONGS,
    };
    raw.iter()
        .filter_map(|json| {
            serde_json::from_str::<NameOnly>(json)
                .ok()
                .map(|n| (n.name, *json))
        })
        .collect()
}

/// Debug-only factory authoring: write a preset into the `_factory` staging
/// directory (then copied into `assets/presets/` and committed).
#[cfg(debug_assertions)]
pub fn export_factory(kind: PresetKind, name: &str, json: &str) -> Result<PathBuf, String> {
    save_json(factory_staging_dir(kind), name, kind, json)
}

// ---------------------------------------------------------------------------
// Starter presets copied into the user folders ([280])
// ---------------------------------------------------------------------------

/// `(subdir, file name, deflated JSON)` — generated by `build.rs` from
/// `assets/presets/default-user/`. Unlike factory presets these become plain
/// user files: renamable, deletable, overwritable.
const DEFAULT_USER_PRESETS: &[(&str, &str, &[u8])] =
    include!(concat!(env!("OUT_DIR"), "/default_user_presets.rs"));

/// Written once the starter presets are in place, so a preset the user
/// deletes does not come back at the next editor open.
const DEFAULT_USER_PRESETS_MARKER: &str = ".default-presets-v1";

/// Copy the starter presets into `Documents/Flash Drum/presets/` the first
/// time the editor opens. UI thread only (file I/O).
pub fn seed_default_user_presets() {
    if let Err(e) = seed_default_user_presets_into(&presets_root()) {
        nih_plug::nih_log!("Starter presets not copied: {e}");
    }
}

/// Never overwrites: a file already there (same name) is left untouched. The
/// marker is only written when every file is in place, so a failed copy is
/// retried at the next editor open.
fn seed_default_user_presets_into(root: &Path) -> Result<usize, String> {
    let marker = root.join(DEFAULT_USER_PRESETS_MARKER);
    if marker.exists() {
        return Ok(0);
    }
    let mut written = 0;
    for (subdir, name, packed) in DEFAULT_USER_PRESETS {
        let dir = root.join(subdir);
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        let json = miniz_oxide::inflate::decompress_to_vec(packed)
            .map_err(|e| format!("{subdir}/{name}: {e:?}"))?;
        let json = String::from_utf8(json).map_err(|e| format!("{subdir}/{name}: {e}"))?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        write_atomic(&path, &json)?;
        written += 1;
    }
    std::fs::write(&marker, "").map_err(|e| e.to_string())?;
    Ok(written)
}

// ---------------------------------------------------------------------------
// Instrument presets filtered by kind (for the Track-tab quick loader)
// ---------------------------------------------------------------------------

/// Where an instrument preset comes from.
#[derive(Debug, Clone)]
pub enum InstrumentPresetSource {
    /// Embedded factory preset (its JSON payload).
    Factory(&'static str),
    /// User preset file on disk.
    User(PathBuf),
}

/// A selectable instrument preset (display name + how to load it).
#[derive(Debug, Clone)]
pub struct InstrumentPresetEntry {
    pub name: String,
    pub source: InstrumentPresetSource,
}

/// Just the `kind` field, to filter instrument presets without a full parse.
#[derive(Deserialize)]
struct KindOnly {
    kind: usize,
}

/// List every instrument preset (factory first, then user) that applies to the
/// given `TrackInstrumentKind` index, sorted factory-then-user, each group by
/// name. Reads/parses files, so callers should cache the result rather than
/// call it every frame.
pub fn list_instrument_presets(kind_index: usize) -> Vec<InstrumentPresetEntry> {
    let mut out = Vec::new();
    for json in crate::factory_presets::INSTRUMENTS {
        if let Ok(k) = serde_json::from_str::<KindOnly>(json) {
            if k.kind == kind_index {
                let name = serde_json::from_str::<NameOnly>(json)
                    .map(|n| n.name)
                    .unwrap_or_else(|_| "preset".to_string());
                out.push(InstrumentPresetEntry {
                    name,
                    source: InstrumentPresetSource::Factory(json),
                });
            }
        }
    }
    let factory_end = out.len();
    for info in list_presets(PresetKind::Instrument) {
        if let Ok(content) = load_json(&info.path) {
            if let Ok(k) = serde_json::from_str::<KindOnly>(&content) {
                if k.kind == kind_index {
                    out.push(InstrumentPresetEntry {
                        name: info.name,
                        source: InstrumentPresetSource::User(info.path),
                    });
                }
            }
        }
    }
    out[factory_end..].sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Resolve an entry to a full `InstrumentPreset` (parses the factory JSON or
/// reads the user file). Returns `None` on any I/O or parse error.
pub fn load_instrument_preset(entry: &InstrumentPresetEntry) -> Option<InstrumentPreset> {
    let json = match &entry.source {
        InstrumentPresetSource::Factory(j) => (*j).to_string(),
        InstrumentPresetSource::User(p) => load_json(p).ok()?,
    };
    serde_json::from_str::<InstrumentPreset>(&json).ok()
}

// ---------------------------------------------------------------------------
// Hex helpers (binary blobs inside JSON)
// ---------------------------------------------------------------------------

pub fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xF) as usize] as char);
    }
    out
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    let bytes = s.as_bytes();
    if bytes.len() % 2 != 0 {
        return Err("odd hex length".to_string());
    }
    let val = |c: u8| -> Result<u8, String> {
        match c {
            b'0'..=b'9' => Ok(c - b'0'),
            b'a'..=b'f' => Ok(c - b'a' + 10),
            b'A'..=b'F' => Ok(c - b'A' + 10),
            _ => Err(format!("invalid hex char {}", c as char)),
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push((val(pair[0])? << 4) | val(pair[1])?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(hex_decode(&hex_encode(&data)).unwrap(), data);
        assert!(hex_decode("abc").is_err()); // odd length
        assert!(hex_decode("zz").is_err()); // invalid char
    }

    #[test]
    fn sanitize_name_strips_specials() {
        assert_eq!(sanitize_name("My Preset!"), "My_Preset");
        assert_eq!(sanitize_name("___"), "preset");
    }

    #[test]
    fn instrument_preset_json_roundtrip() {
        let p = InstrumentPreset {
            version: PRESET_FORMAT_VERSION,
            name: "Punchy".to_string(),
            kind: TrackInstrumentKind::Kick.index(),
            standards: [1.0; 13],
            algo: 2,
            specials: vec![0.5, 1.0, 3.0],
            user_texture: Some("C:/samples/tex.wav".to_string()),
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: InstrumentPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        // [228] A preset written before user textures existed still loads.
        let legacy = json.replace(",\"user_texture\":\"C:/samples/tex.wav\"", "");
        assert!(!legacy.contains("user_texture"));
        let back: InstrumentPreset = serde_json::from_str(&legacy).unwrap();
        assert_eq!(back.user_texture, None);
    }

    #[test]
    fn pattern_preset_roundtrip_preserves_sound_plock() {
        // Live state with a sound plock on slot 0 step 4 (link mode, freq).
        let pattern = SharedPattern::new(&crate::sequencer::pattern::Pattern::empty());
        pattern.set_step_mask(4, 0b1);
        let plock = crate::plock::PlockState::new();
        let seq = crate::plock::SequencerPlockState::new();
        plock.set_field(0, 4, 0, 1234.5); // field 0 = Freq
        let layout = crate::track::TrackLayoutState::modular_default_layout();

        let sound_settings = crate::sound_settings::SoundSettingsState::new(&layout);
        let preset = capture_pattern(
            "T".into(),
            &layout,
            &pattern,
            &plock,
            &seq,
            16,
            &sound_settings,
            &[0i32; 14],
            &crate::user_textures::UserTextures::new(),
        );
        // JSON round trip, like save/load through a file.
        let json = serde_json::to_string(&preset).unwrap();
        let preset: PatternPreset = serde_json::from_str(&json).unwrap();

        // Apply into a FRESH live state (the preset_browser apply path).
        let pattern2 = SharedPattern::new(&crate::sequencer::pattern::Pattern::empty());
        let plock2 = crate::plock::PlockState::new();
        let seq2 = crate::plock::SequencerPlockState::new();
        let plock_bytes = hex_decode(&preset.plock_hex).unwrap();
        let seq_bytes = hex_decode(&preset.seq_plock_hex).unwrap();
        let fusion_bytes = hex_decode(&preset.fusion_hex).unwrap();
        crate::pattern_bank::restore_from_buffers(
            &preset.step_masks,
            &plock_bytes,
            &seq_bytes,
            &fusion_bytes,
            &pattern2,
            &plock2,
            &seq2,
        );

        assert!(plock2.masks.is_active(0, 4), "sound plock mask lost");
        let v = plock2.values.get(0, 4, 0);
        assert!(
            (v - 1234.5).abs() < 0.01,
            "sound plock value lost (got {v})"
        );
        assert!(
            plock2.field_masks.get(0, 4) & 1 != 0,
            "sound plock field mask lost"
        );
    }

    #[test]
    fn pattern_preset_capture_has_decodable_blobs() {
        let pattern = SharedPattern::new(&crate::sequencer::pattern::Pattern::empty());
        pattern.set_step_mask(0, 0b101);
        let plock = crate::plock::PlockState::new();
        let seq = crate::plock::SequencerPlockState::new();
        let layout = crate::track::TrackLayoutState::modular_default_layout();
        let sound = SoundSettingsState::new(&layout);
        let algos = [0i32; MAX_TRACKS];

        let p = capture_pattern(
            "Test".into(),
            &layout,
            &pattern,
            &plock,
            &seq,
            32,
            &sound,
            &algos,
            &crate::user_textures::UserTextures::new(),
        );
        assert_eq!(p.step_masks[0], 0b101);
        assert!(hex_decode(&p.plock_hex).is_ok());
        assert!(hex_decode(&p.seq_plock_hex).is_ok());
        assert!(hex_decode(&p.fusion_hex).is_ok());
        assert_eq!(p.pattern_length, 32);
        // Kit: 4 active lanes (Kick/Snare/HiHat/Tom), slot 4 inactive.
        assert_eq!(p.kit[0], TrackInstrumentKind::Kick.index() as i8);
        assert_eq!(p.kit[3], TrackInstrumentKind::Tom.index() as i8);
        assert_eq!(p.kit[4], -1);
        // Each active lane captured its instrument sound.
        assert_eq!(p.sounds.len(), 4);
        assert_eq!(p.sounds[0].slot, 0);
        assert_eq!(p.sounds[0].kind, TrackInstrumentKind::Kick.index());

        let json = serde_json::to_string(&p).unwrap();
        let back: PatternPreset = serde_json::from_str(&json).unwrap();
        assert_eq!(back.step_masks[0], 0b101);
        assert_eq!(back.kit, p.kit);
        assert_eq!(back.sounds, p.sounds);
    }

    #[test]
    fn rename_preset_updates_json_name_and_filename() {
        let dir =
            std::env::temp_dir().join(format!("fd_rename_test_{:?}", std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        // [246] Start from a file as save_json produces it: double extension.
        let old = dir.join("Old_Name.fdpat.json");
        std::fs::write(&old, r#"{"version":1,"name":"Old Name"}"#).unwrap();

        let new_path = rename_preset(&old, "New Name!", PresetKind::Pattern).unwrap();
        assert!(new_path.ends_with("New_Name.fdpat.json"));
        assert!(!old.exists());
        let content = std::fs::read_to_string(&new_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(value["name"], "New Name!");

        // Renaming to a name that sanitizes to the SAME file stays in place.
        let same = rename_preset(&new_path, "New_Name", PresetKind::Pattern).unwrap();
        assert_eq!(same, new_path);
        assert!(same.exists());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rename_preset_keeps_double_extension_and_stays_listed() {
        let dir = std::env::temp_dir().join(format!(
            "fd_rename_listed_test_{:?}",
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = save_json(
            dir.clone(),
            "Kick punchy",
            PresetKind::Instrument,
            r#"{"version":1,"name":"Kick punchy"}"#,
        )
        .unwrap();
        assert!(saved.ends_with("Kick_punchy.fdinst.json"));

        let renamed = rename_preset(&saved, "Kick punchy v2", PresetKind::Instrument).unwrap();
        assert!(renamed.ends_with("Kick_punchy_v2.fdinst.json"));

        let listed = list_dir(&dir, PresetKind::Instrument);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, renamed);
        assert_eq!(listed[0].name, "Kick punchy v2");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// [261] Windows device names are reserved whatever the extension.
    #[test]
    fn sanitize_name_avoids_reserved_device_names() {
        assert_eq!(sanitize_name("aux"), "aux_");
        assert_eq!(sanitize_name("CON"), "CON_");
        assert_eq!(sanitize_name("Com1"), "Com1_");
        assert_eq!(sanitize_name("NUL"), "NUL_");
        assert_eq!(sanitize_name("lpt9"), "lpt9_");
        assert_eq!(sanitize_name("console"), "console");
        assert_eq!(sanitize_name("aux cord"), "aux_cord");
    }

    /// [261] A failing save must come back as an error (the modal shows it)
    /// instead of being swallowed.
    #[test]
    fn save_into_unwritable_dir_reports_error() {
        let dir =
            std::env::temp_dir().join(format!("fd_save_err_{:?}", std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("blocked");
        // `blocked` is a FILE: creating a directory under it must fail.
        std::fs::write(&blocker, b"file").unwrap();
        let result = save_json(blocker.clone(), "x", PresetKind::Pattern, "{}");
        assert!(result.is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_date_known_days() {
        assert_eq!(format_date(std::time::UNIX_EPOCH), "1970-01-01");
        // 2026-09-22 00:00 UTC = 20718 days after the epoch.
        let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(20718 * 86_400);
        assert_eq!(format_date(t), "2026-09-22");
    }

    #[test]
    fn pattern_preset_without_sounds_field_loads_empty() {
        // A pattern preset saved before the `sounds` field existed must still
        // deserialize (serde default → empty list, sounds left untouched on apply).
        let json = r#"{"version":1,"name":"old","kit":[-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1,-1],"step_masks":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],"plock_hex":"","seq_plock_hex":"","fusion_hex":"","pattern_length":16}"#;
        let back: PatternPreset = serde_json::from_str(json).unwrap();
        assert!(back.sounds.is_empty());
        assert_eq!(back.pattern_length, 16);
    }

    /// [280] The embedded list is exactly the asset folder, byte for byte, and
    /// every file is a loadable preset of the kind its folder says.
    #[test]
    fn default_user_presets_match_the_asset_folder_and_parse() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/presets/default-user");
        let mut on_disk = Vec::new();
        for kind in PresetKind::ALL {
            if let Ok(entries) = std::fs::read_dir(root.join(kind.subdir())) {
                for e in entries.flatten() {
                    on_disk.push((kind.subdir(), e.file_name().into_string().unwrap()));
                }
            }
        }
        on_disk.sort();
        let mut embedded: Vec<_> = DEFAULT_USER_PRESETS
            .iter()
            .map(|(d, n, _)| (*d, n.to_string()))
            .collect();
        embedded.sort();
        assert_eq!(embedded, on_disk);
        assert!(!embedded.is_empty());

        for (subdir, name, packed) in DEFAULT_USER_PRESETS {
            let json = miniz_oxide::inflate::decompress_to_vec(packed).unwrap();
            assert_eq!(
                json,
                std::fs::read(root.join(subdir).join(name)).unwrap(),
                "{name}"
            );
            let json = String::from_utf8(json).unwrap();
            let kind = PresetKind::ALL
                .into_iter()
                .find(|k| k.subdir() == *subdir)
                .unwrap();
            assert!(
                name.ends_with(&format!(".{}", kind.extension())),
                "{subdir}/{name}"
            );
            let parsed = match kind {
                PresetKind::Instrument => serde_json::from_str::<InstrumentPreset>(&json).map(drop),
                PresetKind::Pattern => serde_json::from_str::<PatternPreset>(&json).map(drop),
                PresetKind::Grid => serde_json::from_str::<GridPreset>(&json).map(drop),
                PresetKind::Song => serde_json::from_str::<SongPreset>(&json).map(drop),
            };
            assert!(parsed.is_ok(), "{subdir}/{name}: {parsed:?}");
        }
    }

    /// A fresh installation exposes the complete release preset set through
    /// the same directory listing used by the User preset browser.
    #[test]
    fn fresh_install_lists_all_bundled_user_presets() {
        let root = std::env::temp_dir().join(format!(
            "fd_fresh_seed_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::remove_dir_all(&root).ok();
        assert_eq!(seed_default_user_presets_into(&root).unwrap(), 22);
        assert!(root.join(DEFAULT_USER_PRESETS_MARKER).is_file());
        assert_eq!(
            list_dir(&root.join("instruments"), PresetKind::Instrument).len(),
            4
        );
        assert_eq!(
            list_dir(&root.join("patterns"), PresetKind::Pattern).len(),
            18
        );

        for (subdir, name, packed) in DEFAULT_USER_PRESETS {
            assert_eq!(
                std::fs::read(root.join(subdir).join(name)).unwrap(),
                miniz_oxide::inflate::decompress_to_vec(packed).unwrap(),
                "{subdir}/{name}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// [280] First run copies what is missing and keeps an existing file with
    /// the same name; once the marker is down, a deleted preset stays deleted.
    #[test]
    fn seeding_copies_missing_presets_once_and_never_overwrites() {
        let root =
            std::env::temp_dir().join(format!("fd_seed_test_{:?}", std::thread::current().id()));
        std::fs::remove_dir_all(&root).ok();
        let (subdir, name, _) = DEFAULT_USER_PRESETS[0];
        let mine = root.join(subdir).join(name);
        std::fs::create_dir_all(mine.parent().unwrap()).unwrap();
        std::fs::write(&mine, "mine").unwrap();

        let written = seed_default_user_presets_into(&root).unwrap();
        assert_eq!(written, DEFAULT_USER_PRESETS.len() - 1);
        assert_eq!(std::fs::read_to_string(&mine).unwrap(), "mine");
        let (subdir, name, _) = DEFAULT_USER_PRESETS[DEFAULT_USER_PRESETS.len() - 1];
        let copied = root.join(subdir).join(name);
        assert!(copied.exists());

        std::fs::remove_file(&copied).unwrap();
        assert_eq!(seed_default_user_presets_into(&root).unwrap(), 0);
        assert!(!copied.exists());

        std::fs::remove_dir_all(&root).ok();
    }
}
