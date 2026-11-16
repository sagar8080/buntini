# Mac at a glance

"How's my Mac?", "why is it slow?", "what's filling my disk?" and "what's using my
battery?" get a picture, not a paragraph. The picture answers the question first and
offers one thing to do about it.

Status: G1, G2, G3 and the interactions below are built and not yet run in a signed release.
G4 isn't started. Each card and interaction was checked in the browser preview with sample
data, and the readings and a real quit were run on one MacBook. Not done yet: VoiceOver and Reduce Motion in the built app, an
app with unsaved work, energy figures under real load, and thermal states, since this Mac
has never reported one. App icons on the bars aren't built.

## The card

Read top to bottom, it answers in one look:

1. **A verdict with an icon:** "Your Mac is fine", or the one thing that isn't.
2. **Three rings:** storage, memory and battery. A Mac with no battery shows two. Memory's
   ring shows the state, never used of total, because macOS keeps memory full on purpose.
3. **Busy, last hour:** a CPU line.
4. **Using the most:** up to three apps as bars.
5. **Up to two buttons** that fit the verdict.

The question decides the emphasis. "What's filling my disk?" leads with a large storage
ring, "what's using my battery?" with the battery ring and apps by energy, and "why is it
slow?" with memory, CPU and apps. Nothing here needs a model, and no sentence explains
what the graphics already show.

Charts are hand-written SVG in the app's own theme tokens, so no chart library is added.
They follow light and dark themes and Reduce Motion, and every ring and line has a text
label for VoiceOver. State is never shown by colour alone.

## Interacting with it

Everything here changes only the card: nothing is sent, no message is added and no model runs.

- **Rings open.** Tapping a ring opens its detail in place. Small rings then move between
  details, and tapping the open one goes back to all three. A question opens on its own
  detail: "why is it slow?" on Memory, with the Busy chart.
- **The disk can be pointed at.** Pointing at a part, on the ring or in the list, dims the
  rest and shows its size in the ring's middle. A tap keeps it there.
- **The chart answers "why was it slow earlier?"** It switches between Busy, Memory and
  Battery, and between the last hour and the last 24 hours. Pointing at it reads that moment.
  The 24 hours are kept with the card in ten-minute steps, each holding its worst minute, so a
  spike isn't averaged away. Memory reads as Easy, Tight or Struggling, never as a percentage.
- **Apps rank by any measure:** CPU, Memory or Energy. Tapping an app shows all its numbers
  and, when it can be asked to, Quit. So any app the card lists can be quit, not only the one
  the verdict names, and it still goes through the approval sheet.
- **The latest card is live.** It updates every 5 seconds, or 10 with Energy showing, while
  Buntini has focus and nothing is running. Rings and bars move to their new values, and
  Reduce Motion stops that. Live readings are shown and never saved, so the turn keeps the
  card as it was asked for. Older cards stay as they were, and offer no Quit.

An update costs 38.7 ms of CPU in a debug build, about 0.8% of one core while it's being looked
at. `cargo test --lib glance::tests::measure_live_read_cost -- --ignored --nocapture` repeats it.
Starting each read from scratch cost 566 ms, so one reader is kept between reads.

## Where each number comes from

Each of these was run read-only on this Mac, with no permission prompt, unless the
row says otherwise.

| On the card | Source | Notes |
|---|---|---|
| Free and total storage | `sysinfo` (already used) | |
| Apps, caches | Folder sizes of `/Applications` and `~/Library/Caches` | Slow on a large disk, so measured in the background and kept for a day |
| Your folders | Only folders already approved in Settings → Privacy | No new folder access |
| Everything else | Total used minus the above | Can be large |
| Memory state | `kern.memorystatus_vm_pressure_level`, `vm.swapusage` | Pressure is the real answer. Shown as Easy, Tight or Struggling |
| Battery level, time left | `pmset -g batt` | |
| Battery health, cycles | `system_profiler SPPowerDataType` | Slow, so read once a day |
| CPU | `sysinfo` (already used) | |
| Apps by CPU and memory | `sysinfo`, rolled up | See below |
| Apps by energy | `top -stats pid,power` | Its numbers still need checking under real load |
| Heat | Thermal state from macOS | Shown only when the Mac is hot. No warning has been recorded here, so it's untested |

**Storage can't match System Settings.** macOS gives apps no API for that breakdown, so
the ring shows what Buntini can measure honestly, plus "Everything else". Trash needs a
permission Buntini doesn't ask for, so it stays in "Everything else".

**Apps, not processes.** Helpers roll up to the app they belong to using its bundle path, so
Chrome's many helpers read as Chrome. A process with no app of its own belongs to the app
that started it, so a Terminal's `cargo` counts as Terminal. System processes group as
"macOS", anything else as "Background tasks", and neither is ever offered to quit. Neither is
Buntini, or a helper macOS doesn't list as a running app.

