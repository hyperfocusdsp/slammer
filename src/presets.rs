//! Factory presets and user preset I/O for Niner.
//!
//! A preset is a `(name, ParamSnapshot)` pair. The snapshot type itself lives
//! in `plugin.rs` next to `NinerParams` so there's a single source of truth
//! for every persisted parameter. This module is only responsible for:
//!
//! * Providing the built-in factory presets.
//! * Reading/writing user presets as JSON in a platform-appropriate data
//!   directory (see `util::paths::niner_preset_dir`).
//! * Exposing a merged factory+user list to the UI.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use crate::params::ParamSnapshot;
use crate::util::paths;

/// Current on-disk preset schema version. Bump on breaking changes.
///
/// v2 introduces `ParamSnapshot` as the storage format and adds fields that
/// were missing from v1 (notably `mid_phase_offset`). v1 files are *not*
/// migrated — this is a pre-release refactor.
const PRESET_VERSION: u32 = 2;

/// Max length for sanitized preset filenames (without `.json` extension).
/// Safe on all mainstream filesystems (Windows limits path components to 255).
const MAX_FILENAME_LEN: usize = 120;

/// On-disk JSON wrapper.
///
/// `#[serde(default)]` on `params` means future field additions to
/// `ParamSnapshot` silently get type defaults rather than failing to load
/// older JSON.
#[derive(Serialize, Deserialize)]
struct PresetFile {
    name: String,
    version: u32,
    #[serde(default)]
    params: ParamSnapshot,
}

/// A preset entry returned from `PresetManager` — name + params + origin.
#[derive(Clone)]
pub struct PresetEntry {
    pub name: String,
    pub params: ParamSnapshot,
    pub is_factory: bool,
}

/// Factory presets — read-only, baked in. Each preset's full param state
/// lives as a JSON file under `assets/factory_presets/` (the same on-disk
/// schema as user presets). The order here is the order they appear in the
/// preset list; tests that index by position assume "Punchy Techno" stays
/// at index 1.
pub fn factory_presets() -> Vec<PresetEntry> {
    const FACTORIES: &[(&str, &str)] = &[
        ("Clean Sub",     include_str!("../assets/factory_presets/Clean_Sub.json")),
        ("Punchy Techno", include_str!("../assets/factory_presets/Punchy_Techno.json")),
        ("909-ish",       include_str!("../assets/factory_presets/909-ish.json")),
        ("808",           include_str!("../assets/factory_presets/808.json")),
        ("909",           include_str!("../assets/factory_presets/909.json")),
        ("hh",            include_str!("../assets/factory_presets/hh.json")),
        ("Init",          include_str!("../assets/factory_presets/Init.json")),
        ("overdose",      include_str!("../assets/factory_presets/overdose.json")),
        ("psy",           include_str!("../assets/factory_presets/psy.json")),
        ("sd1",           include_str!("../assets/factory_presets/sd1.json")),
        ("sd2",           include_str!("../assets/factory_presets/sd2.json")),
        ("tight",         include_str!("../assets/factory_presets/tight.json")),
        ("clap",          include_str!("../assets/factory_presets/clap.json")),
        ("niner",         include_str!("../assets/factory_presets/niner.json")),
    ];
    FACTORIES
        .iter()
        .map(|(name, json)| {
            let pf: PresetFile = serde_json::from_str(json)
                .unwrap_or_else(|e| panic!("factory preset {name} malformed: {e}"));
            PresetEntry {
                name: (*name).to_string(),
                params: pf.params,
                is_factory: true,
            }
        })
        .collect()
}

/// Manages factory + user presets.
pub struct PresetManager {
    factory: Vec<PresetEntry>,
    user: Vec<PresetEntry>,
    /// Factory preset names the user has deleted. Filtered out of
    /// `list_all`. Persisted to `niner_hidden_presets_file()`.
    hidden_factories: HashSet<String>,
    dir: PathBuf,
}

impl PresetManager {
    pub fn new() -> Self {
        let dir = paths::niner_preset_dir();
        let factory = factory_presets();
        let mut mgr = Self {
            factory,
            user: Vec::new(),
            hidden_factories: load_hidden_factories(),
            dir,
        };
        mgr.refresh();
        mgr
    }

