#!/bin/sh
# No changes happen without explicit --python plus an action flag.
set -eu
BUNTINI_VOICE_ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
if [ "$#" -eq 0 ]; then
  set -- --help
fi
exec python3 "$BUNTINI_VOICE_ROOT/worker/setup_voice.py" "$@"
