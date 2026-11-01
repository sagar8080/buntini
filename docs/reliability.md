# Buntini reliability review and product direction

Reviewed October 22, 2026. The [capability map](capabilities.md) describes executable behavior and its limits. This review distinguishes code and fixture evidence from real app/model acceptance. Passing automated tests is not a measured real-life success rate.

The existing/default-browser correction passed the complete offline check suite and a final **355-test Rust run** (8 opt-in checks excluded), with **71 Python tests**, **43 frontend tests**, and **75 native contract assertions**. A live public smoke verified default Chrome selection and Google URL dispatch. macOS denied Automation to the separate test process, so source reading in installed Buntini remains unverified. A nonmutating permission preflight now lets this case open Google once through macOS and use the configured API fallback without retrying an uncertain tab creation. See the [current validation record](../evals/existing-browser-validation-2026-10-22.json) and [live observation](../evals/desktop-browser-research-smoke-2026-10-22.json).

Browser-first validation before the Google default change: the complete `./scripts/check.sh` run passed the production build, **340 Rust tests**, **71 Python tests**, **43 frontend tests** and **75 native contract assertions**. The final Rust suite, including the additional historical-source regression, passed **341 tests** (8 opt-in checks excluded). Actual Bing browser discovery/readback and Apple typed research synthesis also passed separately. The earlier failing browser and free-form synthesis attempts are retained. See [the validation record](../evals/browser-first-validation-2026-10-22.json) and [model and workflow evidence](evaluations.md). These checks do not establish a general real-life success rate.

## Standalone planning fixes — October 22, 2026

Buntini retains its own Rust task engine, native Apple helper and existing local/cloud model interfaces. No OpenClaw runtime, package, server or service was added. The implementation consulted OpenClaw's [tool-policy intersection](https://github.com/openclaw/openclaw/blob/main/src/agents/tool-policy.ts), [structured tool-error handling](https://github.com/openclaw/openclaw/blob/main/src/agents/tool-result-error.ts) and [no-progress identity](https://github.com/openclaw/openclaw/blob/main/src/agents/tool-loop-no-progress.ts) as references, without copying their implementation.

- **Unexpected actions:** tools are narrowed on every planning turn to the next unfinished user-requested write. Successful prerequisites are required before dependent writes are offered. Coordinated negations and explanations cannot authorize a later clause; distinct action sentences retain their separate obligations. Dispatch-time validation remains authoritative.
- **Lost note/reminder fields:** explicit note titles/bodies and named reminders with exact dates are bound directly from the user's instruction. A following reminder clause cannot become note content; reminder title and due stay separate through approval. Quoted content remains literal. Missing or ambiguous dates stop for correction instead of silently producing an undated reminder. This binding applies across all brains.
- **Unnecessary Calendar reads:** the exact question “What is my first calendar event on YYYY-MM-DD?” finishes directly from complete typed evidence, selecting the earliest event and ignoring instruction-like display text.
- **Wrong calendar deadlines:** complete typed calendar evidence retains each timestamp's UTC offset at observation time. The supported first-event preparation workflow selects the earliest event, performs checked time arithmetic, keeps the event identity in the reminder, and records its calculation source through approval and receipt persistence. It does not ask the language model to calculate the deadline.
- **Premature or endless follow-through:** supported action-only goals finish from verified receipts once required actions and reads are satisfied. This prevents a small model from starting another read after the approved result already exists. Calculations, translations, explanations and unrecognized continuations retain their answer phase. Their validated prose is preserved alongside the action receipts.
- **Permission recovery:** failed reads say what went wrong and offer Try again. The backend decides retry eligibility, keeps earlier verified writes, and supersedes the failed observation only after the same read succeeds. Failed/unknown writes never become automatically retryable.
- **Native routing:** source bodies and action arguments are excluded from next-tool selection. The helper honors the controller's narrowed parameter enums and deterministically stops on unresolved journal effects.