    /// Rescan the user preset directory.
    pub fn refresh(&mut self) {
        self.user.clear();
        if let Ok(entries) = fs::read_dir(&self.dir) {
            let mut paths: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .map(|e| e.path())
                .collect();
            paths.sort();
            for path in paths {
                if let Ok(data) = fs::read_to_string(&path) {
                    if let Ok(pf) = serde_json::from_str::<PresetFile>(&data) {
                        self.user.push(PresetEntry {
                            name: pf.name,
                            params: pf.params,
                            is_factory: false,
                        });
                    }
                }
            }
        }
    }

    /// All presets: factory first, then user. A user preset whose name
    /// matches a factory preset (case-insensitive) shadows the factory
    /// entry, so "overwriting" a factory means saving a user copy under
    /// the same name. Factories listed in `hidden_factories` are hidden
    /// unconditionally — that's how pure-factory deletion works.
    /// Deleting a user override (or removing a factory from the hidden
    /// list by saving under the same name) restores the factory entry.
    pub fn list_all(&self) -> Vec<PresetEntry> {
        let mut out: Vec<PresetEntry> = self
            .factory
            .iter()
            .filter(|f| {
                !self.hidden_factories.contains(&f.name)
                    && !self
                        .user
                        .iter()
                        .any(|u| u.name.eq_ignore_ascii_case(&f.name))
            })
            .cloned()
            .collect();
        out.extend(self.user.iter().cloned());
        out
    }

    /// Save a snapshot under `name`. Saving under a factory name writes a
    /// user copy that shadows the factory in `list_all` — the factory
    /// definition itself is never touched, so a `delete` restores it.
    /// If the name is in the hidden-factories list, saving clears it so
    /// the user can resurrect a previously-deleted factory by saving
    /// over it.
    pub fn save(&mut self, name: &str, params: ParamSnapshot) -> Result<(), String> {
        let pf = PresetFile {
            name: name.to_owned(),
            version: PRESET_VERSION,
            params,
        };
        let json = serde_json::to_string_pretty(&pf).map_err(|e| e.to_string())?;
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(format!("{}.json", sanitize_filename(name)));
        fs::write(&path, json).map_err(|e| e.to_string())?;
        if self.hidden_factories.remove(name) {
            persist_hidden_factories(&self.hidden_factories);
        }
        self.refresh();
        Ok(())
    }

    /// Delete a preset by name. Two cases:
    ///
    /// * If a user file exists for this name, remove it. If the name
    ///   also matches a factory preset, the factory entry reappears in
    ///   `list_all` on the next refresh.
    /// * Otherwise, if the name matches a factory preset, add it to
    ///   `hidden_factories` and persist. The factory definition is
    ///   baked into the binary, so hiding is the only sense in which
    ///   a factory can be "deleted".
    pub fn delete(&mut self, name: &str) -> Result<(), String> {
        if self.user.iter().any(|e| e.name == name) {
            let path = self.dir.join(format!("{}.json", sanitize_filename(name)));
            fs::remove_file(&path).map_err(|e| e.to_string())?;
            self.refresh();
            return Ok(());
        }
        if self.factory.iter().any(|e| e.name == name) {
            self.hidden_factories.insert(name.to_owned());
            persist_hidden_factories(&self.hidden_factories);
            return Ok(());
        }
        Err(format!("\"{name}\" not found"))
    }
}

/// Load the set of factory preset names the user has explicitly hidden.
/// Missing file → empty set. IO errors are logged and swallowed.
fn load_hidden_factories() -> HashSet<String> {
    let path = paths::niner_hidden_presets_file();
    let Ok(data) = fs::read_to_string(&path) else {
        return HashSet::new();
    };
    data.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// Write the hidden-factories set to disk, one name per line. Best-effort;
/// IO errors are logged and swallowed so a read-only config dir never
/// crashes the plugin.
fn persist_hidden_factories(set: &HashSet<String>) {
    let path = paths::niner_hidden_presets_file();
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(?e, "failed to create hidden-presets dir");
            return;
        }
    }
    let mut lines: Vec<&str> = set.iter().map(String::as_str).collect();
    lines.sort_unstable();
    let body = lines.join("\n");
    if let Err(e) = fs::write(&path, body) {
        tracing::warn!(?e, ?path, "failed to write hidden-presets file");
    }
}

