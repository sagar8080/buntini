# Buntini: ranked backlog and next iteration

Every item comes from the verified findings, with duplicates merged. Line references are the ones the reviewers cited. I re-checked the main code paths: `skills/day.rs:90-95`, `controller.rs:2536-2539`, `controller.rs:1630-1640`, `controller.rs:2706-2709`, `memory_index.rs` refresh and start, `focus.rs:109-130` and `focus.rs:200-233`, `skills/this.rs:35-46` and `completion.rs:179-183`.

Effort scale: S is under half a day, M is 1–3 days, L is about a week, XL is several weeks.

---

## 1. Ranked backlog

### A. High-severity correctness and security fixes

**A1. A memory refresh can put back data the user deleted** (high; reported 4 times and merged)
- **Goal:** Delete everything, Clear history, Forget, Remove folder and turning an app off can never be undone by a refresh that is already running.
- **Why:** `refresh()` copies settings, memories, tasks and files once (`memory_index.rs:489-521`). It then waits minutes for Notes, Mail, 91 Calendar helper calls and Reminders, and only then calls `replace_source` for every source (`:565-576`). Nothing checks the store again before writing. The `cancel` flag (`:612`) is a fresh `AtomicBool` that nothing can set. So a deleted fact or cleared chat comes back into search through `visible()`, because the `memory` and `history` sources are never filtered. Mail and Notes rows also stay on disk after an erase. This breaks the promise in PRIVACY.md:14.
- **Files:** `memory_index.rs`, `store.rs` (`forget` 224, `remove_root` 165, `save_settings` 136, `erase_everything` 458, `clear_history` 490, `delete_task` 249), `controller.rs` (`AppState`), `data.rs:227-258`, `lib.rs:179`.
- **Effort:** M.
- **Acceptance:** A new test erases, forgets and clears between the read phase and the write phase. Afterwards `counts()` is zero and `search("vegetarian")` is empty. An app turned off between read and write gets no rows.

**A2. Background reads open Mail and Notes on their own** (high; reported twice and merged)
- **Goal:** Buntini never opens Mail or Notes unless the user asked for something that needs them.
- **Why:** `NOTES_RECENT` (`apps.rs:371`), `MAIL_LIST` (`:310`), `MAIL_TEXT` (`:335`) and `MAIL_TRIAGE` (`:663`) have no `is not running` guard. The other read scripts do (`:393`, `:434`, `:473-645`, `:700`, `:715`). `memory_index::start` runs 90 seconds after every launch and then every 6 hours. The automatic morning brief also runs triage. The result is that Mail and Notes open unprompted and Mail starts syncing.
- **Files:** `apps.rs`, `memory_index.rs`, `brief.rs`, `skills/day.rs`, `controller.rs` (morning_brief at 1297-1305).
- **Effort:** S–M.
- **Acceptance:**
  - A background refresh with Mail closed leaves Mail closed and keeps the existing mail rows.
  - The automatic brief with Mail closed says the inbox was skipped.
  - "What needs a reply?" can still open Mail.

### B. Medium and low security and correctness fixes

**B1. "My day" is tagged with only one data category** (medium security; reported 3 times and merged)
- **Goal:** A turn records every data category its answer contains.
- **Why:** `day.rs:90-95` reports only Mail, or only Calendar. The calendar events, reminders and battery line in the same answer go unlabelled. `conversation::shareable` (`conversation.rs:25-32`) and `memory_index::history_records` (`:424`) both trust `task.touched`. So a cloud brain allowed only History and Mail receives calendar content.
- **Files:** `skills/mod.rs:27-31`, `skills/day.rs`, `brief.rs`, `controller.rs:2536-2539`, and every place that builds `Outcome::Done`.
- **Effort:** S.
- **Acceptance:** A "My day" turn that shows events and mail is not replayed to an external brain allowed only `[history, mail]`, and its history record carries Calendar.

**B2. Search caps results before filtering them, so it can wrongly report "nothing matches"** (medium)
- **Goal:** `search_memory` returns up to 8 hits this brain may see, if that many exist.
- **Why:** `controller.rs:1634-1640` takes 24 hits and only then filters with `visible`. Mail hits that are hidden from this brain can fill all 24 places and push out a matching note.
- **Files:** `controller.rs` (memory_tool), `memory_index.rs` (`search`, `visible`).
- **Effort:** S.
- **Acceptance:** With 30 matching mail records and 1 note, and an external brain not allowed Mail, the note is returned.

