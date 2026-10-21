"""Bounded JSONL contracts and incremental Qwen/Hermes tool parsing."""

from __future__ import annotations

import json
from pathlib import Path
import re
from typing import Any

PROTOCOL_VERSION = 1
MAX_LINE_BYTES = 262_144
MAX_MESSAGES = 40
MAX_MESSAGE_CHARS = 120_000
MAX_OUTPUT_CHARS = 32_000
MAX_RAW_CHARS = 128_000
MAX_PROMPT_TOKENS = 16_384
MAX_GENERATION_TOKENS = 4_096
MAX_QUERY_CHARS = 500

TOOL_DEFINITIONS = {
    "search_files": {
        "name": "search_files",
        "description": "Search file names and indexed content within folders authorized by the user.",
        "parameters": {
            "type": "object", "properties": {"query": {"type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS}},
            "required": ["query"], "additionalProperties": False,
        },
    },
    "system_info": {
        "name": "system_info",
        "description": "Read a basic snapshot of this Mac's system and storage information.",
        "parameters": {"type": "object", "properties": {}, "additionalProperties": False},
    },
    "web_search": {
        "name": "web_search",
        "description": "Search the live web. The controller requires the user's explicit online search intent.",
        "parameters": {
            "type": "object", "properties": {"query": {"type": "string", "minLength": 1, "maxLength": MAX_QUERY_CHARS}},
            "required": ["query"], "additionalProperties": False,
        },
    },
    # Online integrations. The Rust controller re-validates every argument and the
    # user approves the exact request before anything leaves the Mac.
    "weather": {
        "name": "weather",
        "description": "Current conditions and the 7-day forecast for a named place.",
        "parameters": {
            "type": "object",
            "properties": {
                "place": {"type": "string", "minLength": 1, "maxLength": 120, "description": "City or town, optionally with region or country"},
                "units": {"type": "string", "enum": ["celsius", "fahrenheit"]},
            },
            "required": ["place"], "additionalProperties": False,
        },
    },
    "currency": {
        "name": "currency",
        "description": "Convert money using European Central Bank reference rates.",
        "parameters": {
            "type": "object",
            "properties": {
                "amount": {"type": "number", "minimum": 0.000001, "maximum": 1e12},
                "from": {"type": "string", "minLength": 3, "maxLength": 3, "description": "ISO 4217 code, e.g. USD"},
                "to": {"type": "string", "minLength": 3, "maxLength": 40, "description": "One or more ISO codes separated by commas"},
            },
            "required": ["from", "to"], "additionalProperties": False,
        },
    },
    "wikipedia": {
        "name": "wikipedia",
        "description": "Look up an encyclopedia article summary about a person, place, thing or idea.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 200},
                "language": {"type": "string", "enum": ["en", "de", "fr", "es", "it", "pt", "nl", "sv", "pl", "ru", "uk", "ja", "zh", "ko", "ar", "hi", "tr", "id"]},
            },
            "required": ["query"], "additionalProperties": False,
        },
    },
    "places": {
        "name": "places",
        "description": "Find nearby places of one kind around a street, district or city using OpenStreetMap.",
        "parameters": {
            "type": "object",
            "properties": {
                "category": {"type": "string", "enum": ["cafe", "restaurant", "bar", "pharmacy", "hospital", "atm", "bank", "supermarket", "bakery", "fuel", "charging_station", "parking", "hotel", "museum", "park", "library", "post_office", "gym", "toilets"]},
                "near": {"type": "string", "minLength": 1, "maxLength": 160},
                "radius_m": {"type": "integer", "minimum": 100, "maximum": 5000},
            },
            "required": ["category", "near"], "additionalProperties": False,
        },
    },
    "ebay_search": {
        "name": "ebay_search",
        "description": "Search live eBay listings to compare prices of an item.",
        "parameters": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "minLength": 1, "maxLength": 200},
                "max_price": {"type": "number", "minimum": 0.01, "maximum": 10000000},
                "condition": {"type": "string", "enum": ["any", "new", "used"]},
                "sort": {"type": "string", "enum": ["best_match", "price", "newest"]},
            },
            "required": ["query"], "additionalProperties": False,
        },
    },
    "flight_search": {
        "name": "flight_search",
        "description": "Search flight offers between two airports or cities on given dates. Search only, never booking.",
        "parameters": {
            "type": "object",
            "properties": {
                "origin": {"type": "string", "minLength": 2, "maxLength": 60, "description": "IATA code like LIS, or a city name"},
                "destination": {"type": "string", "minLength": 2, "maxLength": 60},
                "departure_date": {"type": "string", "minLength": 10, "maxLength": 10, "description": "YYYY-MM-DD"},
                "return_date": {"type": "string", "minLength": 10, "maxLength": 10, "description": "YYYY-MM-DD, only for return trips"},
                "adults": {"type": "integer", "minimum": 1, "maximum": 9},
                "cabin": {"type": "string", "enum": ["economy", "premium_economy", "business", "first"]},
            },
            "required": ["origin", "destination", "departure_date"], "additionalProperties": False,
        },
    },
}


