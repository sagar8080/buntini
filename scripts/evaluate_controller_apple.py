#!/usr/bin/env python3
"""Exercise the real Apple backend behind production controller safeguards.

Every app observation and write receipt is synthetic. Explicit literal-field
journeys use deterministic binding, not inference. No personal apps are read or
written. This is a controller regression probe, not a live app acceptance rate.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import re
import subprocess

ROOT = Path(__file__).resolve().parents[1]
TESTS = [
    ("controller::planning_tests::live_apple_note_reminder_journey_preserves_fields_and_finishes_from_receipts", 2),
    ("controller::planning_tests::live_apple_planning_guards_handle_injected_source_denied_read_and_unknown_write", 3),
    ("controller::tests::live_apple_calendar_preparation_finishes_controlled_journeys", 3),
]


def hashes():
    paths = list((ROOT / "src-tauri/src").rglob("*.rs"))
    paths += [ROOT / name for name in ("worker/tools.json", "native/fm/main.swift", "native/buntini-fm", "src-tauri/Cargo.lock")]
    return {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in sorted(paths)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True, help="Fresh JSON report path; existing evidence is never overwritten")
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Choose a new report path; preserve prior runs, including failures.")
    started = datetime.now(timezone.utc).isoformat()
    before = hashes()
    runs = []
    for test, count in TESTS:
        command = ["cargo", "test", "--locked", "--offline", "--manifest-path", "src-tauri/Cargo.toml", "--target-dir", "src-tauri/target", test, "--", "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        try:
            result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=180)
            log, code = result.stdout, result.returncode
        except subprocess.TimeoutExpired as error:
            log = error.stdout or ""
            if isinstance(log, bytes):
                log = log.decode("utf-8", errors="replace")
            code = None
        samples = [json.loads(match.group(1)) for match in re.finditer(r"BUNTINI_CONTROLLED_\w+ (\{[^\n]+\})", log)]
        passed = code == 0 and len(samples) == count and all(row.get("passed") is True and row.get("real_app_writes") == 0 for row in samples)
        runs.append({"test": test, "passed": passed, "exit_code": code, "expected_samples": count, "samples": samples, "log": log[-100000:]})
        print(f"{'PASS' if passed else 'FAIL'} {test}: {len(samples)}/{count} controlled samples", flush=True)
    stable = before == hashes()
    report = {"evidence_type": "production_controller_with_apple_backend_and_synthetic_app_boundaries", "started_at_utc": started, "recorded_at_utc": datetime.now(timezone.utc).isoformat(), "macos": platform.mac_ver()[0], "architecture": platform.machine(), "model": "Apple Foundation Models system default; exact model build unavailable", "source_and_helper_sha256": before, "sources_unchanged_during_run": stable, "passed": stable and all(run["passed"] for run in runs), "samples": sum(len(run["samples"]) for run in runs), "real_app_writes": 0, "limitations": ["Synthetic app observations and executor receipts only", "Literal note/reminder cases bind user fields without model generation", "This does not measure raw model accuracy, live app writes, or general task completion"], "runs": runs}
    with args.output.open("x") as output:
        json.dump(report, output, indent=2)
        output.write("\n")
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
