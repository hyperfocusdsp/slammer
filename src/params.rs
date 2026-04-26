//! All parameter definitions for Slammer.
//!
//! This module owns two closely-related types:
//!
//! * [`SlammerParams`] — the live, host-visible nih_plug parameter tree.
//! * [`ParamSnapshot`] — a plain-data mirror used for preset round-tripping.
//!
//! Adding a new automatable parameter means touching exactly three places
//! in this file:
//!
//! 1. A field on `SlammerParams` (with its `#[id = "..."]` attribute).
//! 2. Its construction in `impl Default for SlammerParams`.
//! 3. A field on `ParamSnapshot` plus matching lines in `capture`/`apply`.
//!
//! Builder helpers (`hz_knob`, `ms_knob`, `pct_knob`, `db_knob`) collapse
//! the most repetitive boilerplate in the `Default` impl so the parameter
//! definitions read as a flat table rather than a wall of chained
//! `FloatParam::new(...).with_unit(...).with_value_to_string(...)` calls.

use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::dsp::engine::KickParams;
use crate::sequencer::{DEFAULT_ACCENT_BITS, DEFAULT_STEP_BITS};

// ---------------------------------------------------------------------------
// Param-builder helpers
// ---------------------------------------------------------------------------

/// Frequency knob in Hz, rounded integer display, skewed range.
fn hz_knob(name: &str, default: f32, min: f32, max: f32, skew: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(skew),
        },
    )
    .with_unit(" Hz")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

/// Time knob in milliseconds, rounded integer display, skewed range.
fn ms_knob(name: &str, default: f32, min: f32, max: f32, skew: f32) -> FloatParam {
    FloatParam::new(
        name,
        default,
        FloatRange::Skewed {
            min,
            max,
            factor: FloatRange::skew_factor(skew),
        },
    )
    .with_unit(" ms")
    .with_value_to_string(formatters::v2s_f32_rounded(0))
}

/// Linear 0..1 knob displayed as a whole-number percentage.
fn pct_knob(name: &str, default: f32) -> FloatParam {
    FloatParam::new(name, default, FloatRange::Linear { min: 0.0, max: 1.0 })
        .with_value_to_string(Arc::new(|v| format!("{:.0}%", v * 100.0)))
}

/// Gain knob with a dB-skewed range and dB display/parse round-trip.
fn db_knob(name: &str, default_db: f32, min_db: f32, max_db: f32) -> FloatParam {
    FloatParam::new(
        name,
        util::db_to_gain(default_db),
        FloatRange::Skewed {
            min: util::db_to_gain(min_db),
            max: util::db_to_gain(max_db),
            factor: FloatRange::gain_skew_factor(min_db, max_db),
        },
    )
    .with_smoother(SmoothingStyle::Logarithmic(10.0))
    .with_unit(" dB")
    .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
    .with_string_to_value(formatters::s2v_f32_gain_to_db())
}

// ---------------------------------------------------------------------------
// Live parameter tree
// ---------------------------------------------------------------------------

