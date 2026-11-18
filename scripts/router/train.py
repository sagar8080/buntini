"""Trains the question router (`src-tauri/src/router.rs`) on the labelled questions in `data/`.

    python3 scripts/router/train.py cv                 # leave one made-up person out at a time
    python3 scripts/router/train.py fit                # train on everyone, write the weights
    python3 scripts/router/train.py score exams.json   # score saved weights on [{q, reads, kind}]

Vectors come from the bundled search model (`embed.py`), cached in `$BUNTINI_ROUTER_CACHE`.
Only questions both labellers agree on are used (`labels2-<tag>.json`, see LABELLING.md).
"""
import glob
import json
import os
import sys

import numpy as np

from embed import vectors_for

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data")
# The weights as little-endian f32 (mean, w1, b1, w2, b2), and what they are in JSON beside them.
WEIGHTS = os.path.join(HERE, "..", "..", "src-tauri", "src", "router.bin")
ABOUT = os.path.join(HERE, "..", "..", "src-tauri", "src", "router.json")
CACHE = os.environ.get("BUNTINI_ROUTER_CACHE", os.path.join(HERE, ".vectors.json"))
APPS = ("mail", "calendar", "reminders")
HEADS = ("general",) + APPS + ("work_out",)
MODEL_ID = "bge-small-en-v1.5@5c38ec7"
HIDDEN = int(os.environ.get("ROUTER_HIDDEN", "192"))
DECAY = float(os.environ.get("ROUTER_DECAY", "1e-4"))
EPOCHS = int(os.environ.get("ROUTER_EPOCHS", "400"))
SEED = 7


def key(item):
    return "general" if item["kind"] == "general" else "+".join(app for app in APPS if app in item["apps"])


def load():
    """Every agreed question as (person tag, text, label row)."""
    rows = []
    for path in sorted(glob.glob(os.path.join(DATA, "questions-*.json"))):
        tag = os.path.basename(path)[len("questions-"):-len(".json")]
        second = os.path.join(DATA, f"labels2-{tag}.json")
        # A person's short questions are held out with the rest of theirs.
        person = tag[len("short-"):] if tag.startswith("short-") else tag
        other = {item["q"]: item for item in json.load(open(second))} if os.path.exists(second) else None
        for item in json.load(open(path)):
            if other is not None:
                check = other.get(item["q"])
                if check is None or key(check) != key(item):
                    continue
            label = [1.0 if item["kind"] == "general" else 0.0] + [1.0 if app in item["apps"] else 0.0 for app in APPS]
            label.append(1.0 if item["work_out"] else 0.0)
            # A work-out both labellers disagree on still teaches the apps, not the work-out.
            work_known = other is None or other[item["q"]]["work_out"] == item["work_out"]
            rows.append((person, item["q"], label, work_known))
    return rows


def features(vectors):
    return np.asarray(vectors, dtype=np.float32) / 127.0


