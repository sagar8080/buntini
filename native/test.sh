#!/bin/sh
set -eu
BUNTINI_NATIVE_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
"$BUNTINI_NATIVE_DIR/build.sh"
BUNTINI_AUDIO_TEST_DIR=$(mktemp -d "${TMPDIR:-/tmp}/buntini-audio-test.XXXXXX")
trap 'rm -rf "$BUNTINI_AUDIO_TEST_DIR"' EXIT HUP INT TERM
"$BUNTINI_NATIVE_DIR/buntini-audio" --self-test --output "$BUNTINI_AUDIO_TEST_DIR/test.wav" > "$BUNTINI_AUDIO_TEST_DIR/events.jsonl"
python3 - "$BUNTINI_AUDIO_TEST_DIR" "$BUNTINI_NATIVE_DIR/buntini-audio" <<'PY'
import json
from pathlib import Path
import stat
import subprocess
import sys
import wave

root = Path(sys.argv[1])
binary = sys.argv[2]
events = [json.loads(line) for line in (root / "events.jsonl").read_text().splitlines()]
assert [event["event"] for event in events] == ["ready", "stopped"], events
stopped = events[-1]
assert stopped["path"] == str(root / "test.wav")
assert 90 <= stopped["durationMs"] <= 110, stopped
assert stopped["bytes"] == (root / "test.wav").stat().st_size
assert stat.S_IMODE((root / "test.wav").stat().st_mode) == 0o600
with wave.open(str(root / "test.wav")) as audio:
    assert audio.getframerate() == 16000
    assert audio.getnchannels() == 1
    assert audio.getsampwidth() == 2
    assert 1440 <= audio.getnframes() <= 1760
    assert any(audio.readframes(audio.getnframes()))

# Existing files are never overwritten or deleted, even on a failed capture.
before = (root / "test.wav").read_bytes()
collision = subprocess.run([binary, "--self-test", "--output", str(root / "test.wav")], capture_output=True, text=True)
assert collision.returncode != 0
assert json.loads(collision.stdout.splitlines()[-1])["code"] == "output_unavailable"
assert (root / "test.wav").read_bytes() == before
for args in (["--output", "relative.wav"], ["--output", str(root / "bad.wav"), "--max-seconds", "121"]):
    invalid = subprocess.run([binary, *args], capture_output=True, text=True)
    assert invalid.returncode != 0
    assert json.loads(invalid.stdout)["code"] == "invalid_arguments"
assert not (root / "bad.wav").exists()
print("Native audio checks passed: 16 kHz mono PCM, resampling, private permissions, duration and overwrite protection.")
PY

# Apple Intelligence helper: builds where the SDK allows, then reports availability.
# Machines without Apple Intelligence still pass; the helper must answer either way.
"$BUNTINI_NATIVE_DIR/build-fm.sh"
if [ -x "$BUNTINI_NATIVE_DIR/buntini-fm" ]; then
  "$BUNTINI_NATIVE_DIR/buntini-fm" --self-test --tools "$BUNTINI_NATIVE_DIR/../worker/tools.json" | python3 -c '
import json, sys
lines = [json.loads(line) for line in sys.stdin]
status = lines[0]
assert status["check"] == "availability" and status["tools"] >= 9, status
if status["available"]:
    assert lines[1]["ok"], lines
print("Apple Intelligence helper checks passed:", "available" if status["available"] else "not available here (" + status["reason"] + ")")
'
fi
