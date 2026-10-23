// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { SpeechQueue } from './speech.ts';

function deferred() {
  let resolve = () => {};
  let reject = (_error: unknown) => {};
  const promise = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function tick() {
  await new Promise<void>(resolve => setTimeout(resolve, 0));
}

test('sentences play serially once; incomplete tails wait until finish', async () => {
  const played: string[] = [];
  const flights: ReturnType<typeof deferred>[] = [];
  const active: boolean[] = [];
  const speech = new SpeechQueue({
    play(text) { played.push(text); const flight = deferred(); flights.push(flight); return flight.promise; },
    async stop() { flights.at(-1)?.resolve(); },
    onActive(value) { active.push(value); },
  });
  const token = speech.begin();
  speech.append('One complete sentence. A second', token);
  await tick();
  assert.deepEqual(played, ['One complete sentence.']);
  speech.append(' sentence! A trailing thought', token);
  await tick();
  assert.equal(played.length, 1);
  flights[0].resolve();
  await tick();
  assert.deepEqual(played, ['One complete sentence.', 'A second sentence!']);
  flights[1].resolve();
  await tick();
  assert.equal(played.length, 2);
  speech.finish(token);
  speech.finish(token);
  speech.append(' ignored late data.', token);
  await tick();
  assert.equal(played[2], 'A trailing thought');
  flights[2].resolve();
  await tick();
  assert.deepEqual(active, [true, false, true, false]);
});

test('cancel immediately invalidates queued text and stops active audio', async () => {
  const played: string[] = [];
  const flight = deferred();
  let stops = 0;
  const speech = new SpeechQueue({
    play(text) { played.push(text); return flight.promise; },
    async stop() { stops += 1; flight.resolve(); },
    onActive() {},
  });
  const token = speech.begin();
  speech.append('Play this. Never play the queued sentence. Old tail', token);
  await tick();
  const stopped = speech.cancel();
  speech.append(' or this stale continuation.', token);
  speech.finish(token);
  await stopped;
  await tick();
  assert.equal(stops, 1);
  assert.deepEqual(played, ['Play this.']);
});

test('a summary fallback replaces queued draft speech with the authoritative answer', async () => {
  const played: string[] = [];
  const old = deferred();
  let stops = 0;
  const speech = new SpeechQueue({
    play(text) { played.push(text); return text === 'Partial draft.' ? old.promise : Promise.resolve(); },
    async stop() { stops += 1; old.resolve(); },
    onActive() {},
  });
  const token = speech.begin();
  const draft = 'Partial draft. Unverified continuation. Unfinished tail';
  speech.append(draft, token);
  await tick();
  speech.completeAnswer(token, 'Original source evidence.', draft);
  speech.append(' Stale delta.', token);
  await tick();
  await tick();
  assert.equal(stops, 1);
  assert.deepEqual(played, ['Partial draft.', 'Original source evidence.']);
});

test('authoritative completion neither replays matching streams nor revives cancelled speech', async () => {
  const played: string[] = [];
  const speech = new SpeechQueue({ play: async text => { played.push(text); }, stop: async () => {}, onActive() {} });
  const token = speech.begin();
  speech.append('A complete answer.', token);
  speech.completeAnswer(token, 'A complete answer.', 'A complete answer.');
  await tick();
  assert.deepEqual(played, ['A complete answer.']);
  const cancelled = speech.begin();
  await speech.cancel();
  speech.completeAnswer(cancelled, 'Do not restart me.', '');
  await tick();
  assert.deepEqual(played, ['A complete answer.']);
});

test('supersession waits for stop and old play completion before new audio', async () => {
  const played: string[] = [];
  const oldPlay = deferred();
  const oldStop = deferred();
  const states: boolean[] = [];
  const speech = new SpeechQueue({
    play(text) { played.push(text); return text === 'Old sentence.' ? oldPlay.promise : Promise.resolve(); },
    stop() { return oldStop.promise; },
    onActive(value) { states.push(value); },
  });
  const old = speech.begin();
  speech.append('Old sentence. Must never play.', old);
  await tick();
  const next = speech.begin();
  assert.deepEqual(states, [true, false]);
  speech.append('New sentence.', next);
  speech.finish(next);
  speech.append('Stale sentence.', old);
  oldPlay.resolve();
  await tick();
  assert.deepEqual(played, ['Old sentence.']);
  oldStop.resolve();
  await tick();
  assert.deepEqual(played, ['Old sentence.', 'New sentence.']);
  assert.deepEqual(states, [true, false, true, false]);
});

test('long completed sentences split at at most 800 characters without dropping words', async () => {
  const played: string[] = [];
  const speech = new SpeechQueue({ async play(text) { played.push(text); }, async stop() {}, onActive() {} });
  const text = Array.from({ length: 400 }, (_, index) => `word${index}`).join(' ');
  const token = speech.begin();
  speech.append(text, token);
  await tick();
  assert.deepEqual(played, []);
  speech.finish(token);
  await tick();
  assert.ok(played.length > 1);
  assert.ok(played.every(chunk => chunk.length <= 800));
  assert.equal(played.join(' '), text);
});

test('speakText resolves after final audio and can recover from playback errors', async () => {
  const errors: unknown[] = [];
  const played: string[] = [];
  const speech = new SpeechQueue({
    async play(text) { played.push(text); if (text === 'Failure.') throw new Error('audio unavailable'); },
    async stop() {}, onActive() {}, onError(error) { errors.push(error); },
  });
  await assert.rejects(speech.speakText('Failure. Queued text.'), /audio unavailable/);
  await speech.speakText('Recovered without punctuation');
  assert.deepEqual(played, ['Failure.', 'Recovered without punctuation']);
  assert.equal(errors.length, 1);
});

test('an explicit read-aloud promise resolves normally when interrupted', async () => {
  const flight = deferred();
  const speech = new SpeechQueue({ play() { return flight.promise; }, async stop() { flight.resolve(); }, onActive() {} });
  const completion = speech.speakText('A long response. More text.');
  await tick();
  await speech.cancel();
  await completion;
});

test('a decimal split across deltas is not mistaken for a sentence boundary', async () => {
  const played: string[] = [];
  const speech = new SpeechQueue({ async play(text) { played.push(text); }, async stop() {}, onActive() {} });
  const token = speech.begin();
  speech.append('CPU is 12.', token);
  await tick();
  assert.deepEqual(played, []);
  speech.append('5 percent. Dr. Fox agrees!', token);
  speech.finish(token);
  await tick();
  assert.deepEqual(played, ['CPU is 12.5 percent.', 'Dr. Fox agrees!']);
});

test('800-character hard wrapping preserves Unicode surrogate pairs', async () => {
  const played: string[] = [];
  const speech = new SpeechQueue({ async play(text) { played.push(text); }, async stop() {}, onActive() {} });
  const text = 'x'.repeat(799) + '🦊' + 'y'.repeat(900);
  await speech.speakText(text);
  assert.equal(played.join(''), text);
  assert.ok(played.every(chunk => chunk.length <= 800 && !/[\uD800-\uDBFF]$/.test(chunk) && !/^[\uDC00-\uDFFF]/.test(chunk)));
});
