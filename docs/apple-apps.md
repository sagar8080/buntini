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

**M2 Messages**
- `send_message` to a person found in Contacts, through Messages, always approved.
- Reading recent messages needs Full Disk Access (the chat database), so it waits.

**M3 TV and Photos**
- TV: now playing and playback, like Music.
- Photos: find photos by date, place or album (PhotoKit), and show them.

**M4 Home and more**
- Ready-made shortcuts for Home scenes and devices, added with one click.
- FaceTime calls (confirmed first), opening App Store pages.

## Privacy

Every new read is an app the user allows in Settings → Apps and, for a cloud brain, a
kind of data in Settings → Privacy. Every send or write goes through the approval sheet.