class Net:
    def __init__(self, inputs, rng):
        self.mean = np.zeros(inputs, dtype=np.float32)
        self.w1 = (rng.standard_normal((inputs, HIDDEN)) * np.sqrt(2.0 / inputs)).astype(np.float32)
        self.b1 = np.zeros(HIDDEN, dtype=np.float32)
        self.w2 = (rng.standard_normal((HIDDEN, len(HEADS))) * np.sqrt(1.0 / HIDDEN)).astype(np.float32)
        self.b2 = np.zeros(len(HEADS), dtype=np.float32)

    def forward(self, x):
        h = np.maximum(0.0, (x - self.mean) @ self.w1 + self.b1)
        return h, 1.0 / (1.0 + np.exp(-(h @ self.w2 + self.b2)))

    def fit(self, x, y, mask, rng):
        self.mean = x.mean(axis=0)
        params = [self.w1, self.b1, self.w2, self.b2]
        moments = [(np.zeros_like(p), np.zeros_like(p)) for p in params]
        batch = 64
        step = 0
        for _ in range(EPOCHS):
            order = rng.permutation(len(x))
            for start in range(0, len(x), batch):
                pick = order[start:start + batch]
                xb, yb, mb = x[pick] - self.mean, y[pick], mask[pick]
                # A little dropout on the vector keeps it from leaning on a few dimensions.
                keep = (rng.random(xb.shape) > 0.1).astype(np.float32) / 0.9
                xb = xb * keep
                h = np.maximum(0.0, xb @ self.w1 + self.b1)
                p = 1.0 / (1.0 + np.exp(-(h @ self.w2 + self.b2)))
                dz = (p - yb) * mb / len(pick)
                gw2 = h.T @ dz + DECAY * self.w2
                gb2 = dz.sum(axis=0)
                dh = (dz @ self.w2.T) * (h > 0)
                gw1 = xb.T @ dh + DECAY * self.w1
                gb1 = dh.sum(axis=0)
                step += 1
                for param, grad, (m, v) in zip(params, [gw1, gb1, gw2, gb2], moments):
                    m *= 0.9
                    m += 0.1 * grad
                    v *= 0.999
                    v += 0.001 * grad * grad
                    param -= 1e-3 * (m / (1 - 0.9 ** step)) / (np.sqrt(v / (1 - 0.999 ** step)) + 1e-8)
        return self


def decide(p, thresholds):
    """The apps a question reads (empty for a general one) and whether it's worked out."""
    if p[0] > thresholds["general"]:
        apps = ()
    else:
        apps = tuple(app for at, app in enumerate(APPS) if p[1 + at] > thresholds["app"])
        if not apps:
            apps = (APPS[int(np.argmax(p[1:4]))],)
    return apps, p[4] > thresholds["work_out"]


def masks(rows):
    return np.asarray([[1.0, 1.0, 1.0, 1.0, 1.0 if work_known else 0.0] for *_, work_known in rows], dtype=np.float32)


def report(name, predictions, truths):
    exact = sum(1 for (apps, _), truth in zip(predictions, truths) if set(apps) == set(truth))
    general_said = sum(1 for (apps, _), truth in zip(predictions, truths) if not apps and truth)
    general_missed = sum(1 for (apps, _), truth in zip(predictions, truths) if apps and not truth)
    missing = sum(1 for (apps, _), truth in zip(predictions, truths) if apps and truth and set(truth) - set(apps))
    extra = sum(1 for (apps, _), truth in zip(predictions, truths) if apps and truth and set(apps) - set(truth))
    print(f"{name}: {exact}/{len(truths)} exact; personal called general {general_said}, general given apps {general_missed}, missing an app {missing}, extra app {extra}")
    return exact


def cv(thresholds):
    rows = load()
    vectors = features(vectors_for([text for _, text, _, _ in rows], CACHE))
    labels = np.asarray([label for _, _, label, _ in rows], dtype=np.float32)
    tags = sorted({tag for tag, *_ in rows})
    probabilities = np.zeros_like(labels)
    for tag in tags:
        test = np.asarray([row[0] == tag for row in rows])
        rng = np.random.default_rng(SEED)
        net = Net(vectors.shape[1], rng).fit(vectors[~test], labels[~test], masks([row for row, held in zip(rows, test) if not held]), rng)
        probabilities[test] = net.forward(vectors[test])[1]
    truths = [tuple(app for at, app in enumerate(APPS) if label[1 + at]) if not label[0] else () for label in labels]
    predictions = [decide(p, thresholds) for p in probabilities]
    report(f"held-out people ({len(tags)})", predictions, truths)
    known = masks(rows)[:, 4] > 0
    work = sum(1 for (_, said), label, k in zip(predictions, labels, known) if k and said == bool(label[4]))
    print(f"work out: {work}/{int(known.sum())} right")
    return probabilities, labels


