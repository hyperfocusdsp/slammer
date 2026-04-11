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
    pub mod click;
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

pub use plugin::Slammer;

nih_export_vst3!(plugin::Slammer);
nih_export_clap!(plugin::Slammer);

/// Entry point for the standalone binary. Called from `src/main.rs`.
pub fn run_standalone() {
    nih_export_standalone::<plugin::Slammer>();
}