**B3. A memory question that mentions files fails when no folder is shared** (medium)
- **Goal:** A recall request is always offered `search_memory`.
- **Why:** `files_off` sets `no_tools` (`controller.rs:2706-2709`) even when the contract requires `search_memory` (`task_contract.rs:255`). The task then ends as "unfinished".
- **Files:** `controller.rs`.
- **Effort:** S.
- **Acceptance:** "What did Ana say about the lease document?" with no folders shared runs `search_memory` and completes.

**B4. "That" and "it" are treated as pointing at the screen** (medium)
- **Goal:** Follow-ups such as "summarise that" act on Buntini's last answer, and text pasted after "translate this to Spanish:" is not swallowed.
- **Why:** `this.rs:35-46` lists "that" and "it" as vague targets and ignores `recent`. The language branch accepts up to 3 words, so pasted text becomes part of the language name.
- **Files:** `skills/this.rs`, `skills/mod.rs:275-289`.
- **Effort:** S.
- **Acceptance:** "summarise that" after an answer goes to the model. "Translate this to Spanish: good morning" does not become `Command::About`.

**B5. Any "this", "her" or "him" pulls the on-screen item into the request** (medium)
- **Goal:** Only read the screen when "this" means an on-screen object.
- **Why:** `mentions_this` (`focus.rs:126-130`) matches "this weekend" and "her birthday". It then runs MAIL_SELECTED or FINDER_SELECTED, puts the result in the prompt and tags the task with Mail, Selection or Files.
- **Files:** `focus.rs`, `controller.rs:2649-2663`.
- **Effort:** S.
- **Acceptance:** "weather this weekend" and "gift for her birthday" do not mention the screen. "Summarise this email" and "reply to her saying yes" do.

**B6. The snapshot of "this" is reused after the user switches apps** (medium)
- **Goal:** "This" means what is in front now.
- **Why:** `current()` (`focus.rs:109-123`) returns any snapshot under 600 seconds old and never compares it with `LAST`.
- **Files:** `focus.rs`, `App.tsx` (capture when the input gains focus).
- **Effort:** S.
- **Acceptance:** After a switch from Mail to Safari and back to Buntini, "summarise this" no longer uses the Mail snapshot.

**B7. Any "reply to …" / "respond to …" becomes a required email draft** (medium)
- **Goal:** Only a reply to a person or an email creates a `draft_email` obligation.
- **Why:** `completion.rs:179-183` matches any clause that starts with "reply to" or "respond to".
- **Files:** `completion.rs`, `task_contract.rs`.
- **Effort:** S.
- **Acceptance:**
  - Should give `[draft_email]`: "reply to this saying yes", "write back to her", "reply to Ana's email".
  - Should give `[]`: "Reply in French", "respond with one word", "respond to this question in one sentence: …", "reply to the following in a friendly tone: …".

**B8. Triage says "nothing needs a reply" past the 12 emails it opens** (medium)
- **Goal:** Say when coverage is partial.
- **Why:** `READ_AT_MOST` is 12 (`triage.rs:12`, used at 76). Direct emails it never opened land in "look" with the reason "sent to you", and `partial` ignores this limit (`:83`).
- **Files:** `triage.rs`.
- **Effort:** S.
- **Acceptance:** With 13 direct emails where only the 13th asks something, the answer does not start with "Nothing…" and mentions the limit.

**B9. Past-chat indexing keeps only the newest 100 tasks** (medium; reported 3 times and merged)
- **Goal:** Index up to `HISTORY_TURNS` (1,000) completed turns.
- **Why:** refresh uses `store.tasks()`, which is limited to 100 (`store.rs:273`). `replace_source` then deletes everything older.
- **Files:** `store.rs`, `memory_index.rs`.
- **Effort:** S.
- **Acceptance:** Index 150 completed tasks, then run a second `replace_source` with the same input. All 150 records remain and their vectors are kept.

