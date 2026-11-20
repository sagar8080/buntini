// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { approach, chooseMove, ground, landing, NudgeRules, standingOn, walkAway, windowTop, zoomies, type World } from './petWorld.ts';

const MINUTE = 60_000;
const area = { x: 0, y: 25, width: 1440, height: 850 };
function world(patch: Partial<World> = {}): World {
  const pet = { x: 600, y: 25 + 850 - 136 + 16, width: 128, height: 136 };
  return { idleSeconds: 2, app: { name: 'Xcode', category: 'code' }, frontWindow: { number: 7, rect: { x: 200, y: 300, width: 900, height: 500 } }, workArea: area, pet, fullscreen: false, meeting: false, health: { batteryPercent: 80, charging: false, cpuPercent: 10, diskFreePercent: 40 }, perched: false, ...patch };
}
const sequence = (...values: number[]) => () => values.length ? values.shift()! : 0;

test('the pet stands on the floor above the Dock or on the front window', () => {
  assert.equal(ground(world(), 1).y, 755);
  assert.equal(standingOn(world(), 1), 'ground');
  const top = windowTop(world(), 1)!;
  assert.equal(top.y, 300 - 136 + 16);
  assert.equal(top.perch, 7);
  assert.equal(standingOn(world({ perched: true }), 1), 'window');
  assert.equal(standingOn(world({ pet: { x: 600, y: 400, width: 128, height: 136 } }), 1), 'air');
});

test('windows too close to the menu bar, too narrow or full screen cannot be perched on', () => {
  assert.equal(windowTop(world({ frontWindow: { number: 1, rect: { x: 0, y: 40, width: 900, height: 800 } } }), 1), null);
  assert.equal(windowTop(world({ frontWindow: { number: 1, rect: { x: 0, y: 300, width: 200, height: 300 } } }), 1), null);
  assert.equal(windowTop(world({ fullscreen: true }), 1), null);
});

test('a dropped pet lands on the window below it, otherwise falls to the floor', () => {
  const above = landing(world({ pet: { x: 500, y: 60, width: 128, height: 136 } }), 1)!;
  assert.deepEqual(above, { kind: 'fall', x: 500, y: 180, perch: 7 });
  const beside = landing(world({ pet: { x: 1250, y: 60, width: 128, height: 136 } }), 1)!;
  assert.deepEqual(beside, { kind: 'fall', x: 1250, y: 755 });
  assert.equal(landing(world(), 1), null);
});

test('moves stay on the current surface and only visit windows when the user is active', () => {
  for (let index = 0; index < 200; index++) {
    const move = chooseMove(world(), 1, 'playful', Math.random, 900);
    if (!move) continue;
    assert.ok(move.x >= 0 && move.x <= 1440 - 128);
    assert.ok([755, 180].includes(move.y), JSON.stringify(move));
  }
  const away = world({ idleSeconds: 600 });
  for (let index = 0; index < 100; index++) assert.notEqual(chooseMove(away, 1, 'playful')?.kind, 'jump');
  assert.equal(chooseMove(world(), 1, 'calm', sequence(0)), null, 'the first option is staying put');
});

test('the pet walks under a window before jumping, so jumps are never long slides', () => {
  const far = world({ pet: { x: 1300, y: 755, width: 128, height: 136 }, frontWindow: { number: 7, rect: { x: 100, y: 300, width: 600, height: 400 } } });
  for (let index = 0; index < 300; index++) {
    const move = chooseMove(far, 1, 'calm');
    if (move?.kind === 'jump') assert.ok(Math.abs(move.x - 1300) <= 140, `jump from 1300 to ${move.x}`);
  }
  // The window spans pet positions 160–1012; find the "visit the window" choice among random moves.
  const visit = (pet: World['pet']) => {
    for (let index = 0; index < 500; index++) {
      const move = chooseMove(world({ pet }), 1, 'calm');
      if (move?.kind === 'jump' || (move?.kind === 'walk' && move.x === 160)) return move;
    }
    return null;
  };
  assert.deepEqual(visit({ x: 20, y: 755, width: 128, height: 136 }), { kind: 'walk', x: 160, y: 755 }, 'walks to the window edge first');
  assert.equal(visit({ x: 150, y: 755, width: 128, height: 136 })?.kind, 'jump', 'already close enough to hop');
  const under = visit({ x: 600, y: 755, width: 128, height: 136 });
  assert.equal(under?.kind, 'jump');
  assert.equal(under?.x, 600);
});

