# The question router

`src-tauri/src/router.rs` decides, for a question with no app tag, which of the person's apps to read (Mail, Calendar, Reminders, or none for a general question) and whether its answer has to be worked out. It is a small network over the bundled search model's sentence vector (bge-small-en-v1.5, the one the memory index uses), so it runs on the Mac in a few milliseconds, needs no download, and doesn't change when macOS updates Apple's model. The weights are `src-tauri/src/router.bin`; `router.json` says what they are.

## How it's used

The controller (`continue_workflow_using`) reads the apps the question's words name, as before. With the router:

- the router's likeliest app is read, and any other app it is nearly sure of (above 0.9);
- an app it only finds likely (above 0.5) is read when the memory index finds the question's own words in that app's records that the other apps don't hold, or when there is no index yet;
- a question it takes for a general one reads nothing, unless an app is named or the rules see a question about the person's own things, in which case the rules route it as before;
- whether the answer is worked out comes from the router instead of the written examples' margin and Apple's one-word choice.

It isn't used for app tags (`/mail …`), for requests to do something, for reads elsewhere (the web, a page), or for text that isn't in the Latin alphabet; the search model is English.

## The data

`data/` holds questions about 12 made-up people, written by one writer per person and labelled again, blind, by another (WRITING.md, LABELLING.md):

- `world-<tag>.json`: each person's mail, calendar and reminders;
- `questions-<tag>.json`: about 200 questions per person;
- `questions-short-<tag>.json`: about 100 short or vague ones per person;
- `questions-general-<n>.json`: 817 general questions that sound personal or touch the apps' topics;
- `labels2-*.json`: the second labels.

That's 4,488 questions; the two labellers agree on the apps and kind of 4,274 (95%), and only those train the router. No question, name or record comes from an exam.

## Retraining

```bash
export BUNTINI_ROUTER_CACHE=/path/to/a/scratch/vectors.json   # optional; vectors are cached here
python3 scripts/router/agree.py            # how far the two labellings agree
python3 scripts/router/train.py cv         # leave one person out at a time
python3 scripts/router/train.py fit        # train on everyone; writes router.bin and router.json
python3 scripts/router/train.py score q.json   # [{q, reads}] scored by the saved weights
```

Vectors come from `router::tests::dump_router_vectors` (through `embed.py`), so training sees exactly what the app computes. `cargo test --lib router::` checks the weights load and fit the search model.

Leaving each person out in turn, the router picks exactly the right apps for 82% of the agreed questions (3,499 of 4,274) and gets the work-out right for 88%. A score on a spent exam means little; only a fresh, blind exam counts.