**B10. "This" reads a browser's window title or selection without checking browser permission or private windows** (low security)
- **Goal:** The model path uses the same browser checks as the browser skill.
- **Why:** `focus.rs:202-207` and `:226-229` never call `apps::allowed` or `ensure_public_window`. A private window's title is saved in history and the index, and goes off the Mac when the brain is external with Browser or Selection allowed. On-device brains may see it, per the data-access principle.
- **Files:** `focus.rs`, `controller.rs:2648-2662`.
- **Effort:** S.
- **Acceptance:** A browser that is not allowed, or a private window, returns Err with the same messages the browser skill uses.

**B11. "Reply to this" has no recipient when text in the email is selected** (low)
- **Why:** The Selection referent wins over Mail (`focus.rs:202`), so `ReplyToThis::prepare` (`recipes.rs:126`) binds nothing.
- **Files:** `focus.rs`, `recipes.rs`.
- **Effort:** S.
- **Acceptance:** A selection inside Mail plus "reply to her saying yes" binds `to` and `subject` from `selected_mail()`.

**B12. "This" errors are dropped on the model path** (low)
- **Why:** `if let Ok(found)` at `controller.rs:2651` hides "Allow Mail…" and the password-field refusal, while the skill path shows them. Impact is limited because chat_tools hides apps that are not allowed.
- **Files:** `controller.rs`.
- **Effort:** S.
- **Acceptance:** Extract `this_context()` and add tests showing the refusal appears, a disallowed referent is not sent, and an allowed one is tagged.

**B13. Query length is counted in bytes, not characters** (low; reported twice and merged)
- **Why:** `controller.rs:548` checks `q.len() > 300`, which counts bytes, while `tools.json:57` allows 300 characters. The sibling checks do the same.
- **Effort:** S.
- **Acceptance:** A 120-character Japanese query is accepted.

### C. Reliability

**C1. Embedding batches use too much memory, and the model stays loaded forever** (medium)
- **Goal:** Keep memory spikes from embedding within about 100 MB.
- **Why:** Batches of 32 are padded to the longest text, up to 512 tokens. Each attention layer then holds four (B,12,L,L) f32 tensors, a peak near 1 GB. The F32 model (about 133 MB) lives in a static `OnceLock` for the whole session.
- **Files:** `embedding.rs:29,42,51`, `memory_index.rs:579`.
- **Effort:** S–M.
- **Acceptance:** Batches of 8 or fewer, sorted by length. A test shows a text embeds the same alone and in a padded batch (dot product ≥ 0.98). The model is released after N idle minutes.

**C2. Refresh ignores active chats and battery, and blocks the query embedding** (low)
- **Goal:** Refresh steps aside for the user.
- **Why:**
  - It does not check `state.active` or battery (`memory_index.rs:606-622`).
  - It holds the global embedder mutex while the chat's own query embedding waits (`controller.rs:1625`).
  - It runs 91 helper processes for Calendar, one per day.
- **Files:** `memory_index.rs`, `embedding.rs`, `controller.rs`, `world.rs` (pmset parsing), `native/fm/main.swift` (ranged `--events`).
- **Effort:** M.
- **Acceptance:** Refresh waits while `active` is non-empty or on battery. A query with the embedder busy falls back to keyword search within 300 ms. Calendar is read with one ranged helper call.

**C3. The Mail index is expensive and fragile** (medium, merged into the A2 area)
- **Why:**
  - No last-refresh time is saved, so every launch re-reads 90 days of inbox.
  - MAIL_TEXT does one lookup per id.
  - One timed-out batch fails the whole source (`apps.rs:2202`).
- **Effort:** M.
- **Acceptance:** A successful refresh time is stored per source and the launch run is skipped if it was recent. A timed-out batch keeps the other batches as a partial result.

**C4. The focus poller has no autorelease pool** (low)
- **Why:** It calls AppKit every 500 ms on a thread that never exits (`focus.rs:57-75`). The same applies to the capture thread (`:85`).
- **Effort:** S.
- **Acceptance:** Each loop iteration is wrapped in `objc2::rc::autoreleasepool`. An optional follow-up is to observe the app-activation notification instead of polling.

**C5. sysinfo 0.40 paths are only covered by ignored tests** (low)
- **Effort:** S.
- **Acceptance:** Non-ignored macOS tests `local::tests::system_snapshot_reads_this_mac` and `world::tests::health_reads_root_disk`.

### D. UX and accessibility (ordered by user harm)

