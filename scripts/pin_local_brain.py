#!/usr/bin/env python3
"""Pins everything one-click local setup downloads, so the app fetches exact bytes.

Writes worker/requirements-local.lock (every package with the hashes of its wheels for
every macOS on Apple silicon) and worker/local-brain.json (Python, the package download
size, and each model's revision, files, sizes and SHA-256).

  python3 scripts/pin_local_brain.py <python> --tag 20261026 --version 3.12.16

<python> is a standalone Python of that version: pip resolves the packages with it.
Run it by hand and review the diff; the app only ever reads the results.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REQUIREMENT = "mlx-lm>=0.30,<0.33"
PYTHON_RELEASES = "https://api.github.com/repos/astral-sh/python-build-standalone/releases/tags/"
# The advisor's models (src-tauri/src/advisor.rs MODELS).
MODELS = [
    "Qwen3.5-0.8B-MLX-4bit", "Qwen3.5-4B-MLX-4bit", "gemma-4-e2b-it-4bit", "gemma-4-e4b-it-4bit",
    "Qwen3.5-9B-4bit", "Qwen3.5-27B-4bit", "Qwen3.5-35B-A3B-4bit", "gemma-4-31b-it-4bit",
]
# What the worker never reads.
SKIPPED_FILES = re.compile(r"(^|/)(\.gitattributes|README\.md|.*\.(md|png|jpg|gif))$", re.IGNORECASE)


def get_json(url: str):
    request = urllib.request.Request(url, headers={"User-Agent": "buntini-pin"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def wheel_fits(filename: str) -> bool:
    """A wheel Python 3.12 can use on Apple silicon, on any macOS."""
    if not filename.endswith(".whl"):
        return False
    python, abi, platform = filename[:-4].rsplit("-", 3)[1:]
    if not (python in ("py3", "py2.py3", "cp312") or (abi == "abi3" and python.startswith("cp3"))):
        return False
    return platform == "any" or ("macosx" in platform and ("arm64" in platform or "universal2" in platform))


def lowest_macos(filename: str) -> tuple[int, int]:
    found = re.findall(r"macosx_(\d+)_(\d+)", filename)
    return min(((int(a), int(b)) for a, b in found), default=(0, 0))


def pin_packages(python: str) -> tuple[list[str], int, tuple[int, int]]:
    with tempfile.TemporaryDirectory() as folder:
        report = Path(folder) / "report.json"
        subprocess.run(
            [python, "-I", "-m", "pip", "install", "--isolated", "--disable-pip-version-check", "--only-binary", ":all:",
             "--dry-run", "--ignore-installed", "--quiet", "--report", str(report), REQUIREMENT],
            check=True,
        )
        resolved = json.loads(report.read_text())["install"]
    lines, largest_total, needs = [], 0, (0, 0)
    for item in sorted(resolved, key=lambda item: item["metadata"]["name"].lower()):
        name, version = item["metadata"]["name"], item["metadata"]["version"]
        files = [file for file in get_json(f"https://pypi.org/pypi/{name}/{version}/json")["urls"] if wheel_fits(file["filename"])]
        if not files:
            sys.exit(f"{name} {version} has no wheel for Python 3.12 on Apple silicon.")
        # The oldest macOS any wheel of this package runs on limits the whole set.
        needs = max(needs, min(lowest_macos(file["filename"]) for file in files))
        largest_total += max(file["size"] for file in files)
        hashes = " ".join(f"--hash=sha256:{file['digests']['sha256']}" for file in sorted(files, key=lambda file: file["filename"]))
        lines.append(f"{name}=={version} {hashes}")
    return lines, largest_total, needs


def pin_python(tag: str, version: str) -> dict:
    name = f"cpython-{version}+{tag}-aarch64-apple-darwin-install_only_stripped.tar.gz"
    asset = next(asset for asset in get_json(PYTHON_RELEASES + tag)["assets"] if asset["name"] == name)
    digest = asset["digest"]
    if not digest.startswith("sha256:"):
        sys.exit("The Python release has no SHA-256 digest.")
    return {"version": version, "url": asset["browser_download_url"], "bytes": asset["size"], "sha256": digest.removeprefix("sha256:")}


def pin_model(repo: str) -> dict:
    full = f"mlx-community/{repo}"
    info = get_json(f"https://huggingface.co/api/models/{full}")
    revision = info["sha"]
    files = []
    for entry in get_json(f"https://huggingface.co/api/models/{full}/tree/{revision}?recursive=1"):
        path = entry["path"]
        if entry["type"] != "file" or SKIPPED_FILES.search(path):
            continue
        if "lfs" in entry:
            sha256, size = entry["lfs"]["oid"], entry["lfs"]["size"]
        else:
            # Small files have only a git hash, so read them to get a SHA-256.
            with urllib.request.urlopen(f"https://huggingface.co/{full}/resolve/{revision}/{path}", timeout=120) as response:
                body = response.read()
            sha256, size = hashlib.sha256(body).hexdigest(), len(body)
        files.append({"path": path, "bytes": size, "sha256": sha256})
    if not any(file["path"] == "config.json" for file in files):
        sys.exit(f"{repo} has no config.json.")
    return {
        "repo": full,
        "revision": revision,
        "license": info.get("cardData", {}).get("license", "unknown"),
        "bytes": sum(file["bytes"] for file in files),
        "files": sorted(files, key=lambda file: file["path"]),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("python")
    parser.add_argument("--tag", required=True)
    parser.add_argument("--version", required=True)
    args = parser.parse_args()
    lines, package_bytes, needs = pin_packages(args.python)
    header = f"# Generated by scripts/pin_local_brain.py from {REQUIREMENT}. Do not edit.\n"
    (ROOT / "worker/requirements-local.lock").write_text(header + "\n".join(lines) + "\n")
    pins = {
        "python": pin_python(args.tag, args.version),
        "packageBytes": package_bytes,
        "minimumMacos": f"{needs[0]}.{needs[1]}",
        "models": {repo: pin_model(repo) for repo in MODELS},
    }
    (ROOT / "worker/local-brain.json").write_text(json.dumps(pins, indent=1) + "\n")
    for repo, model in pins["models"].items():
        print(f"{repo}: {model['bytes'] / 1e9:.2f} GB, {len(model['files'])} files, {model['license']}")
    print(f"{len(lines)} packages, up to {package_bytes / 1e6:.0f} MB; needs macOS {pins['minimumMacos']} or newer")


if __name__ == "__main__":
    main()