class WorkerError(Exception):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


class Cancelled(Exception):
    pass


def request_id(request: dict[str, Any]) -> str:
    value = request.get("id")
    if not isinstance(value, str) or not value or len(value) > 128:
        raise WorkerError("invalid_request", "id must be a nonempty string of at most 128 characters")
    return value


def validate_messages(value: Any) -> list[dict[str, Any]]:
    if not isinstance(value, list) or not 1 <= len(value) <= MAX_MESSAGES:
        raise WorkerError("invalid_request", f"messages must contain 1 to {MAX_MESSAGES} messages")
    result = []
    for message in value:
        if not isinstance(message, dict) or not isinstance(message.get("role"), str) or message["role"] not in {"system", "user", "assistant", "tool"}:
            raise WorkerError("invalid_request", "Every message needs a supported role")
        if not isinstance(message.get("content"), str):
            raise WorkerError("invalid_request", "Message content must be a string")
        # Keep only template-relevant keys; never pass arbitrary model options.
        item = {"role": message["role"], "content": message["content"]}
        if "tool_calls" in message:
            calls = message["tool_calls"]
            if message["role"] != "assistant" or not isinstance(calls, list) or len(calls) != 1:
                raise WorkerError("invalid_request", "assistant tool_calls must contain exactly one call")
            call = calls[0]
            if not isinstance(call, dict) or not isinstance(call.get("function"), dict):
                raise WorkerError("invalid_request", "Invalid assistant tool call")
            fn = call["function"]
            args = fn.get("arguments")
            if isinstance(args, str):
                try:
                    args = json.loads(args)
                except (ValueError, RecursionError) as exc:
                    raise WorkerError("invalid_request", "Invalid assistant tool arguments") from exc
            name, args = validate_tool_call({"name": fn.get("name"), "arguments": args}, set(TOOL_DEFINITIONS))
            item["tool_calls"] = [{"type": "function", "function": {"name": name, "arguments": args}}]
        result.append(item)
    if len(json.dumps(result, ensure_ascii=False)) > MAX_MESSAGE_CHARS:
        raise WorkerError("context_too_large", f"Messages exceed {MAX_MESSAGE_CHARS} characters; start a new task")
    return result


def selected_tools(value: Any = None) -> list[dict[str, Any]]:
    if value is None:
        return [{"type": "function", "function": item} for item in TOOL_DEFINITIONS.values()]
    if not isinstance(value, list) or len(value) > len(TOOL_DEFINITIONS):
        raise WorkerError("invalid_request", "tools must be a list of the supported tools")
    names = []
    for item in value:
        if not isinstance(item, dict):
            raise WorkerError("invalid_request", "Each tool definition must be an object")
        definition = item.get("function", item)
        name = definition.get("name") if isinstance(definition, dict) else None
        if not isinstance(name, str) or name not in TOOL_DEFINITIONS or name in names:
            raise WorkerError("invalid_request", "Unknown or duplicate tool in tools")
        names.append(name)
    # The caller may narrow tool availability, but cannot weaken built-in schemas.
    return [{"type": "function", "function": TOOL_DEFINITIONS[name]} for name in names]


