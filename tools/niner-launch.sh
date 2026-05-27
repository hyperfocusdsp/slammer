#!/usr/bin/env bash
# niner-launch — desktop-launcher shim for the standalone.
#
# What it does:
#   1. Pins baseview's native scale to 1.0 (`--dpi-scale 1.0`). The plugin
#      drives UI scaling itself (egui zoom factor + a matching window resize),
#      reading the saved factor from `$XDG_DATA_HOME/niner/ui_scale.txt` at
#      startup. The launcher must NOT also scale or the two would compound.
#   2. Picks an audio backend, sets up MIDI auto-routing, and execs
#      niner-standalone. Default is `--backend alsa` because:
#        * PipeWire's ALSA bridge auto-routes niner's audio to the default
#          sink as long as `--sample-rate` and `--period-size` match the
#          server's `clock.force-rate` / `clock.force-quantum` (mismatches
#          either underrun or, with smaller period than quantum,
#          *bitcrush* the output — verified 2026-05-09 on geek with PW
#          quantum=512 vs niner period=256).
#        * nih-plug's `--midi-input <name> <cid:pid>` flag subscribes the
#          ALSA seq client directly to the controller, with no host-side
#          auto-routing required.
#      JACK backend is opt-in (sample-accurate `midi.time` frame offsets,
#      better for rapid pad rolls) but in some PipeWire states the JACK
#      client registers without process() callbacks ever firing, so it's
#      not safe as a default. See `feedback_niner_launch_midi_autodetect`
#      memory note for the full reasoning.
#
# Overrides:
#   NINER_FORCE_BACKEND=jack    use the JACK backend instead. The launcher
#                               then runs a `pw-link` sidecar that waits
#                               for niner:midi_input to register, looks up
#                               the BeatStep's JACK MIDI port by numeric
#                               ID (sidesteps the colon-in-port-name issue
#                               with nih-plug's --connect-jack-midi-input),
#                               and wires them together.
#   NINER_MIDI_INPUT="<full>"   ALSA mode only — override the auto-detected
#                               MIDI port. Must include the trailing seq
#                               address, e.g.
#                                 "Arturia BeatStep:Arturia BeatStep MIDI 1 32:0"
#                               (nih-plug's lookup rejects the name alone).
#   NINER_MIDI_PORT="<substr>"  JACK mode only — substring of the JACK
#                               MIDI capture port label to wire from.
#                               Default: "Arturia BeatStep".
#   NINER_PERIOD_SIZE=<N>       ALSA mode only — override the period size.
#                               Don't unless you've matched it to PipeWire
#                               (lower than `clock.force-quantum` causes
#                               bitcrush, not graceful underrun).
set -u

BIN_DIR="$(cd "$(dirname "$0")" && pwd)"

STANDALONE="$BIN_DIR/niner-standalone"
WRAPPER="$BIN_DIR/nih-standalone-wrapper"

