// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { actionDuration, gazeToward, nextBlinkDelay, nextIdleDelay, pickIdleAction, randomGlance } from './petLife.ts';

const sequence = (...values: number[]) => () => values.shift() ?? 0;

test('idle actions follow the personality and the character', () => {
  const seen = new Set<string>();
  for (let index = 0; index < 200; index++) seen.add(pickIdleAction('sleepy', true, sequence(index / 200)));
  assert.deepEqual([...seen].sort(), ['ear', 'nod', 'tailcurl', 'yawn']);
  for (let index = 0; index < 200; index++) {
    const action = pickIdleAction('playful', false, sequence(index / 200));
    assert.ok(['tilt', 'wiggle', 'stretch', 'slowblink'].includes(action), action);
  }
  assert.equal(pickIdleAction('calm', true, sequence(0)), 'look');
  assert.equal(pickIdleAction('calm', true, sequence(.999999)), 'yawn');
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
