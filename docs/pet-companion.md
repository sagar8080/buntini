# Buntini: a cat that's yours

Spec for the next iteration, November 3, 2026. Part 1, petting, is ready to build. Part 2, a cat that knows you, is scoped here and gets its own spec once part 1 has shipped.

Effort scale: S is under half a day, M is 1–3 days, L is about a week.

## Principles

- **Felt, not explained.** No meters, levels, coach marks or text telling the user how petting works. They find it out by touching the cat.
- **No new settings.** Sound follows the Mac's own "Play user interface sound effects", and every behaviour follows the cat's existing personality (`petMotion`: calm, playful, sleepy).
- **No guilt.** The cat never goes hungry, gets ill or nags. A fading bond is the only consequence of time apart, as it is today.
- **On the Mac only.** The cat's memory stays in Buntini's local store. Export includes it and Delete everything erases it.
- **Everyone can pet it.** Keyboard, VoiceOver and Reduce motion each get a full way to pet the cat.

## What exists

- **Gestures** (`usePetLife.ts:325-392`):
  - Stroking more than 140 px within 1.4 s plays `purr`: a 0.12 s shiver, happy eyes and hearts, replayed every 900 ms.
  - After 3 s of purring the cat kneads.
  - A pointer held still for 1.3 s gets a head rub (`bunt`), at most once every 20 s.
  - After 12 s, a playful or cross cat swats, once per session.
  - Petting while it asks (`askpet`) plays `love`.
  - A click plays `love` or `grumpy`, and quick clicks count as a poke.
- **Mood** (`petMood.ts`): five feelings and a `bond` that grows 0.002 for each second of petting and 0.01 for each praise, and fades over weeks. It's saved in `localStorage` under `buntini-cat-mood` (`usePetLife.ts:14`), so data export misses it.
- **Name:** setup's "My name" (`Onboarding.tsx:99`) sets `assistantName`. It's used in the window title, the cat button's labels and announcements, and nowhere else.
- **Reduce motion:** `stillPreferred()` returns early from every pointer handler, so petting does nothing at all.
- **The drawing** (`Cat.tsx`, viewBox 0 0 180 180): a seated, front-facing cat with `cat-head` (ears, face, cheeks), `cat-front` and `cat-ruff` (chest), `cat-paw-*` and `cat-tail`. Its belly isn't visible while it sits.

---

## Part 1: petting

Build these in order, since P1 and P2 underpin the rest.

### P1. The cat's memory moves into the store (S–M)

- **Goal:** The bond, mood, favourite spot and adoption day survive a reinstall, are included in an export and are erased by Delete everything.
- **Build:**
  - A preference row `pet_memory` holding JSON: `{ version, mood, savedAt, adoptedOn, favourite }`.
  - New Tauri commands `pet_memory` and `save_pet_memory`. `save_pet_memory` validates the shape and clamps every feeling to 0–1.
  - The first launch after the update moves `buntini-cat-mood` from `localStorage` into the store, then removes the old key.
  - `adoptedOn` is the date setup finished (or the date of the move, for existing users). `favourite` is chosen at random from `cheeks`, `chin` and `ears` (see P10).
  - `export_snapshot` gains `"cat": { adoptedOn, bond, favourite }`. Moment-to-moment feelings aren't worth exporting.
  - `erase_everything` already deletes `preferences`. The existing `forget` step in `usePetLife.ts:17` stays as is.
- **Files:** `store.rs`, `controller.rs` (commands), `lib.rs` (handler list), `usePetLife.ts`, `petMood.ts` (`memory` and `remembered` read the new shape), `bridge.ts`.
- **Acceptance:**
  - Rust tests: a round trip; an out-of-range or unknown field is rejected; export includes `cat`; erase leaves `pet_memory` empty.
  - Frontend test: an old `localStorage` mood is moved exactly once.

### P2. Where you touch matters (M)

- **Goal:** Each part of the cat answers differently, and the reactions are worked out in pure code that tests can cover.
- **Build:** a new pure module `petTouch.ts`.
  - `zoneAt(x, y)` maps a point in the seated cat's viewBox to `ears | cheeks | chin | head | back | chest | tail | paws`, using the bounds of the parts in `Cat.tsx`.
  - `usePetLife` passes each stroke's zone and distance to a `Petting` state machine, in place of the old `travel > 140` check. A stroke counts from 100 px/s, measured over 600 ms.

  | Zone | Reaction |
  |---|---|
  | Ears, cheeks, chin | Melts: eyes close, head tips into the stroke, purr builds fastest. |
  | Head | The current purr. |
  | Back (the flanks) | Arches up into the stroke. |
| Chest | A playful cat grabs with both paws and kicks after 4 s; a calm one tolerates it; a cat with a close bond leans back for more (P8). |
  | Tail | Flicks it away and turns its head. Ends the purr and nudges `annoyance`. |
  | Paws | Lifts the paw and bats once, like the existing `bat`. |

