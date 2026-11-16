# Brain setup

Every Mac gets a working brain on day one, chosen by Buntini. Nobody needs an API key,
a Terminal or a technical decision. ChatGPT works beside Apple Intelligence as a helper,
and only ever sees what the user approved.

Status: B1, B2 and B3 are built and not yet run in a signed release. B3 has never talked to a real ChatGPT account. It builds on `codex/apple-intelligence-status` (the
stuck-download state, "Use ChatGPT instead" and the sharing question).

## The rule

| The Mac has | Main brain | Helper |
|---|---|---|
| Apple Intelligence ready | Apple Intelligence | ChatGPT, when signed in |
| No Apple Intelligence, ChatGPT plan usable | ChatGPT | None |
| Neither | Local model | ChatGPT hand-off |

One pure function decides it, so it can be tested without a Mac in any state:

```rust
pub fn pick(apple: &AppleStatus, chatgpt: ChatGptPlan, local_ready: bool) -> Brain
```

**"ChatGPT plan usable" is what sign-in returns, not a plan name.** OpenAI's quickstart
says Plus and Pro, and its help article also mentions Go for commercial apps. Buntini
reads the sign-in's plan scope and the `subscription_sharing_*` errors instead of keeping
a list of plans.

### Automatic, until the user chooses

- New field `brain_auto`, true on a new install.
- An existing install keeps what it has (`brain_auto` false), so an update never moves
  someone's brain.
- Picking a brain in Settings → Brain sets it to false. A text button there, "Let
  Buntini choose", sets it back. There is no other new setting.
- API keys stay in Settings → Brain for people who want them. The rule never picks them.

### When the state changes

Only with `brain_auto` on, only between tasks, never during one:

| What changed | What happens | What the user sees |
|---|---|---|
| Apple Intelligence becomes ready | Moves to Apple Intelligence | "Now using Apple Intelligence." once |
| Apple Intelligence stops being available | Moves down the rule | "Apple Intelligence isn't available, so I'm using ChatGPT." once |
| ChatGPT hits its limit or can't be reached, and the local model is ready | That one answer comes from the local model | "ChatGPT isn't available right now, so this came from your Mac." |
| The same, with no local model | No answer | The plan error, and an "Open ChatGPT" hand-off |

`watch_apple` already re-probes every 30 seconds and emits `apple-status`. It stops once
`apple_may_change` is false, which stays correct.

## Setup

The brain step in first-run setup:

- **Apple Intelligence ready:** nothing to choose. It shows as ready, and Continue moves on.
- **Not ready:** two buttons, **Sign in with ChatGPT** and **Set up on this Mac**, the second
  with its download size. The Apple advice and "Open System Settings" stay for the states
  a person can fix.
- **Sign-in works but the plan doesn't qualify:** say so in one line and go straight to
  **Set up on this Mac**.

The three-way picker and API key field leave setup and stay in Settings → Brain.

## One-click local model

This reverses a deliberate rule, written in `advisor.rs` and
`worker/requirements.txt`: the app never installs packages or downloads models. Both
comments and `docs/guide.md`, which gives the Terminal commands, change with it.

Buntini fetches three things into `<app data>/local-brain/`:

| Piece | Source | Pinned by |
|---|---|---|
| Python, arm64 | python-build-standalone release | Exact release and SHA-256 in `worker/local-brain.json` |
| `mlx-lm` and its dependencies | PyPI | `worker/requirements-local.lock`, `--require-hashes --only-binary :all:` |
| The model the advisor picks | Hugging Face (`mlx-community`) | Commit revision and each file's SHA-256 in `worker/local-brain.json` |

`scripts/pin_local_brain.py` writes the two pin files. The lock holds every wheel for every
macOS on Apple silicon, because one resolved on one Mac names that Mac's macOS and would
fail elsewhere. The set needs macOS 14 or newer.

The existing worker runs unchanged. It already loads models with
`trust_remote_code=False` and runs with Hugging Face offline, so once set up the
local brain needs no network. `settings.python_path` and `model_path` point into the new
folder.

The advisor keeps choosing the model by chip and memory, and the user isn't asked. One click
only fetches models under an open licence, because Buntini is the one downloading them: the
Qwen models are Apache 2.0, while Gemma 4 E2B and E4B use Google's own Gemma licence. So a
16 GB M4 Air gets Qwen 9B (6 GB), not the advisor's first choice of Gemma 4 E4B. Settings →
Brain → "Use a model I already have" still leads to every model, set up by hand.

### The steps

1. **Check:** Apple silicon, and free disk for the download plus a margin. If either fails,
   say what's short and offer ChatGPT.
2. **Show the size** on the button. Pressing it is the consent to download.
3. **Download** with one progress bar across all three pieces, a percentage, resume after a
   dropped connection or a restart, and Cancel.
4. **Verify** every hash before anything is unpacked or run. A mismatch deletes that file
   and stops.
5. **Install** with pip's isolated mode, wheels only.
6. **Test** with one short answer through the real worker.
7. **Switch** only after the test passes.

### Never stuck

Friends' complaint about Apple Intelligence was a download stuck with no visibility, so:

- No bytes for 60 seconds shows "Waiting for the connection" and retries with backoff.
- Progress and the current step survive quitting Buntini.
- Every failure names its step and offers Try again.
- Buntini stays usable during the download. Chat says the local model is X% ready.

### Keeping it tidy

- **Remove the local model** in Settings → Brain frees the disk.
- Delete everything removes the folder as well.
- The folder is excluded from Time Machine.
- Unpacking refuses any path that leaves the folder.

## ChatGPT as the helper

