# Buntini design

Buntini is a small creature that lives on your Mac, not a dashboard. Every screen should
feel like it belongs to the cat: warm, soft, quick to answer and never in the way.

## Principles

1. **One character, everywhere.** Personality is carried by words first, then amplified by
   every other channel. Buntini speaks in its own typeface (SF Pro Rounded), its own colour
   (ginger) and its own voice in the copy. Your words are set in the system face, so a
   conversation reads as two parties, not one log.
2. **Content is solid; controls float.** Following macOS 26's material guidance, glass is for
   the control layer that floats above content (the composer, sheets), never for the
   content itself. Conversation sits on a warm, opaque surface that stays readable over any
   wallpaper.
3. **Warm, not grey.** Neutrals are tinted toward the cat's coat (oat in light, cocoa in
   dark). Accent is used for one thing at a time: your message, the primary action, the
   focus ring.
4. **Say less, show state.** No status pills, counts, keyboard legends or monospace labels.
   The cat's pose and one short line say what Buntini is doing. Details (how a task was
   done, sources) are one tap away, not on the page.
5. **Motion explains.** Things arrive from where they come from: the panel grows out of the
   cat, replies rise into place, sheets slide up like a card being handed over. Quick
   (160–280 ms), eased out, with a little spring for things you touch. Reduce Motion turns
   it all off.
6. **Tactile.** Every control answers a press (a slight squash), hover is a gentle lift, and
   focus is always visible.
7. **Accessible by default.** Text and accents meet 4.5:1; targets are at least 28 pt; the
   text-size setting scales everything; nothing is conveyed by colour alone.

## System

- **Type.** Voice: `ui-rounded` (greeting, Buntini's replies, titles, suggestions, sheet
  headings). Interface and your messages: system UI. Body 13.5/1.55, small 12, caption 11.
  Sentence case everywhere.
- **Radii.** Panel 22, sheets 20, bubbles 18 (6 at the tail), cards 14, controls 10–12,
  pills fully round.
- **Spacing.** 4-point grid; panel gutters 16; groups separated by space, not lines.
- **Elevation.** One soft warm shadow for the panel, a lighter one for floating controls;
  hairlines only where two surfaces of the same colour meet.
- **Motion tokens.** `--ease-out: cubic-bezier(.22, 1, .36, 1)`,
  `--spring: cubic-bezier(.34, 1.4, .64, 1)`; durations 160 / 220 / 280 ms.
