#!/bin/bash
# Launch Niner standalone on macOS with a safe audio buffer size.
#
# CoreAudio on Apple Silicon delivers variable-sized buffers that can
# exceed any configured period size, which crashes nih-plug's CPAL
# backend assertion (actual_sample_count <= buffer_size). 4096 is large
# enough to accommodate CoreAudio's actual delivery in practice.
#
# VST3 / CLAP running inside a DAW are unaffected — this workaround is
# only needed for the standalone app.
#
# Upstream issue: https://github.com/robbert-vdh/nih-plug/issues/266

DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/niner-standalone" --period-size 4096 "$@"
