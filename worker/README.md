# Local MLX inference worker

`main.py` is a persistent Python process controlled by Rust over newline-delimited JSON on stdin/stdout. It lazily loads the configured local Qwen model once and keeps it in memory between turns. It streams visible answer text and emits schema-validated tool requests. **It never executes a model's tools.** Rust owns filesystem scope, online consent, credentials, execution, and audit logging.

## Use an existing environment and local model

Select the Python interpreter from your existing Apple Silicon virtual environment in the app's Settings. For example:

```sh
/absolute/path/to/venv/bin/python -u /absolute/path/to/buntini/worker/main.py
```

Set `BUNTINI_MODEL_PATH` to an absolute local MLX model directory, or send `model_path` with each generate request. A Hugging Face cache **snapshot directory** is suitable; a repository ID is deliberately rejected. The folder must contain `config.json`, local model weights, and tokenizer/chat-template files. Both older Qwen Hermes JSON and Qwen 3.5 native function/parameter tool tags are supported. Custom Python model/tokenizer code is disabled.

The app does not create or modify your virtual environment, run package installers, convert models, or download weights. If you choose to prepare another environment, the optional package list is in `requirements.txt`. Real inference was verified on October 21, 2026 with the user's existing `mlx-lm` 0.32.0 environment and local Qwen3.5-9B-4bit model: normal text, a validated `system_info` request, a grounded synthetic tool-result follow-up, ping/cancellation, and reuse of the same worker all passed. This required Metal access outside the tool sandbox.

`HF_HUB_OFFLINE=1`, `TRANSFORMERS_OFFLINE=1`, and disabled telemetry are set before MLX imports. Load failures and missing dependencies produce recoverable error events; they never silently select a test backend.

## Offline voice

`transcribe` lazily imports `mlx_whisper` and uses `BUNTINI_WHISPER_MODEL_PATH` or that request's `model_path`, also an existing absolute local folder containing a valid Whisper dimensions `config.json` and converted Whisper weights. The approved isolated project `.venv-voice` now contains MLX LM 0.32.0, MLX Whisper 0.4.3 and CPU ONNX Runtime 1.31.0. It can run both the user's existing Qwen and the local tiny English Whisper model. The original `pulse-venv` is unchanged.

Rust supplies an absolute path to a captured **mono PCM16 WAV at 16 kHz**, up to 90 seconds. Python's standard `wave` decoder and NumPy convert it to a waveform; this path does not invoke ffmpeg. The worker does not record the microphone or upload audio. The controller is responsible for recording consent and cleaning up its temporary audio files.

Optional `vad_model_path` selects an existing local Silero v5/v6 ONNX model. The worker runs it on CPU and filters recordings without sustained speech before importing Whisper; an empty setting preserves push-to-talk with Whisper's own no-speech threshold. It resets VAD state for each recording, checks cancellation per window, and does not trim audio. See [voice setup](../docs/voice-setup.md) for explicit setup commands, diagnostics, model paths, and the real synthesized-speech test.

MLX generation checks cancellation between tokens and during prompt prefill. The stdin reader remains responsive to ping/cancel while inference runs. Model loading and `mlx_whisper.transcribe` do not expose a safe native interruption API; the controller must terminate/restart the process after its cancellation grace period if those calls stall. Cancelling invalidates any eventual transcript.

## Explicit model-free testing

```sh
python3 -m unittest discover -s worker/tests -v
python3 -u worker/main.py --fixture
```

Fixture mode requires the explicit `--fixture` flag or `BUNTINI_WORKER_FIXTURE=1`. It uses the real IPC reader, bounds, incremental Qwen parser, and tool validation. Its answers and transcripts visibly say `Fixture mode`. It does not perform inference, search, or audio recognition.

In fixture mode, send `fixture:system` to request `system_info`, `fixture:slow` to test cancellation, or `fixture:tool {"name":"search_files","arguments":{"query":"invoice"}}` to test the tool round trip. All other requests return labelled fixture text. These commands have no special meaning to the production MLX backend.

The separate, opt-in real-model smoke harness runs small generations and uses a synthetic tool result; it never executes tools, opens the microphone, or downloads a model:

```sh
/absolute/path/to/venv/bin/python worker/smoke_local.py --model /absolute/local/Qwen
```

The first real response, including the initial 9B load, took 10.544 seconds in the development run. The validated tool request took 5.226 seconds, the grounded follow-up took 7.063 seconds, ping plus cancellation took 68 ms, and the reused worker's short response took 2.303 seconds. These are smoke-test observations, not general performance guarantees or first-token benchmarks.

See [PROTOCOL.md](PROTOCOL.md) for the wire contract and limits.

## API references

The adapters follow the official [MLX LM Python streaming API](https://github.com/ml-explore/mlx-lm/blob/main/README.md#streaming), the [Qwen Hermes tool format](https://qwen.readthedocs.io/en/latest/framework/function_call.html), and the [MLX Whisper Python API](https://github.com/ml-explore/mlx-examples/tree/main/whisper#api).
