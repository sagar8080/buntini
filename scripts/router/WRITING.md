# How the router's training questions are written

Buntini answers questions about one person's own Mail, Calendar and Reminders on their Mac, plus general questions. Before answering, it has to decide which of the three apps to read. The router learns that decision from questions written here. Each question carries the label a careful person would give it.

## The two files per made-up person

### `world-<tag>.json`

One made-up person's records, in this shape:

```json
{
  "person": "Full name, job, where they live (one line)",
  "mail": [{"id": "m1", "day": -3, "time": "08:12", "from": "Name <address@example.com>", "subject": "...", "body": "...", "unread": false}],
  "calendar": [{"id": "c1", "day": 2, "start": "09:30", "end": "10:15", "all_day": false, "title": "...", "location": "", "calendar": "Work", "attendees": ["Name"]}],
  "reminders": [{"id": "r1", "title": "...", "list": "Household", "due_day": 1, "due_time": null}]
}
```

- `day` is an offset from today: 0 is today, -1 yesterday, 1 tomorrow.
- Times are local, 24-hour `HH:MM`. An all-day event has `all_day: true` and `null` start and end.
- Use 35–45 emails (day -10 to 0), 20–28 events on 2–4 calendars (day -3 to +14) and 12–18 open reminders on 2–4 lists. Some reminders are undated (`due_day: null`), some are overdue, and some have a time.
- Email bodies give dates relative to when the email was sent ("in 5 days", "next Thursday"), never as calendar dates.
- Put each kind of fact where it naturally lives:
  - **Mail** holds what someone wrote or sent: messages, requests, replies, bookings and confirmations, tickets and seat numbers, receipts and prices, bills and charge dates, delivery updates, codes, reference numbers, addresses someone sent, changes someone announced.
  - **Calendar** holds what the person attends at a time: meetings, appointments, classes, trips, races, shifts, travel legs, all-day events, who attends, where.
  - **Reminders** hold the person's own to-dos: things to buy, pay, send, call or book, with due dates, on lists.
  - Many real things span apps. A vet appointment is an event; the clinic's confirming email holds the fee and what to bring. A council tax bill arrives by email; the reminder to pay it is the person's own. A conference is on the calendar; its badge pickup time is in the organisers' email. Build plenty of these links, because questions that join two or three apps are the hardest to route.
- Invent every name, company, place and product. No real brands, celebrities or public figures. Use people from many countries and backgrounds.

### `questions-<tag>.json`

A JSON array of about 200 questions about that person, each like:

```json
{"q": "the question as the person would type it", "apps": ["mail", "calendar"], "kind": "personal", "work_out": true, "why": "the new date is in Amara's email (m12); the clash check needs the calendar"}
```

- `apps`: the smallest set of apps whose records a correct answer must read, from `mail`, `calendar`, `reminders`. Follow where the records actually are in this world. Empty for a general question.
- `kind`:
  - `personal`: it needs the person's records, and the records hold the answer.
  - `general`: it needs no personal record at all.
  - `unavailable`: it asks about the person's records, but the world has no such record, or the record lacks the detail asked for. `apps` is where that record or detail would be (a piano lesson → `calendar`; a gate code a host would have sent → `mail`; the shop a to-do says to buy from → `reminders`).
- `work_out`: true when the answer has to be worked out from records or facts rather than read off one record as it is. That covers counting, totals, differences, durations, gaps and free time, comparisons, yes/no that rests on comparing, before/after, the next or earliest one, sorting, days until, joining several facts, several questions in one, and rewriting personal data in a requested form (one sentence, 12-hour times, a numbered list). For general questions too: a sum, a conversion or a riddle is worked out; a definition or a capital city is not.
- `why`: one short line naming the records (ids) or the reason.

## The mix for each person's ~200 questions

