# Buntini

A local-first desktop companion for macOS and Apple Silicon. Buntini lives directly on the desktop as a small animated cat you can dress up. Click it or press **⌘⇧Space** to open an attached command bubble. It starts tucked away, without a Dock icon or a normal application window.

## Run the desktop companion

Requires macOS 13+, Apple Silicon, Node.js 24+, Rust, and Xcode Command Line Tools.

```sh
npm ci
npm run desktop
```

The desktop command builds the native audio helper and launches Tauri. `npm run dev` is only the browser development preview; local tools, Keychain, audio, and desktop overlay behavior require the native app.

For a standalone local app:

```sh
npm run tauri -- build --debug --bundles app --no-sign
open src-tauri/target/debug/bundle/macos/Buntini.app
```

The debug bundle uses the existing local Python environment. It is not a signed or notarized distribution. To prepare a release build, omit `--debug` and configure your own Apple signing/notarization credentials. No cloud backend is required.

## Your existing model

This Mac's confirmed defaults are detected when present:

- Python: `/Users/sagar/projects/pulse-venv/bin/python`
- Qwen: `/Users/sagar/projects/pulse-venv/models/Qwen3.5-9B-4bit`

Settings can change either path. The approved voice setup now uses a separate project interpreter at `/Users/sagar/projects/nexon/.venv-voice/bin/python`, with a local English Whisper model and Silero VAD; the original environment is preserved. Other machines can set `BUNTINI_PYTHON`, `BUNTINI_MODEL_PATH`, and `BUNTINI_WHISPER_MODEL_PATH` before first launch. Only existing local model folders are accepted. Buntini never downloads a model, installs packages, or falls back to simulated answers in production.

The existing MLX environment and Qwen model were verified with real offline generation, a structured `system_info` request, a grounded follow-up, cancellation, and reuse of the persistent worker. Qwen 3.5's native function/parameter tool format and older Qwen JSON tool calls are supported. See [acceptance evidence](docs/acceptance.md) for measured timings and remaining checks.

## Everyday use

- **The companion:** drag the character to move it; click to open or tuck away the bubble. The menu bar icon can show, hide, or quit Buntini. It floats above other windows and follows desktop Spaces.
- **Ask anything:** chat with local Qwen. It can request the fixed, validated local tools. It cannot run a shell or modify files.
- **My files:** add a folder in Settings first. Search matches names and small text documents within that scope. Revoke access at any time. Hidden files and symlinks are skipped; scans are bounded to 10,000 entries, 40 hits, and 64 KB per text file.
- **My Mac:** read CPU, memory, storage, process usage, and supported FileVault/Gatekeeper/SIP status. These tools work without a model or network. Buntini does not claim the OS is up to date or perform security changes.
- **Online lookups:** in Online mode your local model picks one service for the request: weather (Open-Meteo), exchange rates (ECB via Frankfurter), Wikipedia, nearby places (OpenStreetMap/Nominatim), eBay listings, Duffel flight offers, or web search. Before anything is sent, an approval sheet shows the service, host, and the exact values that will leave your Mac; you can allow it once, cancel, or switch to a plain web search. Weather, rates, Wikipedia and places need no key. eBay (your developer app ID and secret) and Duffel (your access token) are optional, stored in macOS Keychain, count toward the monthly request cap, and are search-only: nothing is bought or booked. Results appear as numbered cards with attribution that the answer cites. See [online tools](docs/online-tools.md).
- **The web:** add your Brave or Tavily key in Settings. Only explicitly submitted web queries and saved web searches reach the provider. Results include source URLs and retrieval timestamps. A one-use confirmation shows the exact query/provider before dispatch. The controller can retrieve up to three selected public result pages, preserving citations, excerpts and published structured offers; inaccessible pages remain clearly identified. API keys are stored in macOS Keychain.
- **The task shelf:** local task history plus saved web queries, with pause/resume, check-now, and intervals of at least 15 minutes. Monitoring runs while Buntini is open, retains schedules after restart, and emits an in-app notice when results or errors change. Price watches can repeatedly read an explicitly approved public product page, retain verified structured-price history, and alert when the configured currency/target is reached. Desktop notifications are optional and require a user-initiated macOS permission request. Monitoring does not wake a sleeping or quit app or send external messages.
- **Voice:** hold the microphone button, then release to transcribe locally. Review the transcript before sending. `Esc` cancels audio; Read aloud uses macOS speech and can be interrupted.

## Personalize your assistant

Settings include a local profile: assistant name, personality instructions and presets, four characters (a customizable cat by default, plus nine-tailed fox, fox, or orbit), cat coat (seven, including tabby, tuxedo, calico, and siamese), eye color, accessory (bell collar, bow, bandana, top hat, or flower crown, tinted with the accent color), motion personality (calm, playful, or sleepy; reduced motion still wins), accent color, companion size, installed macOS speaking voice and pace, theme, and reduced motion. The saved profile changes the desktop character and the trusted model context. Custom personality text never grants tools or permissions.

