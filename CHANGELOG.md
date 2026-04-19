# Changelog

All notable changes to Slammer are documented here.

## [0.4.3] — 2026-04-19

### Presets

- **Factory preset library expanded.** Eleven new factory presets baked in
  alongside the original three — `808`, `909`, `909old`, `clap`, `hh`, `Init`,
  `overdose`, `psy`, `sd1`, `sd2`, `tight` — covering sub, snare, hat, clap,
  and psytrance tonal territory. Fourteen factory presets total.
- **Master volume is now part of a preset.** Saved as `master_volume` in
  `ParamSnapshot` and applied on preset load, so presets that vary in
  loudness audition at their intended monitoring level. Backward-compatible:
  older preset files without the field load without touching the current
  master. Most new factories ship with tuned master levels; a handful of
  older ones (`909`, `sd2`, `tight`) intentionally leave master as-is.

### UI

- **Cluster label alignment.** PRECISE (ATK/REL/KNE), CLAP (LVL/FREQ/TAIL),
  and FILTER (FILT/RES) mini-cluster captions now render *below* their knob
  rows instead of above. This lines up each cluster's small knob centers with
  the adjacent big-knob row, eliminating the vertical offset that made the
  right side of the panel look misregistered. Click targets for the CLAP and
  PRE/POST LEDs move with their captions and behave identically.

## [0.4.2] — 2026-04-19

### Brand + metadata

- **Hyperfocus DSP transition.** Plugin vendor, URL, email, and CLAP ID all
  updated from the legacy `REXIST` identity to `Hyperfocus DSP`
  (`https://hyperfocusdsp.com`, `hello@hyperfocusdsp.com`,
  `com.hyperfocusdsp.slammer`). VST3 class ID is unchanged to preserve
  DAW-project compatibility for existing 0.4.x users.
- **Footer mark.** Full `hyperfocus DSP` wordmark (with small-caps suffix and
  ring-as-O from the canonical brand master) rendered in the footer strip.
  Hover tooltip: "Made by Hyperfocus DSP".

### UI

- **UI scale.** Click-to-cycle badge in the header (1× / 1.5× / 2×) that
  persists the chosen scale both inside the DAW project (nih-plug
  `#[persist]`) and to a sidecar file so the standalone launcher forwards it
  via `--dpi-scale` on the next launch. Scaling itself is delegated to
  baseview's `WindowScalePolicy` — no in-editor `set_pixels_per_point`
  fighting baseview.
- **Knob cluster tightening.** AMT / RCT / DRV (macro compressor) now use the
  same 18 px knob / 28 px cell as the PRECISE strip directly below, so the
  two rows read as visually paired instead of one wider than the other.
- **BOUNCE alignment.** The BOUNCE button's right edge is now anchored to the
  cluster-column right edge, so it lines up exactly with KNE, TAIL, POST, and
  the DICE LED row (previously off by 2–4 px).

## [0.4.1] — 2026-04-12

### Fixes

- DICE row moved below the STEP separator line with proper spacing (was touching it).
- BOUNCE button repositioned to just above the footer groove, narrowed from 56 to 48 px.

## [0.4.0] — 2026-04-12

### New features

- **DJ Filter** — bipolar master HP/LP state-variable filter (12 dB/oct, zero-delay
  feedback SVF). The FILT knob is bipolar: center = off, left sweeps a high-pass from
  800 Hz down to 20 Hz, right sweeps a low-pass from 20 kHz down to 200 Hz. RES knob
  (0–1). PRE/POST LED toggle places the filter before or after the master bus
  (compressor + transformer + limiter + tube warmth).
- **METAL** — ring modulation on the TOP click voice using the 909 hat partial ratio
  (1 + √2 ≈ 2.414). Adds inharmonic metallic overtones to the click transient without
  affecting the SUB or MID layers. Bit-identical bypass at zero.
- **DICE** — randomize button with six per-section lock LEDs (S M T X E C for SUB /
  MID / TOP / SAT / EQ / COMP). Roll all unlocked sections at once; values are
  range-safe via the parameter API. Global envelope params (DECAY, DRIFT) always
  randomize regardless of locks.
- **Logo** — Slammer wordmark in the plugin header.

### Fixes

- **Knob double-click reset now works** in all hosts/platforms. `response.double_clicked()`
  is unreliable under baseview (raw mouse events, no synthesised egui input). Replaced
  with manual per-widget timestamp tracking — delta < 0.35 s triggers the reset.
- **DECAY + DRIFT now always randomize** with DICE. Previously they were gated behind
  the SUB lock even though they affect all three layers.
- **Duplicate randomization step removed** — `mid_phase_offset` was being randomized
  twice per DICE roll (second call overwrote the first with a different value).
- **Saturation clip LP coefficient** is now precomputed at initialisation rather than
  recomputed on every sample. No audible change; measurable CPU reduction in Clip mode.

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
