// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { PURR_GAIN, purrGain, PurrSound } from './purrSound.ts';

/** A stand-in AudioContext that records what was built and how loud it was set. */
function fake() {
  const made = { contexts: 0, nodes: 0, targets: [] as number[], state: 'running', suspended: 0 };
  const param = () => ({ value: 0, setTargetAtTime: (value: number) => { made.targets.push(value); } });
  const node = () => { made.nodes++; const self: any = { gain: param(), frequency: param(), connect: (next: unknown) => next, start: () => {} }; return self; };
  const context: any = {
    currentTime: 0, destination: {}, sampleRate: 8000,
    get state() { return made.state; },
    createBuffer: (_: number, length: number) => ({ getChannelData: () => new Float32Array(length) }),
    createBufferSource: node, createBiquadFilter: node, createGain: node, createOscillator: node,
    resume: async () => { made.state = 'running'; },
    suspend: async () => { made.state = 'suspended'; made.suspended++; },
  };
  return { made, make: () => { made.contexts++; return context; } };
}

test('loudness follows the purr and is silent when not allowed', () => {
  assert.equal(purrGain(1, true), PURR_GAIN);
  assert.equal(purrGain(.5, true), PURR_GAIN / 2);
  assert.equal(purrGain(2, true), PURR_GAIN);
  assert.equal(purrGain(1, false), 0);
  assert.ok(20 * Math.log10(PURR_GAIN) < -29.9 && 20 * Math.log10(PURR_GAIN) > -30.1, 'about −30 dBFS');
});

test('no audio is started when a purr may not be heard, or before a click or key press', () => {
  const blocked = fake();
  const sound = new PurrSound(blocked.make);
  sound.unlock();
  for (let at = 0; at < 3000; at += 100) sound.set(1, false, at);
  assert.equal(blocked.made.contexts, 0);
  const locked = fake();
  new PurrSound(locked.make).set(1, true, 0);
  assert.equal(locked.made.contexts, 0, 'browsers need a gesture first');
});

test('a purr builds the sound once, follows it, and pauses after two quiet seconds', () => {
  const { made, make } = fake();
  const sound = new PurrSound(make);
  sound.unlock();
  sound.set(.5, true, 0);
  sound.set(1, true, 100);
  assert.equal(made.contexts, 1);
  assert.deepEqual(made.targets, [PURR_GAIN / 2, PURR_GAIN]);
  sound.set(1, false, 200);
  assert.equal(made.targets.at(-1), 0, 'a call starting silences it');
  sound.set(0, true, 1000);
  assert.equal(made.suspended, 0);
  sound.set(0, true, 2400);
  assert.equal(made.suspended, 1);
});
