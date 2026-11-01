#!/usr/bin/env python3
"""Read-only inspect a DMG, verify its app and compare it with the reviewed app."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import sys
import tempfile

from release_artifact import command, require, validate_bundle, validate_code


def fingerprint(app: Path) -> dict:
    result = {}
    for path in (app, *app.rglob("*")):
        require(not path.is_symlink(), "App comparison refuses symlinks.")
        mode = path.stat().st_mode
        require(stat.S_ISREG(mode) or stat.S_ISDIR(mode), "App comparison refuses special files.")
        digest = None
        if stat.S_ISREG(mode):
            with path.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
        result[path.relative_to(app).as_posix()] = (stat.S_IFMT(mode), stat.S_IMODE(mode), digest)
    return result


def validate_volume(root: Path, volume: Path) -> Path:
    config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
    require(not config["bundle"]["macOS"].get("dmg", {}).get("background"), "Custom DMG backgrounds require an explicitly reviewed verifier policy.")
    app_name = config.get("productName", "Buntini") + ".app"
    entries = {path.name: path for path in volume.iterdir()}
    require(app_name in entries and set(entries) <= {app_name, "Applications", ".DS_Store", ".VolumeIcon.icns"}, "Disk image contains missing or unexpected root entries.")
    if "Applications" in entries:
        link = entries["Applications"]
        require(link.is_symlink() and os.readlink(link) == "/Applications", "Disk image Applications must be a symlink exactly to /Applications.")
    for name in (".DS_Store", ".VolumeIcon.icns"):
        if name not in entries:
            continue
        path = entries[name]
        require(path.is_file() and not path.is_symlink() and path.stat().st_mode & 0o111 == 0 and path.stat().st_size <= 1024 * 1024, "Disk image decoration must be a bounded non-executable regular file.")
        if name == ".DS_Store":
            with path.open("rb") as source:
                require(source.read(8) == b"\0\0\0\1Bud1", "Invalid Finder metadata file.")
        else:
            icons = [root / "src-tauri" / item for item in config["bundle"]["icon"] if Path(item).suffix == ".icns"]
            require(len(icons) == 1 and icons[0].is_file() and not icons[0].is_symlink() and path.read_bytes() == icons[0].read_bytes(), "Disk image icon differs from the reviewed source icon.")
    return entries[app_name]


def verify_image(root: Path, app: Path, dmg: Path, team: str, runner=command, code_verifier=validate_code) -> None:
    require(dmg.is_file() and not dmg.is_symlink(), "Disk image must be a regular file.")
    validate_bundle(root, app)
    work = Path(tempfile.mkdtemp(prefix="buntini-dmg-verify-"))
    mount = work / "volume"
    mount.mkdir()
    try:
        runner(["/usr/bin/hdiutil", "attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", str(mount), str(dmg.resolve())])
        embedded = validate_volume(root, mount)
        executable, entitlements = validate_bundle(root, embedded)
        config = json.loads((root / "src-tauri/tauri.conf.json").read_text())
        code_verifier(embedded, executable, entitlements, team, config["identifier"], None)
        require(fingerprint(app) == fingerprint(embedded), "Disk image app differs from the supplied app, including its signatures, ticket, files or modes.")
    finally:
        # Attempt detach even if attach partially failed or validation raised.
        # Never recursively delete a directory that might still be a volume.
        try:
            runner(["/usr/bin/hdiutil", "detach", str(mount)])
        except BaseException as error:
            raise ValueError(f"Disk image detach could not be confirmed; inspect the read-only mount at {mount}.") from error
        mount.rmdir()
        work.rmdir()


def interrupted(signum, _frame):
    raise SystemExit(128 + signum)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--dmg", type=Path, required=True)
    parser.add_argument("--team", required=True)
    args = parser.parse_args()
    require(sys.platform == "darwin", "Disk image verification requires macOS.")
    # Convert termination signals to exceptions so the finally-detach runs.
    for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
        signal.signal(signum, interrupted)
    try:
        verify_image(Path(__file__).resolve().parent.parent, args.app, args.dmg, args.team)
    except (ValueError, KeyError, TypeError, OSError) as error:
        parser.exit(1, f"Release blocked: {error}\n")
    print("Read-only disk image payload matches the supplied verified app; mount detached.")


if __name__ == "__main__":
    main()
