# MVP delivery and acceptance

This records the implementation against the supplied version 0.1 TDD, verified on October 21, 2026. Implemented behavior and completed checks are distinguished from device/provider acceptance. The delivered artifact is the native Tauri macOS app, not the Vite browser preview.

## Automated evidence

`./scripts/check.sh` runs the production frontend build, sentence-streaming tests, model-free Python worker tests, locked/offline Rust tests, and native Swift audio tests. It uses installed/cached dependencies; it does not install packages, download models, open the microphone, or contact search providers. A Rust HTTP fixture binds loopback locally and needs permission for that local socket in restricted environments.

The final frontend build and all eight speech-queue tests passed. All 38 Python worker tests passed. The Rust suite passed 42 tests; its explicitly ignored timing check was also run separately and passed. Native synthetic audio checks passed: stereo-to-mono resampling, 16 kHz PCM16 WAV, duration bounds, private file permissions, overwrite/symlink protection, and invalid arguments. These tests use labelled fixtures rather than simulated production answers.

Real local-model checks also passed:

- Qwen 3.5 9B: ordinary generation, native structured `system_info` selection, grounding in a supplied synthetic tool result, cancellation, and reuse of the persistent worker. Earlier actual-app checks also completed a real Qwen answer and Rust system/security snapshot.
- Whisper tiny.en plus Silero: a locally synthesized WAV transcribed to “Hello, Nexon, please check my system memory.” Silence returned recoverable `no_speech`. The microphone remained closed.
- The isolated project voice environment passed `pip check`. The original `pulse-venv` was not changed.

Observed Qwen smoke timings were 10.544 seconds for initial load plus `READY`, 5.226 seconds for tool selection, 7.063 seconds for the supplied-result follow-up, 68 ms for ping plus cancellation, and 2.303 seconds for worker reuse. The first-ever Whisper invocation took 26.443 seconds including imports/compilation/load; a later fresh worker took 1.293 seconds and immediate reuse of the identical waveform took 0.033 seconds. The complete system snapshot took 376 ms, including an intentional 200 ms CPU sampling interval. These are small observations, not general performance guarantees. Activation, first speech, physical playback interruption, and microphone latency still need measurement against the TDD targets.

## Delivered phases

| Phase | Delivered behavior |
| --- | --- |
| P0: Foundation | Transparent, borderless, draggable desktop character; pet-only idle surface; attached command/result bubble; menu bar controls and global shortcut; accessory activation without a Dock icon; all-Spaces/full-screen auxiliary behavior; animated task states; streaming Rust controller and persistent JSONL MLX worker. Empty areas around the expanded bubble pass clicks to the desktop. |
| P1: Offline tools | Explicit folder grants/revocation; bounded text/name search and metadata; explicit SQLite FTS5 index rebuild with cancellation; current-file revalidation of indexed hits; CPU/memory/storage/process snapshot; fixed read-only FileVault/Gatekeeper/SIP checks; local preferences, history, audit, and task shelf. An approved Software Update availability check is available separately and never installs updates. |
| P2: Online tools | Brave/Tavily adapters; Keychain credentials; exact-query/provider, expiring one-use web approvals; validated model-directed search within that approval; source URLs/timestamps; up to three selected public page excerpts; published structured offers; bounded retries/cancellation; cache, persisted request caps, and configurable estimated dollar budget. |
| P3: Voice | Native bounded push-to-talk; local English MLX Whisper; optional local Silero speech filtering; transcript review before submission; installed macOS voices and pace; sequential sentence-streamed output; cancellation of active and queued audio; private temporary-file cleanup. |
| P4: Persistent tasks | Saved searches and public-page price watches; intervals of at least 15 minutes; pause/resume/delete/check-now; persistent schedules, results, errors, and price history; target/currency validation; in-app change notices and optional macOS notifications. Unchanged checks stay quiet. Schedules execute while the app is running. |
| Customization | Name, personality instructions/presets, cat (default, with coat, eye color, accessory) or nine-tailed fox/fox/orbit, calm/playful/sleepy motion, four accent colors, companion size, installed speaking voice, pace, light/dark/system appearance, and reduced motion. Custom personality affects style without granting permissions. |

Online integrations (weather, ECB rates, Wikipedia, OpenStreetMap places, eBay listings, Duffel flight search) are delivered behind per-request approval; live eBay and Duffel acceptance needs the user's keys. Social discovery, MCP/OAuth adapters, wake words, richer speech engines, cloud models, and model switching remain future work as designated in the TDD. There are no purchases, arbitrary shell tools, file modifications, external messages, or account changes. Seller/availability fields are published source metadata, not an independent seller-reliability verdict. PDF/Word ingestion, automatic background document ingestion, and scheduling while the app is quit are not delivered.

## Acceptance matrix

