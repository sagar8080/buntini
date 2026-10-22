#!/bin/sh
# Builds the Apple Intelligence helper. It targets macOS 13 with Foundation Models and
# Speech weak-linked, and answers "unsupported" on systems older than macOS 26.
set -eu
BUNTINI_NATIVE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "$(uname -m)" != "arm64" ]; then
  echo "Skipping the Apple Intelligence helper: it needs an Apple silicon Mac." >&2
  exit 0
fi
BUNTINI_SDK=$(xcrun --show-sdk-path)
if [ ! -d "$BUNTINI_SDK/System/Library/Frameworks/FoundationModels.framework" ]; then
  echo "Skipping the Apple Intelligence helper: this SDK has no FoundationModels (needs the macOS 26 SDK)." >&2
  exit 0
fi
xcrun swiftc -O -parse-as-library -target arm64-apple-macosx13.0 \
  -module-cache-path "${TMPDIR:-/tmp}/buntini-swift-module-cache" \
  -Xlinker -weak_framework -Xlinker FoundationModels \
  -Xlinker -weak_framework -Xlinker Speech \
  -framework AVFoundation -framework EventKit \
  "$BUNTINI_NATIVE_DIR/fm/main.swift" -o "$BUNTINI_NATIVE_DIR/buntini-fm"
