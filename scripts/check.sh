#!/bin/sh
# Cached-dependency checks only: no package installation or model downloads.
set -eu

BUNTINI_CHECK_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
BUNTINI_CHECK_PYTHON=${BUNTINI_CHECK_PYTHON:-python3}

if [ "${1:-}" = "--help" ]; then
  cat <<'HELP'
Usage: ./scripts/check.sh

Runs the frontend production build, speech-queue and pet-behavior tests, model-free Python
worker tests, locked offline Rust tests, and synthetic native audio checks.

Prerequisites: existing node_modules, cached Cargo dependencies, the bundled memory
search model (./scripts/fetch-models.sh, once), npm, cargo, Python 3.11+, macOS with
Xcode Command Line Tools. The checks do not install packages, download models, open a
microphone, contact search providers, or mutate a venv.

Set BUNTINI_CHECK_PYTHON to an existing Python interpreter for worker tests.
Real Qwen/Whisper inference and interactive macOS acceptance are separate checks
documented in docs/acceptance.md.
HELP
  exit 0
fi
if [ "$#" -ne 0 ]; then
  echo "Unknown argument. Use ./scripts/check.sh --help." >&2
  exit 2
fi

for BUNTINI_CHECK_TOOL in npm cargo "$BUNTINI_CHECK_PYTHON" python3 xcrun; do
  if ! command -v "$BUNTINI_CHECK_TOOL" >/dev/null 2>&1; then
    echo "Missing prerequisite: $BUNTINI_CHECK_TOOL. Use your existing development tools; this script does not install them." >&2
    exit 1
  fi
done
if ! "$BUNTINI_CHECK_PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)'; then
  echo "The check suite requires Python 3.11 or later." >&2
  exit 1
fi
if [ ! -f "$BUNTINI_CHECK_ROOT/models/bge-small-en-v1.5/model.safetensors" ]; then
  echo "The memory search model is missing. Run ./scripts/fetch-models.sh once, then check again." >&2
  exit 1
fi
if [ "$(uname -s)" != "Darwin" ]; then
  echo "Buntini's desktop/native checks require macOS." >&2
  exit 1
fi
if [ ! -d "$BUNTINI_CHECK_ROOT/node_modules" ]; then
  echo "node_modules is missing. Prepare the project's dependencies before running these offline checks." >&2
  exit 1
fi

cd "$BUNTINI_CHECK_ROOT"
echo "Checking frontend production build..."
npm run build
echo "Checking sentence streaming, interruption and pet behavior..."
npm run test:speech
npm run test:pet
echo "Checking Python JSONL worker without models..."
"$BUNTINI_CHECK_PYTHON" -m unittest discover -s worker/tests -v
echo "Checking release preflight rejection paths..."
"$BUNTINI_CHECK_PYTHON" -m unittest discover -s scripts/tests -v
echo "Checking native audio with synthetic input (microphone stays closed)..."
bash "$BUNTINI_CHECK_ROOT/native/test.sh"
echo "Checking Rust controller with locked, cached dependencies..."
cargo test --locked --offline --target-dir "$BUNTINI_CHECK_ROOT/src-tauri/target" --manifest-path "$BUNTINI_CHECK_ROOT/src-tauri/Cargo.toml"
echo "Validating scenario coverage (device evidence is scored separately per brain)..."
"$BUNTINI_CHECK_PYTHON" "$BUNTINI_CHECK_ROOT/scripts/evaluate.py"
echo "All automated checks passed. Device acceptance remains listed in docs/acceptance.md."
