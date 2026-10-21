# Buntini

A local-first desktop companion for macOS and Apple Silicon. Buntini lives directly on the desktop as a small animated cat you can dress up. Click it or press **⌘⇧Space** to open an attached command bubble. It starts tucked away, without a Dock icon or a normal application window.

## Run the desktop companion

Requires macOS 13+, Apple Silicon, Node.js 24+, Rust, and Xcode Command Line Tools.

```sh
npm ci
npm run desktop
```

The desktop command builds the native audio and Apple Intelligence helpers and launches Tauri. `npm run dev` is only the browser development preview; local tools, Keychain, audio, and desktop overlay behavior require the native app.

For a standalone local app:

```sh
npm run tauri -- build --debug --bundles app --no-sign
open src-tauri/target/debug/bundle/macos/Buntini.app
```

The debug bundle uses the existing local Python environment. It is not a signed or notarized distribution. To prepare a release build, omit `--debug` and configure your own Apple signing/notarization credentials. No cloud account is required.

## First launch

Buntini opens its bubble and walks you through setup as a short conversation: what to call you, its name and coat, which brain to use, an optional Brave or Tavily key with free-tier mode and a daily cap, and whether it may roam and notice what you're doing. Keys go straight to the macOS Keychain. You can skip setup, and **Settings → Run setup again** starts it over. Anyone who already chatted with an earlier version skips it automatically.

## Choose a brain

Settings → Brain picks the model that answers. Every backend gets the same validated tools and the same conversation memory.

