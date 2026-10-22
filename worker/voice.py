"""Local voice model validation, dependency diagnostics, and optional Silero VAD."""

from __future__ import annotations

import importlib
import importlib.metadata
import json
import math
import os
from pathlib import Path
import threading
from typing import Any

from worker.protocol import Cancelled, WorkerError, local_model_path

WHISPER_WEIGHTS = ("model.safetensors", "weights.safetensors", "weights.npz")
WHISPER_DIMENSIONS = ("n_mels", "n_audio_ctx", "n_audio_state", "n_audio_head", "n_audio_layer", "n_vocab", "n_text_ctx", "n_text_state", "n_text_head", "n_text_layer")
MAX_VAD_BYTES = 32 * 1024 * 1024


def whisper_model_path(value: Any, environment_value: str | None = None) -> Path:
    path = local_model_path(value, environment_value, "Whisper")
    config_path = path / "config.json"
    if config_path.stat().st_size > 65_536:
        raise WorkerError("invalid_whisper_model", "Whisper config.json exceeds the safe size limit")
    try:
        config = json.loads(config_path.read_text(encoding="utf-8"))
    except (OSError, ValueError, RecursionError) as exc:
        raise WorkerError("invalid_whisper_model", "The local Whisper configuration is invalid") from exc
    if not isinstance(config, dict) or any(type(config.get(key)) is not int or config[key] <= 0 for key in WHISPER_DIMENSIONS):
        raise WorkerError("invalid_whisper_model", "Choose a converted MLX Whisper folder with a Whisper dimensions configuration")
    if not any((path / name).is_file() and (path / name).stat().st_size > 0 for name in WHISPER_WEIGHTS):
        raise WorkerError("invalid_whisper_model", "The local Whisper folder has no converted model weights")
    return path


def vad_model_path(value: Any, environment_value: str | None = None) -> Path | None:
    raw = value if value is not None else environment_value
    if raw is None or raw == "":
        return None
    if not isinstance(raw, str):
        raise WorkerError("invalid_vad_model", "Select an existing absolute Silero ONNX file, or leave VAD disabled")
    path = Path(raw).expanduser()
    if not path.is_absolute() or path.suffix.lower() != ".onnx" or not path.is_file():
        raise WorkerError("invalid_vad_model", "Silero VAD requires an existing absolute local .onnx file")
    if not 0 < path.stat().st_size <= MAX_VAD_BYTES:
        raise WorkerError("invalid_vad_model", "Silero VAD model must contain at most 32 MB")
    return path.resolve()


def dependency_diagnostics(request: dict[str, Any]) -> dict[str, Any]:
    """Read distribution metadata and local file headers only; no GPU imports."""
    dependencies = {}
    for key, distribution in (("mlx_lm", "mlx-lm"), ("mlx_whisper", "mlx-whisper"), ("onnxruntime", "onnxruntime"), ("numpy", "numpy")):
        try:
            version = importlib.metadata.version(distribution)
        except importlib.metadata.PackageNotFoundError:
            version = None
        dependencies[key] = {"installed": version is not None, "version": version}
    models = {}
    for key, field, env, dependency in (
        ("qwen", "model_path", "BUNTINI_MODEL_PATH", "mlx_lm"),
        ("whisper", "whisper_model_path", "BUNTINI_WHISPER_MODEL_PATH", "mlx_whisper"),
        ("vad", "vad_model_path", "BUNTINI_VAD_MODEL_PATH", "onnxruntime"),
    ):
        raw = request.get(field, os.environ.get(env))
        configured = isinstance(raw, str) and bool(raw.strip())
        error = None
        present = False
        if configured:
            try:
                if key == "whisper":
                    whisper_model_path(raw)
                elif key == "vad":
                    vad_model_path(raw)
                else:
                    path = local_model_path(raw, None, "Qwen")
                    if not any(path.glob("*.safetensors")):
                        raise WorkerError("model_not_found", "The local Qwen folder has no safetensors weights")
                present = True
            except (WorkerError, OSError, ValueError) as exc:
                error = str(exc)
        models[key] = {"configured": configured, "files_present": present, "ready_for_load": present and dependencies[dependency]["installed"] and dependencies["numpy"]["installed"], "error": error}
    return {"python": {"version": os.sys.version.split()[0], "executable": os.sys.executable}, "dependencies": dependencies, "models": models, "inference_verified": False}


class SileroVAD:
    """Silero v5/v6 ONNX state/input/sr contract, CPU only, one session cached.

    Each recording resets the recurrent state and 64-sample context. This is a
    speech-presence filter, not a wake word or automatic microphone controller.
    """

    def __init__(self):
        self.session: Any = None
        self.identity: tuple[Any, ...] | None = None

    def contains_speech(self, audio: Any, model_path: Path, cancel: threading.Event) -> bool:
        if cancel.is_set():
            raise Cancelled()
        try:
            np = importlib.import_module("numpy")
            runtime = importlib.import_module("onnxruntime")
        except (ImportError, OSError, RuntimeError) as exc:
            raise WorkerError("vad_unavailable", f"Optional local Silero VAD is unavailable: {exc}") from exc
        info = model_path.stat()
        identity = (model_path, info.st_size, info.st_mtime_ns)
        try:
            if self.identity != identity:
                if hasattr(runtime, "disable_telemetry_events"):
                    runtime.disable_telemetry_events()
                options = runtime.SessionOptions()
                options.inter_op_num_threads = 1
                options.intra_op_num_threads = 1
                # Bytes prohibit loading external ONNX tensor files relative to
                # an arbitrary folder. No custom operators are registered.
                session = runtime.InferenceSession(model_path.read_bytes(), providers=["CPUExecutionProvider"], sess_options=options)
                if {item.name for item in session.get_inputs()} != {"input", "state", "sr"} or len(session.get_outputs()) != 2:
                    raise WorkerError("invalid_vad_model", "Use the Silero v5/v6 ONNX model with input, state, and sr inputs")
                self.session, self.identity = session, identity
            state = np.zeros((2, 1, 128), dtype=np.float32)
            context = np.zeros((1, 64), dtype=np.float32)
            consecutive = 0
            for offset in range(0, len(audio), 512):
                if cancel.is_set():
                    raise Cancelled()
                chunk = audio[offset : offset + 512]
                if len(chunk) < 512:
                    chunk = np.pad(chunk, (0, 512 - len(chunk)))
                window = np.concatenate((context, chunk.reshape(1, 512)), axis=1).astype(np.float32)
                probability, state = self.session.run(None, {"input": window, "state": state, "sr": np.array(16_000, dtype=np.int64)})
                score = float(probability.reshape(-1)[0])
                if not math.isfinite(score) or not 0 <= score <= 1 or state.shape != (2, 1, 128) or not np.isfinite(state).all():
                    raise WorkerError("invalid_vad_model", "Silero returned an invalid speech probability or recurrent state")
                if cancel.is_set():
                    raise Cancelled()
                consecutive = consecutive + 1 if score >= 0.5 else 0
                if consecutive >= 3:  # 96 ms, preserving brief push-to-talk words.
                    return True
                context = window[:, -64:]
            return False
        except (Cancelled, WorkerError):
            raise
        except Exception as exc:
            raise WorkerError("vad_failed", f"The configured local Silero model could not process this recording: {exc}") from exc
