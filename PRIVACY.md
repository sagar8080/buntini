# Buntini privacy policy

*Last updated: October 22, 2026. This describes what the app does; it is not legal advice. Have it reviewed before a commercial launch.*

Buntini is a desktop companion that runs on your Mac. There are no Buntini servers and no Buntini account. We (the people who make Buntini) never receive your messages, files, settings or usage.

## What stays on your Mac

Buntini keeps its records in its folder under `~/Library/Application Support/app.buntini.companion` and in your macOS Keychain. Its separate browser profiles also use macOS-managed WebKit storage:

- **Your conversation threads and task journals**, including full saved model messages, step inputs, approvals, tool evidence, destination IDs, receipts and which local-data categories were used. These let Buntini resume a task without repeating its completed actions. Also your settings, saved timers, searches, price watches, and the things you ask it to remember. Timers and monitors keep due times, execution/delivery status and errors. Approved intent records retain monitor queries/pages/settings and approved memory facts/timer durations to prevent a repeated approval from duplicating or recreating something you deleted, forgot or cancelled; these remain until Delete everything.
- **The file index** for folders you choose to allow. Buntini reads only those folders, and you can remove access at any time.
- **What you're doing:** if "Notice what I'm doing" is on, the name of the app in front, how long you've been active, window positions (never titles or contents) and battery, CPU and disk levels. This is kept in memory for at most three hours and never saved.
- **API keys** you add, and your **ChatGPT sign-in** if you use one, in the macOS Keychain.
- **An activity log** of tools used and permissions given, and a **crash log** (time, error and code location only).
- **Voice recordings**, after you tap or hold the mic, for up to 30 seconds. They are deleted after transcription, failure or cancellation.
- **Separate browser profiles**, one per approved HTTPS origin, retain that site's cookies, sign-ins and website storage across restarts. They do not use Safari, Chrome or Buntini's main interface profile. Approved reads retain the page address, title and up to 6,000 characters of visible page text in task history. Form fields, passwords and editable text are excluded from that extraction. Website text remains untrusted content.
- **Web research records**, including the original request, approved query, selected source addresses, bounded page text, route attempts and missing-evidence status, are saved with the task. Research creates and reads a dedicated tab in an allowed existing/default browser. It does not read other tabs' titles, addresses or page contents, or read cookie files, but the browser may use its normal cookies and sign-ins and record those visits in its history. Browser-derived evidence retains its Browser data category in saved answers and task journals.

On-device brains (Apple Intelligence and local MLX models) can use all of this, because none of it leaves your Mac.

## What leaves your Mac, and only when

