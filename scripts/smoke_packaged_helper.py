#!/usr/bin/env python3
"""Read-only Apple helper smoke against an explicit installed app bundle.

Proposes (but never executes) a system_info tool, then checks text streaming.
No microphone, personal apps, model downloads, cloud providers or Keychain calls.
"""

import argparse
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import threading
import time


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(bundle):
    helper = bundle / "Contents/MacOS/buntini-fm"
    catalog = bundle / "Contents/Resources/worker/tools.json"
    report = {
        "kind": "packaged_apple_helper_smoke",
        "bundle": str(bundle),
        "helper_sha256": sha256(helper),
        "tool_catalog_sha256": sha256(catalog),
        "scope": "Local Apple model only; proposed tool is never executed; no personal apps, microphone or Keychain access.",
    }
    contracts = subprocess.run(
        [str(helper), "--self-test-contracts"], capture_output=True, text=True, timeout=30
    )
    report["contracts"] = {"exit_code": contracts.returncode, "stdout": contracts.stdout.strip()}
    if contracts.returncode:
        raise RuntimeError("Packaged helper contract self-test failed")
    events = queue.Queue()
    process = subprocess.Popen(
        [str(helper), "--tools", str(catalog)],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
    )

    def read_stdout():
        try:
            while line := process.stdout.readline(262_145):
                if len(line.encode()) > 262_144 or not line.endswith("\n"):
                    events.put(RuntimeError("Oversized or incomplete helper protocol line"))
                    return
                events.put(json.loads(line))
        except Exception as error:
            events.put(error)
        finally:
            events.put(RuntimeError("Helper stdout closed"))

    threading.Thread(target=read_stdout, daemon=True).start()

    def next_event(deadline):
        event = events.get(timeout=max(0.01, deadline - time.monotonic()))
        if isinstance(event, Exception):
            raise event
        return event

    def request(payload):
        process.stdin.write(json.dumps(payload) + "\n")
        process.stdin.flush()
        deadline = time.monotonic() + 90
        received = []
        while True:
            event = next_event(deadline)
            if event.get("id") != payload["id"]:
                raise RuntimeError("Helper returned an unexpected request id")
            received.append(event)
            if event["type"] == "error":
                raise RuntimeError(event.get("message", "Helper returned an error"))
            if event["type"] in {"done", "pong"}:
                return received

    try:
        ready = next_event(time.monotonic() + 30)
        if ready.get("type") != "ready":
            raise RuntimeError("Helper did not send ready")
        report["ready"] = ready
        ping = request({"id": "smoke-ping", "type": "ping"})[-1]
        report["ping"] = ping
        if ping.get("available") is not True:
            raise RuntimeError("Apple Intelligence unavailable: " + str(ping.get("reason")))
        tools = json.loads(catalog.read_text())

        def payload(identifier, prompt, names):
            return {
                "id": identifier,
                "type": "generate",
                "messages": [
                    {"role": "system", "content": "You are a concise desktop companion."},
                    {"role": "user", "content": prompt},
                ],
                "tools": [
                    {key: tools[name][key] for key in ("name", "description", "parameters")}
                    for name in names
                ],
                "max_tokens": 120,
            }

        routed = request(payload("smoke-route", "how much memory is my mac using right now?", ["search_files", "system_info"]))
        calls = [event for event in routed if event["type"] == "tool_call"]
        if len(calls) != 1 or calls[0].get("name") != "system_info":
            raise RuntimeError("The packaged Apple helper did not propose system_info")
        report["routing"] = {"passed": True, "tool": calls[0], "executed": False}
        streamed = request(payload("smoke-stream", "Say hello in five words.", []))
        tokens = "".join(event.get("text", "") for event in streamed if event["type"] == "token")
        done = streamed[-1]
        if not tokens or done.get("text") != tokens or any(event["type"] == "tool_call" for event in streamed):
            raise RuntimeError("Packaged helper stream did not match its final answer")
        report["streaming"] = {"passed": True, "text": tokens, "token_events": sum(event["type"] == "token" for event in streamed)}
        report["passed"] = True
        return report
    finally:
        process.stdin.close()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("bundle", type=Path)
    args = parser.parse_args()
    try:
        result = run(args.bundle.resolve(strict=True))
    except Exception as error:
        print(json.dumps({"kind": "packaged_apple_helper_smoke", "passed": False, "error": str(error)}, indent=2))
        raise SystemExit(1)
    print(json.dumps(result, indent=2))
