#!/usr/bin/env bash
# Install Niner VST3, CLAP, and standalone into standard locations.
# Expects to be run from the extracted release folder (same folder as
# niner.vst3, niner.clap, and niner-standalone).
#
# Flags:
#   --no-desktop   Skip the Linux desktop-launcher install (niner-launch +
#                  ~/.local/share/applications/niner.desktop). Useful for
#                  headless boxes, CI, and users who only want plugin
#                  bundles + the standalone binary.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

NO_DESKTOP=0
for arg in "$@"; do
  case "$arg" in
    --no-desktop) NO_DESKTOP=1 ;;
    -h|--help)
      echo "Usage: install.sh [--no-desktop]"
      exit 0
      ;;
  esac
done

case "$(uname -s)" in
  Linux)
    VST3_DEST="$HOME/.vst3"
    CLAP_DEST="$HOME/.clap"
    ;;
  Darwin)
    VST3_DEST="$HOME/Library/Audio/Plug-Ins/VST3"
    CLAP_DEST="$HOME/Library/Audio/Plug-Ins/CLAP"
    ;;
  *)
    echo "Unsupported OS: $(uname -s). Use install.bat on Windows."
    exit 1
    ;;
esac

VST3_BUNDLE="$SCRIPT_DIR/niner.vst3"
CLAP_BUNDLE="$SCRIPT_DIR/niner.clap"
STANDALONE="$SCRIPT_DIR/niner-standalone"

if [ ! -d "$VST3_BUNDLE" ] && [ ! -e "$CLAP_BUNDLE" ]; then
  echo "Error: no niner.vst3 or niner.clap found in $SCRIPT_DIR"
  echo "Make sure install.sh is in the same folder as the extracted release."
  exit 1
fi

echo "Installing Niner..."

# VST3 bundle — replace any existing bundle to avoid nesting.
if [ -d "$VST3_BUNDLE" ]; then
  mkdir -p "$VST3_DEST"
  rm -rf "$VST3_DEST/niner.vst3"
  cp -r "$VST3_BUNDLE" "$VST3_DEST/"
  echo "  VST3       -> $VST3_DEST/niner.vst3"
fi

# CLAP — bundle directory on macOS, single .clap shared object on Linux.
if [ -e "$CLAP_BUNDLE" ]; then
  mkdir -p "$CLAP_DEST"
  rm -rf "$CLAP_DEST/niner.clap"
  cp -r "$CLAP_BUNDLE" "$CLAP_DEST/"
  echo "  CLAP       -> $CLAP_DEST/niner.clap"
fi

# Standalone (optional — only present on some release archives).
BIN_DIR="$HOME/.local/bin"
if [ -f "$STANDALONE" ]; then
  mkdir -p "$BIN_DIR"
  cp "$STANDALONE" "$BIN_DIR/niner-standalone"
  chmod +x "$BIN_DIR/niner-standalone"
  echo "  Standalone -> $BIN_DIR/niner-standalone"
fi

# Linux desktop launcher — installs `niner-launch` (a shim that reads the
# persisted UI scale and forwards `--dpi-scale` to the standalone) plus a
# .desktop entry so Niner shows up in the application menu / rofi. macOS
# uses scripts/niner-macos.sh for its launcher pattern; Windows uses
# install.bat. Skipped if `--no-desktop` was passed or the standalone
# isn't present in this archive.
if [ "$(uname -s)" = "Linux" ] && [ -f "$STANDALONE" ] && [ "$NO_DESKTOP" = "0" ]; then
  LAUNCHER_SRC="$SCRIPT_DIR/tools/niner-launch.sh"
  DESKTOP_TPL="$SCRIPT_DIR/tools/niner.desktop.template"
  if [ -f "$LAUNCHER_SRC" ] && [ -f "$DESKTOP_TPL" ]; then
    cp "$LAUNCHER_SRC" "$BIN_DIR/niner-launch"
    chmod +x "$BIN_DIR/niner-launch"
    DESKTOP_DIR="$HOME/.local/share/applications"
    mkdir -p "$DESKTOP_DIR"
    sed "s|__BIN_DIR__|$BIN_DIR|g" "$DESKTOP_TPL" > "$DESKTOP_DIR/niner.desktop"
    echo "  Launcher   -> $BIN_DIR/niner-launch"
    echo "  Desktop    -> $DESKTOP_DIR/niner.desktop"
  fi
fi

echo ""
echo "Done! Rescan plugins in your DAW to find Niner."
if [ "$(uname -s)" = "Darwin" ]; then
  echo ""
  echo "macOS note: if this is the first time, run:"
  echo "  xattr -dr com.apple.quarantine \"$VST3_DEST/niner.vst3\" \"$CLAP_DEST/niner.clap\""
  echo "to clear Gatekeeper's quarantine flag on the plugins."
fi
