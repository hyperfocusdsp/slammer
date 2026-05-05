#!/usr/bin/env bash
# niner-launch — desktop-launcher shim for the standalone.
#
# The in-GUI scale badge writes the chosen factor to a sidecar file so it
# survives across launches, but the standalone binary itself only reads the
# scale from the `--dpi-scale` CLI flag. This shim bridges the two: read
# the persisted scale, forward it to `niner-standalone`, and (when present)
# chain through `nih-standalone-wrapper` so the PipeWire quantum is also
# pinned. Invoked from the .desktop entry installed by `install.sh`.
set -u

BIN_DIR="$(cd "$(dirname "$0")" && pwd)"
SCALE_FILE="${XDG_DATA_HOME:-$HOME/.local/share}/niner/ui_scale.txt"
SCALE=$(tr -d ' \n' < "$SCALE_FILE" 2>/dev/null || true)
SCALE="${SCALE:-1.0}"

STANDALONE="$BIN_DIR/niner-standalone"
WRAPPER="$BIN_DIR/nih-standalone-wrapper"

if [ -x "$WRAPPER" ]; then
  exec "$WRAPPER" "$STANDALONE" --dpi-scale "$SCALE" "$@"
fi
exec "$STANDALONE" --dpi-scale "$SCALE" "$@"
