# Manual Tempo Entry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the sequencer row BPM readout interactive in standalone mode — drag to scrub, double-click to type, arrow keys to step (±10 / ±1 with Shift). Plugin mode stays read-only.

**Architecture:** Replace the read-only `painter.text` call in `draw_sequencer_row` (`src/ui/panels.rs`) with an interactive widget that allocates a fixed-size rect, runs a small state machine (Idle → Armed → Editing), and writes through the existing `Sequencer::set_bpm` atomic path. No changes to `Sequencer`. A new `TempoEditState` is added to `SequencerUiState`, which is already held in `Arc<Mutex<_>>` in `editor.rs`.

**Tech Stack:** Rust, egui, nih-plug, parking_lot. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-04-06-manual-tempo-entry-design.md`

**Testing note:** This project does not have UI-level tests (see `#[cfg(test)]` only in `src/dsp/*` and preset/telemetry modules). Following the existing pattern, this plan verifies via `cargo check` / `cargo clippy` and a manual smoke test in standalone. No new unit tests — the only new logic is delta accumulation and parse/clamp, both thin glue over the already-tested `Sequencer::set_bpm`.

---

## File Structure

**Modified:**
- `src/ui/panels.rs` — add `TempoEditState`, add field on `SequencerUiState`, replace the BPM `painter.text` block inside `draw_sequencer_row` with an interactive branch

**Unchanged (for reference):**
- `src/sequencer.rs` — `bpm()` / `set_bpm()` / `is_host_synced()` used as-is
- `src/ui/editor.rs:58` — `SequencerUiState::default()` constructor already handles the new field via `#[derive(Default)]`
- `src/ui/theme.rs` — `WHITE`, `TEXT_DIM` used as-is

---

## Task 1: Add `TempoEditState` and wire it into `SequencerUiState`

**Files:**
- Modify: `src/ui/panels.rs` around line 1012 (existing `SequencerUiState` struct)

- [ ] **Step 1: Add the struct and field**

In `src/ui/panels.rs`, find the existing `SequencerUiState`:

```rust
#[derive(Default)]
pub struct SequencerUiState {
    pub paint_mode: Option<bool>,
    pub last_painted: Option<usize>,
}
```

Replace with:

```rust
/// UI-thread state for the tempo readout's interactive widget. Only
/// relevant in standalone mode (host-synced mode shows a read-only label).
///
/// State machine: Idle → Armed (single click) → Editing (double click).
/// Drag and arrow keys apply in Armed state; Editing uses a `TextEdit`.
#[derive(Default)]
pub struct TempoEditState {
    /// Single click armed the widget: arrow keys are live and the text
    /// is highlighted + underlined.
    pub armed: bool,
    /// Double click put the widget into text-entry mode.
    pub editing: bool,
    /// Scratch buffer for the `TextEdit` while `editing` is true.
    pub edit_buf: String,
    /// Accumulated unconsumed vertical drag pixels. Reset when drag ends.
    pub drag_accum: f32,
}

#[derive(Default)]
pub struct SequencerUiState {
    pub paint_mode: Option<bool>,
    pub last_painted: Option<usize>,
    pub tempo_edit: TempoEditState,
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p slammer`
Expected: clean compile (the new field is unused but `#[derive(Default)]` handles construction at `editor.rs:58`).

- [ ] **Step 3: Commit**

```bash
git add src/ui/panels.rs
git commit -m "feat(ui): add TempoEditState scaffold to SequencerUiState"
```

---

## Task 2: Replace the read-only BPM label with the interactive widget

**Files:**
- Modify: `src/ui/panels.rs:1053-1064` (the current `painter.text` BPM block inside `draw_sequencer_row`)

**Context:** The current block reads:

```rust
let bpm_text = if host_synced {
    format!("{:.0} BPM · HOST", seq.display_bpm())
} else {
    format!("{:.0} BPM", seq.display_bpm())
};
painter.text(
    egui::pos2(panel_rect.left() + CONTENT_LEFT + 60.0, row_label_y),
    egui::Align2::LEFT_TOP,
    bpm_text,
    egui::FontId::new(9.0, egui::FontFamily::Monospace),
    theme::TEXT_DIM,
);
```

This is inside a `{ let painter = ui.painter(); ... }` scope that also draws the "STEP" label and the groove. We need to move the BPM drawing out of that scope so we can call `ui.interact(...)` and `ui.input_mut(...)` (which need `&mut ui`, not a painter borrow).

- [ ] **Step 1: Extract the BPM draw out of the painter scope**

