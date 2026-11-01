#!/bin/sh
# Notarize an existing locally signed app using a saved Keychain profile. No
# certificate export, password arguments, CI secret reads or publication occur.
set -eu
usage() {
  cat <<'HELP'
Usage (macOS with Python 3.11+, Xcode tools):
  sh scripts/release-notarize.sh --app /absolute/Buntini.app \
    --profile buntini-notary --identity 'Developer ID Application: Name (TEAMID)' \
    --team TEAMID --tag vX.Y.Z --output /absolute/new-candidate-directory

Uses only the existing Keychain signing identity/profile. The supplied app must
already pass production signature, entitlements, layout and source-hash checks.
Creates a separate stapled app, zip, signed/stapled DMG, notarization receipts,
build metadata and checksums. An existing output path is never overwritten.
On failure, .INCOMPLETE remains in the output along with any notary receipts.
Never paste your password, p12 or private key into chat or command arguments.
HELP
}
BUNTINI_NOTARY_APP=
BUNTINI_NOTARY_PROFILE=
BUNTINI_NOTARY_IDENTITY=
BUNTINI_NOTARY_TEAM=
BUNTINI_NOTARY_TAG=
BUNTINI_NOTARY_OUTPUT=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --help|-h) usage; exit 0 ;;
    --app|--profile|--identity|--team|--tag|--output)
      if [ "$#" -lt 2 ] || [ -z "$2" ]; then echo "Missing option value." >&2; exit 2; fi
      case "$1" in
        --app) BUNTINI_NOTARY_APP=$2 ;;
        --profile) BUNTINI_NOTARY_PROFILE=$2 ;;
        --identity) BUNTINI_NOTARY_IDENTITY=$2 ;;
        --team) BUNTINI_NOTARY_TEAM=$2 ;;
        --tag) BUNTINI_NOTARY_TAG=$2 ;;
        --output) BUNTINI_NOTARY_OUTPUT=$2 ;;
      esac
      shift 2 ;;
    *) echo "Unknown option. Use --help." >&2; exit 2 ;;
  esac
done
if [ "$(uname -s)" != "Darwin" ] || [ -z "$BUNTINI_NOTARY_APP" ] || [ -z "$BUNTINI_NOTARY_PROFILE" ] || [ -z "$BUNTINI_NOTARY_IDENTITY" ] || [ -z "$BUNTINI_NOTARY_TEAM" ] || [ -z "$BUNTINI_NOTARY_TAG" ] || [ -z "$BUNTINI_NOTARY_OUTPUT" ]; then
  usage >&2
  exit 2
