# Preset Bar Relocation & UI Scaling

## Goal

Two changes to Slammer's GUI:
1. Move the preset bar from the cramped footer into the header row, styled as an LED display
2. Make the entire UI scale proportionally when the window is resized, maintaining aspect ratio

## 1. Preset Bar — Header-Integrated LED Display

### Layout

The preset controls move into the header row, between the TEST button and the right-side info text ("KICK SYNTHESIZER / v0.1.0"). Elements left to right:

```
SLAMMER [LED] TEST | ◂ [==LED DISPLAY==] ▸  SAVE  DEL    KICK SYNTHESIZER
                                                              v0.1.0
```

- **LED display**: Dark inset rectangle (`BG_DISPLAY` background, `BG_DISPLAY_FRAME` border) with scanline overlay. Shows preset name in red (`RED_LED` color) with glow. Min-width ~130px.
- **◂ / ▸ arrows**: Small metal buttons (same style as TEST). Cycle through preset list, wrapping around.
- **SAVE button**: Small metal button. Click to enter save mode.
- **DEL button**: Small metal button. Greyed out (`TEXT_GHOST` color, non-interactive) for factory presets and "Init". Active for user presets — deletes current preset, reverts to "Init".

### Interaction States

**Normal state:**
- LED display shows current preset name in red 7-seg/monospace style
- Arrows cycle presets sequentially (wraps around)
- Clicking the LED display opens a dropdown

**Dropdown state:**
- Dropdown appears directly below the LED display
- Background: `BG_PANEL` or slightly lighter (`#0e0e0e`)
- Factory presets listed first (plain text), separator line, then user presets (prefixed with `*`)
- Current preset highlighted in `RED_LED` color
- Clicking a preset selects it and closes dropdown
- Clicking outside closes dropdown

**Save mode (editing):**
- Triggered by clicking SAVE
- LED display border changes to `RED_LED` with subtle glow
- Display content becomes an editable text field (same red color, same font)
- Arrow buttons disabled (greyed out) during editing
- SAVE button shows as active/highlighted
- Enter: saves preset with typed name, exits save mode
- ESC: cancels, reverts to previous name, exits save mode
- If name is empty, save is rejected

**Keyboard shortcuts:**
- ↑/↓ arrows cycle presets (when not in save mode)

### Footer Removal

The old footer preset bar (`bar_h = 18.0`, `bar_top`, combo box, text input, SAVE/DEL) is removed entirely. The "REXIST INSTRUMENTS" ghost text and footer groove remain.

## 2. UI Scaling — Proportional with Locked Aspect Ratio

### Approach

All UI elements scale proportionally based on window size. Aspect ratio is locked to `BASE_W:BASE_H` (780:450).

**Scale factor:** `s = window_width / BASE_W`

Every pixel position, size, radius, font size, and spacing is multiplied by `s` at draw time. The base constants (`KNOB_SIZE = 32.0`, `CONTENT_LEFT = 30.0`, `RACK_EAR_W = 16.0`, etc.) remain unchanged — they define the layout at 1x scale.

### Implementation Strategy

- `ctx.set_pixels_per_point(ppp)` handles text and egui widget scaling
- All custom painting (knobs, waveform, rack ears, screws, grooves, LED display, preset bar) must multiply coordinates by `s`
- The `ResizableWindow` enforces the aspect ratio via `resize_to_fit` or by constraining in the resize callback
- Minimum window size: `BASE_W x BASE_H` (780x450) — `s` never goes below 1.0

### What Scales

- All `egui::pos2()` and `egui::vec2()` positions/sizes
- All `egui::FontId::new(size, ...)` font sizes
- All radius values (screws, LEDs, knob circles)
- All spacing/padding values (`CONTENT_LEFT`, `KNOB_SPACING`, groove positions)
- Stroke widths for grooves, waveform lines

### Aspect Ratio Lock

The `ResizableWindow` should constrain resizing so that height always equals `width * (BASE_H / BASE_W)`. If egui's `ResizableWindow` doesn't support aspect ratio locking directly, compute the constrained size from the width on each frame and force it.

## Testing

- Resize window from 780x450 up to ~1560x900 (2x) — all elements should scale smoothly
- Preset arrow cycling wraps correctly
- Dropdown opens below LED display, positioned correctly at any scale
- Save mode: type, Enter saves, ESC cancels
- DEL greyed out on factory presets, active on user presets
- At 1x scale, layout matches current G3 design (no regressions)

## Files Changed

- `src/ui/editor.rs` — preset bar rewrite (remove footer, add header-integrated version), scaling factor threading
- `src/ui/knob.rs` — accept scale factor parameter
- `src/ui/theme.rs` — no changes expected
- `src/plugin.rs` — no changes expected (window size already 780x450)
