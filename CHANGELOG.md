# Changelog

All notable changes to Niner (formerly Slammer) are documented here.

## [0.7.0] — 2026-04-26

### Renamed: Slammer → Niner

This release rebrands the plugin from **Slammer** to **Niner** (a nod
to the TR-909 lineage). The rename was driven by a trademark conflict
with another product called Slammer.

**What you need to know:**

- **Plugin IDs changed.** New `CLAP_ID = "com.hyperfocusdsp.niner"` and
  new `VST3_CLASS_ID`. Existing DAW projects saved with the old Slammer
  plugin will show the track as missing/disabled and need to be re-wired
  to Niner manually. Project parameter values themselves are unaffected
  — only the host-side identity changed.
- **Data directory auto-migrates** on first launch. The old
  `~/.local/share/slammer` (or platform equivalent) is renamed to
  `~/.local/share/niner`, so user presets, hidden-preset filters, and
  UI scale settings carry over without intervention.
- **Log filename:** `slammer.log` → `niner.log`. Old log files inside
  the migrated directory remain on disk as harmless artifacts.
- **Bundle / binary names:** `slammer.vst3` → `niner.vst3`,
  `slammer.clap` → `niner.clap`, `slammer-standalone` → `niner-standalone`.
- **Env var:** `SLAMMER_DISABLE_SPECTRUM` → `NINER_DISABLE_SPECTRUM`.
- **Default bounce folder name:** `Slammer Bounces` → `Niner Bounces`
  (in the user's Music folder).
- **AUR package:** `slammer` → `niner`. The old `slammer` AUR entry
  will not auto-update; users on Arch should `yay -R slammer && yay -S niner`.
- **GitHub repo:** `hyperfocusdsp/slammer` → `hyperfocusdsp/niner`.

DSP, parameters, presets, and UI behavior are unchanged from v0.6.0.

## [0.6.0] — 2026-04-26

### Added

- **Per-voice soft-clip** — new `src/dsp/voice_clip.rs` module with three
  modes (`Tanh`, `Diode`, `Cubic`) plus pass-through `Off`. The shaper
  sits **before** each layer's amp envelope inside `KickVoice::tick`,
  matching the analog 909's `VCO → soft-clip → VCA` topology — so
  harmonics are dense at attack and thin out as the layer envelope
  closes, rather than being applied to the already-decayed mix.
  Distinct from the existing master-bus saturation, which still runs
  post-envelope on the summed voices.
- **Two new params**: `kick_clip_mode` (0 Off / 1 Tanh / 2 Diode /
  3 Cubic) and `kick_clip_drive` (0..1). Both default to 0 / 0, so
  every v0.5.x preset loads bit-identical (`apply` short-circuits to
  pass-through when drive ≤ 0). Persisted via the existing
  `ParamSnapshot` round-trip with `#[serde(default)]`, so old preset
  JSON files deserialize cleanly.
- Factory `909` preset now ships with `kick_clip_mode = 1.0` (Tanh)
  and `kick_clip_drive = 0.15`. Closes the largest perceptual gap
  surfaced by the 2026-04-26 TR-909 BD audit (item 3 — per-voice
  waveshaper before mix).
- **MID noise gated to attack** with its own short envelope. New
  param `mid_noise_decay_ms` (default 30 ms) runs the noise channel
  off a separate `AmpEnvelope` instead of riding the tone's
  `mid_amp_env`. Real 909 kicks have a short noise burst at attack
  (15-30 ms) layered over a longer tone tail; legacy slammer let
  noise sustain for as long as `mid_decay_ms`, which on 250-300 ms
  tone tails turned the noise into a hiss bed instead of a snap.
  Closes audit item 4. Factory `909` preset uses `15 ms` for that
  signature 909 click character.
- **Legacy preset compatibility** — `ParamSnapshot` deserialization
  from v0.5.x JSON files leaves `mid_noise_decay_ms = 0.0`. The
  trigger path treats anything below 1 ms as a sentinel and falls
  back to `mid_decay_ms`, so old presets keep their original
  sustained-noise feel until the user explicitly tunes the new knob.

- **909-style Accent.** Closes audit item 7. Two new mechanisms working
  together: a per-step accent bit on the sequencer (parallel to the
  existing step bits, persisted via a new `seq_accents` payload) and
  a host-automatable `accent_amount` param. When a step's accent bit
  is set AND `accent_amount > 0`, the engine multiplies that hit's
  `amp_scale` by `1 + 0.3·a` and `decay_scale` by `1 + 0.5·a`,
  composing on top of the existing drift jitter. Manual triggers
  (button, T key, MIDI note-on) always fire un-accented. UI:
  shift-click a lit step to toggle its accent — accented steps are
  marked by a small white tick at the bottom of the pad. Clearing a
  step also clears its accent so the state can't outlive its host.
  Backward-compat: v0.5.x sessions deserialize `seq_accents` to zero
  (no accents) and `accent_amount` to 0, so no existing pattern
  changes character.

### UI

- **SAT/EQ row restructured** to surface every v0.6.0 audio param. The
  left half of the row is now a 2×4 stacked cluster: top sub-row
  carries the existing SAT MODE LCD selector (compact variant) plus
  small SAT DRIVE / SAT MIX knobs; bottom sub-row carries a new CLIP
  MODE LCD selector (compact, mirroring the SAT MODE shape) plus the
  three new v0.6.0 controls — CLIP DRIVE, MID NOISE DECAY, ACCENT —
  all rendered in the comp-cluster small-knob style (18 px diameter).
  The EQ cluster on the right keeps its full-size 32 px knobs and
  vertical alignment unchanged. Row height grew by 22 px to fit the
  stack; STEP grid and DICE shift down to follow.
- **`lcd_selector` widget refactored** in `seven_seg.rs` to take a
  unique `id_source`, a `modes` slice, and a `compact: bool` flag.
  The compact variant uses 14×18 arrows + 44×16 LCD and drops the
  trailing 8 px gap, fitting cleanly inside the new sub-row height.
  Two selectors with distinct id sources can now coexist in the same
  frame without egui Id collision. The CLIP MODE LCD displays
  `OFF / tAnH / dIOdE / CUbIC` (lowercase `n` because the 7-segment
  glyph table doesn't define uppercase N).

### Fixed

- **Intermittent bitcrush-style audio glitches root-caused.** Across
  five new offline-render audit tests covering default Init, the 909
  preset (drift + Diode sat + Tanh kick-clip), heavy 32nd-note
  retriggering, and comp+limiter+drive+warmth all engaged, the full
  per-sample DSP chain produces bit-clean output (max sample-to-sample
  delta in the natural attack-ramp range, no NaN, no FFT-period
  correlation). The remaining real-time-only artifact under load was
  isolated to the audio-thread spectrum FFT — Slammer's spectrum
  analyzer runs `realfft::process_with_scratch` once per 1024 samples
  on the RT thread (Autokit on the same machine has no audio-thread
  FFT and is glitch-free, the smoking-gun cross-plugin delta).

### Diagnostics

- **`SLAMMER_DISABLE_SPECTRUM` env var.** When set to a non-empty,
  non-`0` value at plugin/standalone init, the audio thread skips
  the spectrum FFT entirely. The OUTPUT display's bars freeze, but
  CPU pressure on the RT thread drops to Autokit-equivalent. Cached
  in a `bool` field, branch-predictor-friendly. v0.6.1 will replace
  this with a click-cycle tri-state OUTPUT toggle (Normal / Bars /
  None) and move the FFT to a worker thread so the default path
  also no longer occupies the audio thread.

### Tests

- **5 new offline-render audit tests** in `plugin.rs::tests` that
  mirror the full per-sample chain (engine + master_bus + dj_filter
  + tube + soft-clip + spectrum FFT feed) over multi-second renders,
  write WAVs to `/tmp/slammer_offline_*.wav` for visual inspection,
  and assert no audible artifacts at the sample-delta level. Total
  test count 137 → 142.

## [0.5.5] — 2026-04-26

### Added

- **Per-trigger amplitude + decay drift.** The `Drift` knob now perturbs
  more than pitch on each hit. A new `Drift::sample_envelope` returns
  one `DriftSample` per trigger with `amp_scale` (±2.5%) and
  `decay_scale` (±5%), both gated by `drift_amount`. `amp_scale`
  multiplies the voice's tick output uniformly across SUB / MID / TOP
  layers; `decay_scale` multiplies every amp-envelope's `decay_ms`
  before triggering. At `drift_amount = 0` both factors are exactly
  1.0 — deterministic v0.5.x behavior is preserved.
- **Analog envelope-tau quantization.** `analog_quantize_tau(tau,
  drift_amount)` snaps an envelope time-constant to ~16 levels per
  decade, lerped in by `drift_amount`. Approximates the cap-array
  stepping of analog envelope-generator chips: with drift engaged,
  two slightly-different `decay_ms` settings can audibly snap to the
  same decay because they round to the same chip step. Inaudible at
  drift = 0; subtle character from drift ≈ 0.3 upward. Wired into
  every `AmpEnvelope::trigger` call across the kick voice.

### Changed

- **Factory `909` preset rewritten** against a TR-909 BD audit
  (reference: `BPB Cassette 909/clean/bd01.wav`). Old values were
  stale (`decay_ms = 934 ms`, `sub_fstart = 90.8 Hz`). New values:
  `decay_ms = 200`, `sub_fstart = 65`, `sub_fend = 50`,
  `top_gain = 0.15`, `mid_noise_gain = 0.1`, `sat_mode = 2.0`
  (Diode), `sat_drive = 0.1`, `drift_amount = 0.2`. The factory
  `909-ish` and `909old` presets are unchanged for now.

### Fixed

- **Preset dropdown scrolls cleanly past ~12 entries.** Rewrote
  `src/ui/preset_bar.rs` so the dropdown popup constrains paint to
  its bounding rect, only iterates the visible window, supports
  mouse-wheel scroll, draws a thin red-LED scrollbar thumb on the
  right edge, and auto-scrolls so the currently-selected preset is
  visible on open. Required because user preset count grew past the
  original fixed-height window's capacity and the old painter-style
  dropdown overflowed onto the knob panel below.
- **Pointer cursor stays on the dropdown.** The knob drag-to-change
  widget calls `set_cursor_icon(ResizeVertical)` and was winning the
  last-write-of-the-frame race against the dropdown's PointingHand,
  so user-preset rows showed up/down resize arrows. The fix stashes
  the dropdown's bounding rect during `PresetBar::render` and
  re-applies the cursor in a new `apply_late_cursor` method called
  from `editor.rs` **after** every knob-panel draw.

## [0.5.3] — 2026-04-24

### Fixed

- **Windows standalone audio-thread underruns on managed / corporate
  setups.** The WASAPI probe's period-size default moved from 2048
  (~43 ms) to 4096 (~85 ms). On machines where Defender / Intune /
  WMI or similar background services steal cycles from the audio
  thread in bursts, 43 ms of buffer wasn't enough to absorb the
  stall and random clicks landed in the output. 85 ms handles the
  worst case observed on a managed W11 box without perceptibly
  affecting a sequencer-driven kick drum. Sample rate probe
  (previous fix in v0.4.4) is unchanged.

### Added

- **WASAPI-probe startup log line.** `src/windows_standalone.rs`
  now emits `WASAPI probe: device=..., sample_rate=..., buffer_size_range=..., chosen_period=...`
  at launch. Makes future underrun reports evidence-driven — if a
  machine clicks even at 4096, the log tells us whether the driver
  is advertising an unusually high minimum (bump the constant) or
  the underrun is somewhere else entirely.

## [0.5.2] — 2026-04-24

### Fixed

- **Output safety clipper prevents DAC crackling on loud presets.** At
  default plugin state the master-bus chain — including the brickwall
  limiter — was bypassed (comp off, drive off, limiter-toggle off), so
  the engine's natural peak (~+0.6 dBFS on default params, higher with
  low-boost EQ or master-volume above unity) went straight to the audio
  device unclipped. On managed Windows / WASAPI setups this manifested
  as hit-to-hit crackling that disappeared when the master volume was
  trimmed below -0.5 dB. A final per-sample soft-clip stage now runs
  after master volume with a `tanh`-asymptoted ceiling at -0.009 dBFS:
  signals below -1.4 dBFS pass through bit-identical, signals above
  smoothly roll off and never reach full-scale. Loud presets still
  sound loud; they just don't hard-clip the converter anymore.

## [0.5.1] — 2026-04-24

### Fixed

- **Sub / mid oscillators now phase-lock on every trigger.** Previously the
  `Drift` knob added ±8.6° of random phase offset to the SUB and MID
  oscillators at trigger time, causing identical MIDI hits to produce
  audibly inconsistent low-end level (constructive vs destructive summing
  against decaying tails and kick-bass interactions in a mix). Trigger
  phase is now deterministic, matching the gated-VCO behaviour of analog
  kick circuits. The `Drift` knob still controls ±14 cents of analog
  pitch drift — that's the part that makes a kick sound analog.

## [0.5.0] — 2026-04-24

### Added

- **Spectrum-analyzer view on the OUTPUT display.** Click the display to
  toggle between the rolling waveform and a 64-band log-frequency spectrum
  (20 Hz → 20 kHz, -60 → 0 dB). The label swaps `OUTPUT ↔ SPECTRUM` so you
  always know which mode you're in. Peak-hold dots per band decay over
  ~500 ms, so transients stay readable after they pass — useful when tuning
  a kick's harmonic content. Mode is sticky across widget rebuilds within
  a DAW session (egui `Memory`) and resets on full project reopen.

  Under the hood: a 1024-point Hann-windowed real FFT (`realfft 3.5`) runs
  on the audio thread every ~21 ms at 48 kHz. All buffers are pre-allocated
  in `initialize()`; `process()` allocates nothing new (verified under
  `assert_process_allocs`). Bin magnitudes publish to the GUI via a
  lock-free `[AtomicU32; 64]`, mirroring the existing `MeterShared` pattern
  — no mutex, no drop-path allocations to worry about.

### Changed

- **Waveform now fills the OUTPUT display vertically.** Previous scaling
  capped at 42% of display height so a full-scale kick looked mid-volume;
  bumped to 95% of available height, proportional throughout. Quieter
  signals still scale proportionally smaller (the OUTPUT display is an
  honest meter, not auto-normalized).

### Unaffected by this release

- Windows WASAPI auto-probe (`src/windows_standalone.rs`) and macOS
  standalone period-size workaround (`scripts/slammer-macos.sh`) are
  untouched. The spectrum feed is per-sample inside the existing process
  loop, so buffer size and backend negotiation are unchanged.

## [0.4.5] — 2026-04-22

### Fixes

- **Windows standalone keyboard shortcuts now work.** T (trigger) and
  Space (sequencer play/stop) were dead on Windows because nih-plug's
  standalone wrapper opens an outer baseview window whose
  `WindowHandler::on_event` returns `EventStatus::Ignored` for every
  event — including `Event::Keyboard`. Windows routes `WM_KEYDOWN` to
  that outer hwnd, so the egui child window never saw any key events.
  Fixed with a Windows-only `GetAsyncKeyState` poll for T and Space,
  gated by a foreground-window-thread check so background presses in
  other apps can't trigger Slammer. Linux, macOS, and plugin-host
  paths are unchanged.
- **BOUNCE no longer crashes on Windows.** Clicking BOUNCE triggered a
  non-unwinding panic inside `wglSwapLayerBuffers` — the synchronous
  `rfd::FileDialog::save_file()` pumps a nested Win32 message loop,
  which re-entered the egui paint while OpenGL was mid-frame. The
  dialog now runs on a dedicated `slammer-bounce` worker thread; the
  editor polls the result on the next paint.
- **Baseview updated to include RustAudio/baseview#212** (keyboard
  event hook on Windows), so plugin hosts that intercept keyboard
  messages (Ableton etc.) deliver them to Slammer.

### Diagnostics

- **Crash-safe logging.** Every panic — including non-unwinding ones —
  now writes location, payload, and backtrace to
  `%APPDATA%\Slammer\slammer\data\logs\slammer.log` and stderr, so
  a closing Windows console no longer swallows the diagnostic.

## [0.4.4] — 2026-04-21

### Fixes

- **Windows standalone now produces audio automatically.** The standalone
  binary used to fall back to the dummy backend on many Windows setups
  because nih-plug's defaults (48 kHz sample rate, 512-sample period)
  mismatched the device's WASAPI mix format or minimum buffer size.
  `run_standalone()` now probes the default output device on Windows and
  forwards matching `--sample-rate` / `--period-size` to nih-plug via
  `nih_export_standalone_with_args`. Linux and macOS paths are
  unchanged. User-supplied `-r` / `-p` / `-b` flags still win.

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
