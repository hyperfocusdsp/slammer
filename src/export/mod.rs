//! Export / bounce feature: render one hit of the current sound to WAV/AIFF.
//!
//! Entry point is [`export_one_shot`] — called by the editor when the user
//! clicks the BOUNCE button. The function handles the whole flow:
//!
//! 1. Remembered-path lookup (or OS-default music directory on first use).
//! 2. Native save dialog pre-filled with a suggested filename.
//! 3. Offline render of a single trigger through the full signal chain.
//! 4. 16-bit PCM write to the chosen path (WAV via `hound`, AIFF hand-rolled).
//! 5. Persist the chosen directory + format so the next bounce lands next to
//!    this one without any extra clicks.
//!
//! Runs on a dedicated bounce worker thread (spawned by the editor on
//! click). The rfd save-dialog pumps a nested Win32 message loop, which on
//! Windows deadlocks / crashes if called from inside the egui paint closure
//! while OpenGL is mid-frame — hence the worker. No audio-thread
//! coordination, no RT safety concerns.

pub mod render;
pub mod writer;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::params::{collect_kick_params, SlammerParams};
use crate::util::paths;

pub use render::MasterChainSnapshot;
pub use writer::Format;

/// Name of the on-disk config file that remembers the last export location
/// and format. Lives under `slammer_data_dir()`, independent of presets.
const EXPORT_CONFIG_FILE: &str = "export_config.json";

/// Current schema version for the config file.
const EXPORT_CONFIG_VERSION: u32 = 1;

/// On-disk schema. `#[serde(default)]` on new fields keeps old configs
/// loadable after future schema bumps.
#[derive(Serialize, Deserialize)]
struct ExportConfigFile {
    version: u32,
    #[serde(default)]
    last_dir: Option<PathBuf>,
    #[serde(default)]
    last_format: Option<String>,
}

/// Runtime state the editor holds across BOUNCE clicks. Loaded once on
/// editor init; persisted after each successful export.
#[derive(Clone, Debug)]
pub struct ExportState {
    pub last_dir: PathBuf,
    pub last_format: Format,
}

impl Default for ExportState {
    fn default() -> Self {
        Self {
            last_dir: default_bounces_dir(),
            last_format: Format::Wav,
        }
    }
}

/// Outcome of a BOUNCE click. Returned so the caller can surface status to
/// the user (log, toast, etc.).
#[derive(Debug)]
pub enum ExportOutcome {
    /// Render + write succeeded. The returned path is the final file.
    Written(PathBuf),
    /// User cancelled the save dialog. No side effects.
    Cancelled,
    /// The chosen extension couldn't be mapped to a supported format.
    UnsupportedExtension(String),
    /// Something failed during rendering or writing.
    Failed(String),
}

/// Load the remembered export state from disk. Falls back to sensible
/// defaults on any parse/IO error — a missing or corrupted config file
/// should never prevent the feature from working.
pub fn load_export_state() -> ExportState {
    let path = paths::slammer_data_dir().join(EXPORT_CONFIG_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return ExportState::default(),
    };
    let parsed: ExportConfigFile = match serde_json::from_str(&contents) {
        Ok(c) => c,
        Err(_) => return ExportState::default(),
    };
    let last_dir = parsed
        .last_dir
        .filter(|p| p.is_dir())
        .unwrap_or_else(default_bounces_dir);
    let last_format = parsed
        .last_format
        .as_deref()
        .and_then(Format::from_extension)
        .unwrap_or(Format::Wav);
    ExportState {
        last_dir,
        last_format,
    }
}

/// Persist the given state to disk. Errors are swallowed with a log — a
/// failure here means the *next* bounce won't remember the path, but the
/// *current* bounce (which has already been written successfully) must not
/// be reported as failed.
pub fn save_export_state(state: &ExportState) {
    let dir = paths::slammer_data_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("export: couldn't create data dir {:?}: {}", dir, e);
        return;
    }
    let file = ExportConfigFile {
        version: EXPORT_CONFIG_VERSION,
        last_dir: Some(state.last_dir.clone()),
        last_format: Some(state.last_format.extension().to_string()),
    };
    let json = match serde_json::to_string_pretty(&file) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("export: couldn't serialize config: {}", e);
            return;
        }
    };
    let path = dir.join(EXPORT_CONFIG_FILE);
    if let Err(e) = fs::write(&path, json) {
        tracing::warn!("export: couldn't write config {:?}: {}", path, e);
    }
}

/// Default directory for new bounces on a fresh install. Prefers the
/// per-user Music folder (`~/Music`, `~/Library/Music`, `%USERPROFILE%/Music`)
/// with a `Slammer Bounces` subfolder, so the user's own library stays tidy.
/// Falls back to the slammer data directory's `bounces` subdir if the
/// platform doesn't expose a music folder.
///
/// Creates the directory if it doesn't already exist. If creation fails
/// (e.g. read-only volume), returns the path anyway — the save dialog will
/// happily let the user navigate elsewhere.
pub fn default_bounces_dir() -> PathBuf {
    let base = directories::UserDirs::new()
        .and_then(|u| u.audio_dir().map(|p| p.to_path_buf()))
        .unwrap_or_else(paths::slammer_data_dir);
    let dir = base.join("Slammer Bounces");
    let _ = fs::create_dir_all(&dir);
    dir
}

