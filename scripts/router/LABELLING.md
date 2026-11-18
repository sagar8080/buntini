# The second, blind labelling

Each person's questions are labelled a second time by someone who never saw the first labels. The router trains only on questions where both agree on the apps and the kind; a work-out the two disagree on teaches the apps but not the work-out.

The second labeller gets the person's `world-<tag>.json` and the bare questions (`q` only, in their own order), and writes `data/labels2-<tag>.json`: one entry per question, in the same order, with `q` copied exactly and `apps`, `kind` and `work_out` decided by the rules under "`questions-<tag>.json`" in WRITING.md, from where the records actually are in that world. Never open `data/questions-<tag>.json`.

Compare the two with:

```bash
python3 scripts/router/agree.py
```