The companion feels alive without running the model: it blinks at random, picks idle habits that match its motion personality (glancing around, slow blinks, head tilts, ear flicks, grooming, stretching, yawning, a playful wiggle-and-hop, or nodding off), and its eyes follow your cursor. Stroke it to make it purr, click it for a happy hop with hearts, and it cheers when a task finishes or droops when one fails. Cursor tracking polls the pointer position locally a few times a second and never stores it. "A little less motion" turns all of this off.

## Voice setup

Voice is prepared in this project without changing `pulse-venv`:

- Interpreter: `.venv-voice/bin/python`
- English Whisper: `models/whisper-tiny.en`
- Speech presence filter: `models/silero_vad.onnx`
- Existing Qwen: `/Users/sagar/projects/pulse-venv/models/Qwen3.5-9B-4bit`

A real synthetic-speech check passed through Silero and MLX Whisper completely offline. The local setup diagnostic checks packages and model files without loading the GPU. See [voice setup](docs/voice-setup.md) for explicit installation/download commands and measured observations. Runtime never installs packages or downloads models.

Buntini requests microphone access only when you deliberately start push-to-talk with a configured model. Hold the mic, release, review the transcript, then send it. Temporary private audio is deleted after success, failure, or cancellation. There is no continuous listening or wake word. Sentence streaming uses installed macOS voices and can be interrupted, including queued speech.

## Privacy and limits

Rust enforces the allowlist independently of the model: scoped file search, system snapshots, and explicitly authorized web searches. Tool arguments, IPC messages, model context, provider responses, and output sizes are bounded. Private file results cannot be forwarded to a search provider by the model. Remote results and file excerpts are treated as untrusted data.

History, preferences, schedules, audit events, an FTS5 file index, search cache, and request counts live in SQLite under the app's macOS Application Support directory (`app.buntini.companion`). Temporary audio uses the corresponding cache directory.

## Upgrading from Nexon

Buntini was previously named Nexon. Quit Nexon before the first Buntini launch. On that launch, if `app.buntini.companion` has no database yet, Buntini imports a consistent snapshot of `app.nexon.companion/nexon.sqlite3` (including unsaved WAL changes), renames the default assistant name "Nexon" to "Buntini" (a custom name is kept), and records the result in the audit log. The old folder is left in place with a `migrated-to-buntini` marker, so the import runs once and can be inspected or removed by hand. Leftover Nexon recordings in the old cache are deleted. Brave/Tavily keys move to the new Keychain entry the first time they are read; macOS may ask once whether Buntini can use the old item. macOS ties microphone and notification permissions to the app identity, so Buntini asks for them again. If the import fails, Buntini starts with fresh data, logs the error, and leaves the Nexon data untouched. Indexing is bounded and explicit; only approved folders are included, and indexed hits are reopened within their current permission scope before use. Revoking a folder removes its index entries. Previously saved conversation excerpts remain in history until it is cleared.

The monthly search cap counts requests, not dollars. Failed or cancelled dispatched searches can consume a reservation. Provider pricing and allowances are managed through your own account. No API key is needed for offline tools. Estimated dollar budgets are configured alongside request caps; every dispatched provider attempt is reserved before it runs.

Unsupported actions are unavailable, rather than awaiting approval: file changes, destructive operations, purchases, account changes, and outbound messages. Flight/social APIs, MCP/OAuth adapters, wake words, Pocket TTS, and cloud model providers remain future work as designated in the TDD. A read-only macOS update check is available only after explicit approval; it never installs updates.

## Verify

```sh
./scripts/check.sh
```

With dependencies already available, this runs the production frontend build, model-free worker tests, Rust policy/persistence/IPC tests, and synthetic Swift audio checks without accessing the microphone or downloading anything. Real microphone capture needs the user’s normal macOS permission. Whisper has been checked with synthetic speech; live provider acceptance still requires an API key. See [the acceptance matrix](docs/acceptance.md).

## Project map

| Path | Role |
| --- | --- |
| `src/` | React command bubble, companion animation, settings and task shelf |
| `src-tauri/src/` | Rust controller, policy, local tools, SQLite, providers, scheduler, audio and IPC |
| `worker/` | Persistent offline MLX Qwen/Whisper process and strict JSONL protocol |
| `native/` | AVAudioEngine capture helper and synthetic audio tests |
| `docs/technical-design.md` | Original supplied TDD |
| `docs/acceptance.md` | Delivered scope, verification and remaining device acceptance |

Implementation references: [Tauri IPC](https://v2.tauri.app/develop/calling-frontend/), [global shortcuts](https://v2.tauri.app/plugin/global-shortcut/), [MLX LM](https://github.com/ml-explore/mlx-lm), and [MLX Whisper](https://github.com/ml-explore/mlx-examples/tree/main/whisper).
