//! [281] Opt-in process resource sampling, for the Bitwig progressive-lag
//! report (15 min in, every action takes seconds, the DAW takes a minute to
//! quit). The UI-frame audit found no unbounded growth in plugin code, so the
//! next step is a measurement on the affected machine: with
//! `FLASH_DRUM_PERF_LOG=1` in the environment, one line is appended to
//! `%TEMP%\flash-drum-perf.log` every 30 s while the editor is open:
//!
//! ```text
//! 2026-10-02 08:31:00 t_s=30 frames=1823 rss_mb=240.1 private_mb=231.9 handles=512 gdi=180 user=64 cpu_s=12.4
//! ```
//!
//! A line whose `gdi`, `user` or `handles` climbs steadily means a handle
//! leak (Windows caps GDI objects at 10 000/process — the exact global
//! slowdown symptom); `rss`/`private` climbing means memory growth; `cpu_s`
//! racing ahead of wall time means CPU saturation. `FLASH_DRUM_PERF_LOG_SECS`
//! overrides the 30 s interval. Everything lives on the UI thread (file I/O
//! is legal there); when the env var is absent the whole thing costs one
//! cached flag load per frame. Windows-only data: no-op elsewhere.

use std::cell::RefCell;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_INTERVAL_SECS: u64 = 30;
/// Old logs are discarded past this size (a session is ~30 lines/MB anyway).
const MAX_LOG_BYTES: u64 = 1 << 20;

struct State {
    /// `None` until the first frame resolves the env vars.
    enabled: Option<bool>,
    interval: Duration,
    session_start: Option<Instant>,
    last_sample: Option<Instant>,
    frames_since_sample: u64,
    header_written: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            enabled: None,
            interval: Duration::from_secs(DEFAULT_INTERVAL_SECS),
            session_start: None,
            last_sample: None,
            frames_since_sample: 0,
            header_written: false,
        }
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Called once per editor frame. Cheap when disabled or between samples.
pub fn sample_frame() {
    STATE.with(|cell| {
        let mut s = cell.borrow_mut();
        let enabled = *s.enabled.get_or_insert_with(env_enabled);
        if !enabled {
            return;
        }
        if let Some(secs) = std::env::var("FLASH_DRUM_PERF_LOG_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|&n| n >= 1 && n <= 3600)
        {
            s.interval = Duration::from_secs(secs);
        }
        let now = Instant::now();
        let session_start = *s.session_start.get_or_insert(now);
        s.frames_since_sample += 1;
        let due = s
            .last_sample
            .map_or(true, |last| now.duration_since(last) >= s.interval);
        if !due {
            return;
        }
        let frames = s.frames_since_sample;
        s.frames_since_sample = 0;
        s.last_sample = Some(now);
        let elapsed = now.duration_since(session_start);
        let snap = Snapshot::take();
        let header = !s.header_written;
        s.header_written = true;
        write_line(header, &elapsed, frames, &snap);
    });
}

fn env_enabled() -> bool {
    std::env::var_os("FLASH_DRUM_PERF_LOG").is_some_and(|v| v == "1")
}

struct Snapshot {
    rss_bytes: u64,
    private_bytes: u64,
    handles: u32,
    gdi_objects: u32,
    user_objects: u32,
    cpu_seconds: f64,
}

impl Snapshot {
    #[cfg(target_os = "windows")]
    fn take() -> Self {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, GetGuiResources, GetProcessHandleCount, GetProcessTimes, GR_GDIOBJECTS,
            GR_USEROBJECTS,
        };

        unsafe {
            let proc = GetCurrentProcess();
            let mut counters = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..std::mem::zeroed()
            };
            let mem_ok = GetProcessMemoryInfo(proc, &mut counters, counters.cb) != 0;
            let mut handles = 0u32;
            GetProcessHandleCount(proc, &mut handles);
            let gdi = GetGuiResources(proc, GR_GDIOBJECTS);
            let user = GetGuiResources(proc, GR_USEROBJECTS);
            let mut zero_ft = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut kernel = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let mut user_ft = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            GetProcessTimes(proc, &mut zero_ft, &mut zero_ft, &mut kernel, &mut user_ft);
            let ticks = |ft: FILETIME| ((ft.dwHighDateTime as u64) << 32 | ft.dwLowDateTime as u64) as f64;
            Self {
                rss_bytes: if mem_ok { counters.WorkingSetSize as u64 } else { 0 },
                private_bytes: if mem_ok { counters.PagefileUsage as u64 } else { 0 },
                handles,
                gdi_objects: gdi,
                user_objects: user,
                cpu_seconds: (ticks(kernel) + ticks(user_ft)) / 10_000_000.0,
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    fn take() -> Self {
        Self {
            rss_bytes: 0,
            private_bytes: 0,
            handles: 0,
            gdi_objects: 0,
            user_objects: 0,
            cpu_seconds: 0.0,
        }
    }
}

fn write_line(header: bool, elapsed: &Duration, frames: u64, snap: &Snapshot) {
    let path = std::env::temp_dir().join("flash-drum-perf.log");
    let line = format!(
        "{} t_s={} frames={} rss_mb={:.1} private_mb={:.1} handles={} gdi={} user={} cpu_s={:.1}\n",
        utc_timestamp(),
        elapsed.as_secs(),
        frames,
        snap.rss_bytes as f64 / (1024.0 * 1024.0),
        snap.private_bytes as f64 / (1024.0 * 1024.0),
        snap.handles,
        snap.gdi_objects,
        snap.user_objects,
        snap.cpu_seconds,
    );
    let result = (|| -> std::io::Result<()> {
        use std::io::Write;
        if header {
            // Restart the file if a previous session left a big one behind.
            let oversize = std::fs::metadata(&path).map_or(false, |m| m.len() > MAX_LOG_BYTES);
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(oversize)
                .append(!oversize)
                .open(&path)?;
            writeln!(
                file,
                "# flash-drum perf log (UTC) build {} pid {}",
                option_env!("DRUM_PATTERN_BUILD_ID").unwrap_or("dev"),
                std::process::id()
            )?;
            file.write_all(line.as_bytes())?;
        } else {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(line.as_bytes())?;
        }
        Ok(())
    })();
    if result.is_err() {
        // Never disturb the session over a diagnostic: drop to disabled.
        STATE.with(|cell| cell.borrow_mut().enabled = Some(false));
    }
}

/// UTC `YYYY-MM-DD HH:MM:SS` without pulling in a datetime crate. The civil
/// date math mirrors `presets::format_date` (same Hinnant algorithm).
fn utc_timestamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let day_secs = secs % 86_400;
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
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        m,
        d,
        day_secs / 3600,
        (day_secs % 3600) / 60,
        day_secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_well_formed_utc() {
        let ts = utc_timestamp();
        assert_eq!(ts.len(), 19, "{ts}");
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], " ");
        assert_eq!(&ts[13..14], ":");
        assert!(ts[..4].parse::<u32>().unwrap() >= 2026, "{ts}");
    }

    #[test]
    fn disabled_when_env_absent() {
        // Without the env var, sample_frame is a no-op that writes nothing.
        std::env::remove_var("FLASH_DRUM_PERF_LOG");
        STATE.with(|cell| *cell.borrow_mut() = State::default());
        let path = std::env::temp_dir().join("flash-drum-perf-test-guard.log");
        std::fs::remove_file(&path).ok();
        sample_frame();
        assert!(!path.exists());
    }
}
