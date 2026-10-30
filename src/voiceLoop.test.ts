// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { Endpointer, isConversationEnd, shouldBargeIn } from './voiceLoop.ts';

test('sign-offs end a voice conversation; requests containing the same words do not', () => {
  for (const said of ["That's all.", 'thanks, that’s it', 'Okay, stop.', 'Stop listening', 'never mind', 'Thank you!', 'No thanks', 'bye', "That's all for now, Buntini"]) {
    assert.equal(isConversationEnd(said), true, said);
  }
  for (const said of ['Stop the timer', 'Cancel my 3pm meeting', 'Is that all I have today?', 'What else is on?', '', 'no', 'Thanks, and what about tomorrow at noon then?']) {
    assert.equal(isConversationEnd(said), false, said);
  }
});

test('only clear speech interrupts a reply', () => {
  assert.equal(shouldBargeIn(''), false);
  assert.equal(shouldBargeIn('um'), false);
  assert.equal(shouldBargeIn('Stop'), true);
  assert.equal(shouldBargeIn('wait'), true);
  assert.equal(shouldBargeIn('what about tomorrow'), true);
});

test('live words: a pause after the last new word ends the turn', () => {
  const turn = new Endpointer({ pauseMs: 1500, noSpeechMs: 8000 });
  turn.begin(true, 0);
  assert.equal(turn.check(1000), 'listening');
  turn.text('what is', 1200);
  turn.text('what is on my calendar', 2000);
  // The same partial again is not a new word.
  turn.text('what is on my calendar', 3000);
  assert.equal(turn.check(3400), 'listening');
  assert.equal(turn.check(3500), 'endpoint');
});

test('nothing said for a while times out instead of sending', () => {
  const turn = new Endpointer({ noSpeechMs: 8000 });
  turn.begin(true, 0);
  // Background noise alone does not count as speech when words stream.
  for (let at = 0; at < 8000; at += 100) turn.level(0.3, at);
  assert.equal(turn.check(7999), 'listening');
  assert.equal(turn.check(8000), 'timeout');
  assert.equal(turn.heardSomething, false);
});

test('while Buntini talks, the clock waits; it starts when the reply ends', () => {
  const turn = new Endpointer({ pauseMs: 1500, noSpeechMs: 8000 });
  turn.begin(true, 0);
  turn.setSpeaking(true, 0);
  assert.equal(turn.check(20000), 'listening');
  turn.setSpeaking(false, 20000);
  assert.equal(turn.check(27999), 'listening');
  assert.equal(turn.check(28000), 'timeout');
});

test('talking over the reply, then pausing after it ends, sends the turn', () => {
  const turn = new Endpointer({ pauseMs: 1500 });
  turn.begin(true, 0);
  turn.setSpeaking(true, 0);
  turn.text('wait what about', 1000);
  // The reply is cut off by the barge-in; the pause is measured from then on.
  turn.setSpeaking(false, 1100);
  assert.equal(turn.check(2500), 'listening');
  assert.equal(turn.check(2600), 'endpoint');
});

test('without live words, quiet after speech louder than the room ends the turn', () => {
  const turn = new Endpointer({ pauseMs: 1500, noSpeechMs: 8000 });
  turn.begin(false, 0);
  let at = 0;
  for (; at < 1000; at += 100) turn.level(0.004, at);
  // A single click is not speech.
  turn.level(0.2, at); at += 100; turn.level(0.004, at); at += 100;
  assert.equal(turn.heardSomething, false);
  for (const end = at + 1500; at < end; at += 100) turn.level(0.12, at);
  assert.equal(turn.heardSomething, true);
  const lastLoud = at - 100;
  for (const end = at + 1500; at < end; at += 100) turn.level(0.005, at);
  assert.equal(turn.check(lastLoud + 1400), 'listening');
  assert.equal(turn.check(lastLoud + 1500), 'endpoint');
});

test('a long turn is capped and still sent', () => {
  const turn = new Endpointer({ maxTurnMs: 30000 });
  turn.begin(true, 0);
  for (let at = 0; at < 30000; at += 500) turn.text(`word ${at}`, at);
  assert.equal(turn.check(30000), 'limit');
});

test('nothing is decided before the microphone opens', () => {
  const turn = new Endpointer({ noSpeechMs: 10 });
  assert.equal(turn.check(1_000_000), 'listening');
});
