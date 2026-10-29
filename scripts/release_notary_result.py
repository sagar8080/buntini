#!/usr/bin/env python3
"""Require an accepted, identifiable notarytool submission without logging its payload."""
import argparse
import json
from pathlib import Path
import uuid


def validate_result(result: dict) -> str:
    if not isinstance(result, dict) or result.get("status") != "Accepted":
        raise ValueError("Apple has not accepted this notarization submission; retain the result and inspect its notary log.")
    try:
        return str(uuid.UUID(result["id"]))
    except (KeyError, ValueError, TypeError, AttributeError) as error:
        raise ValueError("Accepted notarization result must include a valid submission identifier.") from error


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("result", type=Path)
    args = parser.parse_args()
    try:
        submission = validate_result(json.loads(args.result.read_text()))
    except (ValueError, OSError) as error:
        parser.exit(1, f"Release blocked: {error}\n")
    print(f"Notarization accepted: {submission}")


if __name__ == "__main__":
    main()
