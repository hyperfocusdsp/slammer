# Preset Bar & UI Scaling Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the preset bar from the footer into the header as an LED display, and make the entire UI scale proportionally with locked aspect ratio when the window is resized.

**Architecture:** Two changes to `editor.rs`: (1) replace the footer preset bar with a header-integrated preset strip using an LED display, arrows, dropdown, and inline editing; (2) compute a scale factor `s = win_w / BASE_W` each frame and pass it through all custom painting so everything scales proportionally. `knob.rs` gains a `scale` parameter. The `ResizableWindow` resize callback is patched to enforce aspect ratio.

**Tech Stack:** Rust, nih-plug, nih_plug_egui (egui 0.31.1).

**Important egui gotchas (from project history):**
- `rect_stroke` needs 4 args: `(rect, rounding, stroke, StrokeKind::Outside)`
- `Margin::same()` takes `i8`, not `f32`
- Use `clip_rect().width()` not `available_width()` in horizontal layouts

---

### Task 1: Add scale factor computation and aspect-ratio lock

**Files:**
- Modify: `src/ui/editor.rs` (lines 15-20 constants, lines 84-108 render loop top)

- [ ] **Step 1: Add aspect ratio constant and update scaling logic**

In `src/ui/editor.rs`, after the existing constants (line 20), add:

```rust
const ASPECT: f32 = BASE_W / BASE_H;
```

Then replace the scaling + ResizableWindow section (lines 85-108) with:

```rust
            // Scale UI to match window size
            let (win_w, win_h) = editor_state_clone.size();
            let s = (win_w as f32 / BASE_W).max(1.0);
            ctx.set_pixels_per_point(s);

            ResizableWindow::new("slammer_resize")
                .min_size(egui::vec2(BASE_W, BASE_H))
                .show(ctx, &editor_state_clone, |ui| {
                // Enforce aspect ratio: if host gave us a non-matching size, request correction
                let expected_h = (win_w as f32 / ASPECT).round() as u32;
                if win_h != expected_h {
                    editor_state_clone.set_requested_size((win_w, expected_h));
                }

                ui.set_min_size(ui.available_size());
                let panel_rect = ui.max_rect();
                let w = panel_rect.width();
                let h = panel_rect.height();
```

Everything below this point stays the same — positions are defined in logical pixels (BASE_W coordinate space) and `pixels_per_point` scales them automatically.

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -20`

Expected: Clean check (or only pre-existing warnings).

- [ ] **Step 3: Commit**

```bash
git add src/ui/editor.rs
git commit -m "feat(ui): add aspect-ratio locked scaling via pixels_per_point"
```

---

### Task 2: Update PresetBarState for new interactions

**Files:**
- Modify: `src/ui/editor.rs` (lines 43-63, PresetBarState struct)

- [ ] **Step 1: Add editing and dropdown state fields**

Replace the `PresetBarState` struct and its `Default` impl (lines 43-63) with:

```rust
/// Mutable UI state for the preset bar.
struct PresetBarState {
    /// Currently loaded preset name.
    selected_name: String,
    /// Index of current preset in the full list (for arrow cycling).
    selected_index: usize,
    /// Whether the LED display is in edit mode (SAVE clicked).
    editing: bool,
    /// Text being edited in save mode.
    edit_buffer: String,
    /// Whether the preset dropdown is open.
    dropdown_open: bool,
    /// Status message shown briefly after save/delete/error.
    status_msg: String,
    status_timer: f32,
}

