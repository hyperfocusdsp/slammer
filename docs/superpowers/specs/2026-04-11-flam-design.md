# Flam, Ruff & Roll — Sequencer-Level Multi-Stroke Steps

**Status:** design approved, ready for implementation plan
**Date:** 2026-04-11

## Goal

Add per-step flam/ruff/roll capability to the 16-step sequencer so that a single step can fire 2, 3 or 4 closely-spaced hits instead of one. Primary motivation: enable clap-like layered sounds in slammer (which is otherwise a single-hit kick synth), and also fatten kick hits when desired. This is a general-purpose drum-machine feature, not a clap-specific synthesis mode.

## Scope summary

| Item | In scope |
|---|---|
| Per-step Off / Flam / Ruff / Roll state | ✓ |
| Global spread (2–30 ms) and humanize (0–1) params | ✓ |
| Timing + velocity jitter via humanize | ✓ |
| Expand voice pool from 2 → 4 | ✓ |
| Right-click cycle interaction + dot indicator | ✓ |
| Persistence (preset + DAW state) | ✓ |
| Engine-level "clap mode" synthesis | ✗ deferred |
| Per-step (non-global) spread values | ✗ deferred |

## Data model

**New sequencer state (`src/sequencer.rs`):**

```rust
pub const STEPS: usize = 16;

pub struct Sequencer {
    // ...existing fields...
    flam_state: [AtomicU8; STEPS],   // 0=Off, 1=Flam, 2=Ruff, 3=Roll (UI writes, audio reads)
    // Audio-thread-only state. Interior mutability mechanism (UnsafeCell,
    // per-slot atomics, or try_lock Mutex) chosen at implementation time
    // to match how `Sequencer` is already shared between threads — see the
    // existing pattern in `src/sequencer.rs`.
    pending: [PendingHit; 12],       // ring of scheduled upcoming hits
    rng_state: u32,                  // xorshift32 for humanize
}
```

`flam_state` is kept separate from the existing `steps: u16` bitmask because 2 bits × 16 steps (32 bits) doesn't fit in the current field, and making this additive lets old presets deserialize cleanly with flam all-zero.

**New parameters (`src/params.rs`):**

```rust
#[id = "flam_spread"]
pub flam_spread_ms: FloatParam, // 2.0..30.0, default 15.0, skewed

#[id = "flam_human"]
pub flam_humanize: FloatParam,  // 0.0..1.0, default 0.3, linear
```

Both are global (not per-step) and automatable.

**Persistence (`ParamSnapshot` in `src/params.rs`):**

```rust
#[serde(default)]
pub flam_states: [u8; 16],
#[serde(default = "default_flam_spread")]
pub flam_spread_ms: f32,
#[serde(default = "default_flam_humanize")]
pub flam_humanize: f32,
```

The `#[serde(default)]` attribute is required so old presets without these fields deserialize to Off/defaults instead of failing. Also add the new states to `seq_steps` persistence — since flam state lives in its own array, we need a second `#[persist]` field, `seq_flam`, serialized as `[u8; 16]` packed into a single `u64` (2 bits × 16 = 32 bits fit easily; use `u64` for headroom).

## Scheduling: the pending-hit ring

The audio thread schedules multi-stroke hits via a small fixed-size ring on the `Sequencer` struct.

```rust
#[derive(Copy, Clone, Default)]
struct PendingHit {
    samples_until: u32,
    velocity: f32,
    live: bool,
}
```

Stored as a plain `[PendingHit; 12]` — no heap, no reallocation, pre-sized for the pathological case (4 hits × 3 overlapping steps is already beyond musically reasonable). Access from the audio thread only; no cross-thread sync needed because only the audio thread mutates it.

**Step-boundary logic** (runs inside the existing `Sequencer::tick_sample` or equivalent):

1. When the playhead crosses a step boundary and the step is active:
   - Read `flam_state[step]` → number of hits `n` (1 if Off, 2 if Flam, 3 if Ruff, 4 if Roll).
   - Compute base inter-stroke gap in samples: `gap = round(spread_ms * sr / 1000)`.
   - Compute base velocities:
     - Flam: `[0.7, 1.0]`
     - Ruff: `[0.7, 0.85, 1.0]`
     - Roll: `[0.6, 0.75, 0.85, 1.0]`
     - Single (`n=1`): `[velocity]`
   - For each of the `n` hits:
     - `offset_samples = (n - 1 - i) * gap`, plus humanize jitter: `±round(gap * 0.2 * humanize * rng_f32_centered())`.
     - `velocity_final = base_vel[i] * step_velocity * (1.0 + 0.1 * humanize * rng_f32_centered())`.
     - Insert into the first `!live` slot of `pending`: `samples_until = offset_samples`, `live = true`.
2. Offsets are relative to "now" (current sample), so a flammed step whose first stroke lands 100 samples from the buffer end continues scheduling cleanly into the next buffer — no special buffer-boundary handling.

