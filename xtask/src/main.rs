use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
Niner xtask commands:
    bundle <plugin> [args...]   Build VST3/CLAP/standalone bundle (delegates to nih_plug_xtask)
    lock-layout [src.json]      Copy your dev-saved layout JSON into assets/baked_layout.json
                                so the next release ships those tweaks. Defaults to
                                <niner-data>/layout_overrides.json (where the layout editor
                                writes when --features layout_editor is enabled).
    --help                      Show this help
";

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("lock-layout") => match lock_layout(args.next().map(PathBuf::from)) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        // Everything else falls through to nih_plug_xtask::main(), which
        // re-reads std::env::args() itself — so the args we already
        // consumed don't matter.
        _ => match nih_plug_xtask::main() {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("nih_plug_xtask error: {e}");
                ExitCode::FAILURE
            }
        },
    }
}

/// Copy a layout JSON into `assets/baked_layout.json` so subsequent
/// release builds ship those tweaks via `include_bytes!`. Validates the
/// JSON parses as `LayoutOverrides` before overwriting.
fn lock_layout(src: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let workspace = workspace_root();
    let dest = workspace.join("assets").join("baked_layout.json");

    let src_path = match src {
        Some(p) => p,
        None => default_user_layout_path().ok_or(
            "could not locate <niner-data>/layout_overrides.json — pass a path explicitly",
        )?,
    };

    if !src_path.exists() {
        return Err(format!("source layout JSON not found: {}", src_path.display()).into());
    }

    let contents = std::fs::read_to_string(&src_path)?;

    // Sanity-parse so we never bake malformed JSON. We don't depend on
    // the `niner` crate from xtask (would double the build), so just
    // confirm the top-level is a JSON object with a `bulk` object child.
    let v: serde_json::Value =
        serde_json::from_str(&contents).map_err(|e| format!("source JSON invalid: {e}"))?;
    if !v.is_object() || !v.get("bulk").map(|b| b.is_object()).unwrap_or(false) {
        return Err("source JSON missing top-level `bulk` object".into());
    }

    std::fs::create_dir_all(dest.parent().unwrap())?;
    std::fs::write(&dest, &contents)?;
    println!("locked layout: {} → {}", src_path.display(), dest.display());
    println!("rebuild release to ship the new layout: cargo xtask bundle niner --release");
    Ok(())
}

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // xtask/Cargo.toml → workspace root is its parent.
    manifest_dir.parent().unwrap().to_path_buf()
}

fn default_user_layout_path() -> Option<PathBuf> {
    // Mirrors `crate::util::paths::niner_data_dir` for the default install.
    // Linux: $XDG_DATA_HOME/niner or ~/.local/share/niner.
    // macOS: ~/Library/Application Support/niner.
    // Windows: %APPDATA%/niner.
    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
            return Some(
                PathBuf::from(xdg)
                    .join("niner")
                    .join("layout_overrides.json"),
            );
        }
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join(".local/share/niner")
                .join("layout_overrides.json"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support/niner")
                .join("layout_overrides.json"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")?;
        Some(
            PathBuf::from(appdata)
                .join("niner")
                .join("layout_overrides.json"),
        )
    }
}