# ---------------------------------------------------------------------------
# Default path: --backend alsa with auto-subscribed MIDI input.
# ---------------------------------------------------------------------------
if [ "${NINER_FORCE_BACKEND:-alsa}" != "jack" ] && command -v aconnect >/dev/null 2>&1; then
  MIDI_INPUT="${NINER_MIDI_INPUT:-}"
  if [ -z "$MIDI_INPUT" ]; then
    # Parse `aconnect -i` for an Arturia BeatStep client and build the
    # full "<client>:<port> <cid>:<pid>" string nih-plug expects.
    cid=""; cname=""
    while IFS= read -r line; do
      if [[ "$line" =~ ^client[[:space:]]+([0-9]+):[[:space:]]+\'([^\']+)\' ]]; then
        cid="${BASH_REMATCH[1]}"
        cname="${BASH_REMATCH[2]}"
      elif [[ "$line" =~ ^[[:space:]]+([0-9]+)[[:space:]]+\'([^\']+)\' ]]; then
        if [[ "$cname" == *"Arturia BeatStep"* ]]; then
          MIDI_INPUT="${cname}:${BASH_REMATCH[2]} ${cid}:${BASH_REMATCH[1]}"
          break
        fi
      fi
    done < <(aconnect -i 2>/dev/null)
  fi
  if [ -n "$MIDI_INPUT" ]; then
    PW_RATE=""
    PW_QUANTUM=""
    if command -v pw-metadata >/dev/null 2>&1; then
      PW_META=$(pw-metadata -n settings 0 2>/dev/null)
      # PipeWire reports `0` for `clock.force-*` to mean "no override" —
      # treat that the same as empty so we fall through to `clock.rate` /
      # `clock.quantum` (the values the server is actually running at).
      PW_RATE=$(printf '%s\n' "$PW_META" \
        | sed -n "s/.*key:'clock\.force-rate' value:'\([0-9]*\)'.*/\1/p" \
        | head -n1)
      [ "$PW_RATE" = "0" ] && PW_RATE=""
      if [ -z "$PW_RATE" ]; then
        PW_RATE=$(printf '%s\n' "$PW_META" \
          | sed -n "s/.*key:'clock\.rate' value:'\([0-9]*\)'.*/\1/p" \
          | head -n1)
        [ "$PW_RATE" = "0" ] && PW_RATE=""
      fi
      PW_QUANTUM=$(printf '%s\n' "$PW_META" \
        | sed -n "s/.*key:'clock\.force-quantum' value:'\([0-9]*\)'.*/\1/p" \
        | head -n1)
      [ "$PW_QUANTUM" = "0" ] && PW_QUANTUM=""
      if [ -z "$PW_QUANTUM" ]; then
        PW_QUANTUM=$(printf '%s\n' "$PW_META" \
          | sed -n "s/.*key:'clock\.quantum' value:'\([0-9]*\)'.*/\1/p" \
          | head -n1)
        [ "$PW_QUANTUM" = "0" ] && PW_QUANTUM=""
      fi
    fi
    PW_RATE="${PW_RATE:-48000}"
    PERIOD="${NINER_PERIOD_SIZE:-${PW_QUANTUM:-512}}"
    ALSA_ARGS=(--backend alsa --midi-input "$MIDI_INPUT" --sample-rate "$PW_RATE" --period-size "$PERIOD")
    if [ -x "$WRAPPER" ]; then
      exec "$WRAPPER" "$STANDALONE" --dpi-scale 1.0 "${ALSA_ARGS[@]}" "$@"
    fi
    exec "$STANDALONE" --dpi-scale 1.0 "${ALSA_ARGS[@]}" "$@"
  fi
  # No BeatStep found — fall through to JACK below (works without MIDI).
fi

# ---------------------------------------------------------------------------
# JACK path: sample-accurate MIDI via real `midi.time` frame offsets.
# Reached when NINER_FORCE_BACKEND=jack OR no MIDI controller is detected.
# A `pw-link` sidecar wires the BeatStep capture port to niner:midi_input
# after niner registers; nih-plug's --connect-jack-midi-input chokes on
# port names containing a colon, so we use numeric port IDs instead.
# ---------------------------------------------------------------------------
MIDI_PORT_PATTERN="${NINER_MIDI_PORT:-Arturia BeatStep}"

if command -v pw-link >/dev/null 2>&1; then
  (
    for _ in $(seq 1 40); do
      sleep 0.1
      pw-link -I -i 2>/dev/null | grep -q "niner:midi_input" && break
    done
    NINER_ID=$(pw-link -I -i 2>/dev/null \
      | awk '/niner:midi_input/ {print $1; exit}')
    SRC_ID=$(pw-link -I -o 2>/dev/null \
      | awk -v pat="$MIDI_PORT_PATTERN" '$0 ~ pat && /capture/ {print $1; exit}')
    if [ -n "${NINER_ID:-}" ] && [ -n "${SRC_ID:-}" ]; then
      pw-link "$SRC_ID" "$NINER_ID" 2>/dev/null || true
    fi
  ) &
fi

if [ -x "$WRAPPER" ]; then
  exec "$WRAPPER" "$STANDALONE" --dpi-scale 1.0 --backend jack "$@"
fi
exec "$STANDALONE" --dpi-scale 1.0 --backend jack "$@"
