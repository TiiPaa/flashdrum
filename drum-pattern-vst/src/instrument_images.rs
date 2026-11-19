//! Instrument artwork for the Lane Editor.
//!
//! Every kind has an embedded 1024×64 banner. A lane may replace it with a custom
//! PNG: only its path is persisted; the image is decoded and normalized on the
//! UI/main thread, never on the audio thread. A missing or invalid custom file
//! falls back to the instrument name.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use nih_plug::params::persist::PersistentField;
use nih_plug_egui::egui;

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

/// [331] Neon pulse for one banner: `texture` holds, per pixel, the neon colour
/// scaled by how close that pixel is to the banner's dominant tone. It is painted
/// additively, so the picture itself lights up instead of being veiled.
pub struct BannerGlow {
    pub texture: egui::TextureHandle,
}

/// Distance (0..255 RGB units) that still counts as "the same tone".
const GLOW_SIGMA: f32 = 26.0;

/// Editor-only cache: one analysis per banner URI (embedded or custom file).
pub fn banner_glow(
    ctx: &egui::Context,
    uri: &str,
    bytes: &[u8],
) -> Option<Arc<BannerGlow>> {
    let id = egui::Id::new(("banner_glow_v1", uri));
    if let Some(glow) = ctx.data(|d| d.get_temp::<Arc<BannerGlow>>(id)) {
        return Some(glow);
    }
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    let (width, height) = (image.width() as usize, image.height() as usize);
    if width == 0 || height == 0 {
        return None;
    }
    let (pixels, _neon) = glow_pixels(image.as_raw(), width, height)?;
    let color = egui::ColorImage::from_rgba_premultiplied([width, height], &pixels);
    let texture = ctx.load_texture(
        format!("banner-glow-{uri}"),
        color,
        egui::TextureOptions::LINEAR,
    );
    let glow = Arc::new(BannerGlow { texture });
    ctx.data_mut(|d| d.insert_temp(id, glow.clone()));
    Some(glow)
}

