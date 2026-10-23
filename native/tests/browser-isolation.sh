#!/bin/sh
# Explicit opt-in; deliberately excluded from native/test.sh and default suites.
set -eu
BUNTINI_BROWSER_TEST_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "$(uname -s)" != "Darwin" ]; then
  echo "SKIP: browser isolation requires macOS 14 or later."
  exit 77
fi
BUNTINI_BROWSER_MACOS_MAJOR=$(sw_vers -productVersion | cut -d . -f 1)
if [ "$BUNTINI_BROWSER_MACOS_MAJOR" -lt 14 ]; then
  echo "SKIP: browser isolation requires macOS 14 or later."
  exit 77
fi
BUNTINI_BROWSER_PROBE_DIR=$(mktemp -d "${TMPDIR:-/tmp}/buntini-browser-isolation.XXXXXX")
trap 'rm -rf "$BUNTINI_BROWSER_PROBE_DIR"' EXIT HUP INT TERM
xcrun clang -fobjc-arc -Wall -Wextra -framework Cocoa -framework WebKit \
  "$BUNTINI_BROWSER_TEST_DIR/browser-isolation.m" \
  -o "$BUNTINI_BROWSER_PROBE_DIR/buntini-browser-isolation-probe"
"$BUNTINI_BROWSER_PROBE_DIR/buntini-browser-isolation-probe"
