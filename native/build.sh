#!/bin/sh
set -eu

BUNTINI_NATIVE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
BUNTINI_AUDIO_ARCH=${BUNTINI_AUDIO_ARCH:-$(uname -m)}
case "$BUNTINI_AUDIO_ARCH" in
  arm64|x86_64) ;;
  *) echo "Unsupported macOS architecture: $BUNTINI_AUDIO_ARCH" >&2; exit 1 ;;
esac

# Embedded metadata makes direct development invocations safe too: TCC requires
# a microphone usage description before it can display its permission dialog.
xcrun swiftc -O -target "$BUNTINI_AUDIO_ARCH-apple-macosx13.0" \
  -module-cache-path "${TMPDIR:-/tmp}/buntini-swift-module-cache" \
  -framework AVFoundation \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$BUNTINI_NATIVE_DIR/Info.plist" \
  "$BUNTINI_NATIVE_DIR/main.swift" -o "$BUNTINI_NATIVE_DIR/buntini-audio"
