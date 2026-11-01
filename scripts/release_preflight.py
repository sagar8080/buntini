#!/usr/bin/env python3
"""Fail closed before a tagged build can use release credentials."""
from __future__ import annotations

import argparse
import base64
import binascii
import json
import os
from pathlib import Path
import re
import tomllib
from typing import Mapping


def validate_versions(root: Path, tag: str) -> None:
    if not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?", tag):
        raise ValueError("Release tag must be vMAJOR.MINOR.PATCH with an optional prerelease suffix.")
    version = tag[1:]
    cargo = tomllib.loads((root / "src-tauri/Cargo.toml").read_text())["package"]
    lock = tomllib.loads((root / "src-tauri/Cargo.lock").read_text())
    locked_root = [package for package in lock["package"] if package["name"] == cargo["name"] and "source" not in package]
    if len(locked_root) != 1:
        raise ValueError("Cargo.lock must contain exactly one local app package.")
    manifests = {
        "package.json": json.loads((root / "package.json").read_text())["version"],
        "package-lock.json": json.loads((root / "package-lock.json").read_text())["version"],
        "package-lock.json root package": json.loads((root / "package-lock.json").read_text())["packages"][""]["version"],
        "src-tauri/tauri.conf.json": json.loads((root / "src-tauri/tauri.conf.json").read_text())["version"],
        "src-tauri/Cargo.toml": cargo["version"],
        "src-tauri/Cargo.lock app package": locked_root[0]["version"],
    }
    mismatch = [name for name, actual in manifests.items() if actual != version]
    if mismatch:
        raise ValueError(f"Tag version does not match: {', '.join(mismatch)}. Update all manifests before tagging.")


def validate_identity(identity: str, team: str) -> None:
    if not re.fullmatch(r"[A-Z0-9]{10}", team):
        raise ValueError("Apple team must be a ten-character identifier.")
    if any(character in identity for character in ("\r", "\n", "\0")):
        raise ValueError("Signing identity must be a single line.")
    if not identity.startswith("Developer ID Application: ") or not identity.endswith(f" ({team})"):
        raise ValueError("Signing identity must be Developer ID Application for the expected Apple team.")


def validate_signing(env: Mapping[str, str]) -> None:
    required = (
        "APPLE_CERTIFICATE", "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY",
        "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID",
    )
    missing = [name for name in required if not env.get(name, "").strip()]
    if missing:
        # Never include secret values in logs, including malformed values.
        raise ValueError(f"Missing release secrets: {', '.join(missing)}.")
    if any(any(character in env[name] for character in ("\r", "\n", "\0")) for name in required):
        raise ValueError("Release secrets must not contain line breaks or NUL characters; encode the certificate as one base64 line.")
    try:
        if not base64.b64decode(env["APPLE_CERTIFICATE"], validate=True):
            raise ValueError("APPLE_CERTIFICATE must contain a nonempty base64-encoded PKCS#12 certificate.")
    except (binascii.Error, UnicodeError) as error:
        raise ValueError("APPLE_CERTIFICATE is not valid single-line base64.") from error
    validate_identity(env["APPLE_SIGNING_IDENTITY"], env["APPLE_TEAM_ID"])


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    signing = parser.add_mutually_exclusive_group()
    signing.add_argument("--require-signing", action="store_true", help="Validate CI certificate and notarization environment secrets.")
    signing.add_argument("--local-identity", help="Validate a local Keychain Developer ID identity without reading or exporting credentials.")
    parser.add_argument("--team", help="Expected team for --local-identity.")
    args = parser.parse_args()
    try:
        validate_versions(Path(__file__).resolve().parent.parent, args.tag)
        if args.require_signing:
            validate_signing(os.environ)
        if args.local_identity:
            if not args.team:
                raise ValueError("--local-identity requires --team.")
            validate_identity(args.local_identity, args.team)
    except (ValueError, KeyError, OSError) as error:
        parser.exit(1, f"Release blocked: {error}\n")
    print("Release preflight passed.")


if __name__ == "__main__":
    main()