In `src/ui/panels.rs`, find the block starting at `// Label + groove` (around line 1043). It currently looks like:

```rust
// Label + groove
{
    let painter = ui.painter();
    painter.text(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_label_y),
        egui::Align2::LEFT_TOP,
        "STEP",
        egui::FontId::new(11.0, egui::FontFamily::Monospace),
        theme::WHITE,
    );
    let bpm_text = if host_synced {
        format!("{:.0} BPM · HOST", seq.display_bpm())
    } else {
        format!("{:.0} BPM", seq.display_bpm())
    };
    painter.text(
        egui::pos2(panel_rect.left() + CONTENT_LEFT + 60.0, row_label_y),
        egui::Align2::LEFT_TOP,
        bpm_text,
        egui::FontId::new(9.0, egui::FontFamily::Monospace),
        theme::TEXT_DIM,
    );
    draw_groove(
        painter,
        panel_rect.left() + CONTENT_LEFT - 4.0,
        panel_rect.right() - CONTENT_LEFT + 4.0,
        row_groove_y,
    );
}
```

Replace with:

```rust
// "STEP" label + groove
{
    let painter = ui.painter();
    painter.text(
        egui::pos2(panel_rect.left() + CONTENT_LEFT, row_label_y),
        egui::Align2::LEFT_TOP,
        "STEP",
        egui::FontId::new(11.0, egui::FontFamily::Monospace),
        theme::WHITE,
    );
    draw_groove(
        painter,
        panel_rect.left() + CONTENT_LEFT - 4.0,
        panel_rect.right() - CONTENT_LEFT + 4.0,
        row_groove_y,
    );
}

// BPM readout — interactive in standalone, read-only in host-synced mode.
draw_tempo_widget(
    ui,
    egui::pos2(panel_rect.left() + CONTENT_LEFT + 60.0, row_label_y),
    seq,
    host_synced,
    &mut ui_state.tempo_edit,
);
```

- [ ] **Step 2: Add a stub `draw_tempo_widget` function**

Add this function to `src/ui/panels.rs` just above `pub fn draw_sequencer_row` (around line 1018). For this step, the stub just replicates the old read-only behavior so compile + runtime are unchanged. The full logic lands in Task 3.

```rust
/// Draw the BPM readout at `pos`. In host-synced mode this is a plain
/// "{:.0} BPM · HOST" label. In standalone mode the readout is an
/// interactive widget — see `TempoEditState` and Task 3 of the plan.
fn draw_tempo_widget(
    ui: &mut egui::Ui,
    pos: egui::Pos2,
    seq: &crate::sequencer::Sequencer,
    host_synced: bool,
    _state: &mut TempoEditState,
) {
    let text = if host_synced {
        format!("{:.0} BPM · HOST", seq.display_bpm())
    } else {
        format!("{:.0} BPM", seq.display_bpm())
    };
    ui.painter().text(
        pos,
        egui::Align2::LEFT_TOP,
        text,
        egui::FontId::new(9.0, egui::FontFamily::Monospace),
        theme::TEXT_DIM,
    );
}
```

- [ ] **Step 3: Verify it compiles and the UI looks identical**

Run: `cargo check -p slammer`
Expected: clean compile.

Run: `cargo build --release -p slammer` (standalone binary target if present, otherwise skip)
Expected: clean build.

- [ ] **Step 4: Commit**

```bash
git add src/ui/panels.rs
git commit -m "refactor(ui): extract BPM readout into draw_tempo_widget stub"
```

---

## Task 3: Implement the interactive widget (drag, click, double-click, arrow keys)

**Files:**
- Modify: `src/ui/panels.rs` — `draw_tempo_widget` function from Task 2

This is the whole feature. It replaces the Task 2 stub in one step.

- [ ] **Step 1: Replace `draw_tempo_widget` with the full implementation**

Replace the entire `draw_tempo_widget` function from Task 2 with:

