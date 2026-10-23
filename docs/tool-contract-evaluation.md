# Tool contracts and task-policy evaluation

`worker/tools.json` is the shared fixed catalog. Every tool has resource domains, read/write/mixed classification, use/avoid guidance, prerequisites, an output kind, a typed verifier identifier, verification limits, a retry policy and schema-valid examples. The Rust loader rejects missing contracts, unknown domains/verifiers, inconsistent retry policies and examples that drift from their schema. Actual executor validators and receipts remain authoritative; catalog prose does not verify an action.

Rust and Python emit only `name`, `description` and `parameters` to model providers. Python intersects caller enum restrictions with the fixed catalog and preserves mandatory fields when a task requires an additional existing field. These restrictions cannot add tools, actions or arguments or weaken the catalog.

`evals/task-contract-golden.json` contains independent request expectations for production `TaskContract` behavior: quoted instructions, coordinated negation, separate calendar dates, ordered reads/writes, required deadlines, unsupported sending, uncertain intent, requested explanations and calendar-derived dependencies. The Rust evaluator calls the production tool selection, proposal validation, serialization/reopen and completion methods. It checks exact menus and arguments at each reached step, rejects premature or unrelated effects and verifies that receipts do not silently remove a requested answer. Additional adversarial journeys check unknown writes, failed-read recovery and model-generated fields that remain unverified after approval.

These are deterministic policy regressions using synthetic source records and executor receipts. They do not measure model tool-selection accuracy, source completeness in real apps, calendar arithmetic, user intent beyond the conservative English bootstrap, or successful live app writes. In particular, approval records consent, not independent evidence that a model-proposed value is correct. User clarification supplies a value, and a trusted source binding records where it came from. An executor receipt verifies that the approved value was written or used; it does not establish the factual truth of model-generated content. Provenance retains the original source category even after execution succeeds. Raw model evaluations and live destination-app acceptance must continue separately; these fixtures must not be reported as live completion rates.

Run:

```sh
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target tools::
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target task_contract::eval_tests::
python3 -m unittest worker.tests.test_tool_contracts
```

On a supported Mac with Apple Intelligence available, rebuild `native/buntini-fm`, then run the protected controller probe separately:

```sh
python3 scripts/evaluate_controller_apple.py --output evals/apple-controller-NEW-RUN.json
```

It executes only the three explicitly listed ignored tests, captures eight controlled cases with source/helper hashes and logs, and refuses to overwrite earlier evidence. Literal note/reminder cases bypass model generation; Calendar routing uses the real Apple helper. App reads and writes remain injected fixtures. A changed source or helper during the run invalidates the report.

## Public research routing fixtures

`evals/search-intent-cases.json` is a separate corpus for the online research
gateway. Its 47 cases cover direct web and named-site requests, quoted phrases,
case, Unicode, query operators, comparison/freshness/source-quality requests,
private local lookups, negated and informational mentions, and explicit requests
to open a search-results page. The tests call the production intent parser and
check the complete saved goal, literal query content plus visible site scope for
explicitly named sites, literal requirement spans, and the
distinction between research and browser navigation.

Run these fixtures without a model, browser, Keychain access or network:

```sh
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target search_intent_tests::
```

These fixtures establish conservative routing and query preservation. They do
not establish semantic understanding of arbitrary goals, search-result relevance,
source correctness, citation entailment, or successful operation on a live search
engine. Those checks require separate source-grounded and live-browser evidence.

`search_pipeline_tests.rs` adds 17 independent fixture journeys through the
production research orchestrator and task journal. They check browser-first
ordering, exact-query API fallback, cancellation before fallback, selected-source
allowlisting, URL mismatch, fragment deduplication, private URL rejection, bounded
page reads, snippet-only evidence, comparison citations, truncation, exhausted
budget errors, strict summary coverage, false action claims, and reopening
incomplete or legacy snippet-only research before a requested note. A model
coverage gap can trigger the API once while preserving browser evidence and
source numbering. Typed claim tests check controller-generated citations, reject
unread or invalid source references, conflicting answer/claim formats and false
action claims. A browser candidate skipped by the per-route read limit remains
eligible if the API later returns it. Earlier lookup citation slots retain their
numbers, do not consume the current query's read budget, and can receive fresh
read evidence without duplicating the source. Unread historical sources cannot
support the current answer. No fixture dispatches a real app write or network
request.

The focused October 22 run passed all 4 intent tests (47 corpus cases) and all 17
pipeline/journal tests. Run the pipeline checks with:

```sh
cargo test --locked --offline --manifest-path src-tauri/Cargo.toml --target-dir src-tauri/target search_pipeline_tests::
```

`answerVerified` means that the summary passed the controller's coverage-shape
and citation-reference checks. Coverage itself is supplied by the synthesis
model. This is not a factual-entailment score, a verification of source quality,
or evidence that the model correctly applied every user constraint. Retrieval
relevance is a bounded lexical check; distinct hosts do not guarantee independent
sources. Live browser and model acceptance must be recorded separately.

The existing-browser adapter adds four controller regression tests for the
separate app-access and cloud Browser-sharing gates, merging Browser provenance
back into a task, saving/reopening that provenance, revocation blocking cloud
history and resume, and rejecting saved browser evidence before model startup or
answer output. Approval fixtures check normal-profile, cookies, sign-in and
history disclosure, the exact query, and the selected fallback provider. The
focused controller-search run passed all six offline tests; the separate Apple
model check remained opt-in. These fixtures do not read personal tabs, launch a
browser or call a model, and do not establish live browser automation success.