1. **Settings edits are lost or never retried** (medium; `SettingsView.tsx:33-42`, `controller.rs:297-344`). Save immediately when Settings closes. When `active` is busy, queue the save in the backend or retry it. Toggles show the last saved state. Effort S–M.
   - Acceptance: toggle Mail and press Esc at once; Mail is still on after reopening.
2. **Clear conversation history has no confirmation** (medium; `App.tsx:586`). It also wipes the journal, the file index and memory records. Reuse the inline confirm from YourData. Effort S.
3. **Approval sheets claim to be modal but do not trap focus** (medium; `ActionSheet.tsx:17`, `Online.tsx:13`). Make siblings and `.pet-dock` `inert`, trap Tab, restore focus on close, and block `submit` while a proposal is waiting. Effort S.
4. **The welcome actions dead-end on a fresh install** (medium; `App.tsx:559`, `setupSteps.ts:7`). Add an Apps step to setup, or an inline "Allow Mail" button. Let `setPage('settings', tab)` open a specific tab. Effort S–M.
5. **The error toast covers the send and mic buttons** (medium; `styles.css:270`). Place it above `.composer-area`. Do not toast errors already shown in the thread. Effort S.
6. **Contrast fails in light mode** (medium; `themes.ts:19,45-49`, `styles.css:18,36-44`). Derive `--on-accent` from luminance, clamp custom accents to 4.5:1, and darken `--muted`. Effort S.
7. **The morning brief runs invisibly** (low; `App.tsx:469-477`). Give it the same busy state as a chat, with a typing bubble and Stop. Effort S.
8. **Smaller items** (each S):
   - Focus is lost after every message (`App.tsx:570`).
   - The privacy text says messages "always go to a cloud model" even on-device (`PrivacyTab.tsx:47`).
   - Saved searches can be created without a search key (`App.tsx:649`).
   - Text size does not scale settings, sheets and labels (`styles.css:888`).
   - Users see technical copy (`App.tsx:587,594`, `Online.tsx:18`, `TaskProgress.tsx:11-13`).
   - The menu button is too small, has no `aria-expanded`, and Escape collapses the whole panel (`App.tsx:634,243`).
   - JS smooth scrolling ignores reduced motion (`App.tsx:261`, `Onboarding.tsx:32`).
   - Pet visuals: the play burst is usually skipped (`usePetLife.ts:89`) and the yarn ball falls with the cat (`Pet.tsx:19`).
9. **Simplify setup and Settings** (low; `Onboarding.tsx:116-138`, `BrainSettings.tsx:31-74`, `SearchBudget.tsx`, `Connections.tsx`, `Customization.tsx`):
   - Reuse LocalModel and ModelAdvisor in setup.
   - Drop the search-provider step.
   - Take prices and cost per search from built-in tables.
   - Reduce the cosmetic options.

   Effort M. *Done in iteration 2: setup is three questions and Settings drops the technical knobs; saved values from earlier versions still load.*

### E. Test and evaluation gaps not covered by a fix above

- **E1.** Commit a synthetic recall fixture (`evals/memory-recall.json`). Add `memory_index::tests::fused_recall_on_the_synthetic_fixture_meets_the_documented_floor`, asserting fused top-3 ≥ 8 and meaning top-3 ≥ 9 with the bundled model. Effort S–M.
- **E2.** Add scenario journeys: reply-to-open-email, remind-about-this, memory-recall-respects-privacy and needs-reply-partial. Add tool_contract cases: search-memory-valid, search-memory-extra-field, search-memory-too-long and search-memory-unoffered. Effort S.

### F. Capability roadmap (ordered by real usefulness while staying local-first)