/// Finds the tone covering the most coloured area, then keeps only pixels close
/// to it. Returns an RGBA buffer (RGB = neon × weight, A = 255) and the neon tone.
fn glow_pixels(rgba: &[u8], width: usize, height: usize) -> Option<(Vec<u8>, [u8; 3])> {
    const BINS: usize = 72;
    let mut bins = [0.0f32; BINS];
    let mut any = false;
    for i in 0..width * height {
        let (r, g, b) = (rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        if v < 0.10 || s < 0.18 {
            continue;
        }
        any = true;
        bins[((h / 5.0) as usize).min(BINS - 1)] += s * v;
    }
    if !any {
        return None;
    }
    let dominant = (0..BINS)
        .max_by(|a, b| bins[*a].total_cmp(&bins[*b]))
        .unwrap_or(0);
    let centre = dominant as f32 * 5.0 + 2.5;

    // Mean tone of that hue band: one colour, not a whole hue family.
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for i in 0..width * height {
        let (r, g, b) = (rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
        let (h, s, v) = rgb_to_hsv(r, g, b);
        if v < 0.10 || s < 0.18 {
            continue;
        }
        let mut dh = (h - centre).abs();
        if dh > 180.0 {
            dh = 360.0 - dh;
        }
        if dh > 12.0 {
            continue;
        }
        let w = s * v;
        sum[0] += r as f32 * w;
        sum[1] += g as f32 * w;
        sum[2] += b as f32 * w;
        weight += w;
    }
    if weight <= 0.0 {
        return None;
    }
    let tone = [sum[0] / weight, sum[1] / weight, sum[2] / weight];
    let (hue, _, _) = rgb_to_hsv(tone[0] as u8, tone[1] as u8, tone[2] as u8);
    let neon = hsl_to_rgb(hue, 1.0, 0.58);

    let mut out = vec![0u8; width * height * 4];
    for i in 0..width * height {
        let (r, g, b) = (
            rgba[i * 4] as f32,
            rgba[i * 4 + 1] as f32,
            rgba[i * 4 + 2] as f32,
        );
        let d2 = (r - tone[0]).powi(2) + (g - tone[1]).powi(2) + (b - tone[2]).powi(2);
        let w = (-d2 / (2.0 * GLOW_SIGMA * GLOW_SIGMA)).exp();
        out[i * 4] = (neon[0] as f32 * w) as u8;
        out[i * 4 + 1] = (neon[1] as f32 * w) as u8;
        out[i * 4 + 2] = (neon[2] as f32 * w) as u8;
        out[i * 4 + 3] = 255;
    }
    Some((out, neon))
}


/// HSV with hue in degrees, saturation and value in 0..1.
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let mut h = if d == 0.0 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h *= 60.0;
    (h, if max == 0.0 { 0.0 } else { d / max }, max)
}

fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [u8; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let rgb = if hp < 1.0 {
        [c, x, 0.0]
    } else if hp < 2.0 {
        [x, c, 0.0]
    } else if hp < 3.0 {
        [0.0, c, x]
    } else if hp < 4.0 {
        [0.0, x, c]
    } else if hp < 5.0 {
        [x, 0.0, c]
    } else {
        [c, 0.0, x]
    };
    let m = l - c / 2.0;
    rgb.map(|v| (((v + m) * 255.0).round().clamp(0.0, 255.0)) as u8)
}

/// Resize then centre-crop to 16:1, never stretch a custom image.
fn banner_crop(image: image::DynamicImage) -> image::DynamicImage {    let source_w = image.width() as u64;
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
        K::Crown => embedded(K::Snare),
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

    /// [331] Two thirds red, one third blue: only the red area may light up.
    #[test]
    fn glow_keeps_one_tone_and_its_close_variations() {
        let (w, h) = (90usize, 6usize);
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                let (r, g, b) = if x < 60 {
                    // Red with slight shade variations, as a real artwork has.
                    let k = (x as u8 % 7) as u16;
                    ((190 + k) as u8, (40 + k) as u8, (30 + k) as u8)
                } else {
                    (30, 60, 200)
                };
                rgba[i] = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 255;
            }
        }
        let (pixels, neon) = glow_pixels(&rgba, w, h).expect("a saturated banner has a tone");
        assert!(neon[0] > 150 && neon[2] < 90, "neon must keep the dominant hue: {neon:?}");
        let energy = |from: usize, to: usize| {
            let mut sum = 0u64;
            for y in 0..h {
                for x in from..to {
                    let i = (y * w + x) * 4;
                    sum += pixels[i] as u64 + pixels[i + 1] as u64 + pixels[i + 2] as u64;
                }
            }
            sum
        };
        let red = energy(0, 60);
        let blue = energy(60, w);
        assert!(red > blue * 6, "the other tone must stay dark: red {red}, blue {blue}");
    }

    /// [331] The real banners must keep dark areas: a mask that covers almost
    /// everything would flash the whole image, which is what was rejected.
    #[test]
    fn real_banners_keep_part_of_the_image_dark() {
        for kind in [
            TrackInstrumentKind::Kick,
            TrackInstrumentKind::Snare,
            TrackInstrumentKind::HiHat,
            TrackInstrumentKind::Ride,
        ] {
            let (_, bytes) = embedded(kind);
            let image = image::load_from_memory(bytes).unwrap().to_rgba8();
            let (w, h) = (image.width() as usize, image.height() as usize);
            let (pixels, neon) = glow_pixels(image.as_raw(), w, h).expect("saturated artwork");
            let scale = neon[0] as f32 + neon[1] as f32 + neon[2] as f32;
            let mut covered = 0usize;
            let mut lit = 0usize;
            for i in 0..w * h {
                let sum = pixels[i * 4] as f32 + pixels[i * 4 + 1] as f32 + pixels[i * 4 + 2] as f32;
                let weight = if scale > 0.0 { sum / scale } else { 0.0 };
                if weight > 0.25 {
                    covered += 1;
                }
                if weight > 0.6 {
                    lit += 1;
                }
            }
            let covered = covered as f32 / (w * h) as f32;
            let lit = lit as f32 / (w * h) as f32;
            assert!(
                covered > 0.05 && covered < 0.85,
                "{kind:?}: the pulse must target a part of the artwork, covered {covered:.2}"
            );
            assert!(lit < 0.6, "{kind:?}: too much at full brightness ({lit:.2})");
        }
    }

    #[test]
    fn glow_ignores_an_image_without_a_saturated_tone() {
        let (w, h) = (32usize, 4usize);
        let rgba: Vec<u8> = (0..w * h)
            .flat_map(|i| {
                let v = (60 + (i % 40)) as u8;
                [v, v, v, 255]
            })
            .collect();
        assert!(glow_pixels(&rgba, w, h).is_none());
    }

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
