# G3 GUI Overhaul — Design Spec

## Overview

Visual overhaul of Slammer's egui GUI to match the G3 "Chunky Industrial" mockup (`mockups/mockup-g3.html`). No DSP or parameter changes — purely visual.

## Window

680x390 (up from 680x360).

## Layout (3 content rows)

```
┌─────────────────────────────────────────────────┐
│ ▌ SLAMMER              KICK SYNTHESIZER v0.1.0 ▌│  Header
│ ▌──────────────────────────────────────────────▌│
│ ▌ [====WAVEFORM DISPLAY====]  DECAY VEL MASTER ▌│  Master row
│ ▌──────────────────────────────────────────────▌│
│ ▌ SUB                        │ TOP             ▌│  Row 1: SUB | TOP
│ ▌ GAIN START END SWP CRV PHS │ GAIN DCY FRQ BW ▌│
│ ▌──────────────────────────────────────────────▌│
│ ▌ MID                                          ▌│  Row 2: MID
│ ▌ GAIN START END SWP CRV DCY TONE NOISE COLOR  ▌│
│ ▌──────────────────────────────────────────────▌│
│ ▌ SAT              │ EQ                        ▌│  Row 3: SAT | EQ
│ ▌ [<][LCD][>] DRV MIX │ TILT LOW NTCH Q DEPTH  ▌│
│ ▌──────────────────────────────────────────────▌│
│ ▌            REXIST INSTRUMENTS                ▌│  Footer
└─────────────────────────────────────────────────┘
  ▌ = rack ear with vent slots
```