1. **Keep working across quit, login and sleep: phases A–C** (M). Open at login through SMAppService, OS-scheduled timer notifications, and a scheduler run on wake. This is what makes timers and watches trustworthy. *Next iteration.*
2. **Approval inbox and actionable "Review" notifications** (M). Lets work pause for approval without the window open. Routines and goals depend on it. *Next iteration. Iteration 3: changes to existing items (done, moved, changed, added to) have their own lock-screen-safe Review wording.*
3. **Find people in Contacts** (M). "Email Ana" works today only if the model guesses the address. *Next iteration.*
4. **Threaded Mail replies** (M). Bind `in_reply_to` from `Referent::Mail.message_id` and from triage ids, then use Mail's `reply`. Makes "needs a reply → reply to it" actually useful. Depends on B11 and B7. *Done in iteration 2: `reply_email`, bound from Mail's selected message or the saved triage list, with a receipt that checks recipient, subject, text and thread headers. Mail's reply behaviour needs device acceptance. Iteration 3: an email neither open nor listed is found by an inbox search for the sender the user named (one match bound, several picked in the sheet, none a question).*
5. **Change existing items** (L). complete_reminder, update_calendar_event and append_to_note, with item ids bound to the source, before/after shown in the sheet, and a receipt that reads the item back. Makes "My day" actionable. *Done in iteration 2 (see [capabilities](capabilities.md)); the EventKit and Notes writes still need device acceptance. Iteration 3 added moves by an amount of time ("push it back an hour", "tomorrow same time") and `reschedule_reminder` for a reminder's due date; its EventKit write and alarm moves need device acceptance too.*
6. **Scheduled routines** (L). Requires `run_task_headless(app, prompt, sink)`, the routines table and item 2. The morning brief becomes a built-in routine. *Done in iteration 2: `create_routine`, the routines table, `run_task_headless`, and History → Routines with Pause and Delete. Runs need Buntini open and the Mac awake. Iteration 3: Mail and Notes reads a run's model or recipes ask for (`read_mail`, `find_notes`) leave a closed app closed (`Launch::for_request`).*
7. **Read PDF, Word, RTF and Pages files in the helper** (M). Merge the duplicate extension lists (`local.rs:130-155`, `indexer.rs:61-82`). Parse in the helper process with caps. *Done in iteration 2: one list in `local::content_kind`; `--extract-document` in the helper. Pages files without a PDF preview stay name-only.*
8. **Proactive nudges from local data** (M). Meeting prep, people waiting on a reply, reminders due. Needs C3's cached triage and A2's no-launch reads. *Done in iteration 3 as Heads-ups (one switch, off by default): meeting prep with time, place, invited names and keyword-matched notes or emails; reminders past due; people waiting on a reply from an in-memory inbox sort read only while Mail is open. Hashed dedupe, per-kind limits, learned active hours and Focus (only when macOS lets Buntini read it). No leave-now heads-up, since Buntini has no location. Alerts and click-to-open need device acceptance.*
9. **List the user's Shortcuts for run_shortcut** (S). Smart home through Shortcuts. Cheap, and could be done earlier as filler. *Done in iteration 2: `list_shortcuts` bound before any model call, and `run_shortcut` limited to a listed name. Real HomeKit shortcuts need device acceptance.*
10. **Hands-free voice** (L). Partial results, then end-of-speech detection and conversation mode, then an optional wake word last. *Done in iteration 3: live partial words from Apple's on-device SpeechTranscriber (no audio file), end of turn on a pause, a conversation mode that listens again after a spoken answer, barge-in with Buntini's own words filtered out by text, and the recording/Whisper fallback. Verified with synthetic speech through the helper's live pipeline; microphone capture, echo on speakers and barge-in timing need device acceptance. The wake word is left out: there is no on-device keyword API with negligible power.*
11. **Goals that span days** (L). Needs item 6.
12. **MCP client for local stdio servers** (L). *Done in iteration 3: Settings → Connections → Tools you added starts a server from a pasted command or a picked Claude/Codex/Cursor/VS Code config, with a minimal environment, version negotiation, timeouts, size caps, kill on hang and a bounded restart policy. A request that names the server gets one call, shown in the approval sheet unless the user marked that tool safe to read; replies are untrusted evidence under the new "Tools you added" share. Tools whose schemas need nested objects are not offered, remote servers are out of scope, and real third-party servers plus Apple Intelligence routing to them need device acceptance.*
13. **Codex and Claude Code as opt-in brains through a Buntini MCP mode** (L). Needs item 12 and the approval inbox.
14. **Supervised browser operator** (XL). Highest risk. Do it after items 2 and 6 have proven the approval inbox and receipts.
15. **Direct Gmail and Google Calendar connectors** (L). Last, because Mail.app and Calendar.app already cover Google accounts and Gmail's restricted scopes block a public release.
16. **Headless background helper** (XL), phase D of item 1. Move the scheduler, delivery and routine runner into an SMAppService agent that shares the WAL store.

---

## 2. Next iteration

**Scope:**
- The two high fixes: A1 and A2.
- Three cheap privacy and data fixes in the same code: B1, B9 and B13.
- Capability items F1 (phases A–C), F2 and F3.