def fit(thresholds):
    rows = load()
    vectors = features(vectors_for([text for _, text, _, _ in rows], CACHE))
    labels = np.asarray([label for _, _, label, _ in rows], dtype=np.float32)
    rng = np.random.default_rng(SEED)
    net = Net(vectors.shape[1], rng).fit(vectors, labels, masks(rows), rng)
    parts = [net.mean, net.w1, net.b1, net.w2, net.b2]
    np.concatenate([part.astype("<f4").ravel() for part in parts]).tofile(WEIGHTS)
    about = {"model": MODEL_ID, "heads": list(HEADS), "inputs": int(net.mean.size), "hidden": int(net.b1.size), "trained_on": len(rows), "thresholds": thresholds}
    json.dump(about, open(ABOUT, "w"), indent=1)
    open(ABOUT, "a").write("\n")
    print(f"trained on {len(rows)} questions from {len({tag for tag, *_ in rows})} people; wrote {os.path.relpath(WEIGHTS)}")


def saved():
    about = json.load(open(ABOUT))
    flat = np.fromfile(WEIGHTS, dtype="<f4")
    inputs, hidden, heads = about["inputs"], about["hidden"], len(about["heads"])
    net = Net(inputs, np.random.default_rng(SEED))
    sizes = [("mean", (inputs,)), ("w1", (inputs, hidden)), ("b1", (hidden,)), ("w2", (hidden, heads)), ("b2", (heads,))]
    at = 0
    for name, shape in sizes:
        count = int(np.prod(shape))
        setattr(net, name, flat[at:at + count].reshape(shape))
        at += count
    assert at == flat.size
    return net, about


def score(path):
    net, weights = saved()
    items = json.load(open(path))
    probabilities = net.forward(features(vectors_for([item["q"] for item in items], CACHE)))[1]
    groups = sorted({item.get("exam", "") for item in items})
    for group in groups:
        chosen = [(item, p) for item, p in zip(items, probabilities) if item.get("exam", "") == group]
        predictions = [decide(p, weights["thresholds"]) for _, p in chosen]
        report(f"exam {group}" if group else "exam", predictions, [tuple(item["reads"]) for item, _ in chosen])
        if os.environ.get("ROUTER_SHOW"):
            for (item, p), (apps, work) in zip(chosen, predictions):
                if set(apps) != set(item["reads"]):
                    print(f"  want {'+'.join(item['reads']) or 'general'} got {'+'.join(apps) or 'general'} [{' '.join(f'{v:.2f}' for v in p)}] {item['q'][:90]}")


def margin():
    """How the written-example margin in decision.rs (`work_out_margin`) does on the same labels."""
    import re
    source = open(os.path.join(HERE, "..", "..", "src-tauri", "src", "decision.rs")).read()
    lists = {}
    for name in ("WORK_OUT_EXAMPLES", "LOOK_UP_EXAMPLES"):
        body = source[source.index(f"const {name}"):]
        body = body[body.index("= [") + 3:body.index("];")]
        lists[name] = re.findall(r'"((?:[^"\\]|\\.)*)"', body)
    work = features(vectors_for(lists["WORK_OUT_EXAMPLES"], CACHE))
    look = features(vectors_for(lists["LOOK_UP_EXAMPLES"], CACHE))
    unit = lambda m: m / np.linalg.norm(m, axis=1, keepdims=True)
    work, look = unit(work), unit(look)
    rows = [row for row in load() if row[3] and not row[2][0]]
    asked = unit(features(vectors_for([text for _, text, _, _ in rows], CACHE)))
    top3 = lambda sims: np.sort(sims, axis=1)[:, -3:].mean(axis=1)
    scores = top3(asked @ work.T) - top3(asked @ look.T)
    truth = np.asarray([row[2][4] for row in rows]) > 0
    for threshold in (-0.02, 0.0):
        right = int(((scores > threshold) == truth).sum())
        print(f"margin > {threshold}: {right}/{len(rows)} personal work-outs right")


THRESHOLDS = {"general": 0.5, "app": 0.5, "work_out": 0.5}

if __name__ == "__main__":
    command = sys.argv[1] if len(sys.argv) > 1 else "cv"
    if command == "cv":
        cv(THRESHOLDS)
    elif command == "fit":
        fit(THRESHOLDS)
    elif command == "score":
        score(sys.argv[2])
    elif command == "margin":
        margin()
