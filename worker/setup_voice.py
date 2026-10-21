#!/usr/bin/env python3
"""Explicit voice setup for an existing MLX Python environment; no automatic downloads."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent.parent
VAD_URL = "https://raw.githubusercontent.com/snakers4/silero-vad/master/src/silero_vad/data/silero_vad.onnx"


def target_metadata(python: Path) -> dict:
    code = "import importlib.metadata as m,json,platform,sys; print(json.dumps({'python':sys.executable,'platform':sys.platform,'machine':platform.machine(),'packages':{d.metadata['Name'].lower().replace('_','-'):d.version for d in m.distributions() if d.metadata['Name']}}))"
    result = subprocess.run([str(python), "-c", code], check=True, capture_output=True, text=True)
    return json.loads(result.stdout)


def install(python: Path, metadata: dict, with_vad: bool, new_environment: bool = False) -> None:
    if metadata["platform"] != "darwin" or metadata["machine"] != "arm64":
        raise ValueError("MLX voice installation requires the existing Apple Silicon macOS interpreter")
    if not new_environment and not {"mlx-lm", "mlx"}.issubset(metadata["packages"]):
        raise ValueError("Choose the existing environment that already runs your MLX Qwen model; this script does not replace or create it")
    # Every installed package is constrained to its current version. Pip resolves
    # before installation and fails on conflicts instead of downgrading MLX,
    # NumPy, Transformers, or unrelated shared-environment dependencies.
    with tempfile.TemporaryDirectory(prefix="buntini-voice-plan-") as directory:
        constraints = Path(directory) / "existing-packages.txt"
        lines = []
        for name, version in sorted(metadata["packages"].items()):
            if not re.fullmatch(r"[A-Za-z0-9_.-]+", name) or not re.fullmatch(r"[A-Za-z0-9_.+!-]+", version):
                raise ValueError("An installed package has nonstandard metadata; inspect the environment before installing voice dependencies")
            lines.append(f"{name}=={version}")
        constraints.write_text("\n".join(lines) + "\n", encoding="utf-8")
        command = [str(python), "-m", "pip", "install", "--no-input", "--upgrade-strategy", "only-if-needed", "--constraint", str(constraints), "-r", str(ROOT / "worker" / "requirements-voice.txt")]
        if new_environment:
            # Match the Qwen runtime already verified with the user's model.
            command += ["mlx-lm==0.32.0", "mlx==0.32.3"]
        if with_vad:
            command += ["-r", str(ROOT / "worker" / "requirements-vad.txt")]
        print("Installing explicit voice dependencies; all existing package versions are pinned.", flush=True)
        subprocess.run(command, check=True)


def fresh_destination(raw: str, file: bool = False) -> Path:
    path = Path(raw).expanduser()
    if not path.is_absolute() or path.exists() or path.is_symlink():
        raise ValueError("Download destination must be an absolute path that does not already exist")
    if not path.parent.is_dir():
        raise ValueError("Create the destination's parent directory before requesting a download")
    if file and path.suffix.lower() != ".onnx":
        raise ValueError("The VAD destination must end in .onnx")
    return path


def download_whisper(python: Path, repo: str, raw_destination: str) -> None:
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]*/[A-Za-z0-9][A-Za-z0-9_.-]*", repo):
        raise ValueError("Provide an explicit Hugging Face model repository in owner/name format")
    destination = fresh_destination(raw_destination)
    code = """import os,sys