```rust
/// Draw the BPM readout at `pos`.
///
/// - Host-synced: plain "{:.0} BPM · HOST" label, no interaction.
/// - Standalone:
///     - Single click arms the widget (bright text + underline, arrow keys live)
///     - Double click enters text-entry mode (digits only, 3-char limit)
///     - Vertical drag scrubs at 2 px per BPM (up = faster)
///     - Armed + Up/Down: ±10 BPM; Armed + Shift+Up/Down: ±1 BPM
///     - Clicking elsewhere returns to Idle
///
/// BPM is clamped to [40, 240] by `Sequencer::set_bpm`.
fn draw_tempo_widget(
    ui: &mut egui::Ui,
    pos: egui::Pos2,
    seq: &crate::sequencer::Sequencer,
    host_synced: bool,
    state: &mut TempoEditState,
) {
    // Host-synced: early-return with the read-only label.
    if host_synced {
        state.armed = false;
        state.editing = false;
        state.edit_buf.clear();
        state.drag_accum = 0.0;
        ui.painter().text(
            pos,
            egui::Align2::LEFT_TOP,
            format!("{:.0} BPM · HOST", seq.display_bpm()),
            egui::FontId::new(9.0, egui::FontFamily::Monospace),
            theme::TEXT_DIM,
        );
        return;
    }

    // Fixed-size hit rect so the widget doesn't jitter as the number width
    // changes. Big enough for "240 BPM" in the 9 px mono font.
    let rect = egui::Rect::from_min_size(pos, egui::vec2(48.0, 12.0));
    let font = egui::FontId::new(9.0, egui::FontFamily::Monospace);

    // --- Editing branch: TextEdit grabs focus, Enter/Esc commit/cancel. ---
    if state.editing {
        let te_id = egui::Id::new("tempo_edit_textedit");
        let mut child = ui.child_ui(rect, egui::Layout::left_to_right(egui::Align::Center), None);
        let te = egui::TextEdit::singleline(&mut state.edit_buf)
            .id(te_id)
            .font(font.clone())
            .char_limit(3)
            .desired_width(30.0)
            .margin(egui::vec2(0.0, 0.0));
        let resp = child.add(te);

        // Keep focus on the TextEdit until committed / cancelled.
        if !resp.has_focus() && !resp.lost_focus() {
            resp.request_focus();
        }

        // Strip any non-digit characters the user typed/pasted.
        state.edit_buf.retain(|c| c.is_ascii_digit());
        if state.edit_buf.len() > 3 {
            state.edit_buf.truncate(3);
        }

        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));

        if enter {
            if let Ok(n) = state.edit_buf.parse::<u32>() {
                seq.set_bpm(n as f32); // set_bpm clamps to [40, 240]
            }
            state.editing = false;
            state.edit_buf.clear();
            // Armed remains true — user can keep nudging with arrows.
            state.armed = true;
        } else if esc || resp.lost_focus() {
            state.editing = false;
            state.edit_buf.clear();
            state.armed = true;
        }
        return;
    }

    // --- Idle / Armed branch: draw the text, then handle interaction. ---
    let color = if state.armed { theme::WHITE } else { theme::TEXT_DIM };
    let text = format!("{:.0} BPM", seq.display_bpm());
    ui.painter().text(
        pos,
        egui::Align2::LEFT_TOP,
        &text,
        font.clone(),
        color,
    );

    // Underline when armed.
    if state.armed {
        let underline_y = pos.y + 11.0;
        ui.painter().line_segment(
            [
                egui::pos2(pos.x, underline_y),
                egui::pos2(pos.x + 42.0, underline_y),
            ],
            egui::Stroke::new(1.0, theme::WHITE),
        );
    }

    // Allocate the interactive rect. `click_and_drag` gives us click,
    // double-click, and drag events on the same response.
    let response = ui.interact(
        rect,
        egui::Id::new("tempo_edit_hit"),
        egui::Sense::click_and_drag(),
    );

    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    // Double click → enter text entry.
    if response.double_clicked() {
        state.editing = true;
        state.armed = true;
        state.edit_buf = format!("{:.0}", seq.display_bpm());
        state.drag_accum = 0.0;
        return;
    }

    // Single click → arm (disarms on click elsewhere, handled below).
    if response.clicked() {
        state.armed = true;
    }

    // Vertical drag → scrub.
    if response.dragged() {
        state.armed = true;
        state.drag_accum += response.drag_delta().y;
        let px_per_bpm = 2.0;
        // Up (negative y) = faster.
        let delta_bpm = (-state.drag_accum / px_per_bpm).trunc() as i32;
        if delta_bpm != 0 {
            state.drag_accum += delta_bpm as f32 * px_per_bpm;
            let new_bpm = (seq.bpm().round() as i32 + delta_bpm).clamp(40, 240);
            seq.set_bpm(new_bpm as f32);
        }
    }
    if response.drag_stopped() {
        state.drag_accum = 0.0;
    }

    // Click outside our rect → disarm. We detect this by checking for a
    // pointer-down event anywhere this frame whose position is outside
    // `rect`. Drag on our own rect is handled above and re-arms.
    if state.armed {
        let clicked_elsewhere = ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .map(|p| !rect.contains(p))
                    .unwrap_or(false)
        });
        if clicked_elsewhere {
            state.armed = false;
        }
    }

    // Arrow key handling — only when armed and not editing.
    if state.armed {
        ui.input_mut(|i| {
            let up_10 = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
            let up_1 = i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowUp);
            let down_10 = i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
            let down_1 = i.consume_key(egui::Modifiers::SHIFT, egui::Key::ArrowDown);
            let delta = (up_10 as i32) * 10 + (up_1 as i32)
                - (down_10 as i32) * 10
                - (down_1 as i32);
            if delta != 0 {
                let new_bpm = (seq.bpm().round() as i32 + delta).clamp(40, 240);
                seq.set_bpm(new_bpm as f32);
            }
        });
    }
}
```

