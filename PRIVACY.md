# Buntini privacy policy

*Last updated: October 22, 2026. This describes what the app does; it is not legal advice. Have it reviewed before a commercial launch.*

Buntini is a desktop companion that runs on your Mac. There are no Buntini servers and no Buntini account. We (the people who make Buntini) never receive your messages, files, settings or usage.

## What stays on your Mac

Everything Buntini keeps is stored in its folder under `~/Library/Application Support/app.buntini.companion` and in your macOS Keychain:

- **Your conversation**, settings, saved searches, price watches, and the things you ask it to remember.
- **The file index** for folders you choose to allow. Buntini reads only those folders, and you can remove access at any time.
- **What you're doing:** if "Notice what I'm doing" is on, the name of the app in front, how long you've been active, window positions (never titles or contents) and battery, CPU and disk levels. This is kept in memory for at most three hours and never saved.
- **API keys** you add, and your **ChatGPT sign-in** if you use one, in the macOS Keychain.
- **An activity log** of tools used and permissions given, and a **crash log** (time, error and code location only).
- **Voice recordings**, only while you hold the mic. They are deleted once they've been transcribed.

On-device brains (Apple Intelligence and local MLX models) can use all of this, because none of it leaves your Mac.

## What leaves your Mac, and only when

| When | Sent to | What is sent |
| --- | --- | --- |
| You use a **cloud brain** (Anthropic, OpenAI or Google Gemini) with your own key | That provider | Your messages and Buntini's replies. Local data is included only for what you switch on under *Settings → Privacy → Cloud models may see*: what you're doing, Mac health, your files, earlier messages, things you asked it to remember. All of these are off by default. The name you gave Buntini to call you is not sent. |
| You **sign in with ChatGPT** and use it as the brain | OpenAI | Sign-in happens on OpenAI’s own page in your browser. Then the same as a cloud brain above: your messages and replies, plus only what you’ve switched on. Requests are sent with `store: false`, asking OpenAI not to keep them. Your sign-in tokens stay in your Keychain. |
| You approve an **online lookup** | The service shown on the approval sheet: Brave Search, Tavily, Open-Meteo, Frankfurter (ECB rates), Wikipedia, OpenStreetMap Nominatim, eBay or Duffel | Exactly the values shown on the sheet, and nothing from your files or conversation. Public pages from search results may be fetched to cite them. |
| You approve an **action** | The app on your Mac that carries it out (Reminders, Calendar, Notes, Mail, Shortcuts, your browser) | The details you approved. Mail drafts are never sent by Buntini. |
| You allow an **app** in Settings → Apps and ask about it | Nobody. Mail, Calendar, Reminders, Notes, Music, Spotify and your browsers are read on this Mac | Email, events, reminders, notes, what’s playing and browser tabs reach a cloud brain only if you turn on that kind of data in Privacy. Tab addresses are never fetched or sent online, and private windows are skipped where the browser says which they are (not in Safari). |
| You check for **macOS updates** | Apple's Software Update service, through macOS | Nothing from Buntini. |

Each provider handles your data under its own terms and privacy policy. Buntini sends requests only with the keys you add, and it counts and limits them before they go out.

## Your controls

- **Settings → Privacy** lists what Buntini notices, which folders it can read, what cloud models may see, and what it remembers. You can switch each of these off or remove items.
- **Export** saves your conversation, settings, lists and memories as a JSON file. Your keys are not included.
- **Delete everything** erases the conversation, settings, folders, lists, memories, file index, saved keys and recordings, and starts setup again.
- **Copy support info** copies a short summary you can choose to share with us when reporting a bug: app and macOS versions, which brain is set up, recent activity entries and crashes, with your home folder shortened to `~`. It contains no messages, file contents or keys.

Buntini has no analytics and no telemetry, and it doesn't send crash reports automatically.

## Children

Buntini isn't directed at children under 13.

## Changes and contact

We'll update this file when the app's behaviour changes; its history is public on [GitHub](https://github.com/sagar8080/nexon/commits/main/PRIVACY.md). Questions: open an issue at [github.com/sagar8080/nexon/issues](https://github.com/sagar8080/nexon/issues).
