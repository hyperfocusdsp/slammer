#!/usr/bin/env bash
# Install Niner VST3, CLAP, and standalone into standard locations.
# Expects to be run from the extracted release folder (same folder as
# niner.vst3, niner.clap, and niner-standalone).
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

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

if [ ! -d "$VST3_BUNDLE" ] && [ ! -d "$CLAP_BUNDLE" ]; then
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

# CLAP bundle.
if [ -d "$CLAP_BUNDLE" ]; then
  mkdir -p "$CLAP_DEST"
  rm -rf "$CLAP_DEST/niner.clap"
  cp -r "$CLAP_BUNDLE" "$CLAP_DEST/"
  echo "  CLAP       -> $CLAP_DEST/niner.clap"
fi

# Standalone (optional — only present on some release archives).
if [ -f "$STANDALONE" ]; then
  BIN_DIR="$HOME/.local/bin"
  mkdir -p "$BIN_DIR"
  cp "$STANDALONE" "$BIN_DIR/niner-standalone"
  chmod +x "$BIN_DIR/niner-standalone"
  echo "  Standalone -> $BIN_DIR/niner-standalone"
fi

echo ""
echo "Done! Rescan plugins in your DAW to find Niner."
if [ "$(uname -s)" = "Darwin" ]; then
  echo ""
  echo "macOS note: if this is the first time, run:"
  echo "  xattr -dr com.apple.quarantine \"$VST3_DEST/niner.vst3\" \"$CLAP_DEST/niner.clap\""
  echo "to clear Gatekeeper's quarantine flag on the plugins."
fi