- **Apple Intelligence** (default): Apple's on-device Foundation Models through the `native/buntini-fm` helper. Private, about a second per reply, nothing to download. It needs macOS 26 on Apple silicon with Apple Intelligence turned on; Settings explains what to do when it isn't available. Its context is small (about 4K tokens), so Buntini gives it compact tool results and a shorter memory.
- **Local MLX model:** your own model folder (for example Qwen) run by the Python worker. Bigger and slower, fully offline. This Mac's defaults are detected when present (`/Users/sagar/projects/pulse-venv/bin/python` and `…/models/Qwen3.5-9B-4bit`); other machines can set `BUNTINI_PYTHON` and `BUNTINI_MODEL_PATH` before first launch. Existing installs that already used MLX stay on it. Buntini never downloads a model or installs packages.
- **Cloud model:** Anthropic (suggested `claude-opus-5-5`), OpenAI (`gpt-5-mini`) or Google Gemini (`gemini-2.5-flash`) with your own API key; any model name your key can use works. Replies stream as they are written. Each message reserves a cautious cost estimate against your **monthly spending cap** before it is sent, then settles to the token usage the provider reports, at the per-million-token prices shown in Settings (suggested from published October 2026 prices; check your provider's page, their invoice is authoritative). Rejected requests are refunded, Buntini warns once at 80% and stops at the cap. A cloud model sees only what you allow under *What this model may see* (see Privacy below).

The MLX environment and Qwen model were verified with real offline generation, tool calls, follow-ups, cancellation and worker reuse; Apple Intelligence with live routing, streaming and transcription probes. See [acceptance evidence](docs/acceptance.md).

## Everyday use

- **The companion:** drag the character to move it; click to open or tuck away the bubble. The menu bar icon can show, hide, or quit Buntini. It floats above other windows and follows desktop Spaces.
- **Ask anything:** one ongoing conversation. Your messages appear on the right and Buntini's replies (with any cards, files and sources) on the left; earlier turns are remembered within the brain's budget. **New topic** starts fresh: older messages stay visible above a divider but are no longer sent to the model. **Load earlier** pages back through history. Buntini can request the fixed, validated local tools; it cannot run a shell or modify files.
- **My files:** add a folder in Settings first. Search matches names and small text documents within that scope. Revoke access at any time. Hidden files and symlinks are skipped; scans are bounded to 10,000 entries, 40 hits, and 64 KB per text file.
- **My Mac:** read CPU, memory, storage, process usage, and supported FileVault/Gatekeeper/SIP status. These tools work without a model or network. Buntini does not claim the OS is up to date or perform security changes.
- **Online lookups:** in Online mode your local model picks one service for the request: weather (Open-Meteo), exchange rates (ECB via Frankfurter), Wikipedia, nearby places (OpenStreetMap/Nominatim), eBay listings, Duffel flight offers, or web search. Before anything is sent, an approval sheet shows the service, host, and the exact values that will leave your Mac; you can allow it once, cancel, or switch to a plain web search. Weather, rates, Wikipedia and places need no key. eBay (your developer app ID and secret) and Duffel (your access token) are optional, stored in macOS Keychain, count toward the monthly request cap, and are search-only: nothing is bought or booked. Results appear as numbered cards with attribution that the answer cites. See [online tools](docs/online-tools.md).
- **The web:** add your Brave or Tavily key in Settings. Only explicitly submitted web queries and saved web searches reach the provider. Results include source URLs and retrieval timestamps. A one-use confirmation shows the exact query/provider before dispatch. The controller can retrieve up to three selected public result pages, preserving citations, excerpts and published structured offers; inaccessible pages remain clearly identified. API keys are stored in macOS Keychain.
- **The task shelf:** local task history plus saved web queries, with pause/resume, check-now, and intervals of at least 15 minutes. Monitoring runs while Buntini is open, retains schedules after restart, and emits an in-app notice when results or errors change. Price watches can repeatedly read an explicitly approved public product page, retain verified structured-price history, and alert when the configured currency/target is reached. Desktop notifications are optional and require a user-initiated macOS permission request. Monitoring does not wake a sleeping or quit app or send external messages.
- **Voice:** hold the microphone button, then release to transcribe locally. Review the transcript before sending. `Esc` cancels audio; Read aloud uses macOS speech and can be interrupted.

## Personalize your assistant

Settings include a local profile: assistant name, personality instructions and presets, four characters (a customizable cat by default, plus nine-tailed fox, fox, or orbit), cat coat (seven, including tabby, tuxedo, calico, and siamese), eye color, accessory (bell collar, bow, bandana, top hat, or flower crown, tinted with the accent color), motion personality (calm, playful, or sleepy; reduced motion still wins), accent color, companion size, installed macOS speaking voice and pace, theme, and reduced motion. The saved profile changes the desktop character and the trusted model context. Custom personality text never grants tools or permissions.

**Out and about.** Buntini walks along the bottom of the screen, hops up to sit on top of the window you are using and rides along when you move it, and falls back down (or onto the window below) when you drop it or close its perch. It naps when you have been away for three minutes, greets you when you come back, and turns invisible and click-through during meetings (Zoom, Teams, FaceTime, Webex) and full-screen apps. Helpful nudges appear as a small speech bubble with one-click actions: a stretch break after about 50 active minutes (snooze 15 min), a 25-minute focus timer offered in code, writing and design apps, “Look it up?” in a browser, “Find a file?” in Finder, a late-night wind-down reminder, and battery, CPU and disk warnings with a “Check my Mac” shortcut. Nudges are spaced at least ten minutes apart and each kind has its own cooldown. The signals are seconds since your last input, the frontmost app's name, window frames, and coarse battery/CPU/disk levels. They stay in memory on this Mac (app history for at most three hours); window titles and contents are never read and nothing is saved or sent online. Because the model runs on this Mac, every chat gives it this context up front (current and recent apps with minutes, active time, time of day, battery, CPU and disk), so questions like “what am I doing?” are answered directly. Turning awareness off clears the history and stops sharing it. Each part can be switched off under *Make it yours → Out and about*; reduced motion stops all walking.

The companion feels alive without running the model: it blinks at random, picks idle habits that match its motion personality (glancing around, slow blinks, head tilts, ear flicks, grooming, stretching, yawning, a playful wiggle-and-hop, or nodding off), and its eyes follow your cursor. Stroke it to make it purr, click it for a happy hop with hearts, and it cheers when a task finishes or droops when one fails. Cursor tracking polls the pointer position locally a few times a second and never stores it. "A little less motion" turns all of this off.

## Voice setup

Push-to-talk uses Apple's on-device speech recognition (SpeechAnalyzer, macOS 26) by default; macOS may download the language model once. With the local MLX brain and a local Whisper model folder, Buntini uses MLX Whisper instead. This project's optional Whisper setup lives in `.venv-voice/bin/python`, `models/whisper-tiny.en` and `models/silero_vad.onnx`, without changing `pulse-venv`; see [voice setup](docs/voice-setup.md). Runtime never installs packages.

Buntini requests microphone access only when you deliberately start push-to-talk. Hold the mic, release, review the transcript, then send it. Temporary private audio is deleted after success, failure, or cancellation. There is no continuous listening or wake word. Sentence streaming uses installed macOS voices and can be interrupted, including queued speech.

## Privacy and limits

**On-device by design.** The local model may use anything Buntini can see on this Mac (activity, approved folders, system state), because none of it leaves the machine. Restrictions apply at egress: search providers and online integrations receive only the exact request you approve. A cloud model receives your messages, and local data only for the categories you switch on under *What this model may see*: what you're doing, Mac health, your files, earlier conversation (all off by default). Earlier turns that used data you haven't shared are left out of its memory, and your own name is only given to on-device models. `src-tauri/src/privacy.rs` makes that decision for every model-facing path.

Rust enforces the allowlist independently of the model: scoped file search, system snapshots, and explicitly authorized web searches. Tool arguments, IPC messages, model context, provider responses, and output sizes are bounded. Private file results cannot be forwarded to a search provider by the model. Remote results and file excerpts are treated as untrusted data.

History, preferences, schedules, audit events, an FTS5 file index, search cache, and request counts live in SQLite under the app's macOS Application Support directory (`app.buntini.companion`). Temporary audio uses the corresponding cache directory.

## Upgrading from Nexon

Buntini was previously named Nexon. Quit Nexon before the first Buntini launch. On that launch, if `app.buntini.companion` has no database yet, Buntini imports a consistent snapshot of `app.nexon.companion/nexon.sqlite3` (including unsaved WAL changes), renames the default assistant name "Nexon" to "Buntini" (a custom name is kept), and records the result in the audit log. The old folder is left in place with a `migrated-to-buntini` marker, so the import runs once and can be inspected or removed by hand. Leftover Nexon recordings in the old cache are deleted. Brave/Tavily keys move to the new Keychain entry the first time they are read; macOS may ask once whether Buntini can use the old item. macOS ties microphone and notification permissions to the app identity, so Buntini asks for them again. If the import fails, Buntini starts with fresh data, logs the error, and leaves the Nexon data untouched. Indexing is bounded and explicit; only approved folders are included, and indexed hits are reopened within their current permission scope before use. Revoking a folder removes its index entries. Previously saved conversation excerpts remain in history until it is cleared.

**Search budgets.** Free-tier mode (on by default) keeps Brave and Tavily inside their free monthly allowance (about 1,000 searches each as of October 2026) and never estimates any spend. A daily lookup limit (30 by default) keeps one busy afternoon from using the month. Turning free-tier mode off reveals a monthly cap, a dollar budget and a per-search cost. Every request is counted before it is sent, eBay and Duffel are counted separately so they never use up the search allowance, and Buntini tells you once a month when usage reaches 80%. Failed or cancelled requests can still use a reservation. No API key is needed for offline tools.

Unsupported actions are unavailable, rather than awaiting approval: file changes, destructive operations, purchases, account changes, and outbound messages. Social APIs, MCP/OAuth adapters, wake words and Pocket TTS remain future work as designated in the TDD. A read-only macOS update check is available only after explicit approval; it never installs updates.

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
| `worker/` | Persistent offline MLX Qwen/Whisper process, the JSONL protocol and shared tool schemas (`tools.json`) |
| `native/` | AVAudioEngine capture helper, the `buntini-fm` Apple Intelligence and speech helper, and their tests |
| `docs/technical-design.md` | Original supplied TDD |
| `docs/acceptance.md` | Delivered scope, verification and remaining device acceptance |

Implementation references: [Tauri IPC](https://v2.tauri.app/develop/calling-frontend/), [global shortcuts](https://v2.tauri.app/plugin/global-shortcut/), [MLX LM](https://github.com/ml-explore/mlx-lm), and [MLX Whisper](https://github.com/ml-explore/mlx-examples/tree/main/whisper).
