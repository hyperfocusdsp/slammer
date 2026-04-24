//! Windows-only: probe the default WASAPI output device for a stereo
//! sample-rate / buffer-size the backend will actually accept, and build
//! an argv vector that nih-plug's standalone wrapper can consume.
//!
//! Why: nih-plug's CPAL backend hard-requires the requested sample rate
//! and period size to fall inside the device's advertised ranges. Its
//! defaults (48000 Hz, 512 samples) miss many Windows setups — 44.1 kHz
//! mix formats and WASAPI minimum buffer sizes larger than 512 are both
//! common. Probing once at launch lets us pass values the device
//! actually supports.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::SupportedBufferSize;

/// Safe Windows period size, in samples. History:
///
/// * `512` — nih-plug default; underruns on almost every WASAPI setup.
/// * `2048` (v0.4.4, 2026-04-22) — fine on tested dev machines.
/// * `4096` (v0.5.3, 2026-04-24) — needed for managed / corporate W11
///   where background services (Defender, Intune, WMI polling, etc.)
///   steal cycles from the audio thread in bursts that a 43 ms cushion
///   can't absorb. 85 ms of headroom handles the worst case we've
///   observed without perceptibly hurting playback — a kick drum synth
///   is almost always sequencer-driven, and 85 ms of output latency is
///   invisible in that context.
///
/// Bump again (and update this comment) only if a confirmed click
/// report with a bigger buffer comes in. The startup log now reports
/// the device's `buffer_size` range, so the next bump decision is
/// evidence-driven instead of a guess.
const WINDOWS_SAFE_PERIOD: u32 = 4096;

fn arg_already_set(args: &[String], long: &str, short: &str) -> bool {
    args.iter().any(|a| {
        a == long
            || a == short
            || a.starts_with(&format!("{long}="))
            || a.starts_with(&format!("{short}="))
    })
}

/// Return `Some(argv)` with probed defaults injected, or `None` to let the
/// caller fall back to nih-plug's default parser.
pub fn probed_argv() -> Option<Vec<String>> {
    let user_args: Vec<String> = std::env::args().collect();

    let has_sr = arg_already_set(&user_args, "--sample-rate", "-r");
    let has_ps = arg_already_set(&user_args, "--period-size", "-p");
    let has_backend = arg_already_set(&user_args, "--backend", "-b");

    if has_sr && has_ps && has_backend {
        return None;
    }

    let host = cpal::default_host();
    let device = host.default_output_device()?;

    let device_name = device.name().unwrap_or_else(|_| "<unknown>".into());
    let default_cfg = device.default_output_config().ok()?;
    let probed_sr = default_cfg.sample_rate().0;

    // Collect + log the device's reported buffer-size range. If a future
    // Windows machine clicks even at 4096, this line in the log tells us
    // whether the OS/driver is advertising an unusually high minimum
    // (→ bump the constant) or the underrun is somewhere else entirely.
    let buffer_range = device
        .supported_output_configs()
        .ok()
        .and_then(|mut iter| iter.find(|c| c.channels() == 2))
        .map(|c| match c.buffer_size() {
            SupportedBufferSize::Range { min, max } => format!("{min}..{max}"),
            SupportedBufferSize::Unknown => "unknown".to_string(),
        })
        .unwrap_or_else(|| "n/a".to_string());
    tracing::info!(
        "WASAPI probe: device={:?}, sample_rate={}, buffer_size_range={}, chosen_period={}",
        device_name,
        probed_sr,
        buffer_range,
        WINDOWS_SAFE_PERIOD
    );

    let stereo_supported = device.supported_output_configs().ok()?.any(|c| {
        c.channels() == 2
            && c.min_sample_rate().0 <= probed_sr
            && c.max_sample_rate().0 >= probed_sr
    });
    if !stereo_supported {
        return None;
    }

    let mut argv = user_args;
    if !has_sr {
        argv.push("--sample-rate".into());
        argv.push(probed_sr.to_string());
    }
    if !has_ps {
        argv.push("--period-size".into());
        argv.push(WINDOWS_SAFE_PERIOD.to_string());
    }
    Some(argv)
}