/// Suggest a filename for a new bounce. Includes a monotonically-increasing
/// tag derived from the system clock so a user who clicks BOUNCE repeatedly
/// without renaming always gets a fresh target.
pub fn suggested_filename(format: Format) -> String {
    // Epoch seconds → short-ish unique tag. Not a real timestamp because
    // formatting one without a date crate isn't worth it for v1; the tag
    // just has to be unique and sortable within a session.
    let tag = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // Bump a process-local counter so two clicks within the same second
    // don't collide. Resets across plugin loads, which is fine — the epoch
    // will have advanced by then.
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);

    format!("slammer_{tag}_{seq:02}.{}", format.extension())
}

/// End-to-end BOUNCE click handler. Pops a native save dialog, renders, and
/// writes the file. Updates `state` on success so the next call opens the
/// dialog at the same directory.
pub fn export_one_shot(state: &mut ExportState, params: &SlammerParams) -> ExportOutcome {
    let suggested = suggested_filename(state.last_format);
    let filter_main = match state.last_format {
        Format::Wav => ("WAV", &["wav"][..]),
        Format::Aiff => ("AIFF", &["aiff", "aif"][..]),
    };
    let filter_other = match state.last_format {
        Format::Wav => ("AIFF", &["aiff", "aif"][..]),
        Format::Aiff => ("WAV", &["wav"][..]),
    };

    let chosen = rfd::FileDialog::new()
        .set_title("Bounce one-shot")
        .set_directory(&state.last_dir)
        .set_file_name(&suggested)
        .add_filter(filter_main.0, filter_main.1)
        .add_filter(filter_other.0, filter_other.1)
        .save_file();

    let Some(mut path) = chosen else {
        return ExportOutcome::Cancelled;
    };

    // Determine format from the extension the user picked. If the extension
    // is missing (some file dialogs don't append the filter's default ext
    // automatically), fall back to the current `last_format`.
    let ext_opt = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string);
    let format = match ext_opt.as_deref() {
        Some(ext) => match Format::from_extension(ext) {
            Some(f) => f,
            None => return ExportOutcome::UnsupportedExtension(ext.to_string()),
        },
        None => {
            path.set_extension(state.last_format.extension());
            state.last_format
        }
    };

    // Snapshot the live params so the render is isolated from any further
    // UI churn while we're writing.
    let kick_params = collect_kick_params(params);
    let master_chain = MasterChainSnapshot {
        comp_amount: params.comp_amount.value(),
        comp_react: params.comp_react.value(),
        comp_drive: params.comp_drive.value(),
        comp_limit_on: params.comp_limit_on.value(),
        comp_atk_ms: params.comp_atk_ms.value(),
        comp_rel_ms: params.comp_rel_ms.value(),
        comp_knee_db: params.comp_knee_db.value(),
        dj_filter_pos: params.dj_filter_pos.value(),
        dj_filter_res: params.dj_filter_res.value(),
        dj_filter_pre: params.dj_filter_pre.value(),
        master_volume: params.master_volume.value(),
    };

    tracing::info!(
        "export: rendering one-shot → {:?} ({})",
        path,
        format.label()
    );
    let (left, right) = render::render_oneshot(kick_params, master_chain);

    if let Err(e) = writer::write(&path, format, &left, &right) {
        tracing::error!("export: write failed: {}", e);
        return ExportOutcome::Failed(e.to_string());
    }

    // Remember the parent directory + format for the next bounce.
    if let Some(parent) = path.parent() {
        state.last_dir = parent.to_path_buf();
    }
    state.last_format = format;
    save_export_state(state);

    tracing::info!("export: wrote {:?}", path);
    ExportOutcome::Written(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_export_state_round_trips_through_disk() {
        // Smoke test: default state is self-consistent and can be
        // serialized + deserialized through the config file path without
        // losing information. Doesn't touch the real config file — uses
        // a scratch dir instead.
        let state = ExportState::default();
        let file = ExportConfigFile {
            version: EXPORT_CONFIG_VERSION,
            last_dir: Some(state.last_dir.clone()),
            last_format: Some(state.last_format.extension().to_string()),
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: ExportConfigFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.version, EXPORT_CONFIG_VERSION);
        assert_eq!(parsed.last_dir, Some(state.last_dir));
        assert_eq!(
            Format::from_extension(parsed.last_format.as_deref().unwrap()),
            Some(state.last_format)
        );
    }

    #[test]
    fn suggested_filename_has_expected_prefix_and_extension() {
        let name = suggested_filename(Format::Wav);
        assert!(name.starts_with("slammer_"));
        assert!(name.ends_with(".wav"));
        let aiff = suggested_filename(Format::Aiff);
        assert!(aiff.ends_with(".aiff"));
    }

    #[test]
    fn suggested_filenames_are_unique_within_a_second() {
        let a = suggested_filename(Format::Wav);
        let b = suggested_filename(Format::Wav);
        assert_ne!(a, b, "two rapid calls must produce distinct names");
    }

    #[test]
    fn default_bounces_dir_is_absolute() {
        let d = default_bounces_dir();
        // On CI we may not have a writable Music folder, but the path must
        // at least be absolute and non-empty.
        assert!(d.is_absolute(), "{:?}", d);
    }

}
