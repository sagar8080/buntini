from __future__ import annotations

import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import wave

from worker.main import MLXBackend
from worker.voice import WHISPER_DIMENSIONS
from worker.protocol import (
    Cancelled, MAX_LINE_BYTES, MAX_OUTPUT_CHARS, QwenStreamParser, WorkerError,
    adapt_messages, local_model_path, parse_gemma_call, parse_tool_payload, selected_tools, validate_messages, validate_tool_call,
)

ROOT = Path(__file__).resolve().parents[2]


def write_whisper_model(path):
    (path / "config.json").write_text(json.dumps({key: 1 for key in WHISPER_DIMENSIONS}))
    (path / "weights.safetensors").write_bytes(b"test-only-weight-placeholder")


class ProtocolTests(unittest.TestCase):
    def test_tool_schema_rejects_unavailable_and_unknown_fields(self):
        invalid = [
            {"name": "run_shell", "arguments": {}},
            {"name": "system_info", "arguments": {"command": "ls"}},
            {"name": "search_files", "arguments": {"query": "tax", "root": "/"}},
            {"name": "search_files", "arguments": {"query": " "}},
            {"name": "web_search", "arguments": {"query": "x" * 501}},
            {"name": "system_info", "arguments": {}, "approved": True},
        ]
        for call in invalid:
            with self.subTest(call=call), self.assertRaises(WorkerError):
                validate_tool_call(call, {"search_files", "system_info", "web_search"})
        with self.assertRaises(WorkerError):
            validate_tool_call({"name": "web_search", "arguments": {"query": "news"}}, {"system_info"})

    def test_integration_tools_follow_their_schemas(self):
        allowed = {"weather", "currency", "places", "flight_search", "ebay_search"}
        self.assertEqual(validate_tool_call({"name": "weather", "arguments": {"place": " Lisbon ", "units": "Fahrenheit"}}, allowed), ("weather", {"place": "Lisbon", "units": "fahrenheit"}))
        self.assertEqual(validate_tool_call({"name": "places", "arguments": {"category": "Charging Station", "near": "Baixa"}}, allowed)[1]["category"], "charging_station")
        self.assertEqual(validate_tool_call({"name": "ebay_search", "arguments": {"query": "kindle", "max_price": "1,200"}}, allowed)[1]["max_price"], 1200.0)
        invalid = [
            {"name": "weather", "arguments": {"units": "celsius"}},
            {"name": "weather", "arguments": {"place": "Lisbon", "days": 3}},
            {"name": "places", "arguments": {"category": "cafe", "near": "Lisbon", "radius_m": "2.5"}},
            {"name": "places", "arguments": {"category": "cafe", "near": "Lisbon", "radius_m": True}},
            {"name": "places", "arguments": {"category": "casino", "near": "Lisbon"}},
            {"name": "currency", "arguments": {"from": "USD", "to": "EUR", "amount": "lots"}},
            {"name": "flight_search", "arguments": {"origin": "LIS", "destination": "JFK", "departure_date": "2026-11-10", "book": True}},
        ]
        for call in invalid:
            with self.subTest(call=call), self.assertRaises(WorkerError):
                validate_tool_call(call, allowed)
        with self.assertRaises(WorkerError):
            validate_tool_call({"name": "wikipedia", "arguments": {"query": "Turing"}}, allowed)

    def test_app_tools_follow_their_schemas(self):
        allowed = {"read_mail", "control_music", "read_reminders"}
        self.assertEqual(validate_tool_call({"name": "read_mail", "arguments": {"unread": "true", "limit": "3"}}, allowed), ("read_mail", {"unread": True, "limit": 3}))
        self.assertEqual(validate_tool_call({"name": "control_music", "arguments": {"app": "Spotify", "action": "Now Playing"}}, allowed)[1], {"app": "spotify", "action": "now_playing"})
        self.assertEqual(validate_tool_call({"name": "read_reminders", "arguments": {}}, allowed), ("read_reminders", {}))
        for call in [{"name": "read_mail", "arguments": {"unread": "maybe"}}, {"name": "control_music", "arguments": {"action": "shuffle"}}, {"name": "read_mail", "arguments": {"folder": "Sent"}}]:
            with self.subTest(call=call), self.assertRaises(WorkerError):
                validate_tool_call(call, allowed)

    def test_native_tags_carry_integration_numbers_as_text(self):
        payload = "<function=places>\n<parameter=category>\ncafe\n</parameter>\n<parameter=near>\nPorto\n</parameter>\n<parameter=radius_m>\n800\n</parameter>\n</function>"
        self.assertEqual(parse_tool_payload(payload, {"places"}), ("places", {"category": "cafe", "near": "Porto", "radius_m": 800}))

    def test_schema_normalizes_query(self):
        self.assertEqual(validate_tool_call({"name": "search_files", "arguments": {"query": "  invoice  "}}, {"search_files"}), ("search_files", {"query": "invoice"}))

    def test_caller_cannot_weaken_built_in_tool_schema(self):
        tools = selected_tools([{"name": "system_info", "parameters": {"additionalProperties": True}}])
        self.assertIs(tools[0]["function"]["parameters"]["additionalProperties"], False)
        self.assertEqual(selected_tools([]), [])
        with self.assertRaises(WorkerError):
            selected_tools([{"name": "run_shell"}])

    def test_parser_handles_every_tag_boundary(self):
        raw = 'Hello <think>private reasoning</think>world.<tool_call>{"name":"system_info","arguments":{}}</tool_call>'
        for size in range(1, 20):
            parser = QwenStreamParser({"system_info"})
            chunks = [raw[index : index + size] for index in range(0, len(raw), size)]
            text = "".join(part for chunk in chunks for part in parser.feed(chunk))
            text += "".join(parser.feed("", final=True))
            self.assertEqual(text, "Hello world.")
            self.assertEqual(parser.call, ("system_info", {}))

    def test_parser_rejects_malformed_and_incomplete_tools(self):
        for raw in ["<tool_call>{bad}</tool_call>", '<tool_call>{"name":"system_info"}', '<tool_call>{"name":"system_info","arguments":{"shell":"id"}}</tool_call>']:
            with self.subTest(raw=raw), self.assertRaises(WorkerError):
                parser = QwenStreamParser({"system_info"})
                parser.feed(raw)
                parser.feed("", final=True)

    def test_qwen35_native_tool_format_handles_chunk_boundaries(self):
        cases = [
            ("<tool_call>\n<function=system_info>\n</function>\n</tool_call>", ("system_info", {})),
            ("<tool_call>\n<function=search_files>\n<parameter=query>\ninvoice 2026\n</parameter>\n</function>\n</tool_call>", ("search_files", {"query": "invoice 2026"})),
        ]
        for raw, expected in cases:
            for size in range(1, 15):
                parser = QwenStreamParser({"system_info", "search_files"})
                text = "".join(part for index in range(0, len(raw), size) for part in parser.feed(raw[index : index + size]))
                self.assertEqual(text, "")
                self.assertEqual(parser.call, expected)

    def test_gemma4_native_calls_stream_and_parse(self):
        raw = 'Sure.<|channel>thought\nlooking<channel|><|tool_call>call:add_reminder{due:<|"|>2026-10-23 09:00<|"|>,title:<|"|>Call Ana, then mum<|"|>}<tool_call|>'
        for size in range(1, 18):
            parser = QwenStreamParser({"add_reminder"})
            text = "".join(part for index in range(0, len(raw), size) for part in parser.feed(raw[index : index + size]))
            text += "".join(parser.feed("", final=True))
            self.assertEqual(text, "Sure.")
            self.assertEqual(parser.call, ("add_reminder", {"title": "Call Ana, then mum", "due": "2026-10-23 09:00"}))
        self.assertEqual(parse_gemma_call("call:system_info{}", {"system_info"}), ("system_info", {}))
        self.assertEqual(parse_gemma_call("call:add_calendar_event{minutes:45,start:<|\"|>2026-10-24 13:00<|\"|>,title:<|\"|>Lunch<|\"|>}", {"add_calendar_event"})[1]["minutes"], 45)
        for bad in ["call:run_shell{}", "call:search_files{query:<|\"|>x}", "call:search_files{query:<|\"|>x<|\"|>,query:<|\"|>y<|\"|>}", "search_files{}"]:
            with self.subTest(bad=bad), self.assertRaises(WorkerError):
                parse_gemma_call(bad, {"search_files", "system_info"})

    def test_messages_fit_templates_with_and_without_tools(self):
        conversation = [
            {"role": "system", "content": "Be brief."},
            {"role": "user", "content": "find my lease"},
            {"role": "assistant", "content": "", "tool_calls": [{"type": "function", "function": {"name": "search_files", "arguments": {"query": "lease"}}}]},
            {"role": "tool", "content": "lease.pdf"},
        ]
        tools = selected_tools([{"name": "search_files"}])
        native = adapt_messages(conversation, tools, True)
        self.assertEqual(native[3], {"role": "tool", "content": "lease.pdf", "name": "search_files"})
        plain = adapt_messages(conversation, tools, False)
        self.assertEqual([item["role"] for item in plain], ["system", "user", "assistant", "user"])
        self.assertIn('<tool_call>{"name": "search_files", "arguments": {"query": "lease"}}</tool_call>', plain[2]["content"])
        self.assertTrue(plain[3]["content"].startswith("Result from search_files:"))
        self.assertIn("You can use these tools", plain[0]["content"])
        self.assertNotIn("tool_calls", plain[2])

    def test_qwen35_tool_grammar_and_schema_are_strict(self):
        invalid = [
            "<function=run_shell></function>",
            "<function=system_info><parameter=command>ls</parameter></function>",
            "<function=search_files><parameter=query>x</parameter><parameter=query>y</parameter></function>",
            "<function=search_files><parameter=query>x</parameter>trailing</function>",
            "<function=search_files><parameter=query>x</parameter></function><function=system_info></function>",
            '<!DOCTYPE x SYSTEM "file:///etc/passwd"><function=system_info></function>',
        ]
        for payload in invalid:
            with self.subTest(payload=payload), self.assertRaises(WorkerError):
                QwenStreamParser({"system_info", "search_files"}).feed("<tool_call>" + payload + "</tool_call>")

    def test_parser_bounds_answer_and_thinking(self):
        with self.assertRaises(WorkerError):
            QwenStreamParser(set()).feed("a" * (MAX_OUTPUT_CHARS + 1))
        parser = QwenStreamParser(set())
        self.assertEqual(parser.feed("<think>" + "secret " * 3_000), [])
        self.assertLess(len(parser.buffer), 10)

    def test_plain_angle_brackets_are_preserved(self):
        parser = QwenStreamParser(set())
        self.assertEqual("".join(parser.feed("Use x < y, and <xml>.") + parser.feed("", final=True)), "Use x < y, and <xml>.")

    def test_message_limits_and_roles(self):
        for value in [[], [{"role": "developer", "content": "x"}], [{"role": "user", "content": []}], [{"role": "user", "content": "x"}] * 41, [{"role": "user", "content": "x" * 120_001}]]:
            with self.subTest(value=str(value)[:100]), self.assertRaises(WorkerError):
                validate_messages(value)

    def test_assistant_tool_history_is_validated(self):
        history = [{"role": "assistant", "content": "", "tool_calls": [{"type": "function", "function": {"name": "system_info", "arguments": "{}"}}]}]
        self.assertEqual(validate_messages(history)[0]["tool_calls"][0]["function"]["arguments"], {})

    def test_local_models_require_existing_absolute_paths(self):
        for value in [None, "mlx-community/Qwen3", "/a/model/that/does/not/exist"]:
            with self.subTest(value=value), self.assertRaises(WorkerError):
                local_model_path(value, None, "Qwen")
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "config.json").write_text("{}")
            self.assertEqual(local_model_path(directory, None, "Qwen"), Path(directory).resolve())


