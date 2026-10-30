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
assert [event["event"] for event in events] == ["ready", "level", "stopped"], events
assert 0.06 <= events[1]["rms"] <= 0.08, events[1]
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
# Self-tests write to a named file; live capture alone needs none.
assert json.loads(subprocess.run([binary, "--self-test"], capture_output=True, text=True).stdout)["code"] == "invalid_arguments"
print("Native audio checks passed: 16 kHz mono PCM, resampling, loudness, private permissions, duration and overwrite protection.")
PY

# Live recognition: synthetic speech from macOS's own voice goes through the same
# converter and recognizer as the microphone. Macs without macOS 26 or without the
# speech model report why; the check never downloads a model or opens a microphone.
say -o "$BUNTINI_AUDIO_TEST_DIR/speech.aiff" "What is on my calendar tomorrow morning?"
"$BUNTINI_NATIVE_DIR/buntini-audio" --self-test-live --input "$BUNTINI_AUDIO_TEST_DIR/speech.aiff" > "$BUNTINI_AUDIO_TEST_DIR/live.jsonl"
python3 - "$BUNTINI_AUDIO_TEST_DIR" <<'PY'
import json, sys
from pathlib import Path
root = Path(sys.argv[1])
events = [json.loads(line) for line in (root / "live.jsonl").read_text().splitlines()]
live = next(event for event in events if event["event"] == "live")
if not live["available"]:
    print("Live speech recognition not available here (" + live["reason"] + "); push to talk falls back to recorded audio.")
    sys.exit(0)
partials = [event["text"] for event in events if event["event"] == "partial"]
stopped = events[-1]
assert stopped["event"] == "stopped" and "path" not in stopped, stopped
words = stopped["transcript"].lower()
assert "calendar" in words and "tomorrow" in words, stopped
assert len(partials) >= 2 and partials[-1].lower().startswith(partials[0].lower()), partials
assert [path.name for path in root.glob("*.wav")] == ["test.wav"], "live recognition must not write audio"
print("Live speech checks passed:", len(partials), "partial results, final transcript", repr(stopped["transcript"]))
PY

# Apple Intelligence helper: builds where the SDK allows, then reports availability.
# Machines without Apple Intelligence still pass; the helper must answer either way.
"$BUNTINI_NATIVE_DIR/build-fm.sh"
if [ -x "$BUNTINI_NATIVE_DIR/buntini-fm" ]; then
  "$BUNTINI_NATIVE_DIR/buntini-fm" --self-test-contracts | python3 -c '
import json, sys
result = json.load(sys.stdin)
assert result["check"] == "contracts" and result["ok"] and result["assertions"] >= 35, result
print("Apple helper contract checks passed:", result["assertions"], "assertions; no model or app permissions needed")
'
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
