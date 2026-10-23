"""Bounded JSONL contracts and incremental tool parsing for Qwen and Gemma models."""

from __future__ import annotations

import json
from copy import deepcopy
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

# One schema source shared with the Rust controller (src-tauri/src/tools.rs) and the
# Apple Intelligence helper; the caller may narrow tools but never weaken them.
TOOL_DEFINITIONS: dict[str, dict[str, Any]] = json.loads((Path(__file__).with_name("tools.json")).read_text(encoding="utf-8"))


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
        return [{"type": "function", "function": model_definition(name)} for name in TOOL_DEFINITIONS]
    if not isinstance(value, list) or len(value) > len(TOOL_DEFINITIONS):
        raise WorkerError("invalid_request", "tools must be a list of the supported tools")
    definitions = {}
    for item in value:
        if not isinstance(item, dict):
            raise WorkerError("invalid_request", "Each tool definition must be an object")
        definition = item.get("function", item)
        name = definition.get("name") if isinstance(definition, dict) else None
        if not isinstance(name, str) or name not in TOOL_DEFINITIONS or name in definitions:
            raise WorkerError("invalid_request", "Unknown or duplicate tool in tools")
        definitions[name] = model_definition(name, definition.get("parameters"))
    # The caller may narrow availability/choices and require existing optional
    # fields, but cannot weaken fixed schemas or introduce new arguments.
    return [{"type": "function", "function": definition} for definition in definitions.values()]


def model_definition(name: str, restrictions: Any = None) -> dict[str, Any]:
    """Provider schemas contain only fixed public keys, never private routing policy.

    Copy nested schemas as well so a caller adapting one provider's declaration
    cannot mutate the catalog used by subsequent calls or argument validation.
    """
    fixed = TOOL_DEFINITIONS[name]
    definition = deepcopy({key: fixed[key] for key in ("name", "description", "parameters")})
    if not isinstance(restrictions, dict):
        return definition
    parameters = definition["parameters"]
    offered_properties = restrictions.get("properties", {})
    if isinstance(offered_properties, dict):
        for key, spec in parameters["properties"].items():
            restriction = offered_properties.get(key)
            if not isinstance(restriction, dict) or "enum" not in restriction:
                continue
            allowed = restriction["enum"]
            if spec["type"] != "string" or not isinstance(allowed, list) or not allowed or any(not isinstance(choice, str) for choice in allowed):
                raise WorkerError("invalid_request", f"{name}.{key} enum restriction must be a nonempty string array for an existing string field")
            if "enum" in spec:
                spec["enum"] = [choice for choice in spec["enum"] if choice in allowed]
            else:
                # A task can pin a free-text argument (for example Calendar day)
                # while retaining every fixed type and length constraint.
                if any(not spec.get("minLength", 0) <= len(choice) <= spec.get("maxLength", MAX_QUERY_CHARS) for choice in allowed):
                    raise WorkerError("invalid_request", f"{name}.{key} enum choices violate its fixed length limits")
                spec["enum"] = list(dict.fromkeys(allowed))
            if not spec["enum"]:
                raise WorkerError("invalid_request", f"{name}.{key} has no supported choices")
    if "required" in restrictions:
        requested = restrictions["required"]
        if not isinstance(requested, list) or any(not isinstance(key, str) or key not in parameters["properties"] for key in requested):
            raise WorkerError("invalid_request", f"{name} required fields must already exist in its fixed schema")
        required = parameters.get("required", [])
        for key in requested:
            if key not in required:
                required.append(key)
        parameters["required"] = required
    return definition


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
        elif spec["type"] == "boolean":
            if isinstance(value, str) and value.strip().lower() in ("true", "false"):
                value = value.strip().lower() == "true"
            if not isinstance(value, bool):
                raise WorkerError("invalid_tool_call", f"{name}.{key} must be true or false")
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


GEMMA_QUOTE = '<|"|>'


def parse_gemma_call(payload: str, allowed: set[str]) -> tuple[str, dict[str, Any]]:
    """Gemma 4's native call syntax: call:name{key:<|"|>text<|"|>,count:3}.

    Flat arguments only (strings, numbers, true/false), which is all Buntini's tools
    take. Everything still passes the fixed tool schema afterwards.
    """
    match = re.fullmatch(r"\s*call:([a-z][a-z0-9_]*)\{(.*)\}\s*", payload, flags=re.DOTALL)
    if match is None:
        raise WorkerError("invalid_tool_call", "The model produced malformed Gemma tool syntax")
    name, body = match.groups()
    arguments: dict[str, Any] = {}
    position = 0
    while position < len(body):
        key = re.match(r"\s*([a-z][a-z0-9_]*)\s*:\s*", body[position:])
        if key is None:
            raise WorkerError("invalid_tool_call", "The model produced malformed Gemma tool arguments")
        position += key.end()
        if body.startswith(GEMMA_QUOTE, position):
            end = body.find(GEMMA_QUOTE, position + len(GEMMA_QUOTE))
            if end < 0:
                raise WorkerError("invalid_tool_call", "The model left a Gemma tool argument unfinished")
            value: Any = body[position + len(GEMMA_QUOTE) : end]
            position = end + len(GEMMA_QUOTE)
        else:
            raw = re.match(r"[^,]*", body[position:]).group(0)
            position += len(raw)
            raw = raw.strip()
            value = {"true": True, "false": False}.get(raw, raw)
        if key.group(1) in arguments:
            raise WorkerError("invalid_tool_call", "Duplicate tool parameters are not allowed")
        arguments[key.group(1)] = value
        rest = re.match(r"\s*(,|$)", body[position:])
        if rest is None:
            raise WorkerError("invalid_tool_call", "The model produced malformed Gemma tool arguments")
        position += rest.end()
    return validate_tool_call({"name": name, "arguments": arguments}, allowed)