class AdapterTests(unittest.TestCase):
    def test_mlx_uses_local_load_template_and_stream_api(self):
        calls = {}

        def load(path, **kwargs):
            calls["load"] = (path, kwargs)
            return object(), SimpleNamespace(apply_chat_template=template)

        def template(messages, **kwargs):
            calls["template"] = (messages, kwargs)
            return [1, 2, 3]

        def stream(model, tokenizer, prompt, **kwargs):
            calls["stream"] = kwargs
            kwargs["prompt_progress_callback"](1, 3)
            yield SimpleNamespace(text="Hello")

        modules = {
            "mlx_lm": SimpleNamespace(load=load, stream_generate=stream),
            "mlx_lm.sample_utils": SimpleNamespace(make_sampler=lambda **kwargs: kwargs),
        }
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "config.json").write_text("{}")
            request = {"messages": [{"role": "system", "content": "Your configured assistant name is Akari. Be warm and concise."}, {"role": "user", "content": "hello"}], "model_path": directory, "max_tokens": 10, "tools": [{"name": "system_info"}]}
            backend = MLXBackend()
            with patch("worker.main.importlib.import_module", side_effect=lambda name: modules[name]):
                self.assertEqual(list(backend.generate(request, threading.Event())), ["Hello"])
                self.assertEqual(list(backend.generate(request, threading.Event())), ["Hello"])
        self.assertIs(calls["load"][1]["trust_remote_code"], False)
        self.assertIs(calls["load"][1]["tokenizer_config"]["local_files_only"], True)
        self.assertIs(calls["template"][1]["enable_thinking"], False)
        self.assertIn("configured assistant name is Akari", calls["template"][0][0]["content"])
        self.assertNotIn("You are Buntini", calls["template"][0][0]["content"])
        self.assertEqual(calls["template"][1]["tools"][0]["function"]["name"], "system_info")
        self.assertEqual(calls["stream"]["prefill_step_size"], 256)

    def test_dependency_failure_is_recoverable(self):
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / "config.json").write_text("{}")
            with patch("worker.main.importlib.import_module", side_effect=ImportError("no Metal device")):
                with self.assertRaises(WorkerError) as error:
                    MLXBackend().load(Path(directory), threading.Event())
        self.assertEqual(error.exception.code, "dependency_unavailable")

    def test_cancellation_prevents_load(self):
        event = threading.Event()
        event.set()
        with patch("worker.main.importlib.import_module") as importer:
            with self.assertRaises(Cancelled):
                MLXBackend().load(Path("/unused"), event)
        importer.assert_not_called()

    def test_whisper_receives_a_local_waveform_and_local_model(self):
        calls = {}

        class Audio:
            def astype(self, dtype):
                calls["dtype"] = dtype
                return self

            def __truediv__(self, divisor):
                calls["divisor"] = divisor
                return self

        def transcribe(audio, **kwargs):
            self.assertIsInstance(audio, Audio)
            calls["whisper"] = kwargs
            return {"text": "  hello from the microphone  "}

        modules = {
            "numpy": SimpleNamespace(frombuffer=lambda frames, **kwargs: Audio(), float32="float32"),
            "mlx_whisper": SimpleNamespace(transcribe=transcribe),
        }
        with tempfile.TemporaryDirectory() as directory:
            model_path = Path(directory)
            write_whisper_model(model_path)
            audio_path = model_path / "recording.wav"
            with wave.open(str(audio_path), "wb") as wav:
                wav.setparams((1, 2, 16_000, 0, "NONE", "not compressed"))
                wav.writeframes(b"\0\0" * 1_600)
            with patch("worker.main.importlib.import_module", side_effect=lambda name: modules[name]):
                text = MLXBackend().transcribe({"model_path": str(model_path), "audio_path": str(audio_path)}, threading.Event())
            self.assertEqual(calls["whisper"]["path_or_hf_repo"], str(model_path.resolve()))
            self.assertIsNone(calls["whisper"]["verbose"])
            self.assertEqual(calls["divisor"], 32768.0)
            self.assertEqual(text, "hello from the microphone")

    def test_whisper_rejects_wrong_format_before_importing_dependencies(self):
        with tempfile.TemporaryDirectory() as directory:
            model_path = Path(directory)
            write_whisper_model(model_path)
            audio_path = model_path / "recording.wav"
            with wave.open(str(audio_path), "wb") as wav:
                wav.setparams((2, 2, 48_000, 0, "NONE", "not compressed"))
                wav.writeframes(b"\0\0" * 1_600)
            with patch("worker.main.importlib.import_module") as importer:
                with self.assertRaises(WorkerError) as error:
                    MLXBackend().transcribe({"model_path": str(model_path), "audio_path": str(audio_path)}, threading.Event())
            self.assertEqual(error.exception.code, "invalid_audio")
            importer.assert_not_called()


