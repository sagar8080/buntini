# Native surfaces

The island at the notch, the dictation pill and the overlays. This package is SwiftUI and Metal. `src-tauri/build.rs` builds it as a static library and links it into Buntini itself. That way it shares the app's Accessibility grant and adds no executable to the bundle.

## How it fits together

- **Rust owns the state.** Each part of Buntini calls `surface::publish(source, Contribution { pills, offers })`.
  - `surface/board.rs` picks the card in front, and keeps the other news on the scene's `shelf`.
  - A pill names its card in `card` when the two ids differ (an agent session and its steps).
  - A whole `Scene` goes to Swift as JSON, at most once a frame.
- **Swift draws it.**
  - The scene is decoded into `SurfaceKit` types.
  - What the person does comes back as an `Action` through one C callback.
  - `surface::on_action(source, …)` hands it to the part that owns it.
- **Voice bands** skip the scene: `surface::levels([f32; 9])`.
- **Words** are the `surface.*` keys of `locales/<language>.json`. Rust sends them with the first scene and when the language changes (`words`); Swift keeps the last ones (`Words.swift`, `L(.chat)`). A new label goes in `Word`, in `locales/*.json` and in the fixture's first scene.

## What the island shows

- **Hidden** while nothing runs. Pointing at the notch shows the cat.
- **Beside the notch** (`Glance`): the most pressing pill. It shows a mark for what it is and how it's doing, its few characters (time left, a count, a project), then a dot for each of the next three, then "+N". The wings widen to hold it, up to `widestWing`. Resting the pointer there shows the lead in words, or what the Mac needs.
- **Opened by a click** (`Home`): the question field and Chat, then a chip for each thing running (`Chips`), then the chosen chip's card. A choice holds until the island closes. A session's chip goes to its terminal once its card shows, or when it has none.
- **Opened by itself** for what waits on you or news worth an interruption. The cat stays in its wing, except beside a hello, a finished turn or dropped files.
- Spinners and music bars are Core Animation layers (`GlanceViews.swift`), so they cost the app nothing between changes.

## Who owns what

| Part | Rust | Swift payloads | Swift views |
|---|---|---|---|
| Island (A) | `island/`, `surface/cards/island.rs` | `Cards/IslandCards.swift`, `IslandGeometry.swift` | `Cards/IslandCardViews.swift`, `Island/`, `Character/`, `Metal/`, `Sound/` |
| Agents (B) | `agents.rs`, `surface/cards/agents.rs` | `Cards/AgentCards.swift` | `Cards/AgentCardViews.swift` |
| Voice (C) | `flow/`, `surface/cards/flow.rs` | `Cards/FlowCards.swift` | `Cards/FlowCardViews.swift`, `Pill/`, `Flow/` (keys, paste, text field; C entry points in `Flow/FlowExports.swift`, called from `flow/native.rs`) |
| Services (D) | `services/`, `surface/cards/services.rs` | `Cards/ServiceCards.swift` | `Cards/ServiceCardViews.swift` |
| Mail (E) | `mail/`, `surface/cards/mail.rs` | `Cards/MailCards.swift` | `Cards/MailCardViews.swift` |
| Media, utilities (F) | `media/`, `utilities/`, `surface/cards/{media,utilities}.rs` | `Cards/{Media,Utility}Cards.swift` | `Cards/{Media,Utility}CardViews.swift`, `Media/` (covers, the progress and level bars; the players' watcher, its C entry point in `Media/MediaExports.swift`, called from `media/native.rs`), `Overlay/`, `Utilities/` (hot keys, the thumbnail, the capture dimmer, mark-up; C entry points in `Utilities/UtilityExports.swift`, called from `utilities/native.rs`) |

On the React side, each part has its own folder: `src/<part>/model.ts` and `src/<part>/api.ts`. Each also has one command, `<part>_call`, which is already registered and allowed.

## Changing the contract

1. Change the Rust type and the Swift type together. Field names are camelCase on the wire.
2. Update `Fixtures/contract.json` so it still covers every card kind.
3. Bump `CONTRACT` in Rust and `surfaceContract` in Swift if an older build of either side would misread the new one.
4. Check both sides:
   - `cargo test surface`
   - `swift run --package-path native/surface-lab SurfaceChecks native/surface/Fixtures/contract.json`

## Looking at it without building Buntini

- `swift run --package-path native/surface-lab SurfaceLab native/surface/Fixtures/contract.json [index]` opens the real island with fixture scenes and prints every action.
- `cargo run --example surface_host [kind]` does the same through Rust and the C entry points. Here `kind` is a card kind, for example `approval`.
- `swift run --package-path native/surface-lab SurfaceLab <fixture> --render <folder>` draws every scene into pictures without a screen. AppKit views (the Metal glow, the pointer probe) are left out of pictures.
- `SurfaceLab <fixture> <index> --seconds 10 --frames` shows a scene live and reports frame times while the cat moves. Add `--open` to see it opened, as a click opens it.
- `--capture` lets screenshots see the island; Buntini itself keeps it out of them.
- Measured on a 14-inch MacBook Pro at 120 Hz, release build, on 2026-11-17, as CPU time over eight seconds:
  - 0.6% hidden: the cat rests while nothing of it shows;
  - 2.6% peeking with a focus timer, 2.9% opened with a session's steps;
  - frame times while the cat moves: median 7.5 ms, 95th percentile 15 ms, the same as the build before.
- The cat is drawn every frame only while it moves; at rest it isn't drawn at all. The glow is drawn once; Core Animation makes it breathe.

Don't run `swift build` inside this folder. Cargo watches it, and a `.build` folder here makes every Rust build start over. Build through `native/surface-lab` instead.
