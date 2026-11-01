# Apple Intelligence everyday workflow acceptance

November 1, 2026: **36 of 38 user workflows** and **3 of 3 boundary checks** pass automatically, across **91 actual Apple Foundation Models calls**. Three of them Rust now completes with no model call. The two automatic failures are checker false negatives; both answers are correct (see below). On October 29 the same suite completed 31 of 38 and 2 of 3.

The run used one test binary and helper against the current source, with synthetic source facts and real local files, persistent profiles, Notes/Reminders/Calendar records and free public API responses. These are backend acceptance tests. They do not certify a signed release bundle or permission onboarding on a fresh Mac, and **these fixes are not in v0.2.0**.

Machine-readable evidence: [the full run](../evals/apple-workflows-all-2026-11-01.jsonl), and baselines of the writing, controls and files groups against the source before these fixes ([writing](../evals/apple-workflows-writing-baseline-2026-11-01.jsonl), [controls](../evals/apple-workflows-controls-baseline-2026-11-01.jsonl), [files](../evals/apple-workflows-files-baseline-2026-11-01.jsonl)). The October 29 evidence is unchanged: [summary and hashes](../evals/apple-workflows-final-summary-2026-10-29.json).

## Fixed since October 29

Each fix takes the value from the user's words or the record read, in Rust, rather than asking the model to get it right.

| Scenario | What went wrong | Fix |
|---|---|---|
| `travel-itinerary` | Visits scheduled before opening and after closing. | A walking plan that fits the stated opening hours, visit length, walking time and start is worked out in Rust (`worked::itinerary`). Completed with no model call. |
| `notes-compare-quotes` | Totals right ($115, $120), but the "$5 or less, choose the earlier date" rule was not applied. | A choice made by the note's own "lowest cost unless within $N, then earlier" rule is worked out in Rust (`worked::rule_choice`). |
| `notes-handoff-summary` | Owners and deadlines listed separately; the user, as third owner, was dropped. | Each "who does what by when" sentence in the note stays one checklist line, with "I" as "You" (`worked::assignments`). |
| `notes-project-next-step` | The checklist copied "test the microphone" and "verify the slide order" from an example in the helper's prompt, and dropped the owner. | The example is gone. A checklist made from a note drops items most of whose words appear in neither the request nor the note, and keeps a labelled fact the request asks for ("Owner: me"). |
| `notes-travel-checklist` | The exact reminder title was dropped. | A request that opens with when it happens ("On May 20, 2031, remind me …") now has its "Use exactly the reminder title …" sentence bound. |
| `notes-to-calendar-workshop` | 2026 instead of the note's 2031, and no "Study Room". | An event copied from one note takes its date, start, length and place from the note's text when each is stated once (`note_record` provenance). |
| `control_parallel_cooking_timers` | The second timer repeated the first one's 12 minutes. | Each timer clause binds its own length. Completed with no model call. |
| `history_new_thread_isolation_restart` | In a new thread after restart, the model invented a project name. | "What is my …?" with nothing about it in the conversation, memories, screen or this turn's reads gets "I don't know your … yet. What is it?" |

## Regressions found and fixed

A baseline of the current source before these fixes also failed three cases that passed on October 29, and the full runs showed model slips on two more:

| Scenario | Cause | Fix |
|---|---|---|
| `file_added_after_index` | "I just added my Marigold warranty note. Please find it" searched for "Marigold warranty note", because a newer clause parser had already created the search without a query. | The declared name ("Marigold warranty") binds that search. |
| `file_content_only_lookup` | "The repair appointment is Tuesday at 14:20" was cut as an unread calendar claim, though a file read this turn stated it. | A time a file or note read this turn states is supported. |
| `file_scope_hidden_symlink` | The checker didn't recognise "didn’t find" with a typographic apostrophe. | Checker fix. |
| `create-training-calendar-event` | The model moved 14:15 to 15:15. | An event whose request gives the date and times binds them. Completed with no model call. |
| `create-library-return-reminder` | The model didn't subtract the 15-minute lead. | A reminder's due time is the request's time less the lead it names. |

## Checker false negatives

- `subscription-audit`: the answer cancels Video and Fitness, keeps Music and Storage, and saves 33 a month and 396 a year, but says "Selected items (unused)" rather than "cancel". The same wording was adjudicated as a pass on October 29.
- `memory_dietary_dinner`: one vegetarian, peanut-free dinner. The checker didn't count recipe steps that say "cooked" or "fluff" rather than an exact verb from its list. It now matches verb stems, with a unit test; it was not re-run live after that change.

## How cases count

A case passes on its own checks. One that Rust completes with no Foundation Models call is marked `completedInRust` in the report and counted in the summary as `passed_without_a_model_call`; before November 1 such a case counted as a failure. A correct receipt proves the approved app fields were written and read back. It does not by itself prove that the right source fact, date, owner or rule was selected, which is what each case's checks are for.

## Verification

- Live: the full suite above. All 36 Notes, Reminders and Calendar test records created across the three apps runs were removed again; Notes may keep copies in Recently Deleted.
- Automated: 703 Rust tests (including new tests for each fix), 72 worker, 85 frontend, 24 release-tooling tests and 217 helper contract assertions pass, as do the native audio, live speech and scenario checks. Three `local::tests` document-reading tests fail intermittently when the whole Rust suite runs in parallel, on `main` as well; they pass on their own.

## Release boundary

These fixes need review, a new release candidate, signing and notarization, and acceptance of that candidate on a fresh Mac. Keyed and cloud providers, and interactive microphone, Accessibility and browser checks, remain outside this local, free-provider run. No paid provider calls, email sending, purchases or personal content reads were used.