The deterministic calendar template currently covers preparation for the first event on one explicit `YYYY-MM-DD`, with one minute/hour lead time. Empty, partial, all-day, tied, malformed or unsupported event selections pause rather than guess. The literal note grammar supports one named note with `with body`, optionally paired with one explicit reminder; quoted bodies preserve line breaks and instruction-like text. Other separated clauses are refused by this narrow binder. The literal reminder grammar supports one `titled`/`named`/`called` reminder followed by `due` and an exact timestamp. Missing values can be supplied through the saved task's structured fields; partial corrections constrain subsequent proposals without reinterpreting the original ambiguous date. General scheduling, arbitrary multi-step goals and actual destination-app reliability still need broader acceptance. Raw-model failures remain visible in the evaluation reports; controller-protected success is not presented as a general model accuracy rate.

## Tool-call relevance and evidence — October 22, 2026

The standalone controller now persists a versioned task contract alongside its execution journal. It records requested read/write/answer outcomes, their order and dependencies, exact constraints, unresolved fields, revisions, and the receipt or observation satisfying each outcome. A successful note cannot hide an unfinished reminder or a separately requested explanation. Existing journals load through a backward-compatible bootstrap.

All 30 tool definitions share validated routing contracts: resource domains, when to use/avoid the tool, prerequisites, operation type, output kind, verification scope, retry policy and schema-checked examples. Only public schemas reach model providers. The controller selects tools for the next unfinished outcome and validates that call against its constraints. The MLX adapter preserves narrowed enums and additional required fields rather than restoring the full catalog schema.

Every app read now has a typed envelope with observation time, scope, records, returned count, source total and coverage. Calendar/Reminders carry native IDs and offset timestamps; Mail/Notes retain app-local IDs; browser references explicitly distinguish session IDs from snapshot positions. `complete` describes record-set coverage; per-record content flags describe truncation. Unknown totals remain null. Invalid or mismatched evidence cannot satisfy a required source read. Bounded Mail/Notes results are still bounded observations, not proof that every relevant record or its full content was examined.

Each proposed argument retains its origin: user literal/clarification, deterministic calculation with source record and field, or unverified model proposal. Approval records consent; edited fields retain their previous value and source. A successful executor receipt marks execution verified without rewriting a model proposal as source-grounded data. Paused tasks expose missing fields such as dates in chat; structured corrections are applied transactionally, saved across restart and constrained to unfinished outcomes. Approval cannot silently remove task-required fields.

Regression coverage includes clarification after an earlier completed action, restart and approval edits, required deadlines, distinct Calendar dates, malformed/partial observations, source instructions, failed-read recovery, uncertain writes and requested explanations. The [independent task-contract corpus](tool-contract-evaluation.md) exercises production selection and completion logic. These are fixture/controller checks, not live destination-app acceptance or a general model accuracy measurement. Intent bootstrapping remains conservative English extraction; arbitrary plans, unsupported actions and semantic correctness of model-proposed content remain limitations.

## Integrated browser-first research — October 22, 2026

Google is the primary browser search engine. The earlier [embedded-browser Google check](../evals/browser-research-google-default-2026-10-22.json) reached an unusual-traffic verification page on this network; that page was rejected as search evidence. Its separate public page-read fixture passed. That check and the successful Bing checks are historical evidence for the embedded-browser implementation, not acceptance of the current existing-browser route.

Foreground research now has a single transport policy in the standalone
controller. It searches Google in a dedicated tab in an allowed existing/default
browser first, ranks and deduplicates
selected public results, reads source pages, and produces a cited answer. If
browser discovery or page evidence is insufficient, the configured Brave or
Tavily API supplies fallback discovery using the same approved query. A missing
requirement identified during synthesis can also trigger that fallback once,
retaining the earlier page evidence. A source
page that cannot be read through the browser may use the existing public HTML
reader with the same exact selected-URL allowlist. Cancellation never initiates
the fallback. API quota, cache and retry accounting remain in the existing
provider adapter.