- **Acceptance:** `petTouch.test.ts` covers the zone bounds, the reaction for each zone, and that the grab, swat and bat happen only when and as often as listed.

### P3. Two-finger strokes on the trackpad (S)

- **Goal:** Swiping two fingers across the cat is petting, which feels closer to stroking fur than waving the pointer.
- **Build:**
  - A `wheel` listener on the pet element (`{ passive: false }`) adds `Math.hypot(deltaX, deltaY)` to the same `travel`, using the zone under the pointer.
  - It ignores `ctrlKey` (a pinch) and calls `preventDefault` so the page doesn't scroll.
  - The overlay already accepts the pointer over the cat (`overlay.rs` hit testing), and macOS sends scrolling to the window under the pointer whether or not that window is key.
  - A fur-ruffle path in `Cat.tsx` follows the stroke's direction and smooths back over 600 ms.
- **Acceptance:** a unit test feeds synthetic wheel deltas through `touch` and gets a purr, and a pinch (`ctrlKey`) does nothing. Device check: a Magic Trackpad and a built-in trackpad both purr.

### P4. A purr that builds and fades (M)

- **Goal:** Purring is a level from 0 to 1 that rises while you pet and falls slowly after you stop, in place of a 900 ms class replayed over and over.
- **Build:**
  - `touch` keeps `purr` from 0 to 1. Steady stroking raises it about 0.25 a second, the favourite zone twice as fast (P10). It falls about 0.1 a second after the hand leaves.
  - CSS reads `--purr`. The 0.12 s shiver becomes a 2.4 s breathing swell that scales with the level. Hearts appear above 0.6.
  - Kneading starts at a level of 0.8 held for 2 s, in place of the 3 s timer.
  - **Afterglow:** when the hand leaves above 0.5, the cat slow-blinks at the pointer, then settles (`loaf` if it's sleepy, otherwise `tailcurl`).
- **Acceptance:** tests for the rise and fall curves, kneading at the threshold and an afterglow once per petting.

### P5. A purr you can feel (S, plus a device check)

- **Goal:** On a Force Touch trackpad, the user feels a soft tap in time with each purr breath.
- **Build:**
  - Turn on the `NSHapticFeedback` feature of `objc2-app-kit`.
  - A command `pet_purr_tap(strength)` calls `NSHapticFeedbackManager::defaultPerformer().performFeedbackPattern_performanceTime(Alignment, Now)`.
  - The frontend calls it once per breath while purr is above 0.3 and the pointer or a two-finger stroke moved within the last 400 ms. Rust drops any tap that comes sooner than 700 ms after the last.
  - Above 0.7, each breath gets a second tap 120 ms after the first.
- **Unknowns, for the device check:**
  - Whether taps reach the finger while Buntini isn't the active app. The cat lives in a non-activating overlay window.
  - Whether `Alignment` or `Generic` feels closer to a purr.
  - The macOS haptics preference already turns taps off, and mice have none. The sound (P6) carries the purr there.

### P6. A purr you can hear (M)

- **Goal:** A quiet, low purr that rises and falls with the level, heard only when it can't intrude.
- **Build:**
  - Synthesise it in Web Audio, so there's no recorded asset to license: brown noise through a low-pass filter around 180 Hz, amplitude-modulated at 24 Hz, with a slow envelope following the breath.
  - Its loudest point stays around −30 dBFS, well below speech.
  - It plays only when a new command `pet_sound_allowed()` returns true:
    - The global `com.apple.sound.uiaudio.enabled` isn't false.
    - The default input device isn't running: `kAudioDevicePropertyDeviceIsRunningSomewhere` is false, meaning no call or recording is using the microphone.
    - Focus is off, using `nudges::focus_on`.
    - Buntini isn't speaking or listening.
  - The answer is cached for 10 s.
- **Unknowns:**
  - Whether WKWebView lets an `AudioContext` start from a pointer movement, which isn't a user activation. If not, it starts on the first click or key press in Buntini of each launch, and the purr stays silent until then.
- **Acceptance:**
  - A test with each gate false finds no audio graph started.
  - A Rust test reads `uiaudio.enabled` both missing and false.
  - Listening check: the purr sits under speech and is inaudible at a third of the volume slider.

### P7. It leans into your hand (S–M)

- **Goal:** The cat moves toward the touch, not just in place.
- **Build:**
  - While purring, the existing head gaze points at the pointer, and `cat-head` tilts up to 8° toward the side being stroked.
  - When the hand slides off one side above a level of 0.4, the cat leans after it for 400 ms before settling back.
  - The head rub (`bunt`) rubs toward the side the hand is on. This already exists through `--bunt-dir`; keep it.
- **Acceptance:** tests that the tilt follows the stroke's side and that the lean happens once per exit.

### P8. The bond shapes how it takes a pet (M; L with a roll-over pose)

- **Goal:** Users see the bond in how the cat responds, with no number shown.
- **Build:** three bands from `bond`.

  | Band | How it responds |
  |---|---|
  | New (below 0.15) | Purr starts after twice as much stroking. After 8 s it leans away with ears flat, and stroking after that only raises `annoyance`. It asks for pets half as often. |
  | Familiar (0.15–0.5) | Today's behaviour, with P2–P7. |
  | Close (0.5 and up) | Purrs at the first touch. A chest rub rolls it onto its back (new `flop` pose) for more, with no grab unless it's playful and the rub goes past 6 s. It asks for pets more often, and when you come back after an hour or more it walks to the pointer and rubs against it. |

  The bond grows as it does now. Gentle, steady petting counts a little more than frantic petting: travel per second between 200 and 900 px counts in full, and faster than that counts half.
- **Files:** `petMood.ts` (bands), `petTouch.ts`, `Cat.tsx` and `styles.css` (the `flop` pose), `petLife.ts` (how often it asks).
- **Acceptance:** tests run each band through the same stroke sequence and check the reactions differ as listed, and that a new cat can't reach `flop`.

### P9. A fair warning before the swat (S)

- **Goal:** Users can see a swat coming and avoid it by stopping.
- **Build:**
  - 1.5 s before the swat would fire, the tail tip twitches and the ears flatten (`cat-ear` rotation, already drawn). If the user stops or slows below 100 px/s during that time, there's no swat and it ends with a slow blink.
  - The swat condition stays playful-or-cross, now counted from the purr level rather than a 12 s timer.
- **Acceptance:** tests that stopping during the warning cancels the swat, and that the swat stays once per session.

### P10. Its own favourite spot (S)

- **Goal:** Each cat is a little different, and users discover how by petting it.
- **Build:**
  - `favourite` comes from P1. In that zone, purr builds twice as fast, the cheeks blush (`cat-blush-strong`, already drawn) and it kneads sooner.
  - The spot never changes for that cat, and choosing a new character keeps it.
- **Acceptance:** a test that the favourite zone reaches kneading sooner than any other zone.

### P11. Petting without a pointer, and with Reduce motion (S)

- **Goal:** Petting works through the keyboard and VoiceOver, and Reduce motion keeps everything except the movement.
- **Build:**
  - **Keyboard and VoiceOver:** a "Pet {name}" control comes right after the cat button in the tab order, visually hidden until it has focus.
    - Each press counts as one second of steady head strokes, and holding Space keeps petting.
    - Its live region says "{name} is purring" once when the purr starts, and "{name} is kneading" once when kneading starts.
    - VoiceOver users also get it as a custom action on the cat button.
  - **Reduce motion:** remove the early return in the pointer handlers.
    - Touch, purr, haptics and sound all run as normal.
    - Visual changes become instant face swaps: happy eyes, blush, hearts with no float.
    - Tilt, lean, swat, grab and `flop` don't run.
- **Acceptance:** a test drives the purr level from key presses alone, and another checks that Reduce motion runs the same touch sequence with no motion classes. A VoiceOver device check confirms the custom action.

### Device acceptance for part 1

On a Mac with a Force Touch trackpad, and on one with a mouse:

- Pointer and two-finger petting in each zone.
- Taps felt while another app is active.
- The purr silent during a FaceTime call, in Focus and with interface sound effects off.
- Keyboard petting, a VoiceOver pass and Reduce motion.
- The bond, favourite spot and adoption day surviving a quit, a reinstall and an export, and gone after Delete everything.

---

## Part 2: a cat that knows you

To be specified after part 1. Everything comes from data Buntini already reads on the Mac.

1. **Days together.** `adoptedOn` from P1 marks an anniversary each year with a small celebration on the day. The name already exists; Buntini should also answer to it, so "Miso, remind me…" in chat or voice is treated as addressed to it, with the name removed from the request.
2. **Your rhythm.** Learned from `world.rs` activity, idle time, Focus and the calendar:
   - a greeting when you first sit down, with the existing day glance;
   - curling up during meetings and Focus;
   - a stretch and a nudge after a long stretch without a break;
   - waiting at the screen's edge around the time you usually stop.

   Kept as hour-of-week counts on the Mac only, with no setting.
3. **Gifts.** Heads-ups (`nudges.rs`) arrive as the cat bringing something to your window: it trots over, drops a small card and looks up. Later it could also bring back a file or note you were hunting for the day before.
4. **Shared memories.** Moments it keeps: the first task it did for you, a trip you planned together, the day you praised it. It brings one up now and then ("a year ago today we planned Lisbon"). These are stored as memory rows tagged `cat`, and Forget and Delete everything remove them.
5. **It notices you.** Sleepy with you late at night, overjoyed after a week away, and aware of your birthday from your own contact card.

**Open questions for part 2:**
- How often gifts and memories may appear before they feel like interruptions. Start with at most one memory a day.
- Whether the rhythm counts should feed Heads-ups' learned active hours, so the two never disagree.
