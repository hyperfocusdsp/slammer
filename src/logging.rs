//! File-based logging for Slammer via `tracing`.
//!
//! Writes to a platform-appropriate data directory (see `util::paths`).
//! Initialization is idempotent (`Once`), panic-free, and any failure is
//! surfaced to `stderr` as a last resort — the plugin will continue to run
//! without file logging rather than crashing the host.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Once;

use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

use crate::util::paths;

/// Rotate (truncate from the head) once the log file exceeds this many bytes.
const LOG_ROTATE_BYTES: u64 = 500_000;
/// When rotating, keep at most this many trailing lines.
const LOG_ROTATE_KEEP_LINES: usize = 5_000;

static INIT: Once = Once::new();

/// Initialize file logging. Safe to call multiple times — only the first call
/// has any effect. Errors are reported to stderr and otherwise swallowed; the
/// plugin must never panic just because it can't open a log file.
pub fn init() {
    INIT.call_once(|| {
        if let Err(e) = try_init() {
            eprintln!("Slammer: logging init failed: {e}");
        }
    });
}

fn try_init() -> Result<(), String> {
    let dir = paths::slammer_log_dir();
    fs::create_dir_all(&dir).map_err(|e| format!("create log dir {dir:?}: {e}"))?;

    let log_path = dir.join("slammer.log");
    rotate_if_needed(&log_path);

    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("open log file {log_path:?}: {e}"))?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        if cfg!(debug_assertions) {
            EnvFilter::new("slammer=debug,slammer_standalone=debug")
        } else {
            EnvFilter::new("slammer=info,slammer_standalone=info")
        }
    });

    // The writer closure is called repeatedly by tracing-subscriber. If the
    // file handle can't be cloned for some reason (resource limits, OS quirk),
    // fall back to a no-op sink rather than panicking.
    let file_layer = fmt::layer()
        .with_writer(move || SafeWriter::from_clone(&file))
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(true)
        .with_level(true);

    let subscriber = tracing_subscriber::registry().with(filter).with(file_layer);

    // set_global_default fails if a subscriber is already set (e.g. in tests).
    // That's not fatal for us — just move on.
    let _ = tracing::subscriber::set_global_default(subscriber);
    tracing::info!("Slammer logger initialized — log file: {:?}", log_path);
    Ok(())
}

fn rotate_if_needed(log_path: &Path) {
    let Ok(meta) = fs::metadata(log_path) else {
        return;
    };
    if meta.len() <= LOG_ROTATE_BYTES {
        return;
    }
    let Ok(content) = fs::read_to_string(log_path) else {
        return;
    };
    let lines: Vec<&str> = content.lines().collect();
    let keep = if lines.len() > LOG_ROTATE_KEEP_LINES {
        &lines[lines.len() - LOG_ROTATE_KEEP_LINES..]
    } else {
        &lines[..]
    };
    let _ = fs::write(log_path, keep.join("\n"));
}

/// Tracing writer that gracefully degrades to a sink if the log file handle
/// cannot be cloned. Never panics.
enum SafeWriter {
    File(File),
    Sink,
}

impl SafeWriter {
    fn from_clone(file: &File) -> Self {
        match file.try_clone() {
            Ok(f) => SafeWriter::File(f),
            Err(_) => SafeWriter::Sink,
        }
    }
}

impl Write for SafeWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            SafeWriter::File(f) => f.write(buf),
            SafeWriter::Sink => Ok(buf.len()),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            SafeWriter::File(f) => f.flush(),
            SafeWriter::Sink => Ok(()),
        }
    }
}

/// Return the log file path (for diagnostics / about dialog).
#[allow(dead_code)]
pub fn log_file_path() -> PathBuf {
    paths::slammer_log_dir().join("slammer.log")
}