def validate_tool_call(value: Any, allowed: set[str]) -> tuple[str, dict[str, Any]]:
    if not isinstance(value, dict) or set(value) != {"name", "arguments"}:
        raise WorkerError("invalid_tool_call", "Tool calls must contain only name and arguments")
    name, arguments = value["name"], value["arguments"]
    if not isinstance(name, str) or name not in allowed or name not in TOOL_DEFINITIONS:
        raise WorkerError("invalid_tool_call", "The model requested an unavailable tool")
    if not isinstance(arguments, dict):
        raise WorkerError("invalid_tool_call", "Tool arguments must be an object")
    return name, validate_arguments(name, arguments)


def validate_arguments(name: str, arguments: dict[str, Any]) -> dict[str, Any]:
    """Check arguments against the fixed schema. Qwen's native tags carry every value
    as text, so numeric strings are converted; nothing else is coerced."""
    schema = TOOL_DEFINITIONS[name]["parameters"]
    properties, required = schema["properties"], set(schema.get("required", []))
    if not set(arguments) <= set(properties) or not required <= set(arguments):
        raise WorkerError("invalid_tool_call", f"{name} received missing or unexpected arguments")
    clean: dict[str, Any] = {}
    for key, value in arguments.items():
        spec = properties[key]
        if spec["type"] == "string":
            if not isinstance(value, str):
                raise WorkerError("invalid_tool_call", f"{name}.{key} must be text")
            value = value.strip()
            if not spec.get("minLength", 0) <= len(value) <= spec.get("maxLength", MAX_QUERY_CHARS):
                raise WorkerError("invalid_tool_call", f"{name}.{key} has an invalid length")
            if "enum" in spec:
                value = value.lower().replace(" ", "_").replace("-", "_")
                if value not in spec["enum"]:
                    raise WorkerError("invalid_tool_call", f"{name}.{key} must be one of {', '.join(spec['enum'])}")
        else:
            if isinstance(value, str):
                try:
                    value = float(value.strip().replace(",", ""))
                except ValueError as exc:
                    raise WorkerError("invalid_tool_call", f"{name}.{key} must be a number") from exc
            if isinstance(value, bool) or not isinstance(value, (int, float)) or value != value or value in (float("inf"), float("-inf")):
                raise WorkerError("invalid_tool_call", f"{name}.{key} must be a number")
            if spec["type"] == "integer":
                if value != int(value):
                    raise WorkerError("invalid_tool_call", f"{name}.{key} must be a whole number")
                value = int(value)
            if not spec["minimum"] <= value <= spec["maximum"]:
                raise WorkerError("invalid_tool_call", f"{name}.{key} is out of range")
        clean[key] = value
    return clean


def parse_tool_payload(payload: str, allowed: set[str]) -> tuple[str, dict[str, Any]]:
    """Support both Qwen 2/3 Hermes JSON and Qwen 3.5's native tag syntax.

    This is a deliberately narrow grammar, not a general XML parser: no entity
    resolution, attributes, nested functions, coercion, or executable content.
    All parsed arguments still pass the same fixed tool schema.
    """
    payload = payload.strip()
    if payload.startswith("{"):
        try:
            data = json.loads(payload)
        except (ValueError, RecursionError) as exc:
            raise WorkerError("invalid_tool_call", "The model produced malformed tool JSON") from exc
        return validate_tool_call(data, allowed)
    function = re.fullmatch(r"<function=([a-z][a-z0-9_]*)>\s*(.*?)\s*</function>", payload, flags=re.DOTALL)
    if function is None:
        raise WorkerError("invalid_tool_call", "The model produced malformed Qwen tool syntax")
    name, body = function.groups()
    arguments: dict[str, Any] = {}
    while body.strip():
        parameter = re.match(r"\s*<parameter=([a-z][a-z0-9_]*)>\s*(.*?)\s*</parameter>", body, flags=re.DOTALL)
        if parameter is None:
            raise WorkerError("invalid_tool_call", "The model produced malformed Qwen tool parameters")
        key, value = parameter.groups()
        if key in arguments:
            raise WorkerError("invalid_tool_call", "Duplicate tool parameters are not allowed")
        arguments[key] = value
        body = body[parameter.end() :]
    return validate_tool_call({"name": name, "arguments": arguments}, allowed)