- Section labels (SUB, MID, TOP, SAT, EQ): white, bold, 10px monospace, placed ABOVE groove lines
- Groove lines: dark separator (#000 at 60% opacity) + faint highlight below (white at 2% opacity)
- SUB and TOP share row 1, separated by a thin vertical divider
- SAT and EQ share row 3, separated by a thin vertical divider

## Panel

- Background: linear gradient (#1e1e1e -> #161616 -> #131313 -> #161616 -> #1e1e1e)
- Rack ears: 8px strips on left and right edges (#141414), vent slots (#090909 rounded rects)
- Hex screws: 4 corners, radial gradient (#aaa -> #333), hex recess in centre
- Footer: "REXIST INSTRUMENTS" ghost text (white at 7% opacity), centred, 7px monospace

## Knobs (32px diameter)

### Structure (inside out)
1. **Mounting recess**: dark shadow behind knob (rgba(0,0,0,0.35)), radius + 4px
2. **Rubber grip ring**: outer layer, radial gradient (#2a2a2a -> #121212), full 32px diameter
3. **Metal core**: inner circle at 60% of radius, beveled edge ring (#888 -> #444), face with brushed aluminium gradient (#aaa center -> #555 edge)
4. **Centre dimple**: small dark circle at core centre (rgba(0,0,0,0.15))
5. **Tapered indicator**: white line (#eee), slightly wider at base, narrower at tip, rotating 270 degrees from 7 o'clock (225 deg) to 5 o'clock (315 deg)

### Interaction (unchanged)
- Vertical drag to change value
- Shift + drag for fine control
- Ctrl + click to reset to default

### Display
- Value text shown on hover/drag: 8px monospace, centred on knob face
- Label below knob: white (#ddd), 7px monospace

### Tick marks
- 11 ticks around outer edge (0%, 10%, ... 100%)
- Major ticks (0%, 50%, 100%): longer, brighter (rgba(200,200,200,0.45))
- Minor ticks: shorter, dimmer (rgba(200,200,200,0.15))

## Displays

### Waveform display
- Container: deep inset bezel — outer frame #080808 rounded rect, inner fill gradient (#040101 -> #060303 -> #050202)
- Scanlines: every 2px, rgba(0,0,0,0.08)
- Red ambient glow: radial gradient from centre, rgba(40,3,3,0.35) -> transparent
- Inner shadow: top-left edge dark, bottom-right edge faint highlight
- Label "OUTPUT": 6px monospace, red at 20% opacity, top-left corner
- Waveform: drawn in red (#ff1a1a) at 25% opacity, same live telemetry data as current

### LCD mode selector
- Same inset display style as waveform
- Text: 7-segment polygon rendering — each character drawn as 7 filled polygons, not font glyphs
- Lit segments: #ff1a1a with 5px shadow blur glow
- Unlit/ghost segments: rgba(255,20,20,0.035) — faintly visible
- Supported characters: 0-9, A-F, plus O, S, T, I, P, L, H, space, dash (for OFF, SOFT, DIODE, TAPE)
- Arrow buttons: chunky metal rectangles flanking the LCD, linear gradient (#444 -> #1c1c1c), top highlight, bottom shadow

## Colour Palette (theme.rs)

| Constant | Value | Usage |
|----------|-------|-------|
| BG_PANEL | #131313 | Main panel centre |
| BG_PANEL_EDGE | #1e1e1e | Panel top/bottom gradient edges |
| BG_RACK_EAR | #141414 | Rack ear strips |
| BG_VENT | #090909 | Vent slot fills |
| BG_DISPLAY | #040202 | Display inner fill |
| BG_DISPLAY_FRAME | #080808 | Display outer bezel |
| RED_LED | #ff1a1a | LED, 7-segment lit, waveform |
| RED_GLOW | rgba(255,25,20,0.12) | LED halo |
| RED_AMBIENT | rgba(40,3,3,0.35) | Display ambient glow |
| RED_GHOST | rgba(255,20,20,0.035) | 7-segment unlit ghost |
| WHITE | #ddd | Labels, indicators |
| TEXT_DIM | #555 | Secondary text, version |
| KNOB_RUBBER_LIGHT | #2a2a2a | Rubber grip highlight |
| KNOB_RUBBER_DARK | #121212 | Rubber grip shadow |
| KNOB_METAL_LIGHT | #aaa | Metal core highlight |
| KNOB_METAL_DARK | #555 | Metal core shadow |
| KNOB_BEVEL_LIGHT | #888 | Bevel ring highlight |
| KNOB_BEVEL_DARK | #444 | Bevel ring shadow |
| KNOB_RECESS | rgba(0,0,0,0.35) | Mounting recess |
| GROOVE_DARK | rgba(0,0,0,0.6) | Groove line |
| GROOVE_LIGHT | rgba(255,255,255,0.02) | Groove highlight |
| SCREW_LIGHT | #aaa | Screw highlight |
| SCREW_DARK | #2a2a2a | Screw edge |
| BTN_LIGHT | #444 | Arrow button highlight |
| BTN_DARK | #1c1c1c | Arrow button shadow |

## File Changes

### theme.rs — full rewrite
- Replace all colour constants with G3 palette
- Update `setup_style()` for dark panel fill
- Keep font setup (JetBrains Mono) unchanged

### knob.rs — rewrite
- Two-layer rendering: rubber grip ring + beveled metal core
- Tapered indicator line replacing arc indicator
- Tick marks around outer edge
- Mounting recess shadow
- Keep interaction logic (drag, shift-fine, ctrl-reset) unchanged

### editor.rs — restructure
- Layout: SUB+TOP combined row with vertical divider
- Panel decorations: rack ears, vent slots, hex screws, groove lines
- Waveform display: G3 inset bezel styling with red waveform
- LCD selector: 7-segment polygon renderer, metal arrow buttons
- Section labels above groove lines
- Footer with ghost text
- Knob size constant: 28 -> 32

### plugin.rs — window size
- `EguiState::from_size(680, 390)` (was 680, 360)

### ui/mod.rs — no change

## Not Changing

- All DSP code (engine, envelope, oscillator, noise, click, saturation, filter)
- All parameters (plugin.rs SlammerParams)
- Telemetry pipeline
- Knob interaction behaviour
- Build/bundle process