- [ ] **Step 2: Verify it compiles cleanly**

Run: `cargo check -p slammer`
Expected: clean compile, no warnings about the previously unused `_state` parameter.

Run: `cargo clippy -p slammer -- -D warnings`
Expected: no clippy errors (the project has a clean-clippy convention — see recent commit `33e0c9b chore(ui): fix clippy warning in preset dropdown color logic`).

If clippy complains about `child_ui` deprecation or `egui::Id` shadowing, fix inline by following clippy's suggestion. If `child_ui` signature differs in the installed egui version (the 4th arg is `Option<UiStackInfo>` in newer versions, no 4th arg in older), adjust the call site to match. Check the egui version with: `cargo tree -p slammer -i egui | head -5`

- [ ] **Step 3: Manual smoke test in standalone**

Run: `cargo run --release -p slammer` (or whichever standalone entry the project uses — check `src/main.rs`)
Expected:
1. Window opens, sequencer row shows "120 BPM" in dim text
2. Single click on "120 BPM" → text brightens and gets an underline
3. Press Up arrow → reads "130 BPM"; press Shift+Up → reads "131 BPM"; press Down → "121 BPM"
4. Click-drag up on the text → BPM increases; drag down → decreases; cursor shows vertical-resize icon on hover
5. Double click → TextEdit appears with current value; type "90", press Enter → reads "90 BPM"
6. Double click again, type "999", press Enter → clamps to "240 BPM"
7. Double click, press Esc → no change
8. Click elsewhere in the window → underline disappears, arrow keys no longer affect BPM

If any of 1-8 fail, diagnose and fix before committing. Common issues:
- **Focus flicker on TextEdit:** `request_focus` every frame is correct but verify `lost_focus()` isn't firing immediately — if it is, check whether the `resp.has_focus()` guard is too aggressive.
- **Arrow keys leak to other widgets:** check that `consume_key` is being called (it should mark the keys as handled). If pattern pads react to arrows, something else is reading the keys first — move the `input_mut` call earlier in the function.
- **Drag interferes with click:** egui's `click_and_drag` sense fires `clicked()` only on release-without-drag, so this should be fine; if `clicked()` fires spuriously after drag, gate it on `!response.drag_stopped()`.

- [ ] **Step 4: Commit**

```bash
git add src/ui/panels.rs
git commit -m "feat(ui): interactive BPM entry — drag, arrows, type-in, standalone only"
```

---

## Task 4: Final verification

- [ ] **Step 1: Clean build + clippy**

Run: `cargo build --release -p slammer && cargo clippy -p slammer -- -D warnings`
Expected: clean build, no clippy warnings.

- [ ] **Step 2: Confirm host-mode read-only path is untouched**

There's no automated host harness; visually verify by launching slammer in a DAW (Bitwig per `project_slammer_state.md`) and confirming:
1. Sequencer row shows "... BPM · HOST"
2. Click / double-click / drag on the label has no effect
3. Arrow keys do nothing to the BPM

If no DAW is available right now, skip and note it under "Verification still pending" — mirror how `project_slammer_state.md` already tracks manual DAW checks.

- [ ] **Step 3: Update memory snapshot (optional, only if resuming later)**

If this is the last work in the session, nothing to do. If you expect to resume, add a one-liner to `project_slammer_state.md` under known shipped features noting that manual tempo entry lands in this commit. Use the existing format.

---

## Out of scope (explicitly)

- Tap tempo — not requested
- Fractional BPM — spec says integer only
- Host-mode tempo override — spec says host is master in plugin mode
- Persisting standalone BPM across sessions — existing `Sequencer` state handling already decides this; no changes
- New unit tests — project has no UI test harness and the new logic is thin glue over `Sequencer::set_bpm`, which already clamps