def local_model_path(value: Any, environment_value: str | None, label: str) -> Path:
    raw = value if value is not None else environment_value
    if not isinstance(raw, str) or not raw.strip():
        raise WorkerError("model_not_configured", f"Configure a local {label} model folder in Settings")
    path = Path(raw).expanduser()
    if not path.is_absolute():
        raise WorkerError("model_not_found", f"The {label} model must be an absolute local folder path")
    if not path.is_dir() or not (path / "config.json").is_file():
        raise WorkerError("model_not_found", f"The local {label} model folder must exist and contain config.json")
    return path.resolve()


class QwenStreamParser:
    """Hide thinking/tool syntax, even when a tag is split across token chunks.

    Qwen Hermes JSON and Qwen 3.5 native function/parameter tags are supported.
    Only complete, schema-validated calls can become tool events.
    """

    def __init__(self, allowed: set[str]):
        self.allowed = allowed
        self.buffer = ""
        self.state = "text"
        self.raw_chars = 0
        self.text_chars = 0
        self.call: tuple[str, dict[str, Any]] | None = None

    def feed(self, chunk: str, final: bool = False) -> list[str]:
        self.raw_chars += len(chunk)
        if self.raw_chars > MAX_RAW_CHARS:
            raise WorkerError("output_too_large", "Model output exceeded the worker limit")
        self.buffer += chunk
        output = []
        while self.buffer:
            if self.state == "tool":
                end = self.buffer.find("</tool_call>")
                if end < 0:
                    break
                self.call = parse_tool_payload(self.buffer[:end], self.allowed)
                self.buffer = ""
                break
            if self.state == "think":
                end = self.buffer.find("</think>")
                if end < 0:
                    # Keep a possible partial closing tag, not the reasoning body.
                    self.buffer = self.buffer[-len("</think>") :]
                    break
                self.buffer = self.buffer[end + len("</think>") :]
                self.state = "text"
                continue
            tags = ("<think>", "<tool_call>", "</think>")
            found = [(self.buffer.find(tag), tag) for tag in tags if tag in self.buffer]
            if found:
                index, tag = min(found)
                self._add_text(self.buffer[:index], output)
                self.buffer = self.buffer[index + len(tag) :]
                self.state = "tool" if tag == "<tool_call>" else "think" if tag == "<think>" else "text"
                continue
            hold = 0
            if not final:
                for length in range(1, min(len(self.buffer), max(map(len, tags))) + 1):
                    if any(tag.startswith(self.buffer[-length:]) for tag in tags):
                        hold = length
            ready = self.buffer[:-hold] if hold else self.buffer
            self._add_text(ready, output)
            self.buffer = self.buffer[-hold:] if hold else ""
            break
        if final and self.state == "tool" and self.call is None:
            raise WorkerError("invalid_tool_call", "The model ended with an incomplete tool call")
        return output

    def _add_text(self, value: str, output: list[str]) -> None:
        if value:
            self.text_chars += len(value)
            if self.text_chars > MAX_OUTPUT_CHARS:
                raise WorkerError("output_too_large", "Model response exceeded the worker limit")
            output.append(value)
