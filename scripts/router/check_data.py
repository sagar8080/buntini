"""Checks one person's training world and questions (see WRITING.md).

    python3 scripts/router/check_data.py world-<tag>.json questions-<tag>.json
    python3 scripts/router/check_data.py --questions-only questions-general-<n>.json
"""
import collections
import json
import sys

APPS = ("mail", "calendar", "reminders")
KINDS = ("personal", "general", "unavailable")


def main(world_path, questions_path):
    problems = []
    world = json.load(open(world_path)) if world_path else {}
    for app, low, high in (("mail", 35, 45), ("calendar", 20, 28), ("reminders", 12, 18)) if world else ():
        records = world.get(app, [])
        if not low <= len(records) <= high:
            problems.append(f"{app}: {len(records)} records, want {low}-{high}")
        ids = [record.get("id") for record in records]
        if len(set(ids)) != len(ids):
            problems.append(f"{app}: repeated ids")
    for mail in world.get("mail", []):
        for field in ("id", "day", "time", "from", "subject", "body", "unread"):
            if field not in mail:
                problems.append(f"mail {mail.get('id')}: no {field}")
    for event in world.get("calendar", []):
        for field in ("id", "day", "start", "end", "all_day", "title", "location", "calendar", "attendees"):
            if field not in event:
                problems.append(f"event {event.get('id')}: no {field}")
    for reminder in world.get("reminders", []):
        for field in ("id", "title", "list", "due_day", "due_time"):
            if field not in reminder:
                problems.append(f"reminder {reminder.get('id')}: no {field}")

    questions = json.load(open(questions_path))
    seen = set()
    mix = collections.Counter()
    for at, item in enumerate(questions):
        where = f"question {at + 1} ({item.get('q', '')[:40]!r})"
        if set(item) - {"q", "apps", "kind", "work_out", "why"} or not {"q", "apps", "kind", "work_out"} <= set(item):
            problems.append(f"{where}: fields are {sorted(item)}")
            continue
        text = item["q"].strip().lower()
        if text in seen:
            problems.append(f"{where}: repeated")
        seen.add(text)
        if item["kind"] not in KINDS:
            problems.append(f"{where}: kind {item['kind']!r}")
        if not isinstance(item["apps"], list) or any(app not in APPS for app in item["apps"]) or len(set(item["apps"])) != len(item["apps"]):
            problems.append(f"{where}: apps {item['apps']!r}")
        if (item["kind"] == "general") != (item["apps"] == []):
            problems.append(f"{where}: a general question reads no app, and only a general one")
        if not isinstance(item["work_out"], bool):
            problems.append(f"{where}: work_out must be true or false")
        key = "general" if item["kind"] == "general" else "unavailable" if item["kind"] == "unavailable" else "+".join(app for app in APPS if app in item["apps"])
        mix[key] += 1
    for problem in problems:
        print(problem)
    print(f"{len(questions)} questions: " + ", ".join(f"{key} {count}" for key, count in sorted(mix.items())))
    print("ok" if not problems else f"{len(problems)} problems")
    return 1 if problems else 0


if __name__ == "__main__":
    if sys.argv[1] == "--questions-only":
        sys.exit(main(None, sys.argv[2]))
    sys.exit(main(*sys.argv[1:3]))