## Verdict rules

Fixed rules, tested as a pure function. The first match wins:

| Condition | Verdict | Buttons |
|---|---|---|
| Free storage under 5% or 10 GB | "Your disk is nearly full" | Storage settings |
| Memory pressure critical | "Your Mac is short on memory" | Quit the app using the most |
| Thermal state serious or critical | "Your Mac is running hot" | Quit the busiest app |
| CPU above 80% for 5 minutes | "{App} is keeping your Mac busy" | Quit it |
| Battery under 15% and not charging | "Plug in soon" | Quit the app using the most energy |
| Battery health under 80% | "Your battery has aged" | Battery settings |
| Otherwise | "Your Mac is fine" | None |

The thresholds are starting points to tune on real Macs.

The question can add its own next step, up to two buttons in all: Storage settings for a
storage question, Battery settings for a battery question, and for "why is it slow?", quitting
the app using the most when it's using at least 10% of the CPU, or any amount while memory is
short.

## History

A sampler adds one point a minute: CPU, memory pressure and battery level. The card shows
the last hour, and 24 hours are kept in the local database.

- **Numbers only are sampled.** The history table has no column for a name, because which
  apps someone uses is more private than how busy their Mac was. A card the user asked for is
  kept with that turn, with up to nine apps (the top three by each measure), like any other
  reply. Live readings aren't kept.
- The sampler must not show up in the chart itself. The target is 0.1% of one core, 60 ms of
  CPU a sample, and it measured 6.4 ms (0.01%) on one MacBook, `pmset` included.
  `cargo test --lib glance::tests::measure_sampler_cost -- --ignored --nocapture` repeats it.
- Delete everything and data export both include it.

## Asking

A quick-path skill, `skills/system.rs`, recognises the phrases and answers with no model,
so it's instant on every brain and works when a brain isn't ready. The phrases become
routing tests, as for other skills. Anything else, such as "is 16 GB enough for video
editing?", goes to the model as before.

The card also opens from the companion menu, as "Your Mac", and "Check my Mac" on a nudge
opens it too.

The model's `system_info` tool shows the same card beside the model's own reply. A question
to the My Mac mode about FileVault, Gatekeeper, integrity protection or updates keeps those
words above the card, since the card doesn't cover them.

**Privacy:** the card goes to no one. A cloud brain sees this data only if "Mac health"
is allowed in Settings → Privacy, so a turn showing the card is recorded with
`Share::System` and is never replayed to a brain that isn't allowed it.

## Actions

- **Quit an app:** a new `quit_app` action through the approval sheet. The button sends
  "Quit {App}" as the user's message, and typing the same thing works when it names one open
  app; "quit smoking" names none, so it goes to the model. It asks the app to quit, so the
  app can ask about unsaved work, and brings it forward after a moment so that prompt can be
  answered. Buntini never force-quits and never offers system processes. It succeeds only
  when the app is actually gone, and otherwise reports that it's still open. Models aren't
  offered this tool, and nothing with nobody at the window can propose it.
- **Storage settings, Battery settings:** open the pane, with the same mechanism as
  "Open System Settings" for Apple Intelligence.
- **Left out on purpose:** emptying Trash, deleting caches or files, and restarting. They
  can't be undone or lose work, and need their own design.

## Phases

**G1 The card (built).** Snapshot additions, verdict rules, SVG rings, `skills/system.rs`, and
replacing `SystemCard`.

**G2 Apps and history (built).** Rolling processes up to apps, the sampler, the CPU line, energy.

**G3 Actions (built).** `quit_app` and the settings buttons.

**G4 Always in view (not started).** A small glance from the cat. A desktop widget needs a WidgetKit
extension built with Xcode inside the Tauri bundle, the same unknown as App Intents, so it
gets a test build before any promise.

## Acceptance

**G1**
- Each phrase reaches the card with no model and no brain ready.
- A Mac with no battery shows two rings and no battery verdict.
- The verdict function is tested for every row and for each row beating the ones below it.
- Light, dark, Reduce Motion and VoiceOver are each checked in the built app.
- The mock-up's width holds at the panel's narrowest size.

**G2**
- Chrome and its helpers read as one app. No system process appears as an app.
- Restarting Buntini keeps the trend, and a gap shows as a gap, not a line across it.
- The database holds no app names.
- The sampler's cost is measured, and its own use is below the target.

**G3**
- Nothing quits without approval, and an app that refuses is reported as still open.
- An app with unsaved work shows its own save prompt.

## Decisions

1. **History length.** 24 hours. It keeps the data small and answers "why was it slow
   earlier?".

## Later

**Reaction from the cat.** It could look tired when the disk is nearly full or the Mac is
hot, which means less text, but it's easy to overdo. It stays out until the card works.
