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

# A SwiftPM package (native/audio) so FluidAudio is pinned in Package.resolved. FluidAudio
# declares macOS 14; everything is compiled for macOS 13 instead, so the helper still starts
# there and every newer API stays behind an availability check (the compiler enforces it).
# Embedded metadata makes direct development invocations safe too: TCC requires a usage
# description before it can show a permission dialog.
BUNTINI_AUDIO_TARGET="$BUNTINI_AUDIO_ARCH-apple-macosx13.0"
swift build -c release --package-path "$BUNTINI_NATIVE_DIR/audio" --arch "$BUNTINI_AUDIO_ARCH" \
  --disable-sandbox --only-use-versions-from-resolved-file \
  -Xswiftc -target -Xswiftc "$BUNTINI_AUDIO_TARGET" \
  -Xcc -target -Xcc "$BUNTINI_AUDIO_TARGET" \
  -Xcxx -target -Xcxx "$BUNTINI_AUDIO_TARGET" \
  -Xlinker -sectcreate -Xlinker __TEXT -Xlinker __info_plist -Xlinker "$BUNTINI_NATIVE_DIR/Info.plist"
BUNTINI_AUDIO_BIN=$(swift build -c release --package-path "$BUNTINI_NATIVE_DIR/audio" --arch "$BUNTINI_AUDIO_ARCH" --show-bin-path)
cp "$BUNTINI_AUDIO_BIN/buntini-audio" "$BUNTINI_NATIVE_DIR/buntini-audio"
# Tauri sidecars are signed as nested code in Contents/MacOS, rather than copied
# as unsigned resource data. Its bundler requires a target-suffixed source file.
cp "$BUNTINI_NATIVE_DIR/buntini-audio" "$BUNTINI_NATIVE_DIR/buntini-audio-$BUNTINI_AUDIO_TRIPLE"