test('break reminders follow a long active streak and respect the nudge gap', () => {
  const rules = new NudgeRules(50 * MINUTE, 10 * MINUTE);
  const quiet = world({ app: null });
  assert.equal(rules.update(quiet, 0, 14), null);
  assert.equal(rules.update(quiet, 49 * MINUTE, 14), null);
  const nudge = rules.update(quiet, 50 * MINUTE, 14)!;
  assert.equal(nudge.kind, 'break');
  rules.snoozeBreak(50 * MINUTE);
  assert.equal(rules.update(quiet, 60 * MINUTE, 14), null);
  assert.equal(rules.update(quiet, 65 * MINUTE + 1, 14)?.kind, 'break');
});

test('meetings and full screen silence nudges; returning from a long break gets a welcome', () => {
  const rules = new NudgeRules(1 * MINUTE, 0);
  assert.equal(rules.update(world({ meeting: true, app: null }), 0, 14), null);
  assert.equal(rules.update(world({ meeting: true, app: null }), 5 * MINUTE, 14), null);
  const away = new NudgeRules();
  away.update(world({ idleSeconds: 2, app: null }), 0, 14);
  away.update(world({ idleSeconds: 900, app: null }), 15 * MINUTE, 14);
  assert.match(away.update(world({ idleSeconds: 1, app: null }), 16 * MINUTE, 14)!.text, /Welcome back! You were away 16 minutes/);
});

test('health and context offers fire once with their own cooldowns', () => {
  const rules = new NudgeRules(10 * 60 * MINUTE, 0);
  const low = world({ app: null, health: { batteryPercent: 12, charging: false, cpuPercent: 10, diskFreePercent: 40 } });
  assert.equal(rules.update(low, 0, 14)?.kind, 'battery');
  assert.equal(rules.update(low, MINUTE, 14), null, 'battery warns once per discharge');
  const browsing = world({ app: { name: 'Safari', category: 'browser' } });
  assert.equal(rules.update(browsing, 2 * MINUTE, 14), null, 'offers wait until three minutes in the app');
  assert.equal(rules.update(browsing, 5 * MINUTE + 1, 14)?.kind, 'app-browser');
  const fresh = new NudgeRules(10 * 60 * MINUTE, 0);
  fresh.update(browsing, 0, 14);
  assert.equal(fresh.update(browsing, 3 * MINUTE, 14)?.actions[0].action, 'lookUp');
  assert.equal(fresh.update(browsing, 4 * MINUTE, 14), null, 'one offer per app category per hour');
  const hot = new NudgeRules(10 * 60 * MINUTE, 0);
  const busy = world({ app: null, health: { batteryPercent: 80, charging: true, cpuPercent: 96, diskFreePercent: 40 } });
  assert.equal(hot.update(busy, 0, 14), null);
  assert.equal(hot.update(busy, 2 * MINUTE, 14)?.kind, 'cpu');
});

test('focus timers end with a nudge and pause break reminders', () => {
  const rules = new NudgeRules(30 * MINUTE, 0);
  const coding = world({ app: null });
  rules.update(coding, 0, 10);
  rules.startFocus(0, 25);
  assert.equal(rules.update(coding, 24 * MINUTE, 10), null);
  assert.equal(rules.update(coding, 25 * MINUTE, 10)?.kind, 'focus-done');
  assert.equal(rules.update(coding, 26 * MINUTE, 10), null, 'the break clock restarts after focus');
  rules.startFocus(30 * MINUTE, 25, true);
  assert.equal(rules.update(coding, 55 * MINUTE, 10), null, "the island's timer says when it ends");
});