class WorkerProcess:
    def __init__(self, fixture=True):
        env = dict(os.environ)
        for name in ("BUNTINI_MODEL_PATH", "BUNTINI_WHISPER_MODEL_PATH", "BUNTINI_WORKER_FIXTURE"):
            env.pop(name, None)
        self.process = subprocess.Popen(
            [sys.executable, "-u", str(ROOT / "worker" / "main.py"), *( ["--fixture"] if fixture else [])],
            cwd=ROOT, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", env=env,
        )
        self.events = queue.Queue()
        self.pending = []
        self.reader = threading.Thread(target=self._read, daemon=True)
        self.reader.start()
        ready = self.events.get(timeout=3)
        assert ready["type"] == "ready", ready
        self.ready = ready

    def _read(self):
        for line in self.process.stdout:
            self.events.put(json.loads(line))

    def send(self, value):
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()

    def until(self, identifier, event_type="done", timeout=3):
        result = []
        deadline = time.monotonic() + timeout
        while True:
            matched = next((index for index, value in enumerate(self.pending) if value.get("id") == identifier), None)
            event = self.pending.pop(matched) if matched is not None else self.events.get(timeout=max(0.01, deadline - time.monotonic()))
            if event.get("id") != identifier:
                self.pending.append(event)
                continue
            result.append(event)
            if event["type"] == event_type:
                return result

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=3)
        self.process.stdout.close()
        self.process.stderr.close()


