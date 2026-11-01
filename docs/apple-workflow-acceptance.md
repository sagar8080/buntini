# Apple Intelligence everyday workflow acceptance

October 29, 2026: **31 of 38 positive user workflows completed**, plus **2 of 3 boundary checks**, across **103 actual Apple Foundation Models SDK calls**. The requested 30-workflow target is met. **The failures below are still open in v0.2.0.**

All five final cohorts used the same test binary, helper and source fingerprints. Source facts were synthetic; local files, persistent profiles, Notes/Reminders/Calendar records and free public API responses were real. These are backend acceptance tests, with a separate disposable WebView/IPC check. They do not certify the signed release bundle or permission onboarding on a fresh Mac.

The raw results and independent reviews remain unchanged. One automated subscription failure is a wording-recognition error; manual inspection confirms the selected unused items, kept items and $33/month and $396/year. Its wrong model proposal is retained alongside the authoritative literal-input binding.

Machine-readable evidence: [summary and hashes](../evals/apple-workflows-final-summary-2026-10-29.json).

## Completed user workflows

| # | Area | Verified workflow |
|---|---|---|
| 1 | writing | Rewrite a late-delivery email |
| 2 | writing | Turn meeting notes into assigned next steps |
| 3 | writing | Split a shared weekend bill |
| 4 | writing | Review recurring subscriptions |
| 5 | writing | Compare supplied laptop offers |
| 6 | writing | Draft a reply from a pasted invitation |
| 7 | writing | Build a tailored packing checklist |
| 8 | writing | Scale a recipe into a shopping list |
| 9 | writing | Summarize an office policy accurately |
| 10 | writing | Rewrite a team announcement |
| 11 | writing | Translate a practical travel message |
| 12 | files | Read an actual lease note |
| 13 | files | Retrieve two trip details from a file |
| 14 | files | Compare two actual vendor quotes |
| 15 | files | Find a repair note with an unrelated filename |
| 16 | files | Read an indexed document after it changes |
| 17 | files | Find a file added after explicit index build |
| 18 | files | Extract a decision after a 160-character prefix |
| 19 | files | Summarize a real agenda containing hostile assistant instructions |
| 20 | files | Use actual saved app memory in a recommendation |
| 21 | files | Recall an older relevant memory after many newer unrelated facts |
| 22 | files | Restart app state and continue an actual file conversation |
| 23 | apps | Read a grocery note and identify missing ingredients |
| 24 | apps | Read literal amounts and symbols from Notes |
| 25 | apps | Create a checklist Note from rough meeting follow-ups |
| 26 | apps | Create a Reminder with a derived return deadline |
| 27 | apps | Create a Calendar event with duration and room |
| 28 | public | Retrieve Seattle weather and suggest walking preparation |
| 29 | public | Retrieve and summarize cited Space Needle background |
| 30 | public | Find two nearby cafés with exact names and addresses |
| 31 | controls | Ask the assistant to remember a work preference, approve the actual Store write, restart and recall it in a new conversation |

## Remaining blockers

| Scenario | Observed failure |
|---|---|
| `travel-itinerary` | Schedule starts Library09:30 before10:00 opening, Museum11:50 before12:00 opening and Library13:20 after13:00 closing. Visit/walk durations are inconsistent. Presence of all destination names does not establish a valid itinerary. |
| `history_new_thread_isolation_restart` | The old thread correctly replies Larkspur. The restarted new thread has no old-history provenance and does not leak Larkspur, but invents the project name The Future of Work instead of acknowledging that it is unknown. This boundary case fails despite the privacy isolation holding. |
| `notes-compare-quotes` | Actual scoped source and verified75+40=$115 calculation, but recommends A instead of B. The given <=$5 difference rule selects earlier Wednesday B. No recommendation compliance. |
| `notes-handoff-summary` | Reply lists only Alex and Mina as owners, omits user as third owner and does not preserve explicit owner-task associations. Correct deadlines alone do not fulfill the requested three-owner checklist. |
| `notes-project-next-step` | Model-generated checklist adds unsupported slide-order and microphone tasks, omits owner. Exact acceptance gate blocks write; only real seed is cleaned. |
| `notes-travel-checklist` | Correct07:25 derived due time but reminder title drops explicitly required namespace. Exact gate blocks write; seed cleaned. |
| `notes-to-calendar-workshop` | Correct45-minute duration but proposes2026 rather than source2031 and omits Study Room. Exact gate blocks Calendar write; seed cleaned. |
| `control_parallel_cooking_timers` | After the first timer the model attempts its duration again; duplicate protection stops it before the second distinct timer is created. |

The failed action proposals were rejected by the acceptance harness before dispatch. Its exact expected-field gates are test controls, not a claim that production automatically detects every semantic error. Production retains its normal permission, proposal review, write-ahead checkpoint, executor validation and receipt checks.

A correct receipt proves the approved app fields were written and read back. It does not by itself prove the model selected the right source fact, date, owner or conditional rule.

## Repairs and verification

- Required source reads, literal title/query/location binding, bounded plural search and fresh-file discovery; longer bounded excerpts and relevant memory retrieval.
- Structured local numeric operations and checked output rendering; literal supplied-table bindings retain raw model proposals in provenance.
- Correct quoted reply roles and removal of unsupported assistant signatures from user-authored drafts.
- Real Notes and Calendar write/readback repairs, structured checklist arguments, trusted terminal receipts, cited public summaries and source-derived walking suggestions.
- 445 Rust regressions, 72 worker tests, 43 frontend tests and 148 native assertions pass. The final disposable bundled WebView/Apple IPC check passes.

## Release boundary

These repairs shipped in v0.2.0, which CI signed and notarized on November 1, 2026. That artifact still needs acceptance on a fresh Mac. The eight failed scenarios above are still open; keyed/cloud and interactive microphone/Accessibility/browser checks remain outside this local/free-provider run.

Raw local evidence is retained under `evals/apple-workflows-*-final-2026-10-29.jsonl`, with SHA-bound independent reviews and earlier failed attempts. No paid provider calls, email sending, purchases or personal content reads were used.
