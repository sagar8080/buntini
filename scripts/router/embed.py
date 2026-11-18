"""Gives every text the bundled search model's vector, as Buntini makes it at run time.

    python3 scripts/router/embed.py texts.json vectors.json

Texts already in `vectors.json` are kept; new ones run through `router::tests::dump_router_vectors`.
"""
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
CRATE = os.path.join(HERE, "..", "..", "src-tauri")


def vectors_for(texts, cache_path):
    cache = json.load(open(cache_path)) if os.path.exists(cache_path) else {}
    missing = sorted({text for text in texts if text not in cache})
    if missing:
        with tempfile.TemporaryDirectory() as scratch:
            asked = os.path.join(scratch, "texts.json")
            answered = os.path.join(scratch, "vectors.json")
            json.dump(missing, open(asked, "w"))
            env = dict(os.environ, BUNTINI_ROUTER_TEXTS=asked, BUNTINI_ROUTER_VECTORS=answered)
            env.setdefault("CARGO_TARGET_DIR", os.path.expanduser("~/.cargo-target-agents"))
            subprocess.run(
                ["cargo", "test", "--lib", "router::tests::dump_router_vectors", "--", "--ignored", "--exact", "--nocapture"],
                cwd=CRATE, env=env, check=True, stdout=subprocess.DEVNULL,
            )
            for text, vector in zip(missing, json.load(open(answered))):
                cache[text] = vector
        json.dump(cache, open(cache_path, "w"))
    return [cache[text] for text in texts]


if __name__ == "__main__":
    texts = json.load(open(sys.argv[1]))
    print(len(vectors_for(texts, sys.argv[2])), "vectors")