class IntegrationTests(unittest.TestCase):
    def setUp(self):
        self.worker = WorkerProcess()
        self.addCleanup(self.worker.close)

    def generate(self, identifier, text, **extra):
        self.worker.send({"id": identifier, "type": "generate", "messages": [{"role": "user", "content": text}], **extra})
        return self.worker.until(identifier)

    def test_fixture_mode_is_explicit_and_streams(self):
        self.assertEqual(self.worker.ready["backend"], "fixture")
        events = self.generate("hello", "hello")
        self.assertGreater(len([event for event in events if event["type"] == "token"]), 1)
        self.assertIn("Fixture mode", events[-1]["text"])
        self.assertEqual(events[-1]["finish_reason"], "stop")

    def test_tool_call_and_tool_result_round_trip(self):
        events = self.generate("tool", "fixture:system")
        call = next(event for event in events if event["type"] == "tool_call")
        self.assertEqual((call["name"], call["arguments"]), ("system_info", {}))
        self.assertEqual(events[-1]["finish_reason"], "tool_call")
        self.worker.send({"id": "result", "type": "generate", "messages": [{"role": "tool", "content": '{"platform":"macOS"}'}]})
        self.assertIn("macOS", self.worker.until("result")[-1]["text"])

    def test_invalid_model_tool_call_errors_and_worker_recovers(self):
        events = self.generate("bad", 'fixture:tool {"name":"run_shell","arguments":{}}')
        self.assertEqual(events[-2]["code"], "invalid_tool_call")
        self.assertFalse(any(event["type"] == "tool_call" for event in events))
        self.assertEqual(self.generate("good", "hello")[-1]["finish_reason"], "stop")

    def test_unavailable_tools_are_never_emitted(self):
        events = self.generate("narrow", 'fixture:tool {"name":"web_search","arguments":{"query":"news"}}', tools=[{"name": "system_info"}])
        self.assertEqual(events[-2]["code"], "invalid_tool_call")

    def test_ping_and_cancel_remain_responsive_during_generation(self):
        self.worker.send({"id": "slow", "type": "generate", "messages": [{"role": "user", "content": "fixture:slow"}]})
        self.worker.until("slow", "token")
        start = time.monotonic()
        self.worker.send({"id": "ping", "type": "ping"})
        self.assertEqual(self.worker.until("ping", "pong")[-1]["backend"], "fixture")
        self.worker.send({"id": "cancel", "type": "cancel", "target_id": "slow"})
        self.assertTrue(self.worker.until("cancel")[-1]["cancelled"])
        events = self.worker.until("slow")
        self.assertEqual(events[-1]["finish_reason"], "cancelled")
        self.assertLess(time.monotonic() - start, 1)
        self.assertEqual(self.generate("next", "hello")[-1]["finish_reason"], "stop")

    def test_invalid_json_and_oversized_lines_do_not_kill_worker(self):
        for raw in ["{invalid}\n", "x" * (MAX_LINE_BYTES + 1) + "\n"]:
            self.worker.process.stdin.write(raw)
            self.worker.process.stdin.flush()
            events = self.worker.until(None)
            self.assertEqual(events[-1]["finish_reason"], "error")
        self.worker.send({"id": "ping", "type": "ping"})
        self.assertEqual(self.worker.until("ping", "pong")[-1]["type"], "pong")

    def test_nonstring_command_does_not_kill_reader(self):
        for command in [[], {}, None, 42]:
            self.worker.send({"id": "bad-type", "type": command})
            self.assertEqual(self.worker.until("bad-type")[-2]["code"], "invalid_request")
        self.worker.send({"id": "ping", "type": "ping"})
        self.assertEqual(self.worker.until("ping", "pong")[-1]["type"], "pong")

    def test_escaped_surrogates_do_not_poison_transport_or_reader(self):
        for value in [
            {"id": "\ud800", "type": "ping"},
            {"id": "bad", "type": "generate", "messages": [{"role": "user", "content": "\udfff"}]},
        ]:
            self.worker.send(value)
            self.assertEqual(self.worker.until(None)[-2]["code"], "invalid_json")
        self.worker.send({"id": "ping", "type": "ping"})
        self.assertEqual(self.worker.until("ping", "pong")[-1]["type"], "pong")

    def test_busy_is_recoverable_and_does_not_cancel_running_request(self):
        self.worker.send({"id": "slow", "type": "generate", "messages": [{"role": "user", "content": "fixture:slow"}]})
        self.worker.until("slow", "token")
        events = self.generate("busy", "hello")
        self.assertEqual(events[-2]["code"], "busy")
        self.worker.send({"id": "cancel", "type": "cancel", "target_id": "slow"})
        self.worker.until("cancel")
        self.assertEqual(self.worker.until("slow")[-1]["finish_reason"], "cancelled")

    def test_max_tokens_is_bounded(self):
        for count in [0, 4097, True, "1024"]:
            self.assertEqual(self.generate("limit", "hello", max_tokens=count)[-2]["code"], "invalid_request")

    def test_production_mode_never_falls_back_to_fixture_or_download(self):
        worker = WorkerProcess(fixture=False)
        self.addCleanup(worker.close)
        self.assertEqual(worker.ready["backend"], "mlx")
        worker.send({"id": "missing", "type": "generate", "messages": [{"role": "user", "content": "hello"}]})
        events = worker.until("missing")
        self.assertEqual(events[-2]["code"], "model_not_configured")
        self.assertEqual(events[-1]["finish_reason"], "error")
        worker.send({"id": "ping", "type": "ping"})
        self.assertEqual(worker.until("ping", "pong")[-1]["type"], "pong")

    def test_fixture_transcription_is_labelled(self):
        self.worker.send({"id": "voice", "type": "transcribe", "audio_path": "/unused.wav"})
        self.assertIn("Fixture mode", self.worker.until("voice")[-1]["text"])


if __name__ == "__main__":
    unittest.main()