| TDD criterion | Verification and remaining acceptance |
| --- | --- |
| Answer local questions offline | Real offline MLX generation and structured local-tool selection passed; actual-app chat and system/security snapshots passed earlier. Dedicated Files/System modes do not require a model. Repeat a full device run with networking unavailable after the final packaged revision. |
| Respect folder permissions | Rust tests cover component-level `O_NOFOLLOW`, hidden/symlink/sibling-prefix escapes, bounds, cancellation, root revocation, and indexed-hit freshness. App-level grant, index, outside-file absence, and revoke should be checked with a disposable folder. Previously saved excerpts remain in conversation history until cleared. |
| Cited live search | Providers, quotas, retries, approvals, response parsing, retrieval boundaries, and persistence are implemented. Local fixtures cover transport/content limits. Live Brave/Tavily success and provider error responses remain pending a user-supplied Keychain API key; no credentials were invented or external searches executed for acceptance. |
| Validated Qwen local/online tool use | Python and Rust independently validate allowed tools and arguments. Real Qwen local tool selection and grounded follow-up passed. Online tool calls are confined to an approved exact query/provider; a real online Qwen round trip remains pending a provider key. |
| Offline voice input/output | Real local Whisper/Silero synthetic-speech verification and native format tests passed. Actual microphone permission/device capture, audible playback, voice selection, and physical interruption timing remain interactive checks. |
| Approval cannot be bypassed | Model tool schemas exclude destructive/external actions. Rust owns folder grants, one-use web approvals, explicit recurring-monitor consent, OS update consent, quotas, and optional notification permission. Policy and persistence tests passed. |
| Responsive UI and cancellation | Worker ping/cancel, IPC framing, restart, stale speech tokens, sequential output, interrupted read-aloud, and Unicode-safe streaming tests passed. Controller generation guards prevent late microphone/TTS startup after cancellation; settings/monitor operations coordinate with active task claims. Interactive dragging/stop/shortcut under load remains a device check. |
| Recoverable failure | Tests cover invalid/oversized IPC, malformed tool calls, missing models/dependencies, silence, permission boundaries, transport limits, and post-error worker reuse. Missing provider keys and setup problems are surfaced. Live service outages and denied macOS microphone/notification permissions remain device checks. |

## Online and file boundaries

Web mode sends the explicitly approved query, never private file results. Retrieved text is untrusted data. Page reads accept selected public HTTPS URLs, revalidate and pin DNS, verify the connected peer, and disable proxies, redirects, cookies, credentials, scripts, and subresources. HTML/text sizes and deadlines are bounded. Unsupported pages show errors rather than invented content. See [online tool details](online-tools.md).

Price watches use published Schema.org offers with decimal-string amounts and explicit currencies. Ambiguous products, conflicting quotes/stock/sellers, aggregate ranges, and unsupported metadata cannot produce a threshold alert. There is no currency conversion or checkout-total guarantee. Pausing/deleting cancels the active monitor and prevents later retries/notices through its cancellation state.

File scans are limited to approved roots, 10,000 entries, 40 hits, and 64 KB per text file. Index rebuilds are bounded to 5,000 files/20 MiB and atomic; indexed hits reopen current files within current grants. New files require an explicit refresh. Neither cached index text nor remote page text becomes tool authority.

## Remaining interactive sequence

0. Rename migration: with the former Nexon app quit, launch Buntini once. Confirm earlier task history appears, the audit log shows `app.migrated`, `~/Library/Application Support/app.nexon.companion/migrated-to-buntini` exists, and any stored provider key still reports as configured.
1. Launch `src-tauri/target/debug/bundle/macos/Buntini.app`. Confirm the fox is visible without an opaque rectangle, native scrollbars, Dock icon, or open homepage. Check drag, bubble/collapse, menu bar, `⌘⇧Space`, multiple Spaces/full-screen, and display changes. Native screenshot capture was unavailable during prior automation; final visual appearance needs device confirmation.
2. Open Settings, change the name/character/accent/size/personality, save, and restart. Check persistence and the local model's response style. Preview an installed voice and interrupt it.
3. Grant a disposable folder, refresh the index, search a unique file, modify it and repeat, then revoke the root. Verify outside/revoked files stay unavailable. Start/stop a long model answer and immediately submit another; check for stale output.
4. Deliberately hold/release push-to-talk and complete the normal macOS microphone permission flow. Review the transcript; test silence, Escape, focus loss, playback interruption, and cache cleanup. The English model/environment are already prepared; see [voice setup](voice-setup.md).
5. Add a Brave/Tavily key in Settings, approve a query, inspect cited snippets/page evidence, cancel another approval, and test quota/error recovery. Create a saved search and an unambiguous product price watch; check pause/delete during execution and restart persistence. Opt in to desktop notifications only if desired.
6. Run an explicitly approved macOS update check; compare supported security status with macOS. The unsigned local debug bundle is suitable for development; signing/notarization and distribution remain separate release work.
