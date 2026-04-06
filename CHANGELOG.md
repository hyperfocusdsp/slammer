# Changelog

All notable changes to Slammer are documented here.

## [0.2.1] — 2026-04-06

First public release.

### Signal chain

- **Three-layer kick engine** — SUB sine + pitch envelope, MID sine+noise
  blend, TOP bandpass-filtered click transient. Per-layer amp + pitch
  envelopes, drift, and tuning.
- **Five-voice distortion palette** — split-band rational clip, asymmetric
  diode, hysteresis tape, transformer drive (master bus), and auto tube
  warmth (post-bus). Each curve is genuinely distinct in harmonic
  content, symmetry, and frequency response.
- **Master bus** — RMS compressor with 3 macros (amount, reaction,
  drive), transformer drive, and a brickwall limiter with LED indicator.
- **Master EQ** — tilt, low shelf, and variable-Q notch.

### UI

- **680 × 444 single-panel editor** with rack chrome, aspect-ratio-locked
  scaling (resize the DAW window freely; the layout scales without
  distortion).
- **Header-integrated preset bar** — 7-segment LED display, dropdown
  browser, inline rename, save, delete. Up/Down arrows cycle presets
  globally. Factory presets are delete-protected.
- **16-step pattern sequencer** with click/drag paint, four-on-the-floor
  default, and a PLAY/STOP button in standalone mode.
- **Host transport sync** — the sequencer locks to host position and
  tempo inside a DAW. In standalone mode the sequencer runs on an
  internal transport and the tempo readout becomes interactive.
- **Interactive tempo entry** (standalone only) — single-click the BPM
  text to arm, vertical drag to scrub at 2 px/BPM, double-click to type
  a value directly (3-digit, digits only), Left/Right arrows for ±10
  BPM (Shift for ±1). Host-synced mode keeps the readout read-only.
- **Live output waveform scope** with gain-reduction meter.
- **Keyboard test trigger** — press `T` anywhere in the editor to fire
  the engine without MIDI.

### Persistence

- **Factory + user presets** stored as forward-compatible JSON
  (`#[serde(default)]` on every field, so future versions can add
  parameters without breaking old files).
- **Last-preset recall** in standalone: the editor remembers the last
  loaded preset and restores it on next launch.
- **Full DAW state persistence** for parameters and sequencer patterns
  inside project files.

### Platforms

- **Linux x86_64** — VST3, CLAP, Standalone
- **macOS ARM (Apple Silicon)** — VST3, CLAP, Standalone
  (standalone uses `slammer-macos.sh` launch script with
  `--period-size 4096` to work around
  [nih-plug#266](https://github.com/robbert-vdh/nih-plug/issues/266))
- **macOS Intel** — VST3, CLAP, Standalone
- **Windows x86_64** — VST3, CLAP, Standalone

### Known limitations

- No Audio Unit (AU) support — nih-plug framework limitation.
- Standalone on macOS Apple Silicon needs the included launch script
  (adds slight latency; DAW plugins are unaffected).
- Window is aspect-ratio locked but not free-form resizable.
