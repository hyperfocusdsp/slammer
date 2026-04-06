//! Platform-aware data directories for Slammer.
//!
//! Resolves per-OS locations for presets, logs, and any other on-disk state,
//! using the `directories` crate so Linux, macOS, and Windows all land in
//! the conventional places:
//!
//! | OS      | Data root                                  |
//! |---------|--------------------------------------------|
//! | Linux   | `$XDG_DATA_HOME/slammer` or `~/.local/share/slammer` |
//! | macOS   | `~/Library/Application Support/slammer`    |
//! | Windows | `%APPDATA%\slammer\data`                   |
//!
//! All accessors fall back to `std::env::temp_dir().join("slammer")` if the
//! platform dirs can't be resolved (extremely rare — no `$HOME`, no
//! `%APPDATA%`). They never panic.

use std::path::PathBuf;

const ORG_QUALIFIER: &str = "";
const ORG: &str = "Slammer";
const APP: &str = "slammer";

/// Root directory for all Slammer on-disk state.
pub fn slammer_data_dir() -> PathBuf {
    directories::ProjectDirs::from(ORG_QUALIFIER, ORG, APP)
        .map(|p| p.data_dir().to_path_buf())
        .unwrap_or_else(fallback_dir)
}

/// Directory for user preset JSON files.
pub fn slammer_preset_dir() -> PathBuf {
    slammer_data_dir().join("presets")
}

/// Directory for log files.
pub fn slammer_log_dir() -> PathBuf {
    slammer_data_dir().join("logs")
}

/// File that stores the name of the last-selected preset, so the standalone
/// reopens with the same preset the user was last using. Plain text, one
/// line. Plugin mode never touches this — DAWs handle state themselves.
pub fn slammer_last_preset_file() -> PathBuf {
    slammer_data_dir().join("last_preset.txt")
}

/// File listing factory preset names the user has explicitly deleted.
/// One name per line. Factory presets are baked into the binary, so the
/// only way to "delete" one is to record its name here and filter it
/// out of `list_all` at runtime.
pub fn slammer_hidden_presets_file() -> PathBuf {
    slammer_data_dir().join("hidden_presets.txt")
}

fn fallback_dir() -> PathBuf {
    std::env::temp_dir().join("slammer")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_dir_is_absolute_and_ends_with_slammer() {
        let p = slammer_data_dir();
        assert!(p.is_absolute(), "data dir should be absolute: {:?}", p);
        // On all platforms the last component should contain "slammer"
        // (case-insensitive — macOS uses "slammer" lowercase too).
        let last = p
            .components()
            .next_back()
            .and_then(|c| c.as_os_str().to_str())
            .unwrap_or("");
        assert!(
            last.to_lowercase().contains("slammer"),
            "expected slammer in last component, got {:?}",
            p
        );
    }

    #[test]
    fn preset_dir_is_under_data_dir() {
        assert!(slammer_preset_dir().starts_with(slammer_data_dir()));
    }

    #[test]
    fn log_dir_is_under_data_dir() {
        assert!(slammer_log_dir().starts_with(slammer_data_dir()));
    }
}
