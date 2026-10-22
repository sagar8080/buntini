# Technical Design Document (TDD)
## Local-First AI Desktop Companion
**Version:** 0.1 (MVP)  
**Target:** macOS / Apple Silicon  
**Status:** Proposed

### 1. Product Overview

A lightweight, always-accessible AI companion that lives on the desktop as an expressive, animated character. It supports natural language, voice interaction, local computer assistance, and real-time internet research.

The companion is **local-first, privacy-conscious, model-independent, and usable offline**.

**Core principles**
- Minimal interface, no traditional application window required.
- Fully functional local assistance without internet connectivity.
- Bring-your-own API keys for online services.
- Explicit permissions for sensitive computer operations.
- Responsive, interruptible voice and text interactions.

### 2. System Architecture

| Component | Technology | Responsibility |
|---|---|---|
| Desktop UI | Tauri 2 + React | Floating pet, command surface, result cards |
| Core engine | Rust | Agent orchestration, scheduling, permissions |
| Local inference | Python + MLX | Existing Qwen model and tool-call generation |
| Voice input | AVAudioEngine + Silero VAD + MLX Whisper | Offline speech recognition |
| Voice output | macOS TTS; Pocket TTS later | Offline, streamed speech output |
| Memory | SQLite + FTS5 | Preferences, task history, local file search |
| Web research | Brave Search + Tavily | Real-time search and content retrieval |
| Specialized search | eBay, Duffel, Places APIs | Shopping, flights and local discovery |
| Integrations | MCP + OAuth | External applications and services |

**Execution flow**

`User → Companion UI → Rust Controller → MLX Qwen → Tool Router → Local/Online Tools → Validator → Result Card / Voice`

The MLX worker runs as a persistent Python process within the existing virtual environment. Rust communicates with it through local IPC, streams outputs, and manages cancellation.

### 3. Functional Requirements

**FR-01: Desktop Companion**
- Small draggable character with idle, listening, thinking, working, speaking and approval states.
- Expandable command input and contextual result cards.
- Global keyboard shortcut and menu bar controls.
- Light/dark mode, reduced-motion support and hide option.

**FR-02: Local Intelligence**
- Search files within authorized directories.
- Retrieve file metadata and indexed document content.
- Inspect system health, storage, processes and resource usage.
- Check supported OS security settings and update status.
- Retain task history and preferences locally.

**FR-03: Online Intelligence**
- Search the live web and summarize news with source citations.
- Compare products by price, specifications and seller reliability.
- Search flights, fares and destinations.
- Discover public social content and professional information.
- Support saved searches and scheduled monitoring.

**FR-04: Voice Interaction**
- Push-to-talk as the default interaction.
- Offline speech recognition and speech synthesis.
- Streaming speech output where supported.
- Interrupt active speech and cancel pending audio.
- Optional wake-word activation in a later release.

**FR-05: Model Support**
- Qwen through MLX as the default inference engine.
- Structured tool calls with schema validation.
- Optional external model providers in later releases.
- No Ollama or NVIDIA inference dependency.

### 4. Security and Privacy

- Local inference and local storage by default.
- API credentials stored in macOS Keychain.
- Filesystem access scoped to user-approved directories.
- No unrestricted shell execution by the LLM.
- Explicit confirmation for destructive or external actions.
- No automatic upload of private documents.
- External page content treated as untrusted.
- Audit log for tool calls, permissions and sensitive operations.

The Rust policy engine enforces these rules independently of model instructions.

### 5. Agent Execution Design

Each task follows a controlled lifecycle:

`Understand → Plan → Select Tool → Authorize → Execute → Validate → Respond`

The controller supports:
- Tool timeouts, retries and cancellation.
- Structured results with timestamps and source URLs.
- API quotas, caching and spending limits.
- Task persistence and scheduled execution.
- User approval checkpoints.

Long-running tasks operate independently of the companion UI so that the character remains responsive.

### 6. Performance Targets

| Metric | Target |
|---|---|
| Companion activation | Under 150 ms |
| Local tool execution overhead | Under 100 ms |
| Short speech transcription | Under 1 second after speech ends |
| First spoken response | Under 2 seconds for simple local requests |
| Speech interruption | Under 200 ms |
| Idle resource usage | Minimal; no continuous model inference |
| Search accuracy | Source-backed and freshness-aware |

Targets require measurement on the actual Apple Silicon device.

### 7. MVP Scope and Delivery

| Phase | Deliverables |
|---|---|
| P0 — Foundation | Floating companion, Rust controller, persistent MLX Qwen worker, local IPC |
| P1 — Offline tools | File search, system information, SQLite memory, permissions |
| P2 — Online tools | Brave/Tavily integration, sourced answers, search result cards |
| P3 — Voice | Push-to-talk, offline STT/TTS, playback cancellation |
| P4 — Persistent tasks | Saved searches, price tracking, notifications, task shelf |
| Future | Flight APIs, social integrations, wake word, richer speech, model switching |

### 8. Testing and Acceptance Criteria

The MVP is acceptable when:

1. The companion can answer local questions without internet access.
2. File search respects configured folder permissions.
3. Live web searches return cited, timestamped results.
4. Qwen successfully executes validated local and online tool calls.
5. Voice input and output work without cloud services.
6. Sensitive actions cannot bypass user approval.
7. The UI remains responsive during long-running inference or searches.
8. Failed API calls and model errors produce recoverable states rather than application crashes.

Testing will include unit tests for tool contracts, integration tests for MLX IPC, permission-denial tests, and end-to-end voice and search workflows.

### 9. Cost and Dependencies

**Target recurring cost:** $0–$5/month for personal usage within provider allowances.

- Local inference: $0 API cost.
- Voice processing: $0 API cost.
- Storage and scheduling: local.
- Web search: free credits with configurable usage caps.
- Other users: bring their own API keys.
- No cloud backend required for the MVP.

### 10. Explicit Non-Goals

The initial version will not include:

- Autonomous purchases, payments or account changes.
- Full desktop control or unrestricted browser automation.
- Antivirus-grade malware detection.
- AI model fine-tuning or NVIDIA NeMo integration.
- Cloud account synchronization.
- Complex multi-agent workflows.
- A full-screen dashboard or elaborate virtual pet mechanics.

**Design objective:** Build a fast, delightful desktop companion that combines offline computer assistance, natural voice interaction and trustworthy online information without sacrificing user control or privacy.