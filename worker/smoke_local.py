#!/usr/bin/env python3
"""Opt-in real-model smoke test. Requires an existing local model; never downloads."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time


class Connection:
    def __init__(self):
        self.stderr = tempfile.TemporaryFile(mode="w+", encoding="utf-8")
        self.process = subprocess.Popen(
            [sys.executable, "-u", str(Path(__file__).with_name("main.py"))],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.stderr,
            text=True, encoding="utf-8",
        )
        self.queue = queue.Queue()
        self.pending = []
        threading.Thread(target=self.read, daemon=True).start()
        ready = self.queue.get(timeout=10)
        assert ready["type"] == "ready" and ready["backend"] == "mlx", ready

    def read(self):
        for line in self.process.stdout:
            self.queue.put(json.loads(line))

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def wait(self, identifier, event_type="done", timeout=180):
        events = []
        deadline = time.monotonic() + timeout
        while True:
            found = next((index for index, value in enumerate(self.pending) if value.get("id") == identifier), None)
            event = self.pending.pop(found) if found is not None else self.queue.get(timeout=max(0.01, deadline - time.monotonic()))
            if event.get("id") != identifier:
                self.pending.append(event)
                continue
            events.append(event)
            if event["type"] == "error":
                raise RuntimeError(f"{event.get('code')}: {event.get('message')}")
            if event["type"] == event_type:
                return events

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
        self.process.stdout.close()
        self.stderr.seek(0)
        diagnostics = self.stderr.read(3_000)
        self.stderr.close()
        if diagnostics:
            print("Worker diagnostics: " + diagnostics, file=sys.stderr)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", required=True, help="Existing absolute local MLX Qwen model folder")
    args = parser.parse_args()
    model_path = Path(args.model).expanduser()
    if not model_path.is_absolute() or not (model_path / "config.json").is_file():
        parser.error("--model must be an existing absolute local model folder")
    connection = Connection()

    def generate(identifier, messages, tools, max_tokens=128):
        start = time.monotonic()
        connection.send({"id": identifier, "type": "generate", "model_path": str(model_path), "messages": messages, "tools": tools, "max_tokens": max_tokens})
        events = connection.wait(identifier)
        elapsed = round(time.monotonic() - start, 3)
        print(json.dumps({"check": identifier, "seconds": elapsed, "completion": events[-1], "tools": [event for event in events if event["type"] == "tool_call"]}), flush=True)
        return events

    try:
        plain = generate("plain", [{"role": "user", "content": "Reply with exactly the word READY."}], [], max_tokens=64)
        assert "READY" in plain[-1]["text"], plain
        prompt = "Use system_info now to get my current CPU and memory usage. Do not guess; request the tool."
        tool_events = generate("tool", [{"role": "user", "content": prompt}], [{"name": "system_info"}], max_tokens=256)
        tool = next(event for event in tool_events if event["type"] == "tool_call")
        assert tool["name"] == "system_info" and tool["arguments"] == {}, tool
        followup = generate("followup", [
            {"role": "user", "content": prompt},
            {"role": "assistant", "content": "", "tool_calls": [{"type": "function", "function": {"name": "system_info", "arguments": {}}}]},
            {"role": "tool", "content": '{"cpu_percent":12.5,"memory_used_gb":4,"memory_total_gb":16,"note":"Synthetic tool result supplied only for this inference smoke test"}'},
        ], [], max_tokens=128)
        assert "12.5" in followup[-1]["text"], followup
        connection.send({"id": "long", "type": "generate", "model_path": str(model_path), "messages": [{"role": "user", "content": "Write a long story about a friendly fox learning to sail. Use at least 1000 words."}], "tools": [], "max_tokens": 1024})
        connection.wait("long", "token")
        start = time.monotonic()
        connection.send({"id": "health", "type": "ping"})
        connection.wait("health", "pong", timeout=5)
        connection.send({"id": "cancel", "type": "cancel", "target_id": "long"})
        ack = connection.wait("cancel", timeout=5)[-1]
        stopped = connection.wait("long", timeout=10)[-1]
        assert ack["cancelled"] and stopped["finish_reason"] == "cancelled", (ack, stopped)
        print(json.dumps({"check": "cancel", "seconds": round(time.monotonic() - start, 3), "finish_reason": stopped["finish_reason"]}), flush=True)
        reused = generate("reuse", [{"role": "user", "content": "Reply with exactly the word RECOVERED."}], [], max_tokens=64)
        assert "RECOVERED" in reused[-1]["text"], reused
        print("Real local-model smoke checks passed. No tools were executed; the tool result was synthetic.", flush=True)
    finally:
        connection.close()


if __name__ == "__main__":
    main()