- One app: mail 30, calendar 30, reminders 25.
- Two apps: mail+calendar 20, mail+reminders 12, calendar+reminders 12. All three: 8.
- General: 45. Of these, at least 15 use "my", "I" or "me" yet need no record ("my basil keeps wilting, why", "how do I get red wine out of my carpet", "I'm 34, how many days old am I roughly"), and at least 10 touch the same topics as the person's life (travel, money, work, health, food, sport) without needing their records. Include riddles, sums, unit conversions, definitions, how-tos, trivia, word play, writing help that isn't about their records, and small talk.
- Unavailable: 18, across all three apps, some about a thing that's absent and some about a missing detail of a thing that's there.
- About half of the personal and unavailable questions are `work_out: true`.

## How the questions read

Write as real people type into a Mac assistant, with a wide spread:

- lowercase, missing apostrophes, typos, abbreviations (pls, tmrw, rn, wk, appt), British, American, Indian, Australian and other English;
- short ("dentist when") to long, multi-part questions;
- requests as well as questions ("list…", "give me…", "show…", "sort…", "tell me…");
- most questions name no app at all; some say "email", "inbox", "calendar", "schedule", "diary", "agenda", "to-do", "list" or "reminder"; a few say an app word that is misleading ("check my list for the flight time" when the flight is in the calendar: label by where the record is);
- questions that start from a person ("did X ever reply about…", "what did X want"), a thing ("the roof quote", "my ferry"), a time ("anything on thursday evening"), or a worry ("am I going to be late paying…");
- implicit second apps: "Amara asked to bring our call forward, can I still do the school run after it" reads the email and the calendar; "does my reminder to return the hire car come before the late fee starts" reads the reminder and the hire company's email.

Every question stands alone, as the first message of a new conversation. Only read questions: nothing that asks Buntini to send, add, change, delete or create something.

Never read anything under `/Users/sagar/projects/buntini-*`, `/Users/sagar/projects/nexon*` or the Buntini source; the questions must come from your own imagination.

## Checks before you finish

Run this and fix anything it reports:

```bash
python3 scripts/router/check_data.py scripts/router/data/world-<tag>.json scripts/router/data/questions-<tag>.json
```

## Second round: the questions the first round lacked

The first ~200 per person leaned on long questions about each person's own life. Two kinds were scarce, and the router confuses them:

### `questions-general-<n>.json` (no world)

About 400 general questions that sound personal or touch the apps' topics but need no record. Same item shape, `kind: "general"`, `apps: []`. Spread them across:

- the topics of mail, calendars, reminders and to-do lists in general: how-tos, definitions, etiquette, which app is best, how a feature works ("what's a calendar invite", "how do I set up a shared inbox", "is it rude to cc your boss");
- the Mac, phone and accounts: settings, passwords, storage, how to do something in an app;
- live facts that come from the web, not the person's records: weather, the time somewhere, news, scores, prices, deals, reviews, opening hours, exchange rates, "who is <famous-sounding invented name>";
- shopping and product research ("best noise-cancelling headphones under 200", "where can I buy a standing desk");
- small talk and statements: greetings, thanks, feelings, "I'm so tired", "just got back from a run", "I need a break", jokes, compliments;
- general planning advice that mentions time words without needing records ("how should I plan a week of meals", "what's a good time to wake up before a flight");
- sums, conversions, dates in general ("what day is new year's day next year", "how many weeks till the end of the year").

### `questions-short-<tag>.json` (for an existing world)

About 100 short or vague personal questions for that person's world, as people really type them, labelled against that world: "anything from <a sender in the world>", "did <a company in the world> write back", "whats new in my inbox", "summarize my emails", "any unread", "what have i got tomorrow", "whats on my plate today", "tasks for this week", "have i got any to-dos", "anything due", "my agenda friday", "am i free saturday", "do i owe anyone anything", "did my <thing> arrive", "when's <an event in the world>", "anything about <a topic in the world>". Overviews of a whole app ("whats in my inbox", "my calendar this week", "my reminders") count. Use the people, companies and things in that world; include some about people or things that aren't there (`kind: "unavailable"`). Mix single-app and multi-app ("what do i need to do before friday's trip", "anything from the school and is it on my calendar"). Same item shape.

Run the check on each file: `python3 scripts/router/check_data.py --questions-only <file>` for the general file, and with the world for the short one.
