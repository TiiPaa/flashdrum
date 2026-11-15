//! Instrument artwork for the Lane Editor.
//!
//! Every kind has an embedded 1024×64 banner. A lane may replace it with a custom
//! PNG: only its path is persisted; the image is decoded and normalized on the
//! UI/main thread, never on the audio thread. A missing or invalid custom file
//! falls back to the instrument name.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use nih_plug::params::persist::PersistentField;

use crate::track::{TrackInstrumentKind, MAX_TRACKS};

#[derive(Clone, Debug)]
pub struct CustomImage {
    pub uri: String,
    pub bytes: Arc<[u8]>,
}

#[derive(Default)]
pub struct InstrumentImages {
    paths: RwLock<Vec<Option<String>>>,
    images: RwLock<Vec<Option<Arc<CustomImage>>>>,
    errors: RwLock<Vec<Option<String>>>,
}

impl InstrumentImages {
    pub fn new() -> Self {
        Self {
            paths: RwLock::new(vec![None; MAX_TRACKS]),
            images: RwLock::new(vec![None; MAX_TRACKS]),
            errors: RwLock::new(vec![None; MAX_TRACKS]),
        }
    }

    pub fn path(&self, lane: usize) -> Option<PathBuf> {
        self.paths
            .read()
            .ok()
            .and_then(|p| p.get(lane).cloned().flatten())
            .map(PathBuf::from)
    }

    pub fn image(&self, lane: usize) -> Option<Arc<CustomImage>> {
        self.images
            .read()
            .ok()
            .and_then(|images| images.get(lane).cloned().flatten())
    }

    pub fn error(&self, lane: usize) -> Option<String> {
        self.errors
            .read()
            .ok()
            .and_then(|errors| errors.get(lane).cloned().flatten())
    }

    /// Decode and normalize a custom PNG on the UI/main thread.
    pub fn load(&self, lane: usize, path: &Path) -> Result<(), String> {
        if lane >= MAX_TRACKS {
            return Err("no such lane".to_string());
        }
        if let Ok(mut paths) = self.paths.write() {
            paths[lane] = Some(path.to_string_lossy().into_owned());
        }
        match decode_custom(lane, path) {
            Ok(image) => {
                if let Ok(mut images) = self.images.write() {
                    images[lane] = Some(Arc::new(image));
                }
                if let Ok(mut errors) = self.errors.write() {
                    errors[lane] = None;
                }
                Ok(())
            }
            Err(error) => {
                if let Ok(mut images) = self.images.write() {
                    images[lane] = None;
                }
                if let Ok(mut errors) = self.errors.write() {
                    errors[lane] = Some(error.clone());
                }
                Err(error)
            }
        }
    }

    /// Automatic restore never resolves network paths (SMB auth/hash leak and
    /// host-blocking timeouts); an explicit file-picker gesture may load them.
    pub fn load_restored(&self, lane: usize, path: &Path) -> Result<(), String> {
        if !is_local_path(path) {
            if lane < MAX_TRACKS {
                if let Ok(mut paths) = self.paths.write() {
                    paths[lane] = Some(path.to_string_lossy().into_owned());
                }
                if let Ok(mut images) = self.images.write() {
                    images[lane] = None;
                }
                if let Ok(mut errors) = self.errors.write() {
                    errors[lane] = Some(
                        "network path not loaded automatically — re-pick the image".to_string(),
                    );
                }
            }
            return Err("network path not loaded automatically".to_string());
        }
        self.load(lane, path)
    }

    pub fn clear(&self, lane: usize) {
        if lane >= MAX_TRACKS {
            return;
        }
        if let Ok(mut paths) = self.paths.write() {
            paths[lane] = None;
        }
        if let Ok(mut images) = self.images.write() {
            images[lane] = None;
        }
        if let Ok(mut errors) = self.errors.write() {
            errors[lane] = None;
        }
    }

    pub fn copy(&self, source: usize, target: usize) {
        let path = self.path(source);
        let image = self.image(source);
        let error = self.error(source);
        if let Ok(mut paths) = self.paths.write() {
            paths[target] = path.map(|p| p.to_string_lossy().into_owned());
        }
        if let Ok(mut images) = self.images.write() {
            images[target] = image;
        }
        if let Ok(mut errors) = self.errors.write() {
            errors[target] = error;
        }
    }

