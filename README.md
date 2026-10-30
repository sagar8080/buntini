<div align="center">

# Buntini

**A private desktop companion for your Mac.**
A little cat that lives on your screen, chats with you, notices what you’re working on, and helps out, using the AI you choose.

[![CI](https://github.com/sagar8080/buntini/actions/workflows/ci.yml/badge.svg)](https://github.com/sagar8080/buntini/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/sagar8080/buntini?include_prereleases&label=release)](https://github.com/sagar8080/buntini/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![macOS 13+ on Apple silicon](https://img.shields.io/badge/macOS-13%2B%20·%20Apple%20silicon-black)

<img src="docs/media/buntini-demo.webp" alt="Buntini playing a song, summarising a web page, reading tomorrow's calendar, asking before it adds a reminder, setting a timer, and its Apps settings" width="860">

</div>

## Highlights

- **Lives on your desktop.** A cat you can dress up. It roams, naps on your windows and offers a hand when it’s useful.
- **Your choice of brain.** Apple Intelligence, a local Qwen or Gemma model, or Anthropic, OpenAI, Gemini or your own ChatGPT plan.
- **Private by default.** On-device models can use what Buntini sees on your Mac; cloud models only see what you allow.
- **Handles everyday commands.** Play music, open a site, summarise a page, adjust volume or set a persistent timer. Results and unresolved failures stay visible.
- **Works with your apps.** Mail, Calendar, Reminders, Notes, Music, Spotify, Shortcuts and your browser tabs, on your Mac with no accounts, once you allow each one.
- **Gets things done, with your OK.** Review reminders, calendar events, notes, Mail drafts, Shortcuts and model-proposed apps/links before they run. Direct commands such as “open YouTube” run immediately.
- **Continues saved tasks.** Multi-step tasks retain approvals, inputs and verified receipts across restarts. Review the next step or add missing details; uncertain writes stop without automatic replay.
- **Looks things up.** Weather, exchange rates, Wikipedia, places, flights, eBay and web search, inside a budget you set.
- **Remembers.** Saved conversation threads, the facts you ask it to keep, and an optional morning brief.

## Install

Buntini needs an Apple silicon Mac with macOS 13 or later. Apple Intelligence needs macOS 26.

**Public rollout is pending.** Release builds must pass signing, notarization and device acceptance before publication. See the [production readiness checklist](docs/production-readiness.md). Earlier ad-hoc builds are development previews.

1. Download **Buntini-macOS-arm64.dmg** from the [latest release](https://github.com/sagar8080/buntini/releases).
2. Open it and drag **Buntini** to **Applications**.
3. Open Buntini. A public release should pass Gatekeeper with its Developer ID signature and notarization ticket. A Gatekeeper rejection is a release failure to investigate.

Buntini then walks you through a short setup in its chat bubble.

## Choose a brain

| Brain | Where it runs | What you need |
| --- | --- | --- |
| **Apple Intelligence** (default) | On your Mac | macOS 26 with Apple Intelligence turned on |
| **Local model** | On your Mac | A Qwen or Gemma MLX model. Buntini recommends one for your Mac’s chip and memory and gives you the commands to install it. |
| **Anthropic, OpenAI or Gemini** | With the provider | Your API key and a monthly spending cap |
| **ChatGPT** | With OpenAI | **Sign in with ChatGPT** to use your own plan |

Switch any time in **Settings → Brain**.

## Privacy

- Your conversation, settings and memories stay on your Mac. API keys and sign-ins live in the macOS Keychain.
- Online lookups show the exact request for approval. Direct browser commands such as “google weather in Porto” navigate immediately; cloud chat uses the brain you selected.
- Cloud models only receive the local data you switch on in **Settings → Privacy**. All of it is off by default.

Read the full [privacy policy](PRIVACY.md).

## Development

You’ll need Node.js 24, Rust 1.95 or later (CI uses 1.99.1), Python 3.11 or later for the checks, and Xcode Command Line Tools. The Apple Intelligence helper builds with the macOS 26 SDK.

```bash
npm ci
./scripts/fetch-models.sh   # once: the bundled memory search model
npm run desktop      # run the app with live reload
./scripts/check.sh   # build and run every test
```

Every push runs the checks in [GitHub Actions](.github/workflows/ci.yml). A version-matching tag such as `v0.2.0` requires all checks and dependency audits, a Developer ID certificate and Apple notarization, then prepares a **draft** release with a disk image, zip, build metadata and checksums. A maintainer publishes it after final device acceptance. Prerelease tags must match the prerelease version in all manifests. See the [release runbook](docs/production-readiness.md).

| Folder | What’s inside |
| --- | --- |
| `src/` | The React interface: chat, settings, setup and the animated companion |
| `src-tauri/` | The Rust app: model hub, privacy rules, tools, actions, storage and the scheduler |
| `native/` | Swift helpers for Apple Intelligence, speech and audio capture |
| `worker/` | The Python MLX worker for local models and the shared tool definitions |

## Documentation

- [User guide](docs/guide.md): every feature in detail
- [Privacy policy](PRIVACY.md)
- [Online tools](docs/online-tools.md) and [voice setup](docs/voice-setup.md)
- [Acceptance checklist](docs/acceptance.md): what’s been verified, and how to check the rest
- [Production readiness and release runbook](docs/production-readiness.md): security gates, external setup and rollout blockers
- [Capability map](docs/capabilities.md) and [reliability review](docs/reliability.md): current limits, fixes and the path toward a dependable personal assistant
- [Scenario evaluations](docs/evaluations.md): measured model failures and release gates
- [Technical design](docs/technical-design.md)

## License

Buntini is open source under the [MIT License](LICENSE).