#[derive(Params)]
pub struct SlammerParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    /// 16-step sequencer pattern as a bitmask (bit i = step i on).
    /// Persisted by nih-plug for both DAW project state and standalone
    /// session state. The audio thread never touches this — the UI
    /// thread keeps it in sync with the live `Sequencer` atomics via
    /// `toggle_step` / `set_step`, and `Plugin::initialize()` copies it
    /// back into the atomics on load.
    #[persist = "seq_steps"]
    pub seq_steps: Arc<Mutex<u16>>,

    /// 909-style accent bits, parallel to `seq_steps`. v0.5.x sessions
    /// have no `seq_accents` payload; nih-plug deserialization falls back
    /// to the field's `Default` (zero), so old patterns load with no
    /// accents marked — backward-compatible.
    #[persist = "seq_accents"]
    pub seq_accents: Arc<Mutex<u16>>,

    /// Editor display scale: `1.0`, `1.5`, or `2.0`. The footer "UI N×"
    /// badge cycles this value and mirrors it to a sidecar file (see
    /// `util::paths::save_ui_scale`). nih-plug serialises it inside DAW
    /// projects via `#[persist]`; the standalone wrapper reads the
    /// sidecar at launch and forwards it as `--dpi-scale` so the
    /// next-opened window comes up at the requested size.
    #[persist = "ui-scale-v1"]
    pub ui_scale: Arc<Mutex<f32>>,

    #[id = "master_vol"]
    pub master_volume: FloatParam,

    #[id = "decay"]
    pub decay_ms: FloatParam,

    // --- SUB layer ---
    #[id = "sub_gain"]
    pub sub_gain: FloatParam,

    #[id = "sub_fstart"]
    pub sub_fstart: FloatParam,

    #[id = "sub_fend"]
    pub sub_fend: FloatParam,

    #[id = "sub_sweep"]
    pub sub_sweep_ms: FloatParam,

    #[id = "sub_curve"]
    pub sub_sweep_curve: FloatParam,

    /// Sub phase offset in degrees (DSP layer converts to radians).
    #[id = "sub_phase"]
    pub sub_phase_offset: FloatParam,

    // --- MID layer ---
    #[id = "mid_gain"]
    pub mid_gain: FloatParam,

    #[id = "mid_fstart"]
    pub mid_fstart: FloatParam,

    #[id = "mid_fend"]
    pub mid_fend: FloatParam,

    #[id = "mid_sweep"]
    pub mid_sweep_ms: FloatParam,

    #[id = "mid_curve"]
    pub mid_sweep_curve: FloatParam,

    /// Mid phase offset in degrees (DSP layer converts to radians).
    #[id = "mid_phase"]
    pub mid_phase_offset: FloatParam,

    #[id = "mid_decay"]
    pub mid_decay_ms: FloatParam,

    #[id = "mid_tone"]
    pub mid_tone_gain: FloatParam,

    #[id = "mid_noise"]
    pub mid_noise_gain: FloatParam,

    #[id = "mid_noise_col"]
    pub mid_noise_color: FloatParam,

    /// Decay time for the MID noise channel's own envelope. Real 909 kicks
    /// gate noise to a short attack burst (15-30 ms); legacy slammer ran
    /// noise off `mid_decay_ms` so it sustained alongside the tone.
    #[id = "mid_noise_dec"]
    pub mid_noise_decay_ms: FloatParam,

    // --- TOP layer ---
    #[id = "top_gain"]
    pub top_gain: FloatParam,

    #[id = "top_decay"]
    pub top_decay_ms: FloatParam,

    #[id = "top_freq"]
    pub top_freq: FloatParam,

    #[id = "top_bw"]
    pub top_bw: FloatParam,

    #[id = "top_metal"]
    pub top_metal: FloatParam,

    // --- Drift ---
    #[id = "drift"]
    pub drift_amount: FloatParam,

    /// 909-style accent gain. At 0 the per-step accent flag is a no-op;
    /// at 1 an accented hit is ~30% louder and decays ~50% longer. The
    /// per-step flag itself is in the sequencer's accent bits, not here.
    #[id = "accent_amt"]
    pub accent_amount: FloatParam,

    // --- Saturation ---
    #[id = "sat_mode"]
    pub sat_mode: FloatParam,

    #[id = "sat_drive"]
    pub sat_drive: FloatParam,

    #[id = "sat_mix"]
    pub sat_mix: FloatParam,

    // --- Per-voice soft-clip (pre-amp-envelope, separate from master sat) ---
    /// Voice-clip mode (0=Off, 1=Tanh, 2=Diode, 3=Cubic). Default 0 keeps
    /// every v0.5.x preset bit-identical at load time.
    #[id = "kick_clip_mode"]
    pub kick_clip_mode: FloatParam,

    /// Voice-clip drive in [0, 1]. At 0 the shaper is identity for every
    /// mode, so this param is opt-in.
    #[id = "kick_clip_drive"]
    pub kick_clip_drive: FloatParam,

    // --- Master EQ ---
    #[id = "eq_tilt"]
    pub eq_tilt_db: FloatParam,

    #[id = "eq_low"]
    pub eq_low_boost_db: FloatParam,

    #[id = "eq_notch_f"]
    pub eq_notch_freq: FloatParam,

    #[id = "eq_notch_q"]
    pub eq_notch_q: FloatParam,

    #[id = "eq_notch_d"]
    pub eq_notch_depth_db: FloatParam,

    // --- Compressor / Limiter (master bus dynamics) ---
    #[id = "comp_amount"]
    pub comp_amount: FloatParam,

    #[id = "comp_react"]
    pub comp_react: FloatParam,

    #[id = "comp_drive"]
    pub comp_drive: FloatParam,

    #[id = "comp_limit"]
    pub comp_limit_on: BoolParam,

    /// Precise compressor attack in ms. The RCT macro writes this (and
    /// `comp_rel_ms`) when dragged; the DSP reads this field directly, so
    /// once the user touches ATK/REL individually they diverge from RCT.
    #[id = "comp_atk"]
    pub comp_atk_ms: FloatParam,

    #[id = "comp_rel"]
    pub comp_rel_ms: FloatParam,

    /// Soft-knee width in dB. 0 = hard knee.
    #[id = "comp_knee"]
    pub comp_knee_db: FloatParam,

    // --- Clap (909-style parallel layer) ---
    #[id = "clap_on"]
    pub clap_on: BoolParam,

    #[id = "clap_level"]
    pub clap_level: FloatParam,

    #[id = "clap_freq"]
    pub clap_freq: FloatParam,

    #[id = "clap_tail"]
    pub clap_tail_ms: FloatParam,

    // --- DJ Filter (master bus) ---
    #[id = "dj_filt_pos"]
    pub dj_filter_pos: FloatParam,

    #[id = "dj_filt_res"]
    pub dj_filter_res: FloatParam,

    #[id = "dj_filt_pre"]
    pub dj_filter_pre: BoolParam,
}