impl Default for PresetBarState {
    fn default() -> Self {
        Self {
            selected_name: "Init".into(),
            selected_index: 0,
            editing: false,
            edit_buffer: String::new(),
            dropdown_open: false,
            status_msg: String::new(),
            status_timer: 0.0,
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | head -30`

Expected: Errors about removed `name_input` field — these will be fixed when the footer preset bar is replaced in Task 3. That's fine.

- [ ] **Step 3: Commit**

```bash
git add src/ui/editor.rs
git commit -m "refactor(ui): update PresetBarState for header-integrated preset bar"
```

---

### Task 3: Remove footer preset bar, add header-integrated preset bar

**Files:**
- Modify: `src/ui/editor.rs` (lines 447-625 footer section → delete; lines 148-174 header section → extend)

This is the largest task. It removes the entire footer preset bar (combo box, text input, SAVE, DEL) and replaces it with the header-integrated version.

- [ ] **Step 1: Delete the footer preset bar**

Remove the old preset bar code. Keep the footer groove and "REXIST INSTRUMENTS" brand text. Delete everything from the comment `// Preset bar lives in an allocated rect` (line 463) through the end of that `ui.allocate_new_ui` block (line 624, the `});` before the final `});`).

The footer section (lines 447-461) stays — it draws the groove and brand text.

Also remove the `small_metal_button` function (lines 1057-1082) — we'll write a new version below.

- [ ] **Step 2: Add header preset bar drawing code**

After the existing header section (after the version text at line 173, before the `}` closing brace at line 174), insert the header preset bar. This goes inside the same `{ let painter = ui.painter(); ... }` block that draws the header, but the interactive elements need to be outside the painter block.

Close the existing painter block first (the `}` at line 174 stays), then add a new section:

```rust
                // ===== HEADER PRESET BAR =====
                {
                    let header_y = panel_rect.top() + 10.0;
                    // Position after TEST button
                    let preset_x = panel_rect.left() + CONTENT_LEFT + 196.0;
                    let preset_y = header_y + 1.0;
                    let display_w = 130.0;
                    let display_h = 16.0;
                    let arrow_size = 16.0;
                    let btn_w = 28.0;
                    let btn_h = 14.0;

                    // Divider line
                    {
                        let painter = ui.painter();
                        painter.line_segment(
                            [egui::pos2(preset_x - 8.0, preset_y), egui::pos2(preset_x - 8.0, preset_y + display_h)],
                            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x28, 0x28, 0x28)),
                        );
                    }

                    // Fetch preset list
                    let all_presets = {
                        let mut pm = preset_manager.lock();
                        pm.refresh();
                        pm.list_all()
                    };

                    let (selected_name, is_editing, dropdown_open) = {
                        let pb = preset_bar.lock();
                        (pb.selected_name.clone(), pb.editing, pb.dropdown_open)
                    };

                    let is_factory = all_presets.iter().any(|e| e.is_factory && e.preset.name == selected_name);

                    // --- Left arrow ---
                    let left_rect = egui::Rect::from_min_size(
                        egui::pos2(preset_x, preset_y + (display_h - arrow_size) * 0.5),
                        egui::vec2(arrow_size, arrow_size),
                    );
                    if !is_editing {
                        let left_resp = ui.interact(left_rect, egui::Id::new("preset_prev"), egui::Sense::click());
                        {
                            let painter = ui.painter();
                            let color = if is_editing { theme::TEXT_GHOST } else if left_resp.hovered() { theme::WHITE } else { theme::TEXT_DIM };
                            preset_arrow_btn(painter, left_rect, "\u{25C2}", color);
                        }
                        if left_resp.clicked() && !all_presets.is_empty() {
                            let mut pb = preset_bar.lock();
                            let idx = if pb.selected_index == 0 { all_presets.len() - 1 } else { pb.selected_index - 1 };
                            pb.selected_index = idx;
                            let preset = &all_presets[idx].preset;
                            pb.selected_name = preset.name.clone();
                            apply_preset_owned(setter, &params, preset);
                        }
                    } else {
                        let painter = ui.painter();
                        preset_arrow_btn(painter, left_rect, "\u{25C2}", theme::TEXT_GHOST);
                    }

                    // --- LED display ---
                    let led_rect = egui::Rect::from_min_size(
                        egui::pos2(preset_x + arrow_size + 2.0, preset_y),
                        egui::vec2(display_w, display_h),
                    );

                    if is_editing {
                        // Edit mode: draw LED frame with red border, show text input
                        {
                            let painter = ui.painter();
                            painter.rect_filled(led_rect, 0.0, theme::BG_DISPLAY);
                            painter.rect_stroke(
                                led_rect,
                                0.0,
                                egui::Stroke::new(2.0, theme::RED_LED),
                                egui::StrokeKind::Outside,
                            );
                        }
                        let mut edit_buf = {
                            preset_bar.lock().edit_buffer.clone()
                        };
                        let input_rect = led_rect.shrink(2.0);
                        let te_resp = ui.allocate_new_ui(egui::UiBuilder::new().max_rect(input_rect), |ui| {
                            let te = egui::TextEdit::singleline(&mut edit_buf)
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .text_color(theme::RED_LED)
                                .frame(false)
                                .desired_width(input_rect.width());
                            let resp = ui.add(te);
                            // Auto-focus on first frame
                            if !resp.has_focus() {
                                resp.request_focus();
                            }
                            resp
                        }).inner;

                        preset_bar.lock().edit_buffer = edit_buf.clone();

                        // Enter to save
                        if te_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            let name = edit_buf.trim().to_owned();
                            if !name.is_empty() {
                                let snapshot = snapshot_params(&params);
                                let result = preset_manager.lock().save(&name, snapshot);
                                let mut pb = preset_bar.lock();
                                match result {
                                    Ok(()) => {
                                        pb.selected_name = name.clone();
                                        pb.status_msg = format!("Saved: {}", name);
                                        pb.status_timer = 3.0;
                                    }
                                    Err(e) => {
                                        pb.status_msg = e;
                                        pb.status_timer = 4.0;
                                    }
                                }
                                pb.editing = false;
                            }
                        }
                        // ESC to cancel
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            preset_bar.lock().editing = false;
                        }
                    } else {
                        // Normal mode: draw LED display with preset name
                        {
                            let painter = ui.painter();
                            draw_inset_display(painter, led_rect.left(), led_rect.top(), display_w, display_h);
                            draw_7seg_text(painter, led_rect, &selected_name);
                        }

                        // Click to open dropdown
                        let led_resp = ui.interact(led_rect, egui::Id::new("preset_led"), egui::Sense::click());
                        if led_resp.clicked() {
                            let mut pb = preset_bar.lock();
                            pb.dropdown_open = !pb.dropdown_open;
                        }

                        // Draw dropdown if open
                        if dropdown_open {
                            let dd_width = display_w.max(160.0);
                            let dd_max_h = 200.0;
                            let dd_item_h = 16.0;
                            let dd_h = (all_presets.len() as f32 * dd_item_h + 4.0).min(dd_max_h);
                            let dd_rect = egui::Rect::from_min_size(
                                egui::pos2(led_rect.left(), led_rect.bottom() + 1.0),
                                egui::vec2(dd_width, dd_h),
                            );

                            // Background
                            let painter = ui.painter();
                            painter.rect_filled(dd_rect, 2.0, egui::Color32::from_rgb(0x0e, 0x0e, 0x0e));
                            painter.rect_stroke(
                                dd_rect,
                                2.0,
                                egui::Stroke::new(1.0, egui::Color32::from_rgb(0x22, 0x22, 0x22)),
                                egui::StrokeKind::Outside,
                            );

                            let factory_count = all_presets.iter().filter(|e| e.is_factory).count();
                            let mut item_y = dd_rect.top() + 2.0;

                            for (idx, entry) in all_presets.iter().enumerate() {
                                // Separator between factory and user presets
                                if !entry.is_factory && idx == factory_count {
                                    painter.line_segment(
                                        [egui::pos2(dd_rect.left() + 4.0, item_y), egui::pos2(dd_rect.right() - 4.0, item_y)],
                                        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x33, 0x33)),
                                    );
                                    item_y += 2.0;
                                }

                                let item_rect = egui::Rect::from_min_size(
                                    egui::pos2(dd_rect.left(), item_y),
                                    egui::vec2(dd_width, dd_item_h),
                                );

                                let label = if entry.is_factory {
                                    entry.preset.name.clone()
                                } else {
                                    format!("* {}", entry.preset.name)
                                };

                                let item_resp = ui.interact(item_rect, egui::Id::new(format!("dd_{}", idx)), egui::Sense::click());
                                let is_sel = entry.preset.name == selected_name;
                                let color = if is_sel {
                                    theme::RED_LED
                                } else if item_resp.hovered() {
                                    theme::RED_LED
                                } else if entry.is_factory {
                                    egui::Color32::from_rgb(0x77, 0x77, 0x77)
                                } else {
                                    egui::Color32::from_rgb(0x99, 0x99, 0x99)
                                };

                                if item_resp.hovered() {
                                    painter.rect_filled(item_rect, 0.0, egui::Color32::from_rgb(0x1a, 0x1a, 0x1a));
                                }

                                painter.text(
                                    egui::pos2(item_rect.left() + 8.0, item_rect.center().y),
                                    egui::Align2::LEFT_CENTER,
                                    &label,
                                    egui::FontId::new(9.0, egui::FontFamily::Monospace),
                                    color,
                                );

                                if item_resp.clicked() {
                                    apply_preset_owned(setter, &params, &entry.preset);
                                    let mut pb = preset_bar.lock();
                                    pb.selected_name = entry.preset.name.clone();
                                    pb.selected_index = idx;
                                    pb.dropdown_open = false;
                                }

                                item_y += dd_item_h;
                            }

                            // Close dropdown if clicking outside
                            if ui.input(|i| i.pointer.any_click()) && !led_resp.clicked() {
                                let pointer_pos = ui.input(|i| i.pointer.interact_pos());
                                if let Some(pos) = pointer_pos {
                                    if !dd_rect.contains(pos) {
                                        preset_bar.lock().dropdown_open = false;
                                    }
                                }
                            }
                        }
                    }

                    // --- Right arrow ---
                    let right_rect = egui::Rect::from_min_size(
                        egui::pos2(led_rect.right() + 2.0, preset_y + (display_h - arrow_size) * 0.5),
                        egui::vec2(arrow_size, arrow_size),
                    );
                    if !is_editing {
                        let right_resp = ui.interact(right_rect, egui::Id::new("preset_next"), egui::Sense::click());
                        {
                            let painter = ui.painter();
                            let color = if right_resp.hovered() { theme::WHITE } else { theme::TEXT_DIM };
                            preset_arrow_btn(painter, right_rect, "\u{25B8}", color);
                        }
                        if right_resp.clicked() && !all_presets.is_empty() {
                            let mut pb = preset_bar.lock();
                            let idx = if pb.selected_index >= all_presets.len() - 1 { 0 } else { pb.selected_index + 1 };
                            pb.selected_index = idx;
                            let preset = &all_presets[idx].preset;
                            pb.selected_name = preset.name.clone();
                            apply_preset_owned(setter, &params, preset);
                        }
                    } else {
                        let painter = ui.painter();
                        preset_arrow_btn(painter, right_rect, "\u{25B8}", theme::TEXT_GHOST);
                    }

                    // --- SAVE button ---
                    let save_x = right_rect.right() + 6.0;
                    let save_rect = egui::Rect::from_min_size(
                        egui::pos2(save_x, preset_y + (display_h - btn_h) * 0.5),
                        egui::vec2(btn_w, btn_h),
                    );
                    let save_resp = ui.interact(save_rect, egui::Id::new("preset_save"), egui::Sense::click());
                    {
                        let painter = ui.painter();
                        let pressed = save_resp.is_pointer_button_down_on();
                        let highlight = is_editing;
                        let top_color = if pressed || highlight { theme::BTN_DARK } else { theme::BTN_LIGHT };
                        let bot_color = if pressed || highlight { theme::BTN_LIGHT } else { theme::BTN_DARK };
                        let text_color = if highlight { theme::RED_LED } else if save_resp.hovered() { theme::WHITE } else { theme::TEXT_DIM };
                        painter.rect_filled(save_rect, 2.0, bot_color);
                        painter.rect_filled(
                            egui::Rect::from_min_size(save_rect.min, egui::vec2(btn_w, btn_h * 0.45)),
                            2.0,
                            top_color,
                        );
                        painter.text(
                            save_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "SAVE",
                            egui::FontId::new(7.0, egui::FontFamily::Monospace),
                            text_color,
                        );
                    }
                    if save_resp.clicked() {
                        let mut pb = preset_bar.lock();
                        if pb.editing {
                            // Second click = confirm save
                            let name = pb.edit_buffer.trim().to_owned();
                            if !name.is_empty() {
                                let snapshot = snapshot_params(&params);
                                let result = preset_manager.lock().save(&name, snapshot);
                                match result {
                                    Ok(()) => {
                                        pb.selected_name = name.clone();
                                        pb.status_msg = format!("Saved: {}", name);
                                        pb.status_timer = 3.0;
                                    }
                                    Err(e) => {
                                        pb.status_msg = e;
                                        pb.status_timer = 4.0;
                                    }
                                }
                            }
                            pb.editing = false;
                        } else {
                            // First click = enter edit mode
                            pb.editing = true;
                            pb.edit_buffer = pb.selected_name.clone();
                            pb.dropdown_open = false;
                        }
                    }

                    // --- DEL button ---
                    let del_x = save_rect.right() + 4.0;
                    let del_rect = egui::Rect::from_min_size(
                        egui::pos2(del_x, preset_y + (display_h - btn_h) * 0.5),
                        egui::vec2(btn_w, btn_h),
                    );
                    let can_delete = !is_factory && selected_name != "Init";
                    let del_resp = ui.interact(del_rect, egui::Id::new("preset_del"), egui::Sense::click());
                    {
                        let painter = ui.painter();
                        let pressed = del_resp.is_pointer_button_down_on() && can_delete;
                        let top_color = if pressed { theme::BTN_DARK } else { theme::BTN_LIGHT };
                        let bot_color = if pressed { theme::BTN_LIGHT } else { theme::BTN_DARK };
                        let text_color = if !can_delete { theme::TEXT_GHOST } else if del_resp.hovered() { theme::WHITE } else { theme::TEXT_DIM };
                        painter.rect_filled(del_rect, 2.0, bot_color);
                        painter.rect_filled(
                            egui::Rect::from_min_size(del_rect.min, egui::vec2(btn_w, btn_h * 0.45)),
                            2.0,
                            top_color,
                        );
                        painter.text(
                            del_rect.center(),
                            egui::Align2::CENTER_CENTER,
                            "DEL",
                            egui::FontId::new(7.0, egui::FontFamily::Monospace),
                            text_color,
                        );
                    }
                    if del_resp.clicked() && can_delete {
                        let result = preset_manager.lock().delete(&selected_name);
                        let mut pb = preset_bar.lock();
                        match result {
                            Ok(()) => {
                                pb.selected_name = "Init".into();
                                pb.selected_index = 0;
                                pb.status_msg = "Deleted".into();
                                pb.status_timer = 2.0;
                            }
                            Err(e) => {
                                pb.status_msg = e;
                                pb.status_timer = 4.0;
                            }
                        }
                    }

                    // --- Keyboard shortcuts (when not editing) ---
                    if !is_editing && !all_presets.is_empty() {
                        let up = ui.input(|i| i.key_pressed(egui::Key::ArrowUp));
                        let down = ui.input(|i| i.key_pressed(egui::Key::ArrowDown));
                        if up || down {
                            let mut pb = preset_bar.lock();
                            let idx = if up {
                                if pb.selected_index == 0 { all_presets.len() - 1 } else { pb.selected_index - 1 }
                            } else {
                                if pb.selected_index >= all_presets.len() - 1 { 0 } else { pb.selected_index + 1 }
                            };
                            pb.selected_index = idx;
                            let preset = &all_presets[idx].preset;
                            pb.selected_name = preset.name.clone();
                            apply_preset_owned(setter, &params, preset);
                        }
                    }

                    // --- Status message (temporary) ---
                    {
                        let mut pb = preset_bar.lock();
                        if pb.status_timer > 0.0 {
                            pb.status_timer -= ctx.input(|i| i.unstable_dt);
                            let msg = pb.status_msg.clone();
                            drop(pb);
                            let painter = ui.painter();
                            painter.text(
                                egui::pos2(del_rect.right() + 8.0, preset_y + display_h * 0.5),
                                egui::Align2::LEFT_CENTER,
                                &msg,
                                egui::FontId::new(6.0, egui::FontFamily::Monospace),
                                theme::RED_LED,
                            );
                        }
                    }
                }
```

- [ ] **Step 3: Add the `preset_arrow_btn` helper function**

Add this function at the bottom of editor.rs (after `snapshot_params` or near the other drawing helpers):

```rust
fn preset_arrow_btn(painter: &egui::Painter, rect: egui::Rect, glyph: &str, color: egui::Color32) {
    painter.rect_filled(rect, 2.0, theme::BTN_DARK);
    painter.rect_filled(
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), rect.height() * 0.4)),
        2.0,
        theme::BTN_LIGHT,
    );
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::new(10.0, egui::FontFamily::Monospace),
        color,
    );
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check 2>&1 | head -30`

Expected: Clean check. If there are borrow issues with `preset_bar.lock()` being held across `apply_preset_owned`, clone the preset before calling apply.

- [ ] **Step 5: Run tests**

Run: `cargo test 2>&1 | tail -5`

Expected: All 33 tests pass (no DSP changes).

- [ ] **Step 6: Commit**

```bash
git add src/ui/editor.rs
git commit -m "feat(ui): header-integrated preset bar with LED display, dropdown, inline save"
```

---

### Task 4: Build and verify visually

**Files:** None (verification only)

- [ ] **Step 1: Build release**

Run: `cargo build --release 2>&1 | tail -5`

Expected: Clean build.

- [ ] **Step 2: Run standalone and verify**

Run: `cargo run --release 2>&1 &`

Check visually:
- Header shows: SLAMMER [LED] TEST | ◂ [LED: Init] ▸ SAVE DEL | KICK SYNTHESIZER
- Click LED display → dropdown with factory presets, separator, user presets
- Click SAVE → LED becomes editable text field with red border
- Type name, press Enter → saves, LED shows new name
- ESC → cancels edit mode
- ◂/▸ arrows cycle through presets
- ↑/↓ keys cycle presets
- DEL greyed out for factory/Init, active for user presets
- Resize window corner → everything scales proportionally
- Aspect ratio stays locked (no stretching)
- Footer shows groove + "REXIST INSTRUMENTS" ghost text (no preset bar)

- [ ] **Step 3: Fix any visual issues and commit**

If issues found, fix and commit:

```bash
git add -u
git commit -m "fix(ui): adjust header preset bar layout and scaling"
```

---

### Task 5: Cleanup

**Files:** `src/ui/editor.rs`

- [ ] **Step 1: Run cargo clippy**

Run: `cargo clippy 2>&1 | grep "src/ui/" | head -20`

Fix any warnings in UI files.

- [ ] **Step 2: Remove dead code**

Check for and remove:
- The old `small_metal_button` function (if not already removed in Task 3)
- Any unused imports
- Any references to old preset bar layout variables

- [ ] **Step 3: Final test run**

Run: `cargo test 2>&1 | tail -5`

Expected: All 33 tests pass.

- [ ] **Step 4: Commit cleanup**

```bash
git add -u
git commit -m "chore(ui): cleanup dead code from preset bar migration"
```
