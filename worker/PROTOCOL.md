# JSONL protocol v1

One UTF-8 JSON object per line. Stdout is exclusively protocol events; library diagnostics are redirected to stderr. Each request has a unique, nonempty string `id` of at most 128 characters. The worker runs one inference request at a time. Additional inference requests receive a recoverable `busy` error; `ping` and `cancel` remain available. Keep stdin open while a request runs; EOF cancels active work and shuts the worker down.

## Ready and ping

Startup reports transport readiness; it does not claim the selected model has loaded:

```json
{"id":null,"type":"ready","protocol":1,"backend":"mlx","model_loaded":false}
```

Request:

```json
{"id":"health-1","type":"ping"}
```

Response:

```json
{"id":"health-1","type":"pong","protocol":1,"backend":"mlx","model_loaded":false}
```

`backend` is `fixture` only when explicitly enabled. `model_loaded` becomes true after a successful Qwen load.

Ping may include `model_path`, `whisper_model_path`, and `vad_model_path` to inspect configured local assets. Pong also returns `python: {version, executable}`, `dependencies: {mlx_lm, mlx_whisper, onnxruntime, numpy}` (each `{installed, version}`), and `models: {qwen, whisper, vad}` (each `{configured, files_present, ready_for_load, error}`). `inference_verified:false` explicitly distinguishes metadata/file checks from actual model execution. Diagnostics do not import GPU libraries or load models.

## Generate

```json
{"id":"turn-1","type":"generate","model_path":"/absolute/local/Qwen","max_tokens":1024,"messages":[{"role":"user","content":"Find invoices"}],"tools":[{"name":"search_files"},{"name":"system_info"}]}
```

`model_path` may be omitted if `BUNTINI_MODEL_PATH` is set. `max_tokens` defaults to 1024. Messages accept `system`, `user`, `assistant`, or `tool` roles with string content. Assistant history may include one OpenAI-style `tool_calls` entry; its arguments are validated again. No attachments or arbitrary generation arguments are accepted by the model adapter.

`tools` accepts either short objects with `name` or OpenAI-style function definitions. Omitted means all three known tools; an empty array disables tool requests. Rust should pass only the tools available to that turn. Caller-supplied schema/description changes do not override the worker's fixed definitions.

Streaming answer:

```json
{"id":"turn-1","type":"token","text":"I found "}
{"id":"turn-1","type":"token","text":"the answer."}
{"id":"turn-1","type":"done","text":"I found the answer.","finish_reason":"stop"}
```

Tool request:

```json
{"id":"turn-1","type":"tool_call","name":"search_files","arguments":{"query":"invoice"}}
{"id":"turn-1","type":"done","text":"","finish_reason":"tool_call"}
```

At most one tool request is emitted per turn. It is only a request: Rust must independently validate its arguments and enforce permissions before execution. The worker recognizes older Qwen Hermes JSON `<tool_call>{"name":...,"arguments":...}</tool_call>` and Qwen 3.5's native format:

```text
<tool_call>
<function=search_files>
<parameter=query>
invoice
</parameter>
</function>
</tool_call>
```

Both formats become the same validated JSONL event. The native syntax uses a deliberately narrow grammar, without general XML parsing, entity resolution, argument coercion, or duplicate parameters. Reasoning/tool markup is suppressed in visible tokens. Malformed, incomplete, unknown, or unavailable calls are rejected. Plain JSON in an answer does not authorize a tool.

Fixed tool arguments:

| Tool | Arguments |
| --- | --- |
| `search_files` | `{"query":"nonblank string, at most 500 characters"}` |
| `system_info` | `{}` |
| `web_search` | `{"query":"nonblank string, at most 500 characters"}` |

Extra argument fields are rejected. Rust executes an authorized call and submits a **new request ID** with the conversation plus an assistant tool call and a `tool` result. For example:

```json
{"id":"turn-2","type":"generate","messages":[{"role":"user","content":"Find invoices"},{"role":"assistant","content":"","tool_calls":[{"type":"function","function":{"name":"search_files","arguments":{"query":"invoice"}}}]},{"role":"tool","content":"{\"files\":[],\"searched_at\":\"2026-10-21T16:00:00Z\"}"}],"tools":[]}
```

## Transcribe

```json
{"id":"voice-1","type":"transcribe","audio_path":"/absolute/path/recording.wav","model_path":"/absolute/local/whisper","vad_model_path":"/absolute/local/silero_vad.onnx"}
```

`model_path` may use the `BUNTINI_WHISPER_MODEL_PATH` default. Input must be a local mono PCM16 WAV at 16 kHz, up to 90 seconds and 6 MB. The controller grants microphone access, creates the WAV, and cleans up the file.

The Whisper folder must contain a valid converted dimensions configuration and nonempty `model.safetensors`, `weights.safetensors`, or `weights.npz`. Optional `vad_model_path` (or `BUNTINI_VAD_MODEL_PATH`) selects a local Silero v5/v6 ONNX file, at most 32 MB. Empty/unset disables the filter; push-to-talk remains valid and Whisper's no-speech threshold is applied. Configured VAD runs on CPU with fresh state per recording and requires three consecutive speech-positive 32 ms windows before Whisper runs. It does not trim audio, listen continuously, or download weights. No detected speech returns recoverable `no_speech`.

```json
{"id":"voice-1","type":"done","text":"What is my free disk space?","finish_reason":"stop"}
```

## Cancellation

```json
{"id":"cancel-1","type":"cancel","target_id":"turn-1"}
```

The reader acknowledges immediately:

```json
{"id":"cancel-1","type":"done","target_id":"turn-1","cancelled":true}
```

The interrupted request completes at its next cancellation boundary:

```json
{"id":"turn-1","type":"done","text":"partial answer","finish_reason":"cancelled"}
```

`cancelled:false` means the target was no longer active. Cancelling does not make any partial result authoritative. Native model loading and Whisper calls can take longer to return; terminate/restart the process after the controller's grace period if necessary.

## Recoverable errors

```json
{"id":"turn-1","type":"error","code":"model_not_configured","message":"Configure a local Qwen model folder in Settings","recoverable":true}
{"id":"turn-1","type":"done","text":"","finish_reason":"error"}
```

Error responses always end with `done`. Malformed JSON and requests without usable IDs use `id:null`. Codes include `invalid_json`, `invalid_request`, `request_too_large`, `busy`, `model_not_configured`, `model_not_found`, `dependency_unavailable`, `model_load_failed`, `chat_template_failed`, `context_too_large`, `invalid_tool_call`, `output_too_large`, `invalid_audio`, `invalid_whisper_model`, `invalid_vad_model`, `vad_unavailable`, `vad_failed`, `no_speech`, `transcription_unavailable`, `transcription_failed`, `invalid_transcription`, and `inference_failed`.

The controller should display the message, retain settings, and allow a retry. A recoverable model error never switches to fixture output.

## Hard limits

| Resource | Limit |
| --- | --- |
| Input line | 262,144 UTF-8 bytes |
| Conversation | 40 messages / 120,000 serialized characters |
| Model prompt | 16,384 tokens |
| Generated tokens | 1–4,096 per request |
| Visible response | 32,000 characters |
| Raw model output, including reasoning | 128,000 characters |
| Pending/running inference | One request |
| Error message | 2,000 characters |

Limits are rejected explicitly; conversation content is not silently truncated. Fixture-only echoed text is deliberately capped for deterministic tests.