/// Snapshot the currently-smoothed engine parameters into a flat `KickParams`
/// struct. Called once per audio block by `plugin.rs` and on-demand by the
/// offline bounce render in `export/` — centralised here so the two share a
/// single source of truth.
///
/// `master_gain` is intentionally pinned to `1.0`: the master volume knob is
/// applied **after** the engine in the plugin chain (post-comp, post-warmth)
/// and the offline exporter mirrors that, so letting the engine apply it too
/// would double the gain.
pub fn collect_kick_params(p: &SlammerParams) -> KickParams {
    KickParams {
        master_gain: 1.0,
        decay_ms: p.decay_ms.value(),

        sub_gain: p.sub_gain.value(),
        sub_fstart: p.sub_fstart.value(),
        sub_fend: p.sub_fend.value(),
        sub_sweep_ms: p.sub_sweep_ms.value(),
        sub_sweep_curve: p.sub_sweep_curve.value(),
        sub_phase_offset: p.sub_phase_offset.value().to_radians(),

        mid_gain: p.mid_gain.value(),
        mid_fstart: p.mid_fstart.value(),
        mid_fend: p.mid_fend.value(),
        mid_sweep_ms: p.mid_sweep_ms.value(),
        mid_sweep_curve: p.mid_sweep_curve.value(),
        mid_phase_offset: p.mid_phase_offset.value().to_radians(),
        mid_decay_ms: p.mid_decay_ms.value(),
        mid_tone_gain: p.mid_tone_gain.value(),
        mid_noise_gain: p.mid_noise_gain.value(),
        mid_noise_color: p.mid_noise_color.value(),
        mid_noise_decay_ms: p.mid_noise_decay_ms.value(),

        top_gain: p.top_gain.value(),
        top_decay_ms: p.top_decay_ms.value(),
        top_freq: p.top_freq.value(),
        top_bw: p.top_bw.value(),
        top_metal: p.top_metal.value(),

        drift_amount: p.drift_amount.value(),

        // Accent: per-step flag is overlaid by `plugin.rs` at trigger time;
        // this snapshot only captures the host-automatable amount.
        accent: false,
        accent_amount: p.accent_amount.value(),

        sat_mode: p.sat_mode.value() as u8,
        sat_drive: p.sat_drive.value(),
        sat_mix: p.sat_mix.value(),

        kick_clip_mode: p.kick_clip_mode.value() as u8,
        kick_clip_drive: p.kick_clip_drive.value(),

        eq_tilt_db: p.eq_tilt_db.value(),
        eq_low_boost_db: p.eq_low_boost_db.value(),
        eq_notch_freq: p.eq_notch_freq.value(),
        eq_notch_q: p.eq_notch_q.value(),
        eq_notch_depth_db: p.eq_notch_depth_db.value(),

        clap_on: p.clap_on.value(),
        clap_level: p.clap_level.value(),
        clap_freq: p.clap_freq.value(),
        clap_tail_ms: p.clap_tail_ms.value(),
    }
}

