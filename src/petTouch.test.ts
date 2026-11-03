// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { Petting, PURR_FALL, zoneAt, type PetContext, type Zone } from './petTouch.ts';

const calm: PetContext = { cat: true, motion: 'calm', annoyance: 0, favourite: null };
const playful: PetContext = { ...calm, motion: 'playful' };

/** Steady strokes of 12 px every 16 ms (750 px/s) on `zone` for `ms`, from `from`. */
function pet(petting: Petting, zone: Zone, ms: number, context = calm, from = 0) {
  const seen: string[] = [];
  let at = from;
  for (; at <= from + ms; at += 16) {
    const touch = petting.stroke(zone, 12, at, context);
    if (touch.play && seen.at(-1) !== touch.play) seen.push(touch.play);
  }
  return { seen, at, purr: petting.purr(at) };
}

test('each part of the seated cat is its own zone', () => {
  const cases: [number, number, Zone | null][] = [
    [64, 55, 'ears'], [116, 55, 'ears'], [90, 70, 'head'], [62, 106, 'cheeks'], [118, 106, 'cheeks'], [90, 120, 'chin'],
    [90, 138, 'chest'], [66, 138, 'back'], [114, 140, 'back'], [79, 156, 'paws'], [63, 160, 'paws'], [145, 128, 'tail'],
    [10, 10, null], [170, 60, null], [90, 175, null],
  ];
  for (const [x, y, zone] of cases) assert.equal(zoneAt(x, y), zone, `${x},${y}`);
});

test('a purr builds while you stroke, fastest on the face, and fades after', () => {
  const head = pet(new Petting(), 'head', 2000);
  const cheeks = pet(new Petting(), 'cheeks', 2000);
  const chest = pet(new Petting(), 'chest', 2000);
  assert.ok(head.purr > .4 && head.purr < .6, `head ${head.purr}`);
  assert.ok(cheeks.purr > head.purr && head.purr > chest.purr, `${cheeks.purr} > ${head.purr} > ${chest.purr}`);
  const petting = new Petting();
  const { at, purr } = pet(petting, 'head', 2000);
  assert.ok(Math.abs(petting.purr(at + 300 + 2000) - (purr - 2 * PURR_FALL)) < .02, 'falls at its rate once the hand rests');
  assert.equal(petting.purr(at + 60_000), 0);
});

test('a hand resting on the cat or barely moving is not stroking', () => {
  const petting = new Petting();
  for (let at = 0; at < 3000; at += 16) petting.stroke('head', .5, at, calm);
  assert.equal(petting.purr(3000), 0);
});

test('its favourite spot purrs twice as fast and reaches kneading first', () => {
  const favourite = pet(new Petting(), 'cheeks', 1000, { ...calm, favourite: 'cheeks' });
  const other = pet(new Petting(), 'cheeks', 1000, { ...calm, favourite: 'chin' });
  assert.ok(Math.abs(favourite.purr - 2 * other.purr) < .02, `${favourite.purr} vs ${other.purr}`);
  const kneadAt = (favourite: PetContext['favourite']) => {
    const petting = new Petting();
    for (let at = 0; at < 20_000; at += 16) if (petting.stroke('chin', 12, at, { ...calm, favourite }).play === 'knead') return at;
    return Infinity;
  };
  assert.ok(kneadAt('chin') < kneadAt('cheeks'));
});

test('kneading starts once the purr has been strong for two seconds', () => {
  const { seen } = pet(new Petting(), 'cheeks', 6000);
  assert.deepEqual(seen.slice(0, 2), ['purr', 'knead']);
  const petting = new Petting();
  let firstKnead = Infinity;
  for (let at = 0; at < 8000 && firstKnead === Infinity; at += 16) if (petting.stroke('cheeks', 12, at, calm).play === 'knead') firstKnead = at;
  // 0.8 at 0.375 a second is about 2.1 s, then two more seconds.
  assert.ok(firstKnead > 4000 && firstKnead < 4600, String(firstKnead));
});

test('the tail flicks the hand away and ends the purr', () => {
  const petting = new Petting();
  const { at } = pet(petting, 'head', 2000);
  const touch = petting.stroke('tail', 12, at + 16, calm);
  assert.deepEqual(touch, { purr: 0, play: 'flick', mood: 'tail' });
  assert.equal(petting.stroke('tail', 12, at + 32, calm).play, null, 'once, not on every stroke');
  assert.equal(new Petting().stroke('tail', 12, 0, { ...calm, cat: false }).play, 'tilt');
});

test('a paw gets batted at, now and then', () => {
  const { seen } = pet(new Petting(), 'paws', 3000);
  assert.deepEqual(seen, ['bat']);
});

test('a long belly rub gets grabbed by a playful cat, and tolerated by a calm one', () => {
  assert.ok(pet(new Petting(), 'chest', 5000, playful).seen.includes('grab'));
  assert.ok(!pet(new Petting(), 'chest', 9000, calm).seen.includes('grab'));
  assert.ok(!pet(new Petting(), 'back', 5000, playful).seen.includes('grab'), 'only the belly');
});

test('a playful or cross cat swats once a session, a calm one never', () => {
  const playfulSeen = pet(new Petting(), 'head', 14_000, playful).seen;
  assert.ok(playfulSeen.includes('swat'));
  assert.ok(!pet(new Petting(), 'head', 14_000, calm).seen.includes('swat'));
  assert.ok(pet(new Petting(), 'head', 14_000, { ...calm, annoyance: .4 }).seen.includes('swat'));
  const petting = new Petting();
  const first = pet(petting, 'head', 14_000, playful);
  assert.ok(!pet(petting, 'head', 14_000, playful, first.at + 1500).seen.includes('swat'), 'not again until the hand leaves');
  petting.leave();
  assert.ok(pet(petting, 'head', 14_000, playful, first.at + 40_000).seen.includes('swat'));
});

test('after a good purr, one slow blink a second after the strokes stop', () => {
  const petting = new Petting();
  const { at } = pet(petting, 'cheeks', 2500);
  assert.equal(petting.afterglow(at + 500), false, 'still being petted');
  assert.equal(petting.afterglow(at + 1100), true);
  assert.equal(petting.afterglow(at + 1200), false, 'once');
  const brief = new Petting();
  const short = pet(brief, 'head', 600);
  assert.equal(brief.afterglow(short.at + 1100), false, 'a quick pat is no reason for one');
});

test('a mood nudge comes once a second of stroking, not on every movement', () => {
  const petting = new Petting();
  let nudges = 0;
  for (let at = 0; at <= 3000; at += 16) if (petting.stroke('head', 12, at, calm).mood === 'pet') nudges++;
  assert.ok(nudges >= 3 && nudges <= 4, String(nudges));
});
