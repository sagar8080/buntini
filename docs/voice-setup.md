# Offline voice setup

This page covers the Whisper fallback. On macOS 26 Buntini first uses Apple's live on-device recognition, which needs none of this; Whisper is used when that isn't available and a local Whisper model is configured. In the fallback, capture ends when you release the microphone control or, after a tap, when you go quiet after speaking. The local native helper writes a bounded 16 kHz mono PCM16 WAV, the worker optionally checks speech presence with Silero ONNX, and MLX Whisper transcribes it. Audio stays on this Mac. The app never installs packages or downloads models.

The approved project setup uses:

| Setting | Local path |
| --- | --- |
| Python interpreter | `/Users/sagar/projects/nexon/.venv-voice/bin/python` |
| Qwen model | `/Users/sagar/projects/pulse-venv/models/Qwen3.5-9B-4bit` |
| Whisper model | `/Users/sagar/projects/nexon/models/whisper-tiny.en` |
| Optional Silero VAD | `/Users/sagar/projects/nexon/models/silero_vad.onnx` |

The isolated project environment includes MLX LM as well as Whisper, so a single selected interpreter can run both chat and voice. The original `pulse-venv` is unchanged. The tiny English Whisper model is a small baseline; recognition accuracy depends on pronunciation, microphone quality, background sound, and the selected model.

## Explicit setup commands

Inspect an existing interpreter without importing MLX or touching its packages:

```sh
./scripts/setup-voice.sh --python /absolute/path/to/venv/bin/python --check
```

Create an isolated project environment only when explicitly requested:

```sh
mkdir -p /absolute/path/to/buntini/models
./scripts/setup-voice.sh \
  --python /absolute/path/to/buntini/.venv-voice/bin/python \
  --create-venv --base-python /absolute/path/to/existing/venv/bin/python \
  --install --with-vad \
  --download-whisper mlx-community/whisper-tiny.en-mlx \
  --model-dir /absolute/path/to/buntini/models/whisper-tiny.en \
  --download-vad --vad-file /absolute/path/to/buntini/models/silero_vad.onnx
```

`--create-venv` requires a new directory. It installs the verified MLX LM 0.32.0 / MLX 0.32.3 runtime into that new environment. Package installation and each model download require separate explicit flags. Existing model destinations are rejected; files are not overwritten. The model folder and environment should stay out of source control.

If you deliberately choose to install voice into an **existing** MLX environment instead:

```sh
./scripts/setup-voice.sh --python /absolute/path/to/venv/bin/python --install --with-vad
```

The installer pins every already-installed package to its current version. A compatibility conflict stops resolution instead of downgrading MLX, NumPy, Transformers, or other shared packages. A new isolated environment is preferable when a shared environment cannot satisfy those constraints. Whisper's published dependencies include Torch and Numba; the setup may download those packages too. Silero filtering itself uses CPU ONNX Runtime and does not import Torch.

Model downloads are optional, separate operations. `--install` alone never downloads weights. To use existing weights, skip download flags and choose the chat model folder in Settings → Brain; for Whisper and Silero, set `BUNTINI_WHISPER_MODEL_PATH` and `BUNTINI_VAD_MODEL_PATH` before the first launch (Settings no longer asks for them, and folders saved by an earlier version keep working). To explicitly fetch models later, use the download flags and a new destination without `--install`.

Sources: the [Whisper tiny.en MLX model card](https://huggingface.co/mlx-community/whisper-tiny.en-mlx), [official MLX Whisper package](https://github.com/ml-explore/mlx-examples/tree/main/whisper), [official Silero VAD implementation](https://github.com/snakers4/silero-vad), and [ONNX Runtime's CPU guidance for macOS](https://onnxruntime.ai/docs/get-started/with-python.html).

## Speech presence and cancellation

Silero VAD is optional. Leave its setting empty to use push-to-talk with Whisper's own `no_speech_threshold=0.6`. When configured, the selected local `.onnx` file must implement the Silero v5/v6 `input`, `state`, and `sr` contract. The worker reads a bounded model file, uses one CPU thread, and resets recurrent state for every recording. Three consecutive 32 ms windows above probability 0.5 indicate speech. Silence and isolated noise produce a recoverable “No speech was detected” message before Whisper is imported or run. The recording is not trimmed, so the filter does not cut leading or trailing words.

This is speech filtering after a bounded push-to-talk capture. It does not add continuous microphone listening, a wake word, or live partial transcripts; the app ends a tapped recording on quiet using the helper's loudness readings, not this filter. The worker checks cancellation between VAD windows; the controller can terminate a blocked native transcription call. Temporary recordings are deleted by the controller after success, error, or cancellation.

## Diagnostics and checks

The Settings diagnostic action uses `ping`. It reads package versions and local configuration/weight presence without importing GPU libraries. `ready_for_load` means the required package metadata and local files are present; a real transcription or generation is the inference check. A missing native library or incompatible model still produces a recoverable runtime error.

```sh
python3 -m unittest discover -s worker/tests -v
```

These model-free tests cover VAD state reset, context framing, speech/noise detection, invalid model contracts, cancellation boundaries, local Whisper configuration, and diagnostics without GPU imports. Real voice verification should use synthesized speech first, then the app's microphone control when the user deliberately starts recording. A microphone permission dialog remains a normal macOS setup step.

The approved setup was completed and verified on October 21, 2026. The isolated environment contains MLX 0.32.3, MLX LM 0.32.0, MLX Whisper 0.4.3, and ONNX Runtime 1.31.0; `pip check` found no broken requirements. The downloaded English weights occupy 71 MB and Silero occupies 2.2 MB (SHA-256 `1a153a22f4509e292a94e67d6f9b85e8deb25b4988682b7e174c65279d8788e3`).

The real smoke test synthesized speech with macOS `say`, converted it with `afconvert`, and sent the resulting local WAV through Silero and Whisper. The exact transcript was **“Hello, Nexon, please check my system memory.”** Silence produced recoverable `no_speech`. The first-ever invocation, including dependency imports/compilation and model load, took 26.443 seconds; a later fresh worker took 1.293 seconds and immediate reuse with the identical synthetic waveform took 0.033 seconds. These observations are not a general transcription-latency guarantee. No microphone was opened.

Repeat that explicit synthetic check with Metal access:

```sh
/Users/sagar/projects/nexon/.venv-voice/bin/python worker/smoke_voice.py \
  --model /Users/sagar/projects/nexon/models/whisper-tiny.en \
  --vad /Users/sagar/projects/nexon/models/silero_vad.onnx
```