/// Remember the name of the last-selected preset so the standalone reopens
/// with the same choice. Best-effort — IO errors are logged and swallowed.
pub fn save_last_preset_name(name: &str) {
    let path = paths::niner_last_preset_file();
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(?e, "failed to create last-preset dir");
            return;
        }
    }
    if let Err(e) = fs::write(&path, name) {
        tracing::warn!(?e, ?path, "failed to write last-preset file");
    }
}

/// Read back the last-selected preset name, if any.
pub fn load_last_preset_name() -> Option<String> {
    let path = paths::niner_last_preset_file();
    let data = fs::read_to_string(&path).ok()?;
    let trimmed = data.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Make a string safe to use as a filename component on Linux, macOS, and
/// Windows. Disallows path separators, shell-ish metachars, and caps length.
fn sanitize_filename(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim()
        .replace(' ', "_");

    // Cap length to stay well under filesystem limits (Windows: 255).
    // Truncating on a char boundary — safe because all retained chars are
    // single-byte ASCII after the filter above.
    if cleaned.len() > MAX_FILENAME_LEN {
        cleaned[..MAX_FILENAME_LEN].to_string()
    } else if cleaned.is_empty() {
        "untitled".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_separators() {
        assert_eq!(sanitize_filename("../etc/passwd"), "___etc_passwd");
        assert_eq!(sanitize_filename("a\\b"), "a_b");
    }

    #[test]
    fn sanitize_replaces_spaces_with_underscore() {
        assert_eq!(sanitize_filename("my kick"), "my_kick");
    }

    #[test]
    fn sanitize_caps_length() {
        let long = "a".repeat(500);
        assert_eq!(sanitize_filename(&long).len(), MAX_FILENAME_LEN);
    }

    #[test]
    fn sanitize_empty_becomes_untitled() {
        assert_eq!(sanitize_filename(""), "untitled");
        assert_eq!(sanitize_filename("   "), "untitled");
    }

    #[test]
    fn factory_presets_load_cleanly() {
        // Every JSON in `assets/factory_presets/` parses, the list is
        // non-empty, and "Punchy Techno" stays at index 1 (round-trip test
        // depends on it).
        let presets = factory_presets();
        assert!(presets.len() >= 10);
        assert_eq!(presets[1].name, "Punchy Techno");
    }

    /// Round-trip: a snapshot serialized to JSON and read back equals itself.
    /// This is the poor-man's version of the full capture → apply → capture
    /// round-trip (which needs a real `ParamSetter` wired to `NinerParams`
    /// and is covered by in-host manual testing). The serde path is the only
    /// part that can break silently when fields are added.
    #[test]
    fn param_snapshot_json_round_trip() {
        let original = factory_presets().remove(1).params; // "Punchy Techno"
        let pf = PresetFile {
            name: "round-trip".into(),
            version: PRESET_VERSION,
            params: original.clone(),
        };
        let json = serde_json::to_string(&pf).unwrap();
        let decoded: PresetFile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.version, PRESET_VERSION);
        assert_eq!(decoded.params, original);
    }

    /// Older JSON missing a field (e.g. pre-v2 files without `mid_phase_offset`)
    /// should load with the field defaulted to 0.0, thanks to `#[serde(default)]`.
    #[test]
    fn param_snapshot_tolerates_missing_fields() {
        let json = r#"{
            "name": "old",
            "version": 1,
            "params": { "decay_ms": 250.0 }
        }"#;
        let pf: PresetFile = serde_json::from_str(json).unwrap();
        assert_eq!(pf.params.decay_ms, 250.0);
        assert_eq!(pf.params.mid_phase_offset, 0.0);
        assert_eq!(pf.params.sub_gain, 0.0);
    }
}
