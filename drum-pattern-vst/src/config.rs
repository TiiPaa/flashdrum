use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// [263] Serializes the tests that mutate `FLASH_DRUM_CONFIG_DIR`: the env
/// var is process-global, and a parallel test reading `config_path()` in the
/// mutation window (e.g. `paths::user_dirs_share_one_root`) would observe the
/// redirected path and fail.
#[cfg(test)]
pub(crate) static CONFIG_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Global user preferences stored outside the DAW project.
///
/// Lives in `Documents/Flash Drum/config.json` ([260]: the real Documents
/// folder, OneDrive/macOS-proof) so settings survive across sessions and are
/// shared between all plugin instances.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobalConfig {
    /// Default value for the `Analog` parameter when a slot is created or reset.
    pub default_analog: f32,
    /// Default global MIDI channel used for MIDI input/output (1-16).
    pub global_midi_channel: u8,
    /// Active UI skin name (see `ui::theme::SKINS`).
    #[serde(default = "default_skin")]
    pub skin: String,
    /// [283] Help mode: explanatory tooltips over the features (header
    /// "Help" keycap). Off by default; older config files load it off.
    #[serde(default)]
    pub help_mode: bool,
    /// [282] Editor zoom in percent, one of `ZOOM_STEPS` (Settings → Zoom).
    #[serde(default = "default_zoom_percent")]
    pub zoom_percent: u16,
}

/// [282] The zoom levels offered in Settings.
pub const ZOOM_STEPS: [u16; 3] = [100, 125, 150];

fn default_zoom_percent() -> u16 {
    100
}

fn default_skin() -> String {
    // Must match `ui::theme::DEFAULT_SKIN_NAME` ("Dark").
    "Dark".to_string()
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            default_analog: 0.5,
            global_midi_channel: 10,
            skin: default_skin(),
            help_mode: false,
            zoom_percent: default_zoom_percent(),
        }
    }
}

impl GlobalConfig {
    /// Load the config from disk, creating a default file if it doesn't exist.
    pub fn load() -> Self {
        let path = Self::config_path();
        if let Ok(contents) = std::fs::read_to_string(&path) {
            if let Ok(config) = serde_json::from_str::<GlobalConfig>(&contents) {
                // Clamp to a sensible range in case the file was hand-edited.
                return config.clamped();
            }
            // [261] An unreadable config is preserved, not silently
            // overwritten by the defaults: rename it aside for the user.
            let _ = std::fs::rename(&path, path.with_extension("json.bad"));
        }
        let config = GlobalConfig::default();
        let _ = config.save();
        config
    }

    /// Persist the current config to disk.
    pub fn save(&self) -> std::io::Result<()> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        // [261] Temp file + rename: no truncated config on crash.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &path)
    }

    /// Path to the config file: `Documents/Flash Drum/config.json` — the real
    /// Documents folder ([260], OneDrive/macOS-proof).
    /// [263] `FLASH_DRUM_CONFIG_DIR` redirects the whole config (tests, CI):
    /// without it `DrumFlashParams::default()` reads — and creates — the real
    /// user config, making tests depend on the machine's `default_analog`.
    pub(crate) fn config_path() -> PathBuf {
        if let Some(dir) = std::env::var_os("FLASH_DRUM_CONFIG_DIR") {
            return PathBuf::from(dir).join("config.json");
        }
        crate::paths::flash_drum_dir().join("config.json")
    }

    fn clamped(mut self) -> Self {
        self.default_analog = self.default_analog.clamp(0.0, 1.0);
        self.global_midi_channel = self.global_midi_channel.clamp(1, 16);
        if !ZOOM_STEPS.contains(&self.zoom_percent) {
            self.zoom_percent = default_zoom_percent();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_env_override_redirects_the_config() {
        // [263] Hermetic tests: FLASH_DRUM_CONFIG_DIR must win over the user
        // folder.
        let _guard = crate::config::CONFIG_ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("fd_cfg_{:?}", std::thread::current().id()));
        std::env::set_var("FLASH_DRUM_CONFIG_DIR", &dir);
        let path = GlobalConfig::config_path();
        std::env::remove_var("FLASH_DRUM_CONFIG_DIR");
        assert!(path.starts_with(&dir));
        assert!(path.ends_with("config.json"));
    }

    #[test]
    fn unreadable_config_is_moved_aside_not_erased() {
        // [261] A corrupt config.json must be renamed to .bad, never silently
        // replaced by defaults.
        let _guard = crate::config::CONFIG_ENV_LOCK.lock().unwrap();
        let dir =
            std::env::temp_dir().join(format!("fd_cfg_bad_{:?}", std::thread::current().id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("config.json"), b"{ not json").unwrap();
        std::env::set_var("FLASH_DRUM_CONFIG_DIR", &dir);
        let config = GlobalConfig::load();
        std::env::remove_var("FLASH_DRUM_CONFIG_DIR");
        assert_eq!(config, GlobalConfig::default());
        let bad = std::fs::read_to_string(dir.join("config.json.bad")).unwrap();
        assert_eq!(bad, "{ not json");
        // A fresh default config was written next to it.
        assert!(dir.join("config.json").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_analog_is_05() {
        let config = GlobalConfig::default();
        assert!((config.default_analog - 0.5).abs() < 1e-6);
        assert_eq!(config.global_midi_channel, 10);
    }

    #[test]
    fn clamped_rejects_out_of_range_values() {
        let mut high = GlobalConfig::default();
        high.default_analog = 2.5;
        assert_eq!(high.clamped().default_analog, 1.0);

        let mut low = GlobalConfig::default();
        low.default_analog = -1.0;
        assert_eq!(low.clamped().default_analog, 0.0);

        let mut chan = GlobalConfig::default();
        chan.global_midi_channel = 25;
        assert_eq!(chan.clamped().global_midi_channel, 16);

        let mut chan = GlobalConfig::default();
        chan.global_midi_channel = 0;
        assert_eq!(chan.clamped().global_midi_channel, 1);

        let mut zoom = GlobalConfig::default();
        zoom.zoom_percent = 130;
        assert_eq!(zoom.clamped().zoom_percent, 100);
    }

    /// [282] A config written before the zoom existed opens at 100 %.
    #[test]
    fn zoom_defaults_to_100_percent() {
        let old = r#"{"default_analog":0.5,"global_midi_channel":10,"skin":"Dark"}"#;
        let cfg: GlobalConfig = serde_json::from_str(old).unwrap();
        assert_eq!(cfg.zoom_percent, 100);
    }

    /// [283] A config written before the help mode existed loads with the
    /// help mode off, and the setting survives a save/load round trip.
    #[test]
    fn help_mode_defaults_off_and_roundtrips() {
        let old = r#"{"default_analog":0.5,"global_midi_channel":10,"skin":"Dark"}"#;
        let cfg: GlobalConfig = serde_json::from_str(old).unwrap();
        assert!(!cfg.help_mode);

        let on = GlobalConfig {
            help_mode: true,
            ..Default::default()
        };
        let back: GlobalConfig =
            serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert!(back.help_mode);
    }
}