test('ordinary roaming never sprints: zoomies come in their own bursts', () => {
  for (const motion of ['calm', 'playful', 'sleepy'] as const) {
    for (let index = 0; index < 300; index++) assert.ok(!chooseMove(world(), 1, motion, Math.random, 900)?.dash, motion);
  }
});

test('zoomies sprint to the far end and most of the way back, on the surface it stands on', () => {
  const [there, back] = zoomies(world(), 1, sequence(.5, .5));
  assert.deepEqual(there, { kind: 'walk', x: 1312, y: 755, perch: undefined, dash: true });
  assert.ok(back.dash && back.x < 1312 && back.x > 600 && back.y === 755, JSON.stringify(back));
  const perched = zoomies(world({ perched: true, pet: { x: 300, y: 180, width: 128, height: 136 } }), 1, sequence(.5, .5));
  assert.ok(perched.every(move => move.y === 180 && move.perch === 7 && move.x >= 160 && move.x <= 1012), JSON.stringify(perched));
  assert.deepEqual(zoomies(world({ pet: { x: 600, y: 400, width: 128, height: 136 } }), 1), [], 'not in mid-air');
  const narrow = world({ workArea: { x: 0, y: 25, width: 300, height: 850 }, pet: { x: 100, y: 755, width: 128, height: 136 }, frontWindow: null });
  assert.deepEqual(zoomies(narrow, 1), [], 'no room for a sprint');
});

test('fussed over, the cat walks off away from the pointer, or the other way when cornered', () => {
  const off = walkAway(world(), 1, 900, sequence(.5))!;
  assert.equal(off.kind, 'walk');
  assert.ok(off.x < 600 - 150, `away from a pointer on its right: ${off.x}`);
  assert.ok(walkAway(world(), 1, 100, sequence(.5))!.x > 750, 'away from a pointer on its left');
  const cornered = walkAway(world({ pet: { x: 0, y: 755, width: 128, height: 136 } }), 1, 900, sequence(.5))!;
  assert.ok(cornered.x > 150, `cornered on the left, it goes right: ${cornered.x}`);
  assert.equal(walkAway(world({ pet: { x: 600, y: 400, width: 128, height: 136 } }), 1, 900), null, 'not in mid-air');
});

test('coming over, the cat walks under your window, hops up and sits below the pointer', () => {
  // The window spans pet positions 160 to 1012; the pointer is at x 900.
  const under = approach(world({ pet: { x: 100, y: 755, width: 128, height: 136 } }), 1, 900)!;
  assert.deepEqual(under, { kind: 'walk', x: 836, y: 755 });
  const up = approach(world({ pet: { x: 836, y: 755, width: 128, height: 136 } }), 1, 900)!;
  assert.deepEqual(up, { kind: 'jump', x: 836, y: 180, perch: 7 });
  assert.equal(approach(world({ perched: true, pet: { x: 836, y: 180, width: 128, height: 136 } }), 1, 900), null, 'there');
  const along = approach(world({ perched: true, pet: { x: 300, y: 180, width: 128, height: 136 } }), 1, 900)!;
  assert.deepEqual(along, { kind: 'walk', x: 836, y: 180, perch: 7 });
  // A pointer past the window's end: the nearest end of the window.
  assert.equal(approach(world({ perched: true, pet: { x: 300, y: 180, width: 128, height: 136 } }), 1, 1400)!.x, 1012);
  // Still perched on a window that's gone behind another: down to the floor first.
  const behind = world({ perched: true, pet: { x: 300, y: 420, width: 128, height: 136 } });
  assert.deepEqual(approach(behind, 1, 900), { kind: 'jump', x: 300, y: 755 });
  assert.deepEqual(zoomies(behind, 1), [], 'no sprint along a window it can’t see the ends of');
  assert.equal(walkAway(behind, 1, 900), null);
  // No window to sit on: the floor below the pointer.
  assert.deepEqual(approach(world({ frontWindow: null }), 1, 200), { kind: 'walk', x: 136, y: 755 });
  assert.equal(approach(world({ frontWindow: null, pet: { x: 140, y: 755, width: 128, height: 136 } }), 1, 200), null);
});