    /// `order[new] = old`, shared with the other per-lane stores.
    pub fn reorder(&self, order: &[usize]) {
        let old_paths = PersistentField::map(self, |paths| paths.clone());
        let old_images = self.images.read().map(|v| v.clone()).unwrap_or_default();
        let old_errors = self.errors.read().map(|v| v.clone()).unwrap_or_default();
        let mut paths = vec![None; MAX_TRACKS];
        let mut images = vec![None; MAX_TRACKS];
        let mut errors = vec![None; MAX_TRACKS];
        for (new, &old) in order.iter().enumerate().take(MAX_TRACKS) {
            paths[new] = old_paths.get(old).cloned().flatten();
            images[new] = old_images.get(old).cloned().flatten();
            errors[new] = old_errors.get(old).cloned().flatten();
        }
        if let Ok(mut current) = self.paths.write() {
            *current = paths;
        }
        if let Ok(mut current) = self.images.write() {
            *current = images;
        }
        if let Ok(mut current) = self.errors.write() {
            *current = errors;
        }
    }

    fn reload_all(&self) {
        let paths = PersistentField::map(self, |paths| paths.clone());
        for (lane, path) in paths.iter().enumerate().take(MAX_TRACKS) {
            match path {
                Some(path) => {
                    let _ = self.load_restored(lane, Path::new(path));
                }
                None => self.clear(lane),
            }
        }
    }
}

impl<'a> PersistentField<'a, Vec<Option<String>>> for InstrumentImages {
    fn set(&self, new_value: Vec<Option<String>>) {
        let mut paths = new_value;
        paths.resize(MAX_TRACKS, None);
        if let Ok(mut current) = self.paths.write() {
            *current = paths;
        }
        self.reload_all();
    }

    fn map<F, R>(&self, f: F) -> R
    where
        F: Fn(&Vec<Option<String>>) -> R,
    {
        match self.paths.read() {
            Ok(paths) => f(&paths),
            Err(_) => f(&vec![None; MAX_TRACKS]),
        }
    }
}

/// Resize then centre-crop to 16:1, never stretch a custom image.
fn banner_crop(image: image::DynamicImage) -> image::DynamicImage {
    let source_w = image.width() as u64;
    let source_h = image.height() as u64;
    let (width, height) = if source_w * 64 > source_h * 1024 {
        (((source_w * 64 + source_h - 1) / source_h) as u32, 64)
    } else {
        (1024, ((source_h * 1024 + source_w - 1) / source_w) as u32)
    };
    let mut resized = image::imageops::resize(
        &image,
        width,
        height,
        image::imageops::FilterType::Lanczos3,
    );
    let x = (width - 1024) / 2;
    let y = (height - 64) / 2;
    image::DynamicImage::ImageRgba8(image::imageops::crop(&mut resized, x, y, 1024, 64).to_image())
}

fn decode_custom(lane: usize, path: &Path) -> Result<CustomImage, String> {
    let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let decoded = image::ImageReader::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let decoded = banner_crop(decoded);
    let mut bytes = Vec::new();
    decoded
        .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(CustomImage {
        uri: format!("instrument-custom-{lane}-{modified}-{}", metadata.len()),
        bytes: Arc::from(bytes),
    })
}

#[cfg(target_os = "windows")]
fn is_local_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    matches!(
        path.components().next(),
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
    )
}

#[cfg(not(target_os = "windows"))]
fn is_local_path(path: &Path) -> bool {
    path.is_absolute()
}

