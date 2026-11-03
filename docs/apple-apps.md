# Apple apps

Buntini as the agent across the Mac's own apps: everything Apple offers freely to a
Mac app, before any cloud brain. Execution stays on the Mac; only what an Apple service
itself needs (a Maps search, say) leaves it, exactly as when the user opens that app.

## Ways in

| Way | What it reaches | Notes |
|---|---|---|
| Apple frameworks in `buntini-fm` | MapKit, EventKit, Contacts, PhotoKit | No app needs to open. Free; no key. |
| AppleScript | Music, TV, Messages, Photos, Finder, iWork | Values passed as arguments, never script text. |
| App links | Maps, TV, FaceTime, App Store | Opens the app at the right place. |
| Shortcuts | Home and every App Intent an app offers | Needs the shortcut; Buntini can ship ready-made ones. |
| Accessibility | Anything else | Last resort: slow and brittle. |

Not available to a Mac app: other apps' App Intents directly, HomeKit outside Mac
Catalyst, and Siri's own orchestration. WeatherKit needs a per-app capability and
quota, so weather stays on Open-Meteo.

## Phases

**M1 Maps** (this branch)
- Places through Apple Maps (MapKit points of interest) instead of OpenStreetMap,
  which stays as the fallback when the helper is missing. No one-request-a-second wait.
- `travel_time`: how long from one place to another by car, on foot or by transit,
  with a link that opens the route in Maps.
- Both stay approved online lookups: the sheet names Apple Maps and shows exactly what
  is sent.
- Each result shows a map drawn on the Mac (MapKit snapshots): the route with its two
  ends, or the places numbered as listed. Clicking it opens the same view in Maps. The
  latest 40 maps are kept in the cache.

**M2 Messages** (this branch)
- `send_message`: "text mom that I'm late", "send a text to Ana saying …". The person
  is looked up in Contacts (a phone number first, else an address iMessage can use);
  several numbers are offered in the sheet. Nothing sends until the user approves the
  exact recipient and words. Messages is allowed in Settings → Apps like any other app.
- The receipt says Messages took the text; delivery isn't confirmed, and an unclear
  result is never sent again automatically.
- Reading recent messages needs Full Disk Access (the chat database), so it waits.

**M3 TV and Photos**
- TV (this branch): a third player beside Music and Spotify, allowed in Settings → Apps.
  "What am I watching?", "pause the show", "next episode", "play Severance on TV" (from
  the TV library). Changes go through the same approval and before-and-after check of
  the player as Music. Streaming apps inside TV that don't report what's on stay out.
- Photos (this branch): "show me photos from last weekend", "my favourite photos",
  "screenshots from today", "selfies from March", "photos from my Goa trip album". The
  helper reads the library with PhotoKit (never downloading from iCloud for it) and
  saves small thumbnails in the cache (the latest 240); the reply shows the newest 12
  and opens one in Photos when clicked. No model sees the pictures. Searching by place
  would send the place to Apple, so it waits.

**M4 Home and more**
- Ready-made shortcuts for Home scenes and devices, added with one click.
- FaceTime calls (confirmed first), opening App Store pages.

## Privacy

Every new read is an app the user allows in Settings → Apps and, for a cloud brain, a
kind of data in Settings → Privacy. Every send or write goes through the approval sheet.