**Per-sample tick in the process loop:**

```rust
for slot in &mut self.pending {
    if !slot.live { continue; }
    if slot.samples_until == 0 {
        engine.trigger_with_velocity(slot.velocity);
        slot.live = false;
    } else {
        slot.samples_until -= 1;
    }
}
```

This loop is 12 iterations, branch-predictable, entirely stack-local. RT-safe by construction.

**PRNG:** `xorshift32` inline, seeded once at plugin init from a non-zero constant (or `instant::now` nanos if available). Deterministic per-run so tests can assert exact outputs at a given seed; the user still hears variation across sessions because humanize jitter is evaluated on every flammed step, not cached.

## Voice pool change

`NUM_VOICES`: `2` → `4` in `src/dsp/engine.rs`.

- A roll fires 4 hits within up to ~90 ms. With 2 voices, hits 3 and 4 steal voices 1 and 2 while their amp envelopes are still at ~full level, producing an audible dip in the earliest hits' body.
- 4 voices give every hit in a roll its own slot, so voice stealing only kicks in on the *next* downbeat — which is musically correct (the roll's tail is still sounding when the new hit arrives, the 5 ms fadeout smooths the transition).
- 3 would be too tight: after a 4-hit roll, the next step's trigger would steal mid-tail.
- >4 buys nothing musical; stops being audible past overlapping patterns that are already mud.

Cost is trivial — 4 × voice struct is still single-digit KB, pre-allocated in `KickEngine::new()`, no RT-safety implications.

The existing 5 ms linear fadeout voice-stealing logic is unchanged.

## UI

**Files:** `src/ui/panels.rs` (sequencer row + STEP knobs), `src/sequencer.rs` (state accessors).

### Step pad interaction

In `draw_sequencer_row`:

- **Left-click / left-drag:** unchanged. Toggles the `steps` bit; paint-drag fills.
- **Right-click on an active step:** cycles `flam_state[i]` Off → Flam → Ruff → Roll → Off.
- **Right-click on an inactive step:** no-op.
- **Turning a step off (left-click):** clears `flam_state[i]` to 0 atomically, so re-enabling later starts from Off rather than inheriting ghost state.

Add a new branch in the existing click-handling block that checks `resp.secondary_clicked()`.

### Dot indicator

For each active step where `flam_state[i] > 0`, paint small filled circles above the pad:

- Count: `flam_state[i] + 1` (Flam → 2 dots, Ruff → 3, Roll → 4)
- Diameter: 3 px
- Gap: 2 px
- Centered horizontally above the pad
- Vertical position: pad top − 6 px (in the otherwise-unused gutter above the row)
- Color: `theme::RED_GHOST`
- Never painted on inactive steps

Dots draw *before* the playhead ring so the ring overlays them cleanly when the playhead visits the step.

### Tooltip

On right-click hover over any active step, the `on_hover_text` shows `"Right-click: Flam (2) / Ruff (3) / Roll (4)"` so the feature is discoverable without docs.

### SPREAD and HUMAN knobs

Two new knobs in the STEP row, painted in the right-hand gap after step 16. Layout:

- `SPREAD` knob — labeled `"SPRD"`, shows `flam_spread_ms`, unit `" ms"`, rounded integer.
- `HUMAN` knob — labeled `"HUM"`, shows `flam_humanize`, percentage format.

Sized and painted using the existing small-knob style from the COMP strip. Section color: a new `theme::SECTION_STEP` (or reuse `RED_GHOST` if a new section color is overkill — decision at implementation time).

## Persistence details

Two concrete additions:

1. **New persist field on `SlammerParams`:**
   ```rust
   #[persist = "seq_flam"]
   pub seq_flam: Arc<Mutex<u64>>, // 16 × 2 bits packed
   ```
2. **`Plugin::initialize`** copies packed `seq_flam` bits into `Sequencer::flam_state` atomics, mirroring how `seq_steps` is already restored.
3. **`ParamSnapshot::capture`/`apply`** read/write `flam_states` as `[u8; 16]` (simpler than packed — JSON preset files are text anyway). `#[serde(default)]` gated to survive old presets.

State-restored race protection is already handled by the existing `state_restored` gate — no new logic needed.

## Testing

### Sequencer unit tests (`src/sequencer.rs` or a new `tests/flam.rs`)

- `flam_state_default_off` — new Sequencer has all zeros; plain steps fire exactly once.
- `flam_2_schedules_two_hits` — flam on step 4, SR=48k, spread=15 ms, humanize=0; step sample-by-sample, assert exactly 2 engine-trigger calls separated by 720 samples with velocities `[0.7, 1.0]`.
- `ruff_schedules_three_hits` — 3 fires, `[0.7, 0.85, 1.0]`.
- `roll_schedules_four_hits` — 4 fires, `[0.6, 0.75, 0.85, 1.0]`.
- `flam_spans_buffer_boundary` — schedule a flam with 120 samples left in the simulated buffer; advance past the boundary; assert all hits still fire at the correct absolute offsets from the step boundary.
- `humanize_is_deterministic_per_seed` — fixed PRNG seed, run twice, identical output.
- `humanize_zero_is_exact_spread_and_velocity` — `humanize=0` produces exact base amplitudes and exact base spreads regardless of PRNG state.
- `humanize_one_produces_bounded_jitter` — `humanize=1`, 1000 trials, assert velocity jitter stays within ±10% and timing jitter within ±20% of base.
- `turning_step_off_clears_flam_state` — left-click on an active flammed step atomically clears `flam_state[i]` to 0.
- `right_click_cycles_through_four_states` — simulated right-click on step 0 four times, assert state sequence Off → Flam → Ruff → Roll → Off.

The engine is a test double (`trait TriggerSink` with a fake that records `(sample, velocity)` pairs) so these tests don't need a real `KickEngine`.

### Engine integration test (`src/dsp/engine.rs`)

- `four_voice_pool_handles_roll` — construct a KickEngine with the new 4-voice pool, fire a roll step, run process() through ~100 ms at 48k, assert that the output RMS is non-zero across the full roll duration and that no sample window longer than 2 ms is fully silent in the middle. This is a "the sound exists" test, not a frequency-response test, so wideband RMS is acceptable (see `feedback_saturation_test_fundamental` — this explicitly isn't a saturation test).

### Persistence round-trip tests

- `param_snapshot_roundtrip_flam` — set all 16 flam states to varied values and both new params to non-default values; serialize `ParamSnapshot` to JSON; deserialize; assert equality.
- `old_preset_loads_with_flam_defaults` — deserialize a pre-flam JSON preset blob (missing all new fields); assert `flam_states` are all zero, `flam_spread_ms == 15.0`, `flam_humanize == 0.3`.

### Manual DAW verification (add to pending list in `project_slammer_state.md`)

- Flam / Ruff / Roll audibly distinct in Bitwig on a loaded factory preset.
- Humanize knob sweep 0 → 1 produces clearly increasing jitter without the pattern losing sync.
- Right-click cycle discoverable; dots render cleanly and don't clip the playhead ring.
- Pattern with heavy flam state survives save / reload round-trip.
- Clap-like preset (high MID noise, short decay, ruff on every backbeat) sounds like an analog clap.

## Files touched

| File | Change |
|---|---|
| `src/sequencer.rs` | `flam_state`, `pending` ring, `rng_state`, new scheduling logic, accessors for UI |
| `src/dsp/engine.rs` | `NUM_VOICES: 2 → 4`, `trigger_with_velocity` entrypoint if not already present |
| `src/params.rs` | `flam_spread_ms`, `flam_humanize` on `SlammerParams`; `flam_states`, `flam_spread_ms`, `flam_humanize` on `ParamSnapshot` (with `#[serde(default)]`); `seq_flam` persist field |
| `src/plugin.rs` | `initialize()` copies persisted `seq_flam` into `Sequencer::flam_state` |
| `src/ui/panels.rs` | Right-click handling, dot rendering, SPREAD/HUMAN knobs in STEP row, tooltip |
| `src/ui/theme.rs` | Optionally add `SECTION_STEP` color |
| `src/sequencer.rs` tests (or new `tests/flam.rs`) | All unit tests listed above |

No changes to `master_bus`, `saturation`, `tube`, `filter`, or the export/bounce module — flam is contained entirely in sequencer + engine + params + UI.

## Deferred scope (future work, captured here)

After flam ships, the following features are next, in recommended order of impact:

1. **EQ restructure for intent-based naming** — replace the current `tilt / low_boost / notch_freq / notch_q / notch_depth` EQ row with `PUNCH / BODY / CLICK / TILT` (peaking EQ at 80 Hz / 200 Hz, high shelf at 3 kHz, overall tone tilt). Biggest UX impact; breaks old presets' notch settings (graceful default on load). Spec to be written when pulled in.
2. **Per-voice low-cut on SUB and MID** — 12 dB/oct HPF biquad per voice, `sub_lowcut_hz` and `mid_lowcut_hz` params (20–400 Hz skewed, 20 Hz = bypassed). Cheap; composes with the new EQ to cleanly separate *layer shaping* (per-voice HPF) from *bus shaping* (master EQ). TOP is excluded — it's a transient click, HPF would gut it.
3. **Compressor shape controls** — split the current `REACT` macro into explicit `ATTACK` (1.5–30 ms) and `RELEASE` (40–400 ms) knobs; add `KNEE` (0–12 dB soft knee). Keeps `AMT` and `DRV` and `LIM` unchanged. Refinement of the shipped comp.

None of these are part of this spec; they're recorded so the design intent is preserved when they're pulled in.

## Open questions

None as of approval. The PRNG seed source (constant vs wall-clock nanos) is a small implementation-time decision with no spec impact.
