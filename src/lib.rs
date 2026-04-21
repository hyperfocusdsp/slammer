//! Slammer — kick drum synthesizer plugin.
//!
//! This crate exposes Slammer as a VST3 + CLAP plugin (via `cdylib`) and as
//! a standalone binary (`slammer-standalone`). The module tree is declared
//! here; `main.rs` is a thin wrapper that calls `run_standalone`.

use nih_plug::prelude::*;

mod export;
mod logging;
mod params;
mod plugin;
mod presets;
mod sequencer;

mod dsp {
    pub mod clap;
    pub mod click;
    pub mod dj_filter;
    pub mod drift;
    pub mod engine;
    pub mod envelope;
    pub mod filter;
    pub mod master_bus;
    pub mod noise;
    pub mod oscillator;
    pub mod saturation;
    pub mod tube;
}

mod ui;

mod util;

#[cfg(target_os = "windows")]
mod windows_standalone;

pub use plugin::Slammer;

nih_export_vst3!(plugin::Slammer);
nih_export_clap!(plugin::Slammer);

/// Entry point for the standalone binary. Called from `src/main.rs`.
///
/// On Windows we probe the default WASAPI output device and forward matching
/// `--sample-rate` / `--period-size` to nih-plug, because its defaults
/// (48 kHz / 512) mismatch most WASAPI mix formats and the backend has no
/// negotiation path. Linux and macOS use nih-plug's default parser unchanged.
pub fn run_standalone() {
    #[cfg(target_os = "windows")]
    {
        if let Some(argv) = windows_standalone::probed_argv() {
            nih_export_standalone_with_args::<plugin::Slammer, _>(argv);
            return;
        }
    }

    nih_export_standalone::<plugin::Slammer>();
}
