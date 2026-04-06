# Manual Tempo Entry — Design

**Date:** 2026-04-06
**Scope:** Make the sequencer row BPM readout interactive in standalone mode so users can set tempo without a host.

## Motivation

Slammer currently displays tempo as a read-only label in the sequencer row (`src/ui/panels.rs`, inside `draw_sequencer_row`). The underlying `Sequencer` already owns a `bpm_milli: AtomicU32` with `set_bpm()` that clamps to `[MIN_BPM=40, MAX_BPM=240]`, but there is no UI path to change it. In standalone mode this means the user is stuck at the 120 BPM default.

## Constraints

- **Plugin mode (host-synced) stays read-only.** When `seq.is_host_synced()` is true, the host is authoritative — the readout keeps showing `"{:.0} BPM · HOST"` with no interaction. This matches how the PLAY/STOP button already behaves (see `panels.rs:1083`).
- **Integer BPM only.** No fractional values; no comma / decimal entry. 2–3 digit input.
- **Clamp to existing range** `[40, 240]` via `Sequencer::set_bpm`.
- **RT-safe.** All writes go through the existing atomic path — no new shared state.

## Interactions

All interactions apply only when `!seq.is_host_synced()`.

| Interaction | Effect |
|---|---|
| Single click on BPM text | Enters **Armed** state (visual highlight + underline) |
| Click elsewhere | Returns to **Idle** state |
| Double-click | Enters **Editing** state with inline `TextEdit` |
| Vertical drag (any state except Editing) | Live-adjusts BPM, 2 px per 1 BPM, up = faster; cursor becomes `ResizeVertical`. Remains Armed on release. |
| `↑` / `↓` while Armed | ±10 BPM |
| `Shift + ↑` / `Shift + ↓` while Armed | ±1 BPM |
| Enter while Editing | Parse, clamp, commit → Armed |
| Esc / focus lost while Editing | Discard buffer → Armed |

Arrow keys are intentionally `±10` / `Shift±1` per the feature request (larger step by default, fine step with Shift) — this is the user's explicit preference and inverts the usual convention.

## State

A new small struct lives on the UI side alongside the existing `SequencerUiState`:

```rust
#[derive(Default)]
pub struct TempoEditState {
    pub armed: bool,
    pub editing: bool,
    pub edit_buf: String,
    pub drag_accum: f32, // accumulated unconsumed drag pixels
}
```

Ownership: added as a field on `SequencerUiState` in `panels.rs` (or wherever the editor currently stores it — identified during implementation). No changes to the `Sequencer` struct itself.

## State machine

```
Idle ──click──► Armed ──click-elsewhere──► Idle
 │                │
 │                ├──drag-vertical──► updates bpm live, stays Armed
 │                └──arrow/shift-arrow──► updates bpm, stays Armed
 │
 └──double-click──► Editing (TextEdit focused)
                        │
                        ├──Enter──► parse → clamp → set_bpm → Armed
                        └──Esc / focus lost──► discard → Armed
```

Double-clicking directly from Idle bypasses Armed and goes to Editing. Drag from Idle implicitly arms the widget.

## Rendering

- **Idle:** current style — `theme::TEXT_DIM`, 9 px mono, same position (`CONTENT_LEFT + 60.0, row_label_y`).
- **Armed:** `theme::WHITE` + a 1 px underline drawn under the text baseline to signal "keys active".
- **Editing:** `egui::TextEdit::singleline(&mut edit_buf)` with `char_limit(3)`, a digit-only input filter, sized to fit "240" in the 9 px mono font.
- **Host-synced:** early-return with today's static text path; no interactive rect allocated.

The interactive rect is a fixed size (~48 × 12 px, enough for "240 BPM") so hit-testing doesn't jitter as the number changes width.

## Drag math

```rust
state.drag_accum += response.drag_delta().y;
let px_per_bpm = 2.0;
let delta_bpm = (state.drag_accum / -px_per_bpm).trunc() as i32; // up = faster
if delta_bpm != 0 {
    state.drag_accum += delta_bpm as f32 * px_per_bpm;
    seq.set_bpm((seq.bpm().round() as i32 + delta_bpm).clamp(40, 240) as f32);
}
```

`drag_accum` resets to 0 when drag ends so stray residuals don't leak across gestures.

## Arrow key handling

```rust
if state.armed && !state.editing {
    ui.input_mut(|i| {
        let up_10   = i.consume_key(egui::Modifiers::NONE,  egui::Key::ArrowUp);
        let up_1    = i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
        let down_10 = i.consume_key(egui::Modifiers::NONE,  egui::Key::ArrowDown);
        let down_1  = i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
        let delta = (up_10 as i32) * 10 + (up_1 as i32) - (down_10 as i32) * 10 - (down_1 as i32);
        if delta != 0 {
            let new_bpm = (seq.bpm().round() as i32 + delta).clamp(40, 240);
            seq.set_bpm(new_bpm as f32);
        }
    });
}
```

`consume_key` prevents the event from leaking to other widgets (e.g. the sequencer pattern).

## Commit path (editing → armed)

```rust
if let Ok(n) = state.edit_buf.parse::<u32>() {
    seq.set_bpm((n as f32).clamp(40.0, 240.0));
}
state.edit_buf.clear();
state.editing = false;
// armed remains true
```

Parse failure is silent — the buffer is discarded and the previous BPM stands.

## Files touched

- `src/ui/panels.rs`
  - Add `TempoEditState` struct (or inline on `SequencerUiState`)
  - Replace the static `painter.text(... bpm_text ...)` block at lines ~1053–1064 with an interactive widget branch for `!host_synced`
  - Keep the existing branch for `host_synced`
- Editor state owner (found during implementation) to hold `TempoEditState` across frames

No changes to `src/sequencer.rs` — the existing `bpm()` / `set_bpm()` API is sufficient.

## Out of scope

- Fractional BPM
- Tap-tempo
- BPM automation / parameter exposure to the host (slammer is host-synced in plugin mode)
- Visual tempo LED / seven-seg redesign — the readout style is unchanged apart from Armed highlight
