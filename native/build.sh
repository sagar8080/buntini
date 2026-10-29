#!/bin/sh
set -eu

BUNTINI_NATIVE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BUNTINI_AUDIO_ARCH=${BUNTINI_AUDIO_ARCH:-$(uname -m)}
case "$BUNTINI_AUDIO_ARCH" in
  arm64) BUNTINI_AUDIO_TRIPLE=aarch64-apple-darwin ;;
  x86_64) BUNTINI_AUDIO_TRIPLE=x86_64-apple-darwin ;;
  *) echo "Unsupported macOS architecture: $BUNTINI_AUDIO_ARCH" >&2; exit 1 ;;
esac
if [ -n "${TAURI_ENV_TARGET_TRIPLE:-}" ] && [ "$TAURI_ENV_TARGET_TRIPLE" != "$BUNTINI_AUDIO_TRIPLE" ]; then
  echo "The audio helper architecture must match the Tauri target." >&2
  exit 1
fi

# Embedded metadata makes direct development invocations safe too: TCC requires
# a microphone usage description before it can display its permission dialog.
xcrun swiftc -O -target "$BUNTINI_AUDIO_ARCH-apple-macosx13.0" \
  -module-cache-path "${TMPDIR:-/tmp}/buntini-swift-module-cache" \
  -framework AVFoundation \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$BUNTINI_NATIVE_DIR/Info.plist" \
  "$BUNTINI_NATIVE_DIR/main.swift" -o "$BUNTINI_NATIVE_DIR/buntini-audio"
# Tauri sidecars are signed as nested code in Contents/MacOS, rather than copied
# as unsigned resource data. Its bundler requires a target-suffixed source file.
cp "$BUNTINI_NATIVE_DIR/buntini-audio" "$BUNTINI_NATIVE_DIR/buntini-audio-$BUNTINI_AUDIO_TRIPLE"