Everything else stays in the backlog.

### N1. Refresh cannot write back deleted or revoked data (A1, includes B9)

**Store (`store.rs`)**
- Add the field `index_epoch: std::sync::atomic::AtomicU64` and the methods `pub fn index_epoch(&self) -> u64` and `fn bump_index(&self)`.
- Call `bump_index()` inside `forget`, `remove_root`, `clear_history`, `erase_everything`, `delete_task`, and in `save_settings` whenever `purge_disallowed` runs.
- Add `pub fn history_for_index(&self, limit: usize) -> Result<Vec<TaskRecord>, String>`. It runs `SELECT data FROM tasks ORDER BY created_at DESC LIMIT ?` with limit `HISTORY_TURNS * 2`, then filters in Rust exactly as `history_records` does. This is B9.

**AppState (`controller.rs:16`)**
- Add `pub index_cancel: Arc<AtomicBool>`. `memory_index::start` (`memory_index.rs:611-613`) uses this flag instead of creating its own.
- The erase command (`data.rs:227`) and `clear_history` (`controller.rs:445`) set it. `refresh` clears it at the start of each run.

**Split `memory_index::refresh` into two phases**
- `async fn read_app_sources(settings: &Settings, models: &ModelHub, cancel: &AtomicBool) -> Vec<(&'static str, SourceRead)>`
  - `enum SourceRead { Records(Vec<Record>), Skipped /* not running, not allowed or cancelled */, Failed(String) }`
  - Covers notes, mail, calendar and reminders only.
- `fn apply(store: &Store, epoch_at_start: u64, app_sources: Vec<(&'static str, SourceRead)>) -> Result<RefreshResult, String>`
  - All of it runs under the store lock.
  - Re-read `settings()`, `memories()`, `history_for_index()` and the file_index rows, and build the memory, history and files records there, so they are never stale.
  - For each app source:
    - If `store.index_epoch() != epoch_at_start`, or the app is no longer allowed, or the read is `Skipped`, do not write.
    - On `Failed`, keep the existing rows.
  - Call `purge_disallowed(db, &current_settings)` at the end.
- The embed loop checks `cancel` and the epoch before each `set_vector`. It is fine that a `set_vector` for a deleted id affects 0 rows.

**Tests (`memory_index::tests`)**
- `erase_during_refresh_is_not_undone`: build app sources, call `store.erase_everything()`, then `apply` with the old epoch. Assert that `counts()` is all zero and `search("vegetarian")` is empty.
- `forget_during_refresh_is_not_undone`.
- `clear_history_during_refresh_is_not_undone`.
- `an_app_turned_off_before_apply_is_not_written`.
- `a_skipped_source_keeps_its_rows`, which A2 relies on.
- `store::tests::memory_history_source_reads_up_to_history_turns`: 150 tasks give 150 records, and a second apply keeps the vectors.

### N2. Background reads never open Mail or Notes (A2)

**`apps.rs`**
- Add `pub enum Launch { IfRunning, Allow }`.
- Give `NOTES_RECENT`, `MAIL_LIST`, `MAIL_TEXT` and `MAIL_TRIAGE` a guarded version that starts with `if application "Mail" is not running then return "__buntini_not_running__"` (Notes for the Notes script). Pick the version by `Launch`.
- Thread the parameter through `recent_notes(days, launch)`, `recent_mail(days, launch)` and `unreplied_mail(..., launch)`.
- Map the sentinel to a typed `Err(AppsError::NotRunning)`. If the error type is a `String`, use a constant `NOT_RUNNING` and a helper `is_not_running(&str)`.

**Callers**
- `memory_index` uses `Launch::IfRunning` and maps NotRunning to `SourceRead::Skipped`.
- `brief::gather` takes a `launch` argument. The automatic brief from bootstrap (`controller.rs:214-217`) passes `IfRunning`. When Mail was skipped, `compose` adds "Mail isn't open, so I skipped your inbox."
- Explicit requests pass `Allow`: the "My day" chip, `Command::NeedsReply` and the `read_mail` tool.

**Tests**
- `apps::tests::background_scripts_check_the_app_is_running`: the IfRunning script text contains `is not running` for all four scripts.
- `brief::tests::a_skipped_inbox_is_stated`.
- `a_skipped_source_keeps_its_rows` (from N1).