/// Embedded artwork, keyed by the persisted instrument kind.
pub fn embedded(kind: TrackInstrumentKind) -> (&'static str, &'static [u8]) {
    use TrackInstrumentKind as K;
    match kind {
        K::Kick => ("instrument-kick", include_bytes!("../assets/instruments/banners/kick.png")),
        K::Snare => ("instrument-snare", include_bytes!("../assets/instruments/banners/snare.png")),
        K::HiHat => ("instrument-hihat", include_bytes!("../assets/instruments/banners/hihat.png")),
        K::OpenHiHat => ("instrument-open-hihat", include_bytes!("../assets/instruments/banners/open_hihat.png")),
        K::Tom => ("instrument-tom", include_bytes!("../assets/instruments/banners/tom.png")),
        K::Clap => ("instrument-clap", include_bytes!("../assets/instruments/banners/clap.png")),
        K::Ride => ("instrument-ride", include_bytes!("../assets/instruments/banners/ride.png")),
        K::Cymbal => ("instrument-cymbal", include_bytes!("../assets/instruments/banners/cymbal.png")),
        K::Snare606 => ("instrument-snare606", include_bytes!("../assets/instruments/banners/snare606.png")),
        K::BassDrum808 => ("instrument-bassdrum808", include_bytes!("../assets/instruments/banners/bassdrum808.png")),
        K::Perc1 => ("instrument-perc1", include_bytes!("../assets/instruments/banners/perc1.png")),
        K::Bd6smp => ("instrument-bd6smp", include_bytes!("../assets/instruments/banners/bd6smp.png")),
        K::Sd6smp => ("instrument-sd6smp", include_bytes!("../assets/instruments/banners/sd6smp.png")),
        K::Ch6smp => ("instrument-ch6smp", include_bytes!("../assets/instruments/banners/ch6smp.png")),
        K::Buzz => ("instrument-buzz", include_bytes!("../assets/instruments/banners/buzz.png")),
        K::Sdrex => ("instrument-sdrex", include_bytes!("../assets/instruments/banners/sdrex.png")),
        K::Bd6Ac => ("instrument-bd6ac", include_bytes!("../assets/instruments/banners/bd6ac.png")),
        K::Sd6Ac => ("instrument-sd6ac", include_bytes!("../assets/instruments/banners/sd6ac.png")),
        K::Hh6Ac => ("instrument-hh6ac", include_bytes!("../assets/instruments/banners/hh6ac.png")),
        K::Oh6Ac => ("instrument-oh6ac", include_bytes!("../assets/instruments/banners/oh6ac.png")),
        K::Cl6Ac => ("instrument-cl6ac", include_bytes!("../assets/instruments/banners/cl6ac.png")),
        K::Tm6Ac => ("instrument-tm6ac", include_bytes!("../assets/instruments/banners/tm6ac.png")),
        K::Oh6smp => ("instrument-oh6smp", include_bytes!("../assets/instruments/banners/oh6smp.png")),
        K::Rift => ("instrument-rift", include_bytes!("../assets/instruments/banners/rift.png")),
        K::OneShot => ("instrument-oneshot", include_bytes!("../assets/instruments/banners/oneshot.png")),
        K::Reverb => ("instrument-reverb", include_bytes!("../assets/instruments/banners/reverb.png")),
        K::Delay => ("instrument-delay", include_bytes!("../assets/instruments/banners/delay.png")),
        K::Rumble => ("instrument-rumble", include_bytes!("../assets/instruments/banners/rumble.png")),
        K::MidiOut => ("instrument-midi-out", include_bytes!("../assets/instruments/banners/midi_out.png")),
        // New engines reuse the existing family artwork, not a different lane's
        // custom image. The banner title is rendered separately from this image.
        K::IonBloom | K::AbyssFm => embedded(K::Kick),
        K::Ferroskin => embedded(K::Snare),
        K::PrismHat => embedded(K::HiHat),
        K::OrbitClay | K::NeonTom => embedded(K::Tom),
        K::ArcZap => embedded(K::Buzz),
        K::TidalAlloy => embedded(K::Ride),
        K::MobiusSkin => embedded(K::Perc1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_decodable_banner_artwork() {
        for kind in TrackInstrumentKind::ALL {
            let (uri, bytes) = embedded(kind);
            let image = image::load_from_memory(bytes)
                .unwrap_or_else(|e| panic!("{uri}: {e}"));
            assert_eq!((image.width(), image.height()), (1024, 64), "{uri}");
        }
    }

    #[test]
    fn custom_image_loads_and_moves_with_the_lane() {
        let lane = 0;
        let path = std::env::temp_dir().join(format!(
            "flash-drum-instrument-image-test-{}.png",
            std::process::id()
        ));
        std::fs::write(&path, embedded(TrackInstrumentKind::Kick).1).unwrap();
        let images = InstrumentImages::new();
        images.load(lane, &path).unwrap();
        let custom = images.image(lane).unwrap();
        let decoded = image::load_from_memory(&custom.bytes).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (1024, 64));
        images.copy(lane, 3);
        assert!(images.image(3).is_some());
        images.reorder(&(0..MAX_TRACKS).rev().collect::<Vec<_>>());
        assert!(images.image(MAX_TRACKS - 4).is_some());
        images.clear(MAX_TRACKS - 4);
        assert!(images.image(MAX_TRACKS - 4).is_none());
        let _ = std::fs::remove_file(path);
    }
}
