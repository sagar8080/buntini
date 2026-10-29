#!/usr/bin/env python3
"""Check the exact app layout and optional production signatures without changing it."""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import plistlib
import re
import subprocess
import sys

from release_preflight import validate_versions


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def regular_files(root: Path) -> set[str]:
    require(root.is_dir() and not root.is_symlink(), f"Missing or symlinked artifact directory: {root.name}.")
    files = set()
    for path in root.rglob("*"):
        require(not path.is_symlink(), "App bundles must not contain symlinks.")
        if path.is_file():
            files.add(path.relative_to(root).as_posix())
        else:
            require(path.is_dir(), "App bundles must not contain special files.")
    return files


def validate_bundle(root: Path, app: Path) -> tuple[str, dict]:
    config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
    validate_versions(root, "v" + config["version"])
    require(app.is_dir() and not app.is_symlink(), "App must be a real directory.")
    contents = app / "Contents"
    require({path.name for path in app.iterdir()} == {"Contents"} and contents.is_dir() and not contents.is_symlink(), "App root must contain only a real Contents directory.")
    require((contents / "Info.plist").is_file() and not (contents / "Info.plist").is_symlink(), "Bundle Info.plist is missing or symlinked.")
    info = plistlib.loads((contents / "Info.plist").read_bytes())
    expected_info = plistlib.loads((root / "src-tauri/Info.plist").read_bytes())
    expected_info.update({
        "CFBundleIdentifier": config["identifier"],
        "CFBundleShortVersionString": config["version"],
        "CFBundleVersion": config["bundle"]["macOS"].get("bundleVersion", config["version"]),
        "CFBundlePackageType": "APPL",
        "LSMinimumSystemVersion": config["bundle"]["macOS"]["minimumSystemVersion"],
    })
    require(all(info.get(key) == value for key, value in expected_info.items()), "Bundle identity, version, OS floor or permission descriptions differ from the reviewed source.")
    executable = info.get("CFBundleExecutable", "")
    require(isinstance(executable, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", executable) is not None, "Invalid bundle executable name.")
    require(executable not in ("buntini-audio", "buntini-fm"), "The app cannot use a helper as its main executable.")
    binaries = {executable, "buntini-audio", "buntini-fm"}
    resources = {}
    require(isinstance(config["bundle"]["resources"], dict), "Release resources must use explicit file mappings.")
    for source, destination in config["bundle"]["resources"].items():
        destination_path = PurePosixPath(destination)
        require(not destination_path.is_absolute() and ".." not in destination_path.parts and "\\" not in destination, "Unsafe resource destination.")
        source_path = root / "src-tauri" / source
        require(source_path.is_file() and not source_path.is_symlink(), "Release resources must be individual regular files.")
        require(destination not in resources, "Duplicate release resource destination.")
        resources[destination] = source_path
    icon = info.get("CFBundleIconFile", "")
    require(isinstance(icon, str) and PurePosixPath(icon).name == icon and icon not in ("", ".", ".."), "Invalid bundle icon name.")
    icon_sources = [root / "src-tauri" / source for source in config["bundle"]["icon"] if PurePosixPath(source).name == icon]
    require(len(icon_sources) == 1 and icon_sources[0].is_file(), "Bundle icon does not match a reviewed source icon.")
    resources[icon] = icon_sources[0]
    expected_files = {"Info.plist", "_CodeSignature/CodeResources"}
    # stapler attaches the app's ticket here, separately from its signature's
    # resource envelope. Ticket authenticity is checked by stapler/Gatekeeper.
    if (contents / "CodeResources").exists():
        expected_files.add("CodeResources")
    expected_files.update("MacOS/" + name for name in binaries)
    expected_files.update("Resources/" + destination for destination in resources)
    require(regular_files(contents) == expected_files, "App bundle contains missing or unexpected files.")
    expected_dirs = {PurePosixPath(name).parent.as_posix() for name in expected_files}
    expected_dirs |= {parent.as_posix() for name in expected_files for parent in PurePosixPath(name).parents if parent.as_posix() != "."}
    actual_dirs = {path.relative_to(contents).as_posix() for path in contents.rglob("*") if path.is_dir()}
    require(actual_dirs == expected_dirs - {"."}, "App bundle contains unexpected directories.")
    for name in binaries:
        require((contents / "MacOS" / name).stat().st_mode & 0o111 != 0, "A bundled executable is not executable.")
    for destination, source in resources.items():
        packaged = contents / "Resources" / destination
        require(hashlib.sha256(packaged.read_bytes()).digest() == hashlib.sha256(source.read_bytes()).digest(), "A packaged resource differs from the reviewed source.")
    entitlements = plistlib.loads((root / "src-tauri/Entitlements.plist").read_bytes())
    return executable, entitlements


def validate_entitlements(payload: bytes, expected: dict) -> None:
    try:
        actual = plistlib.loads(payload)
    except (plistlib.InvalidFileException, ValueError) as error:
        raise ValueError("An executable has unreadable signature entitlements.") from error
    require(actual == expected, "Signed entitlements differ from the reviewed policy; debugger, JIT and other exceptions are not permitted.")


def validate_signature(details: str, team: str, runtime: bool, identifier: str | None = None) -> None:
    require(re.fullmatch(r"[A-Z0-9]{10}", team) is not None, "Invalid expected Apple team identifier.")
    lines = details.splitlines()
    require("Signature=adhoc" not in lines, "Ad-hoc signatures cannot be distributed.")
    require(f"TeamIdentifier={team}" in lines, "Signature belongs to a different Apple team or is ad-hoc.")
    require(any(line.startswith("Authority=Developer ID Application: ") for line in lines), "Signature must use Developer ID Application.")
    require(any(line.startswith("Timestamp=") and line.removeprefix("Timestamp=").strip() for line in lines), "Signature must have a secure timestamp.")
    if identifier:
        require(f"Identifier={identifier}" in lines, "Main executable signature has the wrong bundle identifier.")
    flags = re.search(r"\bflags=0x[0-9a-fA-F]+\(([^)]*)\)", details)
    require(flags is not None and "adhoc" not in flags[1].split(","), "Ad-hoc signatures cannot be distributed.")
    if runtime:
        require("runtime" in flags[1].split(","), "An executable is missing the hardened runtime.")


def command(args: list[str]) -> subprocess.CompletedProcess:
    try:
        result = subprocess.run(args, capture_output=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"Could not complete {Path(args[0]).name} artifact verification.") from error
    require(result.returncode == 0, f"{Path(args[0]).name} rejected an artifact.")
    return result


def validate_code(app: Path, executable: str, expected_entitlements: dict, team: str, identifier: str, dmg: Path | None) -> None:
    for name in (executable, "buntini-audio", "buntini-fm"):
        binary = str(app / "Contents/MacOS" / name)
        command(["/usr/bin/codesign", "--verify", "--strict", binary])
        details = command(["/usr/bin/codesign", "-dv", "--verbose=4", binary])
        validate_signature((details.stdout + details.stderr).decode(), team, True, identifier if name == executable else None)
        require(command(["/usr/bin/lipo", "-archs", binary]).stdout.decode().split() == ["arm64"], "An executable is not exclusively arm64.")
        entitlements = command(["/usr/bin/codesign", "-d", "--entitlements", ":-", binary])
        validate_entitlements(entitlements.stdout, expected_entitlements)
    if dmg:
        require(dmg.is_file() and not dmg.is_symlink(), "Disk image must be a regular file.")
        command(["/usr/bin/codesign", "--verify", "--strict", str(dmg)])
        details = command(["/usr/bin/codesign", "-dv", "--verbose=4", str(dmg)])
        validate_signature((details.stdout + details.stderr).decode(), team, False)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--dmg", type=Path)
    parser.add_argument("--tag")
    parser.add_argument("--check-signatures", action="store_true")
    parser.add_argument("--team")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    try:
        if args.tag:
            validate_versions(root, args.tag)
        executable, entitlements = validate_bundle(root, args.app)
        if args.check_signatures:
            require(sys.platform == "darwin" and bool(args.team), "Production signature checks require macOS and --team.")
            config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
            validate_code(args.app, executable, entitlements, args.team, config["identifier"], args.dmg)
    except (ValueError, KeyError, TypeError, OSError, plistlib.InvalidFileException) as error:
        parser.exit(1, f"Release blocked: {error}\n")
    print("Bundle layout, identity, version and resource hashes passed" + ("; production signatures and entitlements passed." if args.check_signatures else ". Signature and notarization acceptance remain separate."))


if __name__ == "__main__":
    main()