fi
case "$BUNTINI_NOTARY_OUTPUT" in /*) ;; *) echo "Output must be an absolute path." >&2; exit 2 ;; esac
if [ -e "$BUNTINI_NOTARY_OUTPUT" ] || [ -L "$BUNTINI_NOTARY_OUTPUT" ]; then
  echo "Output path already exists; choose a new candidate directory." >&2
  exit 1
fi
BUNTINI_NOTARY_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
python3 "$BUNTINI_NOTARY_ROOT/scripts/release_preflight.py" --tag "$BUNTINI_NOTARY_TAG" --local-identity "$BUNTINI_NOTARY_IDENTITY" --team "$BUNTINI_NOTARY_TEAM"
python3 "$BUNTINI_NOTARY_ROOT/scripts/release_artifact.py" --app "$BUNTINI_NOTARY_APP" --tag "$BUNTINI_NOTARY_TAG" --check-signatures --team "$BUNTINI_NOTARY_TEAM"
codesign --verify --deep --strict "$BUNTINI_NOTARY_APP"
umask 077
mkdir "$BUNTINI_NOTARY_OUTPUT"
touch "$BUNTINI_NOTARY_OUTPUT/.INCOMPLETE"
BUNTINI_NOTARY_WORK=$(mktemp -d "${TMPDIR:-/tmp}/buntini-notarize.XXXXXX")
trap 'rm -rf "$BUNTINI_NOTARY_WORK"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
ditto "$BUNTINI_NOTARY_APP" "$BUNTINI_NOTARY_WORK/Buntini.app"
ditto -c -k --keepParent "$BUNTINI_NOTARY_WORK/Buntini.app" "$BUNTINI_NOTARY_WORK/app-upload.zip"
xcrun notarytool submit "$BUNTINI_NOTARY_WORK/app-upload.zip" --keychain-profile "$BUNTINI_NOTARY_PROFILE" --wait --timeout 30m --output-format json > "$BUNTINI_NOTARY_OUTPUT/app-notarization.json"
python3 "$BUNTINI_NOTARY_ROOT/scripts/release_notary_result.py" "$BUNTINI_NOTARY_OUTPUT/app-notarization.json"
xcrun stapler staple "$BUNTINI_NOTARY_WORK/Buntini.app"
xcrun stapler validate "$BUNTINI_NOTARY_WORK/Buntini.app"
spctl --assess --type execute --verbose=2 "$BUNTINI_NOTARY_WORK/Buntini.app"
mkdir "$BUNTINI_NOTARY_WORK/dmg"
ditto "$BUNTINI_NOTARY_WORK/Buntini.app" "$BUNTINI_NOTARY_WORK/dmg/Buntini.app"
ln -s /Applications "$BUNTINI_NOTARY_WORK/dmg/Applications"
hdiutil create -volname Buntini -srcfolder "$BUNTINI_NOTARY_WORK/dmg" -format UDZO "$BUNTINI_NOTARY_OUTPUT/Buntini-macOS-arm64.dmg"
codesign --force --sign "$BUNTINI_NOTARY_IDENTITY" --timestamp "$BUNTINI_NOTARY_OUTPUT/Buntini-macOS-arm64.dmg"
xcrun notarytool submit "$BUNTINI_NOTARY_OUTPUT/Buntini-macOS-arm64.dmg" --keychain-profile "$BUNTINI_NOTARY_PROFILE" --wait --timeout 30m --output-format json > "$BUNTINI_NOTARY_OUTPUT/dmg-notarization.json"
python3 "$BUNTINI_NOTARY_ROOT/scripts/release_notary_result.py" "$BUNTINI_NOTARY_OUTPUT/dmg-notarization.json"
xcrun stapler staple "$BUNTINI_NOTARY_OUTPUT/Buntini-macOS-arm64.dmg"
APPLE_TEAM_ID="$BUNTINI_NOTARY_TEAM" RELEASE_TAG="$BUNTINI_NOTARY_TAG" sh "$BUNTINI_NOTARY_ROOT/scripts/verify-release.sh" "$BUNTINI_NOTARY_WORK/Buntini.app" "$BUNTINI_NOTARY_OUTPUT/Buntini-macOS-arm64.dmg"
ditto -c -k --keepParent "$BUNTINI_NOTARY_WORK/Buntini.app" "$BUNTINI_NOTARY_OUTPUT/Buntini-macOS-arm64.zip"
python3 - "$BUNTINI_NOTARY_ROOT" "$BUNTINI_NOTARY_TAG" "$BUNTINI_NOTARY_OUTPUT" <<'PY'
import json
from pathlib import Path
import subprocess
import sys
root, tag, output = Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3])
commit = subprocess.run(['git', '-C', str(root), 'rev-parse', 'HEAD'], check=True, capture_output=True, text=True).stdout.strip()
dirty = bool(subprocess.run(['git', '-C', str(root), 'status', '--porcelain'], check=True, capture_output=True, text=True).stdout.strip())
info = {'tag': tag, 'commit': commit, 'sourceTreeDirty': dirty, 'signing': 'Developer ID Application', 'notarization': 'app and DMG accepted; staples, entitlements and Gatekeeper verified'}
(output / 'BUILD-INFO.json').write_text(json.dumps(info, indent=2) + '\n')
PY
(cd "$BUNTINI_NOTARY_OUTPUT" && shasum -a 256 Buntini-macOS-arm64.zip Buntini-macOS-arm64.dmg BUILD-INFO.json app-notarization.json dmg-notarization.json > SHA256SUMS.txt)
rm "$BUNTINI_NOTARY_OUTPUT/.INCOMPLETE"
echo "Verified candidate prepared at $BUNTINI_NOTARY_OUTPUT. Device acceptance and publication remain separate."
