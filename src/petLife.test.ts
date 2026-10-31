// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { actionDuration, gazeToward, Hunter, isToyAction, nextBlinkDelay, nextHabitDelay, nextIdleDelay, pickIdleAction, randomGlance, toyPlay, toyActions } from './petLife.ts';

const sequence = (...values: number[]) => () => values.shift() ?? 0;

test('idle actions follow the personality and the character', () => {
  const seen = new Set<string>();
  for (let index = 0; index < 200; index++) seen.add(pickIdleAction('sleepy', true, sequence(index / 200)));
  assert.deepEqual([...seen].sort(), ['ear', 'loaf', 'nod', 'nudge', 'tailcurl', 'yawn']);
  for (let index = 0; index < 200; index++) {
    const action = pickIdleAction('playful', false, sequence(index / 200));
    assert.ok(['tilt', 'wiggle', 'stretch', 'slowblink'].includes(action), action);
  }
  assert.equal(pickIdleAction('calm', true, sequence(0)), 'look');
  assert.equal(pickIdleAction('calm', true, sequence(.999999)), 'yawn');
  // A playful cat plays with its ball every way; a sleepy cat only nudges it; other
  // characters have no ball.
  const playful = new Set<string>(), sleepy = new Set<string>();
  for (let index = 0; index < 400; index++) {
    playful.add(pickIdleAction('playful', true, sequence(index / 400)));
    sleepy.add(pickIdleAction('sleepy', true, sequence(index / 400)));
    assert.ok(!isToyAction(pickIdleAction('playful', false, sequence(index / 400))));
  }
  for (const toy of ['bat', 'pounce', 'nudge']) assert.ok(playful.has(toy), toy);
  assert.ok(sleepy.has('nudge') && !sleepy.has('bat') && !sleepy.has('pounce'));
});

test('each play has its own strength and play comes in bursts', () => {
  assert.deepEqual(toyPlay(sequence(0)), { roll: .55, lift: .75 });
  const strong = toyPlay(sequence(.999999, .999999));
  assert.ok(strong.roll > 1.44 && strong.roll <= 1.45 && strong.lift > 1.24 && strong.lift <= 1.25);
  const usual = nextIdleDelay('playful', sequence(.5));
  assert.equal(nextIdleDelay('playful', sequence(.5, .1), true), usual * .35, 'a quick return after playing');
  assert.equal(nextIdleDelay('playful', sequence(.5, .9), true), usual, 'or the usual wait');
  assert.equal(nextIdleDelay('sleepy', sequence(.5, .1), true), nextIdleDelay('sleepy', sequence(.5)), 'sleepy cats don\u2019t rush back');
});

test('a quick return after playing comes once the play has finished, so it is not skipped', () => {
  for (const motion of ['calm', 'playful'] as const) for (const toy of toyActions) {
    const burst = nextHabitDelay(motion, toy, sequence(0, .1));
    assert.ok(burst > actionDuration[toy], `${motion} ${toy}: the next habit comes after the play ends`);
    assert.ok(burst < nextHabitDelay(motion, toy, sequence(0, .9)), `${motion} ${toy}: and sooner than usual`);
  }
  assert.equal(nextHabitDelay('calm', null, sequence(.5)), nextIdleDelay('calm', sequence(.5)), 'nothing played: the usual wait');
});

test('every action has a duration and playful pets act more often', () => {
  for (const motion of ['calm', 'playful', 'sleepy'] as const) {
    for (let index = 0; index < 50; index++) assert.ok(pickIdleAction(motion, true) in actionDuration);
  }
  for (const roll of [0, .5, 1]) assert.ok(nextIdleDelay('playful', sequence(roll)) < nextIdleDelay('calm', sequence(roll)));
  assert.ok(nextBlinkDelay('sleepy', sequence(0)) > nextBlinkDelay('calm', sequence(0)));
});

test('gaze points toward the cursor and stays within range', () => {
  assert.deepEqual(gazeToward(3, -4), { x: 0, y: 0 });
  const right = gazeToward(400, 0);
  assert.equal(right.x, 1);
  assert.equal(right.y, 0);
  const near = gazeToward(0, 80);
  assert.equal(near.x, 0);
  assert.ok(near.y > 0 && near.y < 1);
  for (let index = 0; index < 50; index++) {
    const glance = randomGlance();
    assert.ok(Math.abs(glance.x) <= 1 && glance.y >= -.7 && glance.y <= .5);
  }
});

test('a pointer that darts about nearby and then freezes gets pounced on, not one far away', () => {
  const hunter = new Hunter();
  // Darting back and forth 100–180 px to the cat's left.
  for (let at = 0, side = 1; at < 600; at += 100, side = -side) hunter.moved(-140 + side * 40, -20, at);
  assert.equal(hunter.ready(800, 'playful'), null, 'it waits until the pointer stops');
  assert.equal(hunter.ready(1200, 'playful'), -1, 'then pounces toward it');
  assert.equal(hunter.ready(1300, 'playful'), null, 'once, then rests');
  const lazy = new Hunter();
  for (let at = 0, side = 1; at < 600; at += 100, side = -side) lazy.moved(-140 + side * 40, -20, at);
  assert.equal(lazy.ready(1200, 'sleepy'), null, 'a sleepy cat can’t be bothered');
  const far = new Hunter();
  for (let at = 0, side = 1; at < 600; at += 100, side = -side) far.moved(-600 + side * 40, -20, at);
  assert.equal(far.ready(1200, 'playful'), null, 'too far away to bother');
});
