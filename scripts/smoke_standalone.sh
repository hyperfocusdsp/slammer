#!/usr/bin/env bash
# Standalone smoke test. Builds niner-standalone in release mode and
# confirms the linked binary is non-empty and exits cleanly when given
# `--help` (which nih-plug-standalone routes through its arg parser
# WITHOUT trying to open an audio backend or window).
#
# This is the cheapest end-to-end regression net for "does the link
# graph still resolve" and "do the cdylib/standalone exports still
# match" — it would catch a broken `nih_export_*!` macro, a missing
# Rust feature gate, or a Linux dep regression long before manual
# DAW-load testing.
#
# CI consumers: invoke directly. Local: same.

set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> building niner-standalone (release, default features)"
cargo build --release --bin niner-standalone --no-default-features --quiet

BIN=target/release/niner-standalone
if [ ! -x "$BIN" ]; then
    if [ -x "${BIN}.exe" ]; then
        BIN="${BIN}.exe"
    else
        echo "FAIL: niner-standalone not found"
        exit 1
    fi
fi

SIZE=$(stat -c %s "$BIN" 2>/dev/null || stat -f %z "$BIN")
if [ "$SIZE" -lt 100000 ]; then
    echo "FAIL: niner-standalone is suspiciously small ($SIZE bytes) — link graph broken?"
    exit 1
fi
echo "==> binary present (${SIZE} bytes)"

# nih-plug exposes --help via clap; this exercises the arg parser and
# all `nih_export_standalone!`-driven init code WITHOUT opening audio
# or a window. Headless-friendly. Pipefail ignored — `--help` exits 0.
echo "==> niner-standalone --help"
HELP_LOG=$(mktemp)
trap 'rm -f "$HELP_LOG"' EXIT
if timeout 10s "$BIN" --help > "$HELP_LOG" 2>&1; then
    HELP_EXIT=0
else
    HELP_EXIT=$?
fi

if [ "$HELP_EXIT" -ne 0 ] && [ "$HELP_EXIT" -ne 124 ]; then
    echo "FAIL: niner-standalone --help exited with $HELP_EXIT"
    cat "$HELP_LOG"
    exit 1
fi

if grep -qiE "panic|backtrace|fatal" "$HELP_LOG"; then
    echo "FAIL: panic/backtrace in --help output"
    cat "$HELP_LOG"
    exit 1
fi

echo "OK: niner-standalone smoke passed"
