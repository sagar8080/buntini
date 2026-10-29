#!/bin/sh
# Builds the Apple Intelligence helper. It targets macOS 13 with Foundation Models and
# Speech weak-linked, and answers "unsupported" on systems older than macOS 26.
set -eu
BUNTINI_NATIVE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "$(uname -m)" != "arm64" ]; then
  rm -f "$BUNTINI_NATIVE_DIR/buntini-fm"
  rm -f "$BUNTINI_NATIVE_DIR/buntini-fm-aarch64-apple-darwin"
  if [ "${BUNTINI_REQUIRE_FM:-0}" = "1" ]; then
    echo "Release builds require Apple silicon to build the Apple Intelligence helper." >&2
    exit 1
  fi
  echo "Skipping the Apple Intelligence helper: it needs an Apple silicon Mac." >&2
  exit 0
fi
BUNTINI_SDK=$(xcrun --show-sdk-path)
if [ ! -d "$BUNTINI_SDK/System/Library/Frameworks/FoundationModels.framework" ]; then
  rm -f "$BUNTINI_NATIVE_DIR/buntini-fm"
  rm -f "$BUNTINI_NATIVE_DIR/buntini-fm-aarch64-apple-darwin"
  if [ "${BUNTINI_REQUIRE_FM:-0}" = "1" ]; then
    echo "Release builds require the macOS 26 SDK for the Apple Intelligence helper." >&2
    exit 1
  fi
  echo "Skipping the Apple Intelligence helper: this SDK has no FoundationModels (needs the macOS 26 SDK)." >&2
  exit 0
fi
if [ -n "${TAURI_ENV_TARGET_TRIPLE:-}" ] && [ "$TAURI_ENV_TARGET_TRIPLE" != "aarch64-apple-darwin" ]; then
  echo "The Apple Intelligence helper only supports the aarch64-apple-darwin target." >&2
  exit 1
fi
xcrun swiftc -O -parse-as-library -target arm64-apple-macosx13.0 \
  -module-cache-path "${TMPDIR:-/tmp}/buntini-swift-module-cache" \
  -Xlinker -weak_framework -Xlinker FoundationModels \
  -Xlinker -weak_framework -Xlinker Speech \
  -framework AVFoundation -framework EventKit \
  "$BUNTINI_NATIVE_DIR/fm/main.swift" -o "$BUNTINI_NATIVE_DIR/buntini-fm"
cp "$BUNTINI_NATIVE_DIR/buntini-fm" "$BUNTINI_NATIVE_DIR/buntini-fm-aarch64-apple-darwin"