def adapt_messages(messages: list[dict[str, Any]], tools: list[dict[str, Any]], native_tools: bool) -> list[dict[str, Any]]:
    """Fit the conversation to the model's chat template.

    Templates that understand tools (Qwen, Gemma 4) get each tool result labelled with
    the tool's name. Templates that don't (such as Gemma 3) get the tools described in
    the system message, calls written as <tool_call> JSON, and results as user turns,
    with neighbouring turns of the same role merged so roles keep alternating.
    """
    adapted: list[dict[str, Any]] = []
    last_call = ""
    for message in messages:
        item = dict(message)
        if item.get("tool_calls"):
            last_call = item["tool_calls"][0]["function"]["name"]
        if native_tools:
            if item["role"] == "tool":
                item["name"] = last_call
            adapted.append(item)
            continue
        if item.get("tool_calls"):
            call = item.pop("tool_calls")[0]["function"]
            item["content"] = (item["content"] + "\n" if item["content"].strip() else "") + "<tool_call>" + json.dumps({"name": call["name"], "arguments": call["arguments"]}, ensure_ascii=False) + "</tool_call>"
        elif item["role"] == "tool":
            item = {"role": "user", "content": f"Result from {last_call or 'the tool'}:\n{item['content']}"}
        if adapted and adapted[-1]["role"] == item["role"]:
            adapted[-1] = {**adapted[-1], "content": adapted[-1]["content"] + "\n\n" + item["content"]}
        else:
            adapted.append(item)
    if not native_tools and tools:
        menu = "\n".join(json.dumps(tool["function"], ensure_ascii=False) for tool in tools)
        guide = ("You can use these tools:\n" + menu + "\nTo use one, reply with only "
                 '<tool_call>{"name": "tool_name", "arguments": {...}}</tool_call> and nothing else. '
                 "Otherwise answer normally.")
        if adapted and adapted[0]["role"] == "system":
            adapted[0] = {**adapted[0], "content": adapted[0]["content"] + "\n\n" + guide}
        else:
            adapted.insert(0, {"role": "system", "content": guide})
    return adapted


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


# Opening tag -> (state, closing tag). Qwen writes <think> and <tool_call>; Gemma 4
# writes <|channel>thought…<channel|> and <|tool_call>call:…<tool_call|>.
STREAM_TAGS = {
    "<think>": ("think", "</think>"),
    "<|channel>": ("think", "<channel|>"),
    "<tool_call>": ("tool", "</tool_call>"),
    "<|tool_call>": ("gemma_tool", "<tool_call|>"),
}


class QwenStreamParser:
    """Hide thinking/tool syntax, even when a tag is split across token chunks.

    Qwen Hermes JSON, Qwen 3.5 native function/parameter tags and Gemma 4 native calls
    are supported. Only complete, schema-validated calls can become tool events.
    """

    def __init__(self, allowed: set[str]):
        self.allowed = allowed
        self.buffer = ""
        self.state = "text"
        self.raw_chars = 0
        self.text_chars = 0
        self.call: tuple[str, dict[str, Any]] | None = None
        self.closing = ""

    def feed(self, chunk: str, final: bool = False) -> list[str]:
        self.raw_chars += len(chunk)
        if self.raw_chars > MAX_RAW_CHARS:
            raise WorkerError("output_too_large", "Model output exceeded the worker limit")
        self.buffer += chunk
        output = []
        while self.buffer:
            if self.state in ("tool", "gemma_tool"):
                end = self.buffer.find(self.closing)
                if end < 0:
                    break
                payload = self.buffer[:end]
                self.call = parse_gemma_call(payload, self.allowed) if self.state == "gemma_tool" else parse_tool_payload(payload, self.allowed)
                self.buffer = ""
                break
            if self.state == "think":
                end = self.buffer.find(self.closing)
                if end < 0:
                    # Keep a possible partial closing tag, not the reasoning body.
                    self.buffer = self.buffer[-len(self.closing) :]
                    break
                self.buffer = self.buffer[end + len(self.closing) :]
                self.state = "text"
                continue
            tags = (*STREAM_TAGS, "</think>", "<channel|>")
            found = [(self.buffer.find(tag), -len(tag), tag) for tag in tags if tag in self.buffer]
            if found:
                index, _, tag = min(found)
                self._add_text(self.buffer[:index], output)
                self.buffer = self.buffer[index + len(tag) :]
                self.state, self.closing = STREAM_TAGS.get(tag, ("text", ""))
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
        if final and self.state in ("tool", "gemma_tool") and self.call is None:
            raise WorkerError("invalid_tool_call", "The model ended with an incomplete tool call")
        return output

    def _add_text(self, value: str, output: list[str]) -> None:
        if value:
            self.text_chars += len(value)
            if self.text_chars > MAX_OUTPUT_CHARS:
                raise WorkerError("output_too_large", "Model response exceeded the worker limit")
            output.append(value)
