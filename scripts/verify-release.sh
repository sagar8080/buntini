#!/bin/sh
# Validate the final distribution artifacts. Does not sign, notarize or publish.
set -eu
if [ "$#" -ne 2 ] || [ "$(uname -s)" != "Darwin" ]; then
  echo "Usage (macOS): APPLE_TEAM_ID=... sh scripts/verify-release.sh Buntini.app Buntini.dmg" >&2
  exit 2
fi
: "${APPLE_TEAM_ID:?Set the expected Apple team identifier}"
BUNTINI_VERIFY_APP=$1
BUNTINI_VERIFY_DMG=$2
test -d "$BUNTINI_VERIFY_APP"
test -f "$BUNTINI_VERIFY_DMG"
BUNTINI_VERIFY_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ -n "${RELEASE_TAG:-}" ]; then
  python3 "$BUNTINI_VERIFY_ROOT/scripts/release_preflight.py" --tag "$RELEASE_TAG"
fi
python3 "$BUNTINI_VERIFY_ROOT/scripts/release_artifact.py" --app "$BUNTINI_VERIFY_APP" --dmg "$BUNTINI_VERIFY_DMG" --check-signatures --team "$APPLE_TEAM_ID"

codesign --verify --deep --strict --verbose=2 "$BUNTINI_VERIFY_APP"
xcrun stapler validate "$BUNTINI_VERIFY_APP"
spctl --assess --type execute --verbose=2 "$BUNTINI_VERIFY_APP"
hdiutil verify "$BUNTINI_VERIFY_DMG"
xcrun stapler validate "$BUNTINI_VERIFY_DMG"
spctl --assess --type open --context context:primary-signature --verbose=2 "$BUNTINI_VERIFY_DMG"
python3 "$BUNTINI_VERIFY_ROOT/scripts/release_dmg.py" --app "$BUNTINI_VERIFY_APP" --dmg "$BUNTINI_VERIFY_DMG" --team "$APPLE_TEAM_ID"
echo "Release artifacts passed metadata, resource hashes, Developer ID, team, timestamp, entitlements, hardened-runtime, architecture, notarization and Gatekeeper checks."