| When | Sent to | What is sent |
| --- | --- | --- |
| You use a **cloud brain** (Anthropic, OpenAI or Google Gemini) with your own key | That provider | Your messages and Buntini's replies. Local data is included only for what you switch on under *Settings → Privacy → Cloud models may see*: what you're doing, Mac health, your files, earlier messages, things you asked it to remember. All of these are off by default. The name you gave Buntini to call you is not sent. |
| You **sign in with ChatGPT** and use it as the brain | OpenAI | Sign-in happens on OpenAI’s own page in your browser. Then the same as a cloud brain above: your messages and replies, plus only what you’ve switched on. Requests are sent with `store: false`, asking OpenAI not to keep them. Your sign-in tokens stay in your Keychain. |
| You approve **web research** | Google in your existing browser and the configured Brave/Tavily fallback shown on the approval sheet; selected public HTTPS source sites and the resources loaded by your browser | The exact approved query goes to browser search first and to the configured API only if needed. Up to six selected source pages may be read in the dedicated research tab, or fetched without browser credentials when browser reading is unavailable. Your browser may use existing cookies and sign-ins. Buntini reads only its research tab; browser-derived results and page text reach a cloud brain only with Browser sharing enabled. API/public-HTML evidence can be summarized without Browser sharing. |
| You approve another **online lookup** | The service shown on the approval sheet: Open-Meteo, Frankfurter (ECB rates), Wikipedia, OpenStreetMap Nominatim, eBay or Duffel | Exactly the values shown on the sheet. A model-proposed query or argument may contain information from your request; review those values before approval. The rest of your files or conversation is not sent to the lookup service. |
| You approve a **recurring search or price watch**, in chat or Scheduled work | Your selected Brave/Tavily provider for searches, or the approved public HTTPS page for prices | The exact approved query or page address, repeatedly at the approved interval. Creating the monitor is local; the first network check happens after that interval while Buntini is running. You can pause or delete it. |
| You approve an **action** | The app on your Mac that carries it out (Reminders, Calendar, Notes, Mail, Shortcuts, your browser) | The details you approved. Mail drafts are never sent by Buntini. |
| You approve **opening and reading a page in Buntini's separate browser** | The approved HTTPS site and its public HTTPS resources | The page address, ordinary browser requests, and cookies for that separate site profile. You may sign in manually. Buntini reads bounded visible page text; a cloud brain receives it only with Browser sharing enabled. This feature requires macOS 14+ and native isolation checks. Other-origin navigation, popups, downloads and private-network connections are blocked. Buntini does not click, fill or submit website forms. |
| You give a recognized **direct browser or music command** | Your browser, search engine, Music or Spotify, and their services | The address, search words or music request you gave. “Open YouTube” or “open Google search results for cafés in Porto” runs directly from that navigation request. “Google cafés in Porto” instead starts the approved web-research flow above. |
| You allow an **app** in Settings → Apps and ask about it | Mail, Calendar, Reminders, Notes, Music, Spotify and your browsers are read on this Mac | Email, events, reminders, notes, what’s playing and browser data reach a cloud brain only if you turn on that kind of data in Privacy. Listing/reading tabs does not fetch their addresses; shared browser data can include titles, addresses and page text. Private windows are skipped where the browser says which they are (not in Safari). |
| You check for **macOS updates** | Apple's Software Update service, through macOS | Nothing from Buntini. |

Each provider handles your data under its own terms and privacy policy. Browser
research and keyless integrations do not need an API key. Keyed APIs use only
credentials you add, and their requests are reserved against the configured
limits before dispatch. Browser search does not consume a search-API reservation.

## Your controls

- **Settings → Privacy** lists what Buntini notices, which folders it can read, what cloud models may see, and what it remembers. You can switch each of these off or remove items.
- **Export** saves all conversation turns, thread metadata/membership, task journals, settings, lists, memories, timers, monitor run/delivery history and approved monitor and local-control intent records as a JSON file. Saved browser readbacks in task history are included. Keys, browser cookies, sign-in sessions and website cache/storage are not exported.
- **Delete everything** closes the separate browser and asks WebKit to delete each of its profiles, then erases the conversation, settings, folders, lists, memories, timers, monitor history and intent records, file index, saved keys and recordings, and starts setup again. If WebKit cannot confirm browser profile removal, deletion reports an error and retains the profile identifiers so it can be retried. It does not erase history, cookies or sign-ins in your existing browser.
- Deletion applies to Buntini's current app data and cache, including the current database and its journal files. If you upgraded from the former Nexon app, migration preserves the original `~/Library/Application Support/app.nexon.companion` folder as a separate backup; Delete everything does not remove that legacy folder. Copies you exported, destination records created in other apps, Time Machine backups and APFS snapshots also remain. Deletion is logical removal, not a guarantee of forensic erasure from your disk or backups.
- **Copy support info** copies a short summary you can choose to share with us when reporting a bug: app and macOS versions, which brain is set up, recent activity entries and crashes, with your home folder shortened to `~`. It contains no messages, file contents or keys.

Buntini has no analytics and no telemetry, and it doesn't send crash reports automatically.

## Children

Buntini isn't directed at children under 13.

## Changes and contact

We'll update this file when the app's behaviour changes; its history is public on [GitHub](https://github.com/sagar8080/nexon/commits/main/PRIVACY.md). Questions: open an issue at [github.com/sagar8080/nexon/issues](https://github.com/sagar8080/nexon/issues).