Beside Apple Intelligence, ChatGPT gets **the question and nothing else**: no history,
memory, files or sharing ticks. As the main brain the sharing ticks apply as they do today.

| When | What happens |
|---|---|
| A public internet question (`look_up_online`), signed in | The approval sheet names ChatGPT and shows the exact question. Allow once sends it with hosted `web_search`, and the answer comes back with its sources. |
| The same, not signed in | Today's route: Google in the browser, then Brave or Tavily |
| ChatGPT can't take it (web search refused, a limit, no connection) | The sheet for today's route. The user approved ChatGPT, so the browser is never used without asking. Search stays with the browser for 10 minutes, or until the next sign-in if ChatGPT refused it |
| "Ask ChatGPT …" | The same sheet, with the user's words |
| Apple Intelligence or the local model can't answer | An **Ask ChatGPT** button on the latest reply. It's never automatic. |
| Images, or a question ChatGPT itself couldn't answer | An **Open in ChatGPT** hand-off |
| No usable plan | The hand-off, on the reply and on the browser route's sheet |

The hand-off copies the question and opens chatgpt.com. A pre-filled link would need `?q=`, which OpenAI doesn't document and reports disagree about, and it would put the question in the address and the browser's history.

The reply always shows the exact question that was sent, under "Asked ChatGPT".

`ModelHub::ask_chatgpt` makes the call with settings derived for ChatGPT, so it uses the
existing sign-in, streaming and errors. Nothing else answers if ChatGPT can't, since ChatGPT
is what the user approved. `Settings` itself doesn't change. `helper.rs` holds the request
and the rules.

`cloud.rs` requests `web_search` on this route only, and `helper::sources` reads
`url_citation` annotations into sources. The decoder keeps `web_search_call` items without
treating them as tool calls for Buntini to run.

### Always allow

The sheet for a public internet question has **Always allow** beside Allow once. After
that, such questions go to ChatGPT without the sheet, and the reply shows the exact
question that was sent.

The sheet still appears when:

- Anything local was read in this task (mail, a file, a page, memory), so private text
  can't leave inside a question nobody reviewed, including through instructions hidden in
  that content.
- The request came from a routine, a goal or anything else running with nobody at the window.
  A task saved before this was recorded counts as one of these.
- The question has a word the user didn't write, so a model can't add something it remembers
  or saw, or it's longer than 300 characters.
- The question is something other than a public internet question. "Ask ChatGPT" and the
  reply button always show it.

Turning it off is a toggle in Settings → Privacy, shown only once it's on. Signing out of
ChatGPT and Delete everything also reset it. Today's browser and search routes keep their
own approval.

Plan-only ChatGPT, where it plans and never sees results, is a separate spec.

## Phases

**B1 The rule.** `pick`, `brain_auto`, the setup screen, switching and its notices.
Settings → Brain's "Let Buntini choose".

**B2 One-click local.** Pin script, lock file, `local_setup.rs`, progress UI, removal,
docs. Ships in the same release as B1, or free-account friends on Macs without Apple
Intelligence get a Terminal screen.

**B3 The helper.** Web search and sources, "Ask ChatGPT", the reply button, the hand-off.

## Checks before building

| Check | Why it matters | Phase |
|---|---|---|
| What a free ChatGPT account returns at sign-in and on first request | The whole "doesn't qualify" path depends on it | B1 |
| Downloaded Python and MLX run under a signed, notarized release on a clean Mac | Passed under a hardened Developer ID parent with Buntini's entitlements. The notarized app on a clean Mac is still to do | B2 |
| Real download sizes for Python and packages | Measured: Python 25 MB, packages up to 119 MB, 526 MB unpacked | Done |
| Licence terms for each model fetched automatically | One click fetches Apache 2.0 models only. The Qwen and Apache licences themselves still need a read before release | B2 |
| Tool-call evaluation on Qwen 4B on an 8 GB Mac | The smallest supported brain becomes some people's whole experience | B2 |
| Hosted `web_search` on a real account and model, and that replies carry `url_citation` annotations | OpenAI lists it as "subject to model and account/workspace policy". The request and the reading of replies are tested only on hand-written examples | B3 |
| The exact errors a refused `web_search` returns | Decides how long search stays with the browser. The code treats a capability error, a 400 and a 403 as lasting | B3 |
| Whether a link can pre-fill a question in ChatGPT | Not relied on: the hand-off copies the question instead | Done |

## Acceptance

**B1**
- `pick` is tested for every Apple state with each ChatGPT state and local state.
- An update leaves an existing install's brain and `brain_auto` untouched.
- A brain never changes during a running task, and each switch is announced once.
- A manual pick is never overridden.

**B2**
- From a clean Mac with no Python, one press leads to a local answer, with no Terminal and
  no folder picker.
- A tampered file at any of the three pieces stops setup before it runs.
- Pulling the network mid-download, then quitting and reopening, resumes without
  starting over.
- Cancel and Remove leave nothing behind in `local-brain/`.
- After setup, the local brain answers with the network off.

**B3**
- Nothing reaches OpenAI without the sheet, except on a Mac where ChatGPT is the main brain
  or under Always allow.
- With Always allow on, a task that read anything local still shows the sheet, and so does
  any run with nobody at the window.
- With Always allow on, the reply shows the exact question sent.
- A helper request contains only the question shown in the sheet, checked by a test on the
  request body.
- Answers show their sources, and a refused `web_search` falls back to today's route.

## Decisions

1. **Web search goes to ChatGPT first when signed in.** It gives better answers and uses the
   account people pay for, and OpenAI sees those questions. The browser route is the fallback.
2. **Falling back to the local model when ChatGPT is unavailable.** It's in the spec because
   it sends less data, not more, though answers may be weaker.