### N3. Every data category a turn read is recorded (B1)

- **`skills/mod.rs:27-31`:** change `Outcome::Done { answer, share: Option<Share> }` to `Done { answer, shares: Vec<Share> }` and update every constructor (`grep -rn "Outcome::Done" src-tauri/src`).
- **`brief.rs`:** add `impl Day { pub fn shares(&self, battery_low: bool) -> Vec<Share> }`:
  - `Calendar` if `events.is_some() || reminders.is_some()`
  - `Mail` if `mail.is_some()`
  - `System` if `battery_low`
- **`skills/day.rs:90-95`:** return `shares: day.shares(battery_low)`.
- **`controller.rs:2536`:** `for share in shares { touch(task, share); }`.
- **Check `morning_brief` (`controller.rs:1297`):** if it saves a task or conversation turn, it must touch the same shares.
- **Tests:**
  - `skills::day::tests::my_day_touches_every_category_it_read`: events and mail, reminders only, nothing allowed, battery low.
  - `conversation::tests::a_my_day_turn_is_not_shared_without_calendar_permission`: an External brain with `[history, mail]`, where `touched = [mail, calendar]`, means `shareable` is false.

### N4. Query length counts characters (B13)

- In `controller.rs:548`, use `q.chars().count() > 300`. Apply the same to the sibling checks whose tools.json schema uses `maxLength`.
- Add a test with a 120-character CJK query.
- Add the tool_contract cases `search-memory-valid` and `search-memory-too-long` to `evals/scenarios.json`.

### N5. Keep working across quit, login and sleep, phases A–C (F1)

**Phase A: Open at login**
- Add `login.rs`. It calls `SMAppService.mainAppService` through `objc2-service-management`, or a `--login-item register|unregister|status` command in buntini-fm.
- Status values: `enabled | requiresApproval | notFound | unavailable`. Report `unavailable` for unsigned or dev builds and never claim success there.
- Add the setting `open_at_login: bool` in `types.rs`, default false.
- `save_settings` applies the change and records an audit entry.
- Show one switch in Settings → Buntini, its status in `background_status` (`delivery.rs:430`), and an "Open at login" choice on the setup Done step.

**Phase B: Timer alerts survive quit**
- In `delivery::schedule` (`delivery.rs:153`), also add a `UNNotificationRequest` with identifier `timer-<id>` and a `UNTimeIntervalNotificationTrigger` (seconds until due, minimum 1).
- The in-app claim, cancel and `erase_everything` all call `removePendingNotificationRequestsWithIdentifiers` and `removeDeliveredNotificationsWithIdentifiers` before emitting.
- On launch, `delivery::reconcile()` checks timers that are past due against `getDeliveredNotifications`:
  - If the alert was delivered, mark it delivered with receipt channel `"os_scheduled"`.
  - Otherwise deliver it now.
- Keep the LIMITATION text in delivery.rs:9 and update docs/capabilities.md. Sleep still delays alerts.

**Phase C: Catch up on wake**
- Observe `NSWorkspaceDidWakeNotification` on the main thread (or the didActivate notification together with C4) and trigger `scheduler::tick_now()` and the delivery claim.

**Tests**
- `delivery::tests::scheduling_registers_one_os_request_and_claim_removes_it`, using a trait `OsNotifier` with a fake.
- `delivery::tests::reconcile_does_not_realert_a_delivered_timer`.
- `data::tests::erase_clears_pending_os_requests`.
- `login::tests::unbundled_build_reports_unavailable`.

### N6. Approval inbox and a "Review" notification (F2)

**Store and controller**
- Add `Store::pending_approvals() -> Vec<PendingApprovalSummary { task_id, approval_id, tool, description, created_at }>` from workflows in the paused or waiting state (the states queried at `store.rs:273`).
- Check first whether the in-memory `pending_actions` entries (`controller.rs:25`, which have an `expires`) survive a restart. If they do not, persist what is needed to rebuild the sheet, or show the expired entry as "Ask again".
- New IPC command `pending_approvals` returns those summaries. `open_approval(approval_id)` returns the stored fields and goes through the existing claim path, so a stale id is refused.

