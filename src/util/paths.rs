//! Platform-aware data directories for Niner.
//!
//! Resolves per-OS locations for presets, logs, and any other on-disk state,
//! using the `directories` crate so Linux, macOS, and Windows all land in
//! the conventional places:
//!
//! | OS      | Data root                                  |
//! |---------|--------------------------------------------|
//! | Linux   | `$XDG_DATA_HOME/niner` or `~/.local/share/niner` |
//! | macOS   | `~/Library/Application Support/niner`      |
//! | Windows | `%APPDATA%\niner\data`                     |
//!
//! All accessors fall back to `std::env::temp_dir().join("niner")` if the
//! platform dirs can't be resolved (extremely rare — no `$HOME`, no
//! `%APPDATA%`). They never panic.

use std::path::PathBuf;

const ORG_QUALIFIER: &str = "";
const ORG: &str = "Niner";
const APP: &str = "niner";

/// Root directory for all Niner on-disk state.
pub fn niner_data_dir() -> PathBuf {
    directories::ProjectDirs::from(ORG_QUALIFIER, ORG, APP)
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(fallback_dir)
}

/// Directory for user preset JSON files.
pub fn niner_preset_dir() -> PathBuf {
    niner_data_dir().join("presets")
}

/// Directory for log files.
pub fn niner_log_dir() -> PathBuf {
    niner_data_dir().join("logs")
}

/// File that stores the name of the last-selected preset, so the standalone
/// reopens with the same preset the user was last using. Plain text, one
/// line. Plugin mode never touches this — DAWs handle state themselves.
pub fn niner_last_preset_file() -> PathBuf {
    niner_data_dir().join("last_preset.txt")
}

/// File listing factory preset names the user has explicitly deleted.
/// One name per line. Factory presets are baked into the binary, so the
/// only way to "delete" one is to record its name here and filter it
/// out of `list_all` at runtime.
pub fn niner_hidden_presets_file() -> PathBuf {
    niner_data_dir().join("hidden_presets.txt")
}

/// Sidecar file storing the user's UI scale preference (`1.0` / `1.5` / `2.0`).
/// nih-plug's `#[persist]` round-trips the value within DAW projects, but the
/// standalone wrapper doesn't reuse that machinery between launches, so we
/// also serialize the chosen scale to disk and have `niner-launch` forward
/// it to the standalone via `--dpi-scale`.
pub fn niner_ui_scale_file() -> PathBuf {
    niner_data_dir().join("ui_scale.txt")
}

/// Read the persisted UI scale. Returns `1.0` if the file is missing, the
/// contents are unparseable, or the value falls outside the supported
/// `[1.0, 2.0]` range.
pub fn load_ui_scale() -> f32 {
    std::fs::read_to_string(niner_ui_scale_file())
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .map(|v| v.clamp(1.0, 2.0))
        .unwrap_or(1.0)
}

/// Persist the UI scale to disk. Silently ignores IO failures — losing a
/// preference is never worth a panic on the GUI thread.
pub fn save_ui_scale(scale: f32) {
    let path = niner_ui_scale_file();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, format!("{scale}\n"));
}

/// One-shot migration from the legacy Slammer data directory to the new
/// Niner one. Renames the old directory to the new path on first run after
/// the rebrand. Idempotent: returns immediately if the new dir already
/// exists, and silently no-ops if the legacy dir doesn't exist either.
/// Never panics.
pub fn migrate_legacy_data_dir() {
    let new_dir = niner_data_dir();
    if new_dir.exists() {
        return;
    }
    if let Some(old) = directories::ProjectDirs::from("", "Slammer", "slammer") {
        let old_dir = old.data_dir().to_path_buf();
        if old_dir.exists() {
            if let Some(parent) = new_dir.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::rename(&old_dir, &new_dir) {
                Ok(_) => tracing::info!(
                    "migrated legacy slammer data dir → {}",
                    new_dir.display()
                ),
                Err(e) => tracing::warn!("legacy data dir migration failed: {e}"),
            }
        }
    }
}

fn fallback_dir() -> PathBuf {
    std::env::temp_dir().join("niner")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_absolute_and_ends_with_niner() {
        let p = niner_data_dir();
        assert!(p.is_absolute(), "data dir should be absolute: {:?}", p);
        // On all platforms the last component should contain "niner"
        // (case-insensitive — macOS uses "niner" lowercase too).
        let last = p
            .components()
            .next_back()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        assert!(
            last.to_lowercase().contains("niner"),
            "expected niner in last component, got {:?}",
            p
        );
    }

    #[test]
    fn preset_dir_is_under_data_dir() {
        assert!(niner_preset_dir().starts_with(niner_data_dir()));
    }

    #[test]
    fn log_dir_is_under_data_dir() {
        assert!(niner_log_dir().starts_with(niner_data_dir()));
    }
}
