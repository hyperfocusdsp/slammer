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

/// Safe Windows period size. nih-plug#147 documents that WASAPI commonly
/// reports minimums of 1056 or 2048; 2048 is a round number that works
/// across the Windows devices the project has been tested on.
const WINDOWS_SAFE_PERIOD: u32 = 2048;

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

    let default_cfg = device.default_output_config().ok()?;
    let probed_sr = default_cfg.sample_rate().0;

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