from pathlib import Path
os.environ.pop('HF_HUB_OFFLINE',None)
os.environ.pop('TRANSFORMERS_OFFLINE',None)
os.environ['HF_HUB_DISABLE_TELEMETRY']='1'
from huggingface_hub import snapshot_download
sys.path.insert(0,sys.argv[3])
from worker.voice import whisper_model_path
snapshot_download(repo_id=sys.argv[1],local_dir=sys.argv[2],allow_patterns=['config.json','model.safetensors','weights.safetensors','weights.npz','README.md'])
whisper_model_path(sys.argv[2])
"""
    with tempfile.TemporaryDirectory(prefix=".buntini-whisper-", dir=destination.parent) as directory:
        staged = Path(directory) / "model"
        subprocess.run([str(python), "-c", code, repo, str(staged), str(ROOT)], check=True)
        if destination.exists():
            raise ValueError("The destination appeared during download; no existing files were replaced")
        staged.rename(destination)
    print(f"Downloaded the explicitly requested Whisper model to {destination}")


def download_vad(raw_destination: str) -> None:
    destination = fresh_destination(raw_destination, file=True)
    digest = hashlib.sha256()
    with tempfile.TemporaryDirectory(prefix=".buntini-vad-", dir=destination.parent) as directory:
        staged = Path(directory) / "silero.onnx"
        size = 0
        with urllib.request.urlopen(VAD_URL, timeout=60) as response, staged.open("wb") as output:
            while chunk := response.read(65_536):
                size += len(chunk)
                if size > 32 * 1024 * 1024:
                    raise ValueError("The VAD download exceeded 32 MB")
                output.write(chunk)
                digest.update(chunk)
        if not size:
            raise ValueError("The VAD download was empty")
        # Exclusive creation protects an existing file even if one appears
        # between validation and completion of a network request.
        with destination.open("xb") as output, staged.open("rb") as source:
            while chunk := source.read(65_536):
                output.write(chunk)
    print(f"Downloaded the explicitly requested official Silero ONNX model to {destination}")
    print(f"SHA-256: {digest.hexdigest()}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--python", required=True, help="Absolute existing MLX virtual environment Python; required for every action")
    parser.add_argument("--check", action="store_true", help="Report package versions only; default when no mutation flag is supplied")
    parser.add_argument("--install", action="store_true", help="Explicitly install mlx-whisper into that environment without changing existing versions")
    parser.add_argument("--create-venv", action="store_true", help="Explicitly create a new isolated environment at --python's bin/python path; requires --base-python and --install")
    parser.add_argument("--base-python", help="Existing absolute interpreter used only to create the isolated venv")
    parser.add_argument("--with-vad", action="store_true", help="Also install optional CPU onnxruntime when --install is present")
    parser.add_argument("--download-whisper", metavar="OWNER/REPO", help="Explicitly download this converted Whisper model; never implied by --install")
    parser.add_argument("--model-dir", help="New absolute destination for the explicitly requested Whisper download")
    parser.add_argument("--download-vad", action="store_true", help="Explicitly download the official Silero ONNX model; never implied by --install")
    parser.add_argument("--vad-file", help="New absolute .onnx destination for the explicitly requested VAD download")
    args = parser.parse_args()
    python = Path(args.python).expanduser()
    if args.create_venv:
        base = Path(args.base_python or "").expanduser()
        if not args.install or not base.is_absolute() or not base.is_file() or not os.access(base, os.X_OK):
            parser.error("--create-venv requires --install and an existing absolute --base-python")
        if not python.is_absolute() or python.name != "python" or python.parent.name != "bin" or python.parent.parent.exists():
            parser.error("For --create-venv use a new absolute /path/to/environment/bin/python")
    elif not python.is_absolute() or not python.is_file() or not os.access(python, os.X_OK):
        parser.error("--python must be an existing absolute executable path")
    if bool(args.download_whisper) != bool(args.model_dir) or bool(args.download_vad) != bool(args.vad_file):
        parser.error("Model download flags require their matching explicit destination")
    if args.with_vad and not args.install:
        parser.error("--with-vad requires --install; it does not silently install anything")
    if args.check and (args.install or args.download_whisper or args.download_vad):
        parser.error("--check cannot be combined with installation or model downloads")
    try:
        if args.create_venv:
            subprocess.run([str(base), "-m", "venv", str(python.parent.parent)], check=True)
        metadata = target_metadata(python)
        if args.install:
            install(python, metadata, args.with_vad, new_environment=args.create_venv)
            metadata = target_metadata(python)
        if args.download_whisper:
            download_whisper(python, args.download_whisper, args.model_dir)
        if args.download_vad:
            download_vad(args.vad_file)
        print(json.dumps({"python": metadata["python"], "platform": metadata["platform"], "machine": metadata["machine"], "dependencies": {name: metadata["packages"].get(name) for name in ("mlx", "mlx-lm", "mlx-whisper", "numpy", "onnxruntime")}}, indent=2))
    except (ValueError, OSError, subprocess.CalledProcessError, json.JSONDecodeError) as exc:
        parser.exit(1, f"Voice setup stopped: {exc}\nNo existing package version was intentionally changed. Resolve dependency conflicts in a separate environment rather than relaxing the constraints.\n")


if __name__ == "__main__":
    main()