**`notifications.rs`**
- Register the category `buntini.approval` with the actions `review` (foreground) and `later`.
- Add a `UNUserNotificationCenterDelegate` through `objc2::define_class!`. `didReceiveNotificationResponse` emits the Tauri event `approval-open { approvalId }` on the main thread without blocking.
- The notification body is the tool's human description only, for example "Buntini drafted an email and needs your OK". It never includes field values, because the body shows on the lock screen.

**Controller**
- When a task stops at a proposal and the window is hidden or collapsed, send the notification right away, not only on completion (replace the 30-second rule at `controller.rs:3351-3367` for this case).

**UI**
- A "Waiting for you" list in `TaskProgress.tsx`, shown only when the list is not empty.
- `App.tsx` listens for `approval-open`, expands the panel and opens `ActionSheet` with the restored fields. Never offer approve-from-banner.

**Tests**
- `store::tests::pending_approvals_lists_paused_proposals_once`.
- `controller::tests::a_stale_approval_id_is_refused`.
- `notifications::tests::approval_body_has_no_field_values`.

### N7. Find people in Contacts (F3)

**Native helper (`native/fm/main.swift`)**
- Add `--find-contact <query>`. It uses CNContactStore with the given name, family name, nickname, email and phone keys and returns at most 5 matches as JSON `[{ "id", "name", "emails": [..], "phones": [..] }]`.
- Return `{"error":"denied"}` when permission is refused.

**Permissions and categories**
- `Info.plist`: add `NSContactsUsageDescription`.
- `Entitlements.plist`: add `com.apple.security.personal-information.addressbook`.
- `apps.rs` APPS: add a `contacts` entry, opt-in like the others.
- `privacy.rs`: add `Share::Contacts` ("Names, emails and phone numbers from Contacts").

**Binding the recipient**
- New read tool `find_contact { name: string ≤ 100 }` in tools.json and `apps::find_contact`.
- In `source_fields.rs` and `task_contract.rs`, bind `draft_email.to` from `find_contact` evidence in the same journal, and only when there is exactly one contact with exactly one email.
- With several matches, leave `to` blank and pass the candidates to ActionSheet as a picker. Never bind from model text.
- The tool's result touches `Share::Contacts`.

**Tests**
- `source_fields::tests::one_unambiguous_contact_binds_to`.
- `source_fields::tests::two_matches_leave_to_for_the_user`.
- `privacy::tests::contacts_need_their_own_external_share`.

**Out of scope:** indexing contacts in the memory index. If that is added later, `visible()` must learn about Contacts.

---

## 3. What is still unknown

- **macOS behaviour:**
  - Whether a background `tell application "Mail"` shows Mail's window or only puts it in the Dock on macOS 26.
  - Whether the `is not running` guard also stops the Automation permission prompt.
- **Login and signing:** whether `SMAppService.mainAppService` works with the current signing and notarisation setup, and how "requires approval" in System Settings looks to users.
- **Notifications:** whether OS-scheduled timer notifications fire on time after quit, during Focus modes, and after a restart. Whether removing the request in the claim prevents a double alert in practice.
- **Embedding:** the real peak memory and time per batch on an 8 GB Mac with a real Notes and Mail corpus, and whether candle's CPU BERT handles F16 or BF16 weights correctly.
- **Real-inbox performance:** how long Mail, Calendar and Reminders reads take on large real accounts. That sets the C2 and C3 thresholds and the triage limit of 12.
- **Contacts:** how the TCC prompt looks for the separate helper binary, since macOS attributes permissions per executable, and whether users accept a Contacts prompt.
- **Product decisions for the user:**
  - Whether "What needs a reply?" may open Mail when it is closed (this spec says yes).
  - Whether the automatic morning brief should exist at all.
  - Whether Open at login is offered during setup.
- **Pending approvals:** whether pending proposals survive a restart today (`PendingAction.expires` is held in memory). Reading the journal code will settle this before N6 starts.

**Areas with no verified findings.** Reviewers reported nothing for the write and receipt path (`actions.rs`), approvals, `security.rs`, `browser_workspace.rs` and `browser_session.rs`, `cloud.rs`, `chatgpt.rs`, `model.rs` brain selection, the scheduler outside saved searches, and `delivery.rs` correctness. Two findings were refuted: the Apps tab hanging on "Looking at your apps…", and the redesign hiding the shortcut mentions. This means nothing was confirmed there, not that those areas were proven clean.