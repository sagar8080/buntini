#!/usr/bin/env python3
"""Opt-in local Whisper/Silero smoke using synthesized speech, never a microphone."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import subprocess
import tempfile
import time
import wave

from smoke_local import Connection


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help="Existing absolute converted MLX Whisper folder")
    parser.add_argument("--vad", help="Existing absolute local Silero ONNX file")
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="buntini-synthetic-voice-") as directory:
        root = Path(directory)
        speech, audio = root / "sample.aiff", root / "sample.wav"
        subprocess.run(["/usr/bin/say", "-v", "Samantha", "-r", "155", "-o", str(speech), "Hello Buntini. Please check my system memory."], check=True)
        subprocess.run(["/usr/bin/afconvert", "-f", "WAVE", "-d", "LEI16@16000", "-c", "1", str(speech), str(audio)], check=True)
        connection = Connection()
        try:
            start = time.monotonic()
            connection.send({"id": "voice", "type": "transcribe", "model_path": args.model, "audio_path": str(audio), "vad_model_path": args.vad or ""})
            result = connection.wait("voice")[-1]
            elapsed = round(time.monotonic() - start, 3)
            print(json.dumps({"check": "synthetic_speech", "seconds": elapsed, "text": result["text"], "vad_enabled": bool(args.vad)}), flush=True)
            assert "system memory" in result["text"].lower(), result
            start = time.monotonic()
            connection.send({"id": "voice-warm", "type": "transcribe", "model_path": args.model, "audio_path": str(audio), "vad_model_path": args.vad or ""})
            warm = connection.wait("voice-warm")[-1]
            print(json.dumps({"check": "synthetic_speech_warm", "seconds": round(time.monotonic() - start, 3), "text": warm["text"]}), flush=True)
            assert "system memory" in warm["text"].lower(), warm
            if args.vad:
                silence = root / "silence.wav"
                with wave.open(str(silence), "wb") as wav:
                    wav.setparams((1, 2, 16_000, 0, "NONE", "not compressed"))
                    wav.writeframes(b"\0\0" * 16_000)
                connection.send({"id": "silence", "type": "transcribe", "model_path": args.model, "audio_path": str(silence), "vad_model_path": args.vad})
                try:
                    connection.wait("silence")
                except RuntimeError as exc:
                    assert str(exc).startswith("no_speech:"), exc
                    print(json.dumps({"check": "silence_filter", "code": "no_speech"}), flush=True)
                else:
                    raise AssertionError("Silero accepted a silent recording")
            connection.send({"id": "health", "type": "ping", "whisper_model_path": args.model, "vad_model_path": args.vad or ""})
            diagnostics = connection.wait("health", "pong")[-1]
            print(json.dumps({"check": "diagnostics", "dependencies": diagnostics["dependencies"], "models": diagnostics["models"]}), flush=True)
            print("Real offline voice smoke passed; no microphone was opened.", flush=True)
        finally:
            connection.close()


if __name__ == "__main__":
    main()