The saved research intent retains the original goal and exact query, plus
literal comparison, freshness, source-quality and answer-format requirements.
The search query preserves case, quotes and operators; an explicitly named site
becomes visible `site:` scope before approval. Explicitly opening search
results remains navigation; opening a tab or returning snippets cannot satisfy a
research outcome. Search and source reads reuse the research tab without
reading other tabs' titles, addresses or contents. The browser's normal profile can use existing cookies and
sign-ins. Browser-derived evidence needs Browser sharing before cloud synthesis
and retains that provenance in saved conversations. App and browser automation
permissions are required; Chrome, Brave and Edge support native research-tab
reading, while other browsers open Google and use the approved API and public
HTML fallback. Scripting-disabled pages can use existing Accessibility access.
Research does not fall back to an embedded WebView.

The research report saves attempted routes, selected sources, read pages,
limitations, retrieval readiness and summary status. Synthesis must declare
complete or partial coverage and missing requirements. Typed claims supply
source numbers that the controller formats into citations. The controller checks
that numbered citations resolve to pages actually read; comparisons need cited
pages from two distinct hosts, or two distinct pages for an explicitly restricted
single-site query. A malformed or uncited summary receives one bounded correction
attempt against the same evidence. False action-completion claims are rejected
before the answer is displayed. Incomplete research cannot unlock a dependent
write or become complete when its journal is reopened. Legacy snippet-only
journals also remain unverified for new dependent writes.

These checks have specific limits. Result ranking and page relevance use bounded
lexical heuristics. Two hosts do not prove editorial independence. The model's
coverage declaration and the report's `answerVerified` field are a structural
completion gate, not independent fact checking or proof that every sentence is
entailed by its citation. Arbitrary semantic intent, actual source correctness,
freshness, live search-layout changes, CAPTCHA and authenticated pages still
require acceptance testing. See [online-tool behavior](online-tools.md) and
[the independent routing and pipeline fixtures](tool-contract-evaluation.md).

## Product assessment

Buntini is already a substantial Mac companion: it combines local/cloud brains, conversation and memory, voice, scoped files, system information, Apple app reads and approved writes, deterministic music/browser/Mac controls, daily briefs and recurring public research. Its strongest starting position is helping with the user's existing Mac apps while keeping local data under explicit sharing controls.

Meta presents Muse as an agent that completes multi-step tasks across connected apps and a persistent browser computer, tracks goals, requests approval for critical actions and keeps working after the app closes. Those are advertised capabilities; this review has not measured Muse's actual success rate. [Meta's Muse product page, accessed October 22, 2026](https://ai.meta.com/muse/).

The follow-through implementation now saves task journals, resumes after approvals, verifies supported app writes, persists timers and exposes delivery failures. That closes several architectural gaps, but it does not establish head-to-head parity. Buntini still needs device acceptance, stronger measured model routing and a separate executor if work must continue while the app is quit.

## Reliability fixes made in this pass

| Failure found | Resulting behavior | Regression evidence |
| --- | --- | --- |
| Negated/informational commands could execute; “stop the timer” could pause music | Deterministic commands require a direct request and a matching target; timer cancellation reaches the timer | Skill routing/phrase tests, including negations, explanations, unrelated targets and music follow-ups |
| Overflowing/negative timer durations; browser URLs lost paths or punctuation | Invalid durations are refused; explicit HTTPS paths, query strings and case survive routing | Duration and URL fixtures |
| Mail search ignored unread filtering; malformed/truncated agenda output looked like missing data | Both filters apply; helpers return complete bounded UTF-8 or an error, and agenda rows must match their types | Mail filter, >4 KB agenda, invalid JSON/types, oversized-output and invalid UTF-8 tests |
| Accessibility page reads could bypass Chromium private-window filtering | Reading/closing the current tab checks the supported browser's front-window privacy state | Scope/command tests; actual browser behavior still needs device acceptance |
| Spotify verification checked only the title | Verification checks track and artist, and reports uncertainty when it cannot confirm the requested result | Playback matching/ranking tests; live Spotify UI remains a dependency |
| UTC timestamps became local wall-clock times; all-day reminders acquired midnight times | Zoned action times convert before approval; invalid/repeated DST times are refused; native reminder reads preserve date-only values | Rust date tests and native contract assertions |
| Invalid approval edits consumed the request; actions could overlap other work or race revocation | Preflight failure keeps the editable sheet; a one-use claim serializes execution and checks current permissions atomically | Controller claim, expiry, revocation, contention and one-use tests |
| Follow-ups/actions lost original data categories; health context was tagged only as activity | Included history, live context and proposed actions carry their data provenance into later cloud-sharing decisions | Context/history provenance tests and controller action tests |
| Tool narration appeared before a tool had actually run | Tool-capable model turns withhold speculative text; only a validated final answer is delivered as an answer | Scripted JSONL tool round trip captures emitted chat deltas |
| Failed summaries left queued draft speech | Final-answer reconciliation replaces queued stale speech and respects cancellation | Speech replacement, matching-stream and cancelled-token tests. Already spoken audio cannot be retracted. |
| Disconnected/truncated cloud replies could succeed; malformed arguments could become `{}`; extra calls were silently dropped | Incomplete replies and invalid/multiple calls fail explicitly; provider reasoning/signatures survive follow-ups | Offline OpenAI, Anthropic, Gemini and ChatGPT stream fixtures |
| Missing usage metadata refunded accepted cloud requests | Reservations remain unless complete usage is reported; an explicit zero is distinguished from missing data | Usage-reporting tests |
| A non-reading local worker could block cancellation; MLX token exhaustion appeared successful | Queue waits, startup, writes and reads observe cancellation; invalid final messages and MLX token-limit exhaustion fail and can recover | Scripted blocked-worker, queued-cancel, shutdown, malformed-completion/recovery tests and the MLX adapter length test |
| Cancelled monitors overwrote their comparison baseline; result/schedule writes could split | Saved-search results and baselines commit atomically; cancelled searches/watches preserve the last verified baseline | Transaction rollback and cancellation-baseline tests |
| Native helper silently substituted invalid dates/arguments and retained finished tasks | Invalid inputs fail explicitly; completed/cancelled tasks are removed without deleting a replacement with the same ID | Native contract self-test, run by `scripts/check.sh` |

The UI also blocks duplicate submissions synchronously, keeps validation errors inside the action sheet, and records action execution failures in conversation history. Direct deterministic commands and model-proposed approval flows are now distinguished in the README, guide and privacy description.

## What must be proven on real devices

Use the [existing acceptance sequence](acceptance.md#remaining-interactive-sequence) with disposable records and controlled browser windows. For each enabled brain and app integration, record the exact build, request, approved fields, resulting object/state, latency and any intervention. Include successful runs, denied permissions, cancellation, retries and restart/sleep recovery. Do not mark an integration verified just because its schema fixture passes.

Start with three complete daily workflows:

1. **Plan my day:** compare the brief and follow-up questions with actual recurring/all-day calendar events and due reminders. Check DST and travel-timezone cases. Surface native 40-event/50-reminder limits rather than interpreting partial lists as complete.
2. **Turn a request into a useful record:** create a reminder/event/note or prepare a Mail draft, edit its fields, cancel once, then approve once. Confirm the exact destination contents. Simulate timeout/restart and establish whether a retry duplicates the record.
3. **Find and keep track of something:** retrieve cited evidence, create a saved search/price watch, verify a meaningful change, and pause/delete during execution. Confirm quiet unchanged checks and sleep/restart behavior.

Voice transcription, audible interruption, foreground/private browser selection, Spotify recording selection, actual permission prompts and live keyed providers remain acceptance work. Apple's helper now receives the complete saved workflow journal and can propose successive tools. Its requested response-token limit silently truncated a live probe, so that limit was removed; bounded transport output and execution deadlines remain. Most importantly, the revised actual Apple routing probe passed only **5 of 9 phases**: three unwanted reminder proposals and a wrong derived calendar time remain. No app effects were dispatched in that probe. The controller blocks proposals absent an explicit user action request, stops failed prerequisites and withholds unverified completion claims, but those defenses do not turn incorrect model arguments into correct ones. See [evaluation evidence](evaluations.md).

## Implementation of the five priorities

| Priority | Implemented behavior | Evidence and remaining limit |
| --- | --- | --- |
| Verified action receipts | Reminders, Calendar, Notes and Mail drafts return destination IDs and verify exact created fields. Results are succeeded, failed or unknown. Approved intents are claimed durably before dispatch; repeated approval returns the saved task instead of writing again. Model-proposed music/tab controls verify observed state; memory/timers/monitors use transactional local receipts. | Typed-result, mismatch, idempotency and rollback fixtures; native scripts syntax-compiled against installed app dictionaries. Actual app writes/readback still require disposable-record acceptance. Open/Shortcut receipts attest only to their narrow handoff/CLI result. |
| Durable task execution | SQLite journals preserve the goal, incrementally selected steps, model messages, input provenance, approvals, evidence and receipts. One task can cross several action/online approvals, resume after restart, accept missing details and stop future work on cancellation. Pending approval sheets restore exact fields. Interrupted writes become unknown and cannot replay. | A synthetic two-app journey restarts between approvals, completes both steps and refuses duplicate dispatch. Additional tests cover interrupted reads, revoked sharing, duplicate edited inputs and storage failure after dispatch. This proves controller recovery, not arbitrary model planning or app correctness. Twelve recorded steps maximum. |
| Durable time and delivery | Timers persist with due time and delivery state. Overdue timers/checks run once on return and record lateness. History shows last success, next check and alert failures. Interrupted delivery is unknown and is not silently repeated. | Restart, overdue, single-claim, cancellation and failed-delivery fixtures. Closing the window keeps execution running; quitting/sleeping stops it. No background helper, hosted executor, wake alarm or delivery-time guarantee. |
| Scenario evaluations | Versioned scenarios, routing/tool contract fixtures, explicit per-brain release scoring and a reusable real Apple probe. Complete task success, false success, unintended action, duplicates, intervention and latency are scored separately. | [Evaluation guide and recorded failures](evaluations.md). Apple currently fails the multi-step gate. Other configured brains and real app interactions have not passed this new end-to-end gate. Do not infer task accuracy from fixture pass counts. |
| Broader browser/app work | Saved searches and price watches can be created within the durable workflow with approved recurring scope and stable receipts. Separately from foreground research, the approved `browse_page` workspace on macOS 14+ returns bounded URL/title/text observations, with persistent per-origin profiles and a verified network gate. | Eight policy/protocol tests and a disposable native blank-WebView isolation/deletion probe pass. Live remote/authenticated-site acceptance is not established. Cross-origin navigation, popups and downloads are blocked; manual sign-in may be limited by those boundaries. Arbitrary form filling, sending, bookings, purchases and account changes are not implemented. |

## Conversation continuity

Conversations and tasks have different lifetimes. A saved conversation contains the user's related turns; individual tasks retain their own progress and approvals inside that thread. New topic starts a separate conversation. Opening a past conversation must load the thread and scope subsequent model context to it. Legacy history is preserved without guessing boundaries that were never recorded.

## Completion and recovery rules

- A final app-action response comes from verified receipts. Common omitted requested actions and unsupported success claims keep the task unfinished.
- A model proposal cannot authorize itself from tool output. The explicit saved user goal controls which actions can be proposed. Ambiguous phrasing can require clarification in Additional task details; this conservative English-language check is not a semantic proof.
- Explicitly requested prerequisite reads require saved successful evidence before dependent actions or completion. Repeated requests for the same action kind require separate receipts, including explicit object counts. Failed prerequisite reads stop the task. Unknown writes stop all subsequent work. Neither a restart nor a repeated click may silently repeat the mutation.
- Resuming uses the original brain identity and current privacy permissions. Changing a brain or revoking a source category requires a new request rather than leaking the old journal to another provider.
- Cancellation stops future work. It does not undo already completed actions, and a write interrupted after dispatch may remain unknown.
- Clearing history removes conversation/task journals; Delete everything also removes persistent monitor/control intent tombstones and schedules. Destination records already created in other apps remain there.

The remaining release gate is a real, recorded task suite on each supported brain and integration, including incorrect arguments and app failures. The implementation must report those failures honestly until that evidence exists.