impl Default for SlammerParams {
    fn default() -> Self {
        // EguiState size is the LOGICAL window size. Actual on-screen scaling
        // is applied by baseview's WindowScalePolicy — for standalone the
        // launcher passes `--dpi-scale N` (read from `ui_scale.txt`); DAW
        // hosts call `Editor::set_scale_factor`. Multiplying the logical
        // size by the scale here would double-scale (window grows, content
        // doesn't). Mirrors the squelchbox approach.
        let ui_scale = crate::util::paths::load_ui_scale();

        Self {
            editor_state: EguiState::from_size(680, 444),

            seq_steps: Arc::new(Mutex::new(DEFAULT_STEP_BITS)),
            seq_accents: Arc::new(Mutex::new(DEFAULT_ACCENT_BITS)),

            ui_scale: Arc::new(Mutex::new(ui_scale)),

            master_volume: db_knob("Master Volume", 0.0, -60.0, 6.0),

            decay_ms: ms_knob("Decay", 400.0, 50.0, 3000.0, -1.5)
                .with_smoother(SmoothingStyle::Linear(20.0)),

            // --- SUB ---
            sub_gain: FloatParam::new("Sub Gain", 0.85, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(10.0)),

            sub_fstart: hz_knob("Sub Pitch Start", 150.0, 20.0, 800.0, -2.0),
            sub_fend: hz_knob("Sub Pitch End", 45.0, 20.0, 400.0, -1.5),
            sub_sweep_ms: ms_knob("Sub Sweep", 60.0, 5.0, 500.0, -1.5),

            sub_sweep_curve: FloatParam::new(
                "Sub Curve",
                3.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 12.0,
                    factor: FloatRange::skew_factor(-0.5),
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            sub_phase_offset: FloatParam::new(
                "Sub Phase",
                90.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 360.0,
                },
            )
            .with_unit("°")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            // --- MID ---
            mid_gain: FloatParam::new("Mid Gain", 0.5, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(10.0)),

            mid_fstart: hz_knob("Mid Pitch Start", 400.0, 100.0, 2000.0, -1.5),
            mid_fend: hz_knob("Mid Pitch End", 120.0, 50.0, 800.0, -1.5),
            mid_sweep_ms: ms_knob("Mid Sweep", 30.0, 3.0, 300.0, -1.5),

            mid_sweep_curve: FloatParam::new(
                "Mid Curve",
                4.0,
                FloatRange::Skewed {
                    min: 0.1,
                    max: 12.0,
                    factor: FloatRange::skew_factor(-0.5),
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            mid_phase_offset: FloatParam::new(
                "Mid Phase",
                90.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 360.0,
                },
            )
            .with_unit("°")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            mid_decay_ms: ms_knob("Mid Decay", 150.0, 20.0, 1000.0, -1.5),

            mid_tone_gain: FloatParam::new(
                "Mid Tone",
                0.7,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            mid_noise_gain: FloatParam::new(
                "Mid Noise",
                0.3,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            mid_noise_color: FloatParam::new(
                "Mid Noise Color",
                0.4,
                FloatRange::Linear { min: 0.0, max: 1.0 },
            ),
            mid_noise_decay_ms: ms_knob("Mid Noise Decay", 30.0, 1.0, 400.0, -1.0),

            // --- TOP ---
            top_gain: FloatParam::new("Top Gain", 0.25, FloatRange::Linear { min: 0.0, max: 1.0 })
                .with_smoother(SmoothingStyle::Linear(10.0)),

            top_decay_ms: FloatParam::new(
                "Top Decay",
                6.0,
                FloatRange::Skewed {
                    min: 1.0,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            top_freq: hz_knob("Top Freq", 3500.0, 1000.0, 8000.0, -1.0),

            top_bw: FloatParam::new("Top BW", 1.5, FloatRange::Linear { min: 0.2, max: 3.0 })
                .with_unit(" oct")
                .with_value_to_string(formatters::v2s_f32_rounded(1)),

            top_metal: pct_knob("Top Metal", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),

            // --- Drift ---
            drift_amount: pct_knob("Drift", 0.0),

            // --- Accent ---
            accent_amount: pct_knob("Accent Amount", 0.0),

            // --- Saturation ---
            sat_mode: FloatParam::new("Sat Mode", 0.0, FloatRange::Linear { min: 0.0, max: 3.0 })
                .with_step_size(1.0)
                .with_value_to_string(Arc::new(|v| match v as u8 {
                    1 => "Soft".into(),
                    2 => "Diode".into(),
                    3 => "Tape".into(),
                    _ => "Off".into(),
                })),

            sat_drive: pct_knob("Sat Drive", 0.0),
            sat_mix: pct_knob("Sat Mix", 1.0),

            // --- Per-voice clip ---
            kick_clip_mode: FloatParam::new(
                "Kick Clip Mode",
                0.0,
                FloatRange::Linear { min: 0.0, max: 3.0 },
            )
            .with_step_size(1.0)
            .with_value_to_string(Arc::new(|v| match v as u8 {
                1 => "Tanh".into(),
                2 => "Diode".into(),
                3 => "Cubic".into(),
                _ => "Off".into(),
            })),
            // Smoothed because the value is read per-sample inside
            // `voice_clip::apply`. Without smoothing, dragging the knob
            // creates audible glitches at every block boundary as the
            // shaper's gain steps.
            kick_clip_drive: pct_knob("Kick Clip Drive", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),

            // --- EQ ---
            eq_tilt_db: FloatParam::new(
                "EQ Tilt",
                0.0,
                FloatRange::Linear {
                    min: -6.0,
                    max: 6.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            eq_low_boost_db: FloatParam::new(
                "EQ Low Boost",
                0.0,
                FloatRange::Linear {
                    min: -3.0,
                    max: 9.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            eq_notch_freq: hz_knob("EQ Notch Freq", 250.0, 100.0, 600.0, -1.0),

            eq_notch_q: FloatParam::new(
                "EQ Notch Q",
                0.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 10.0,
                },
            )
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            eq_notch_depth_db: FloatParam::new(
                "EQ Notch Depth",
                12.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 20.0,
                },
            )
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            // --- Compressor ---
            // Defaults chosen so a fresh instance is bit-identical to the
            // pre-compressor build: amount = 0 and limiter off together
            // hit the bypass branch in `plugin.rs`.
            comp_amount: pct_knob("Comp Amount", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            comp_react: pct_knob("Comp React", 0.35)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            comp_drive: pct_knob("Comp Drive", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),
            comp_limit_on: BoolParam::new("Comp Limiter", false),

            // Precise comp knobs. Defaults match RCT=0.35 under the old
            // inverse-coupled formula (atk = 30 + 0.35·(1.5 − 30) ≈ 20.0,
            // rel = 400 + 0.35·(40 − 400) ≈ 274.0) so existing sessions and
            // presets sound the same after this refactor.
            comp_atk_ms: FloatParam::new(
                "Comp Attack",
                20.0,
                FloatRange::Skewed {
                    min: 0.3,
                    max: 50.0,
                    factor: FloatRange::skew_factor(-1.5),
                },
            )
            .with_unit(" ms")
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            comp_rel_ms: FloatParam::new(
                "Comp Release",
                274.0,
                FloatRange::Skewed {
                    min: 20.0,
                    max: 800.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" ms")
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            comp_knee_db: FloatParam::new(
                "Comp Knee",
                6.0,
                FloatRange::Linear {
                    min: 0.0,
                    max: 12.0,
                },
            )
            .with_unit(" dB")
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(1)),

            // --- Clap ---
            clap_on: BoolParam::new("Clap", false),

            clap_level: FloatParam::new(
                "Clap Level",
                0.9,
                FloatRange::Linear {
                    min: 0.0,
                    max: 1.5,
                },
            )
            .with_smoother(SmoothingStyle::Linear(10.0))
            .with_value_to_string(formatters::v2s_f32_rounded(2)),

            clap_freq: FloatParam::new(
                "Clap Freq",
                1200.0,
                FloatRange::Skewed {
                    min: 500.0,
                    max: 5000.0,
                    factor: FloatRange::skew_factor(-1.0),
                },
            )
            .with_unit(" Hz")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            clap_tail_ms: FloatParam::new(
                "Clap Tail",
                180.0,
                FloatRange::Linear {
                    min: 50.0,
                    max: 400.0,
                },
            )
            .with_unit(" ms")
            .with_value_to_string(formatters::v2s_f32_rounded(0)),

            // --- DJ Filter ---
            dj_filter_pos: FloatParam::new(
                "DJ Filter",
                0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_smoother(SmoothingStyle::Linear(5.0))
            .with_value_to_string(Arc::new(|v| {
                if v.abs() < 0.001 {
                    "OFF".into()
                } else {
                    let t = v.abs();
                    let freq = if v > 0.0 {
                        20.0 * (800.0f32 / 20.0).powf(t)
                    } else {
                        20000.0 * (200.0f32 / 20000.0).powf(t)
                    };
                    let prefix = if v > 0.0 { "HP" } else { "LP" };
                    if freq >= 1000.0 {
                        format!("{prefix} {:.1}kHz", freq / 1000.0)
                    } else {
                        format!("{prefix} {freq:.0}Hz")
                    }
                }
            })),

            dj_filter_res: pct_knob("DJ Filter Res", 0.0)
                .with_smoother(SmoothingStyle::Linear(10.0)),

            dj_filter_pre: BoolParam::new("DJ Filter Pre", false),
        }
    }
}

// ---------------------------------------------------------------------------
// Plain-data snapshot for preset round-tripping
// ---------------------------------------------------------------------------

/// Plain-data mirror of every automatable parameter in `SlammerParams`.
///
/// Captures and applies via `ParamSetter` so host automation/undo see the
/// changes. `master_volume` is stored as `Option<f32>`: new presets capture
/// the current gain so A/B-ing loudness-varying presets stays level-matched,
/// while legacy preset files (no field) deserialize to `None` and leave the
/// user's current master untouched. `editor_state` is not in the snapshot —
/// it's handled by nih_plug's `#[persist]` mechanism.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParamSnapshot {
    pub decay_ms: f32,

    pub master_volume: Option<f32>,

    pub sub_gain: f32,
    pub sub_fstart: f32,
    pub sub_fend: f32,
    pub sub_sweep_ms: f32,
    pub sub_sweep_curve: f32,
    pub sub_phase_offset: f32,

    pub mid_gain: f32,
    pub mid_fstart: f32,
    pub mid_fend: f32,
    pub mid_sweep_ms: f32,
    pub mid_sweep_curve: f32,
    pub mid_phase_offset: f32,
    pub mid_decay_ms: f32,
    pub mid_tone_gain: f32,
    pub mid_noise_gain: f32,
    pub mid_noise_color: f32,
    pub mid_noise_decay_ms: f32,

    pub top_gain: f32,
    pub top_decay_ms: f32,
    pub top_freq: f32,
    pub top_bw: f32,
    pub top_metal: f32,

    pub drift_amount: f32,

    pub accent_amount: f32,

    pub sat_mode: f32,
    pub sat_drive: f32,
    pub sat_mix: f32,

    pub kick_clip_mode: f32,
    pub kick_clip_drive: f32,

    pub eq_tilt_db: f32,
    pub eq_low_boost_db: f32,
    pub eq_notch_freq: f32,
    pub eq_notch_q: f32,
    pub eq_notch_depth_db: f32,

    pub comp_amount: f32,
    pub comp_react: f32,
    pub comp_drive: f32,
    pub comp_limit_on: bool,
    pub comp_atk_ms: f32,
    pub comp_rel_ms: f32,
    pub comp_knee_db: f32,

    pub clap_on: bool,
    pub clap_level: f32,
    pub clap_freq: f32,
    pub clap_tail_ms: f32,

    pub dj_filter_pos: f32,
    pub dj_filter_res: f32,
    pub dj_filter_pre: bool,
}

impl ParamSnapshot {
    /// Read current values off every persisted param.
    pub fn capture(p: &SlammerParams) -> Self {
        Self {
            decay_ms: p.decay_ms.value(),

            master_volume: Some(p.master_volume.value()),

            sub_gain: p.sub_gain.value(),
            sub_fstart: p.sub_fstart.value(),
            sub_fend: p.sub_fend.value(),
            sub_sweep_ms: p.sub_sweep_ms.value(),
            sub_sweep_curve: p.sub_sweep_curve.value(),
            sub_phase_offset: p.sub_phase_offset.value(),

            mid_gain: p.mid_gain.value(),
            mid_fstart: p.mid_fstart.value(),
            mid_fend: p.mid_fend.value(),
            mid_sweep_ms: p.mid_sweep_ms.value(),
            mid_sweep_curve: p.mid_sweep_curve.value(),
            mid_phase_offset: p.mid_phase_offset.value(),
            mid_decay_ms: p.mid_decay_ms.value(),
            mid_tone_gain: p.mid_tone_gain.value(),
            mid_noise_gain: p.mid_noise_gain.value(),
            mid_noise_color: p.mid_noise_color.value(),
            mid_noise_decay_ms: p.mid_noise_decay_ms.value(),

            top_gain: p.top_gain.value(),
            top_decay_ms: p.top_decay_ms.value(),
            top_freq: p.top_freq.value(),
            top_bw: p.top_bw.value(),
            top_metal: p.top_metal.value(),

            drift_amount: p.drift_amount.value(),

            accent_amount: p.accent_amount.value(),

            sat_mode: p.sat_mode.value(),
            sat_drive: p.sat_drive.value(),
            sat_mix: p.sat_mix.value(),

            kick_clip_mode: p.kick_clip_mode.value(),
            kick_clip_drive: p.kick_clip_drive.value(),

            eq_tilt_db: p.eq_tilt_db.value(),
            eq_low_boost_db: p.eq_low_boost_db.value(),
            eq_notch_freq: p.eq_notch_freq.value(),
            eq_notch_q: p.eq_notch_q.value(),
            eq_notch_depth_db: p.eq_notch_depth_db.value(),

            comp_amount: p.comp_amount.value(),
            comp_react: p.comp_react.value(),
            comp_drive: p.comp_drive.value(),
            comp_limit_on: p.comp_limit_on.value(),
            comp_atk_ms: p.comp_atk_ms.value(),
            comp_rel_ms: p.comp_rel_ms.value(),
            comp_knee_db: p.comp_knee_db.value(),

            clap_on: p.clap_on.value(),
            clap_level: p.clap_level.value(),
            clap_freq: p.clap_freq.value(),
            clap_tail_ms: p.clap_tail_ms.value(),

            dj_filter_pos: p.dj_filter_pos.value(),
            dj_filter_res: p.dj_filter_res.value(),
            dj_filter_pre: p.dj_filter_pre.value(),
        }
    }

    /// Push every field of the snapshot back into the live params, going
    /// through `ParamSetter` so host automation/undo see the changes.
    pub fn apply(&self, setter: &ParamSetter, p: &SlammerParams) {
        macro_rules! set {
            ($param:expr, $val:expr) => {
                setter.begin_set_parameter(&$param);
                setter.set_parameter(&$param, $val);
                setter.end_set_parameter(&$param);
            };
        }
        set!(p.decay_ms, self.decay_ms);

        if let Some(v) = self.master_volume {
            set!(p.master_volume, v);
        }

        set!(p.sub_gain, self.sub_gain);
        set!(p.sub_fstart, self.sub_fstart);
        set!(p.sub_fend, self.sub_fend);
        set!(p.sub_sweep_ms, self.sub_sweep_ms);
        set!(p.sub_sweep_curve, self.sub_sweep_curve);
        set!(p.sub_phase_offset, self.sub_phase_offset);

        set!(p.mid_gain, self.mid_gain);
        set!(p.mid_fstart, self.mid_fstart);
        set!(p.mid_fend, self.mid_fend);
        set!(p.mid_sweep_ms, self.mid_sweep_ms);
        set!(p.mid_sweep_curve, self.mid_sweep_curve);
        set!(p.mid_phase_offset, self.mid_phase_offset);
        set!(p.mid_decay_ms, self.mid_decay_ms);
        set!(p.mid_tone_gain, self.mid_tone_gain);
        set!(p.mid_noise_gain, self.mid_noise_gain);
        set!(p.mid_noise_color, self.mid_noise_color);
        set!(p.mid_noise_decay_ms, self.mid_noise_decay_ms);

        set!(p.top_gain, self.top_gain);
        set!(p.top_decay_ms, self.top_decay_ms);
        set!(p.top_freq, self.top_freq);
        set!(p.top_bw, self.top_bw);
        set!(p.top_metal, self.top_metal);

        set!(p.drift_amount, self.drift_amount);

        set!(p.accent_amount, self.accent_amount);

        set!(p.sat_mode, self.sat_mode);
        set!(p.sat_drive, self.sat_drive);
        set!(p.sat_mix, self.sat_mix);

        set!(p.kick_clip_mode, self.kick_clip_mode);
        set!(p.kick_clip_drive, self.kick_clip_drive);

        set!(p.eq_tilt_db, self.eq_tilt_db);
        set!(p.eq_low_boost_db, self.eq_low_boost_db);
        set!(p.eq_notch_freq, self.eq_notch_freq);
        set!(p.eq_notch_q, self.eq_notch_q);
        set!(p.eq_notch_depth_db, self.eq_notch_depth_db);

        set!(p.comp_amount, self.comp_amount);
        set!(p.comp_react, self.comp_react);
        set!(p.comp_drive, self.comp_drive);
        setter.begin_set_parameter(&p.comp_limit_on);
        setter.set_parameter(&p.comp_limit_on, self.comp_limit_on);
        setter.end_set_parameter(&p.comp_limit_on);
        set!(p.comp_atk_ms, self.comp_atk_ms);
        set!(p.comp_rel_ms, self.comp_rel_ms);
        set!(p.comp_knee_db, self.comp_knee_db);

        setter.begin_set_parameter(&p.clap_on);
        setter.set_parameter(&p.clap_on, self.clap_on);
        setter.end_set_parameter(&p.clap_on);
        set!(p.clap_level, self.clap_level);
        set!(p.clap_freq, self.clap_freq);
        set!(p.clap_tail_ms, self.clap_tail_ms);

        set!(p.dj_filter_pos, self.dj_filter_pos);
        set!(p.dj_filter_res, self.dj_filter_res);
        setter.begin_set_parameter(&p.dj_filter_pre);
        setter.set_parameter(&p.dj_filter_pre, self.dj_filter_pre);
        setter.end_set_parameter(&p.dj_filter_pre);
    }
}

#[cfg(test)]
mod clap_param_tests {
    use super::*;

    #[test]
    fn clap_params_defaults() {
        let p = SlammerParams::default();
        assert!(!p.clap_on.value());
        assert!((p.clap_level.value() - 0.9).abs() < 1e-4);
        assert!((p.clap_freq.value() - 1200.0).abs() < 1.0);
        assert!((p.clap_tail_ms.value() - 180.0).abs() < 1e-4);
    }

    #[test]
    fn param_snapshot_roundtrip_clap() {
        let snap = ParamSnapshot {
            clap_on: true,
            clap_level: 1.1,
            clap_freq: 1800.0,
            clap_tail_ms: 250.0,
            ..ParamSnapshot::default()
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ParamSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn old_preset_loads_with_clap_defaults() {
        let json = r#"{ "decay_ms": 120.0 }"#;
        let snap: ParamSnapshot = serde_json::from_str(json).unwrap();
        assert!(!snap.clap_on);
        assert_eq!(snap.clap_level, 0.0);
        assert_eq!(snap.clap_freq, 0.0);
        assert_eq!(snap.clap_tail_ms, 0.0);
    }

    #[test]
    fn dj_filter_and_metal_defaults() {
        let p = SlammerParams::default();
        assert!((p.dj_filter_pos.value()).abs() < 1e-4);
        assert!((p.dj_filter_res.value()).abs() < 1e-4);
        assert!(!p.dj_filter_pre.value());
        assert!((p.top_metal.value()).abs() < 1e-4);
    }

    #[test]
    fn param_snapshot_roundtrip_dj_filter() {
        let snap = ParamSnapshot {
            dj_filter_pos: -0.6,
            dj_filter_res: 0.4,
            dj_filter_pre: true,
            top_metal: 0.5,
            ..ParamSnapshot::default()
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ParamSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn old_preset_loads_with_filter_defaults() {
        let json = r#"{ "decay_ms": 120.0 }"#;
        let snap: ParamSnapshot = serde_json::from_str(json).unwrap();
        assert!((snap.dj_filter_pos).abs() < 1e-6);
        assert!(!snap.dj_filter_pre);
        assert!((snap.top_metal).abs() < 1e-6);
    }

    #[test]
    fn param_snapshot_roundtrip_master_volume() {
        let snap = ParamSnapshot {
            master_volume: Some(0.5),
            ..ParamSnapshot::default()
        };
        let json = serde_json::to_string(&snap).unwrap();
        let back: ParamSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);
        assert_eq!(back.master_volume, Some(0.5));
    }

    #[test]
    fn legacy_preset_leaves_master_volume_untouched() {
        // Pre-feature presets have no master_volume field; they must
        // deserialize to None so apply() skips the master, preserving the
        // user's current monitoring level.
        let json = r#"{ "decay_ms": 120.0 }"#;
        let snap: ParamSnapshot = serde_json::from_str(json).unwrap();
        assert_eq!(snap.master_volume, None);
    }

    #[test]
    fn old_preset_with_flam_fields_loads_cleanly() {
        // Unknown serde fields are ignored by default. A preset saved by
        // the previous flam-era build should still load without error.
        let json = r#"{ "decay_ms": 120.0, "flam_on": true, "flam_spread_ms": 15.0, "flam_humanize": 0.3 }"#;
        let snap: ParamSnapshot = serde_json::from_str(json).unwrap();
        assert!(!snap.clap_on);
    }
}
