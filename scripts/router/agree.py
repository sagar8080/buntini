"""How far the first and second labels agree, per person (see LABELLING.md).

    python3 scripts/router/agree.py [--show]
"""
import collections
import glob
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data")
APPS = ("mail", "calendar", "reminders")


def key(item):
    return "general" if item["kind"] == "general" else "+".join(app for app in APPS if app in item["apps"])


def main(show):
    total = agreed = work_total = work_agreed = 0
    confusions = collections.Counter()
    for path in sorted(glob.glob(os.path.join(DATA, "questions-*.json"))):
        tag = os.path.basename(path)[len("questions-"):-len(".json")]
        second = os.path.join(DATA, f"labels2-{tag}.json")
        if not os.path.exists(second):
            print(f"{tag}: no second labels yet")
            continue
        other = {item["q"]: item for item in json.load(open(second))}
        first = json.load(open(path))
        here = same = work = 0
        for item in first:
            check = other.get(item["q"])
            here += 1
            if check is None:
                confusions[(key(item), "unlabelled")] += 1
                continue
            if key(check) == key(item):
                same += 1
                work += check["work_out"] == item["work_out"]
            else:
                confusions[(key(item), key(check))] += 1
                if show:
                    print(f"  {tag}: first {key(item)} second {key(check)}: {item['q']}")
        print(f"{tag}: apps and kind agree on {same}/{here}; work out agrees on {work}/{same}")
        total += here
        agreed += same
        work_total += same
        work_agreed += work
    if total:
        print(f"all: {agreed}/{total} agree ({agreed / total:.0%}); work out {work_agreed}/{work_total}")
        for (first, second), count in confusions.most_common(12):
            print(f"  {count:3d}  first {first:28s} second {second}")


if __name__ == "__main__":
    main("--show" in sys.argv)
