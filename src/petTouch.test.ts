// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { BREATH, KEY_STROKE, Petting, PURR_FALL, tapsDue, tiltFor, zoneAt, type PetContext, type Zone } from './petTouch.ts';

const calm: PetContext = { cat: true, motion: 'calm', annoyance: 0, favourite: null, band: 'familiar', still: false };
const playful: PetContext = { ...calm, motion: 'playful' };

/** Steady strokes of `step` px every 16 ms (12 px is 750 px/s) on `zone` for `ms`, from `from`. */
function pet(petting: Petting, zone: Zone, ms: number, context = calm, from = 0, step = 12) {
  const seen: string[] = [];
  let at = from;
  for (; at <= from + ms; at += 16) {
    const touch = petting.stroke(zone, step, at, context);
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
  const after = pet(petting, 'head', 14_000, playful, first.at + 16);
  assert.ok(!after.seen.includes('swat'), 'not again until the hand leaves');
  assert.equal(after.purr, 0, 'nor any purr');
  petting.leave(first.at + 39_000);
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

test('a tap at the start of each breath once the purr is going, two when it is strong', () => {
  assert.equal(tapsDue(0, BREATH - 50, BREATH + 50, .5), 1);
  assert.equal(tapsDue(0, BREATH - 50, BREATH + 50, .9), 2);
  assert.equal(tapsDue(0, BREATH - 50, BREATH + 50, .2), 0, 'a faint purr is not felt');
  assert.equal(tapsDue(0, BREATH + 100, BREATH + 200, .9), 0, 'mid-breath');
  let taps = 0;
  for (let at = 0; at < 3 * BREATH; at += 100) taps += tapsDue(0, at, at + 100, .5);
  assert.equal(taps, 3, 'one a breath: at 2.4, 4.8 and 7.2 s');
});

test('the head tips toward the stroked side, most for cheeks and ears, more as the purr builds', () => {
  assert.equal(tiltFor('cheeks', 1, -1), -8);
  assert.equal(tiltFor('ears', 1, 1), 8);
  assert.equal(tiltFor('cheeks', .25, 1), 4);
  assert.ok(Math.abs(tiltFor('chin', 1, 1)) < Math.abs(tiltFor('head', 1, 1)));
  assert.equal(tiltFor('tail', 1, 1), 0);
});

test('it leans after a hand that leaves mid-purr, not after a quick pat', () => {
  const petting = new Petting();
  const { at } = pet(petting, 'cheeks', 2000);
  assert.equal(petting.leave(at + 16), true);
  const brief = new Petting();
  const short = pet(brief, 'head', 500);
  assert.equal(brief.leave(short.at + 16), false);
});

test('a playful cat at full purr warns before it swats, and slowing down calms it', () => {
  const petting = new Petting();
  let warned = -1, swatted = -1;
  for (let at = 0; at < 14_000 && swatted < 0; at += 16) {
    const play = petting.stroke('cheeks', 12, at, playful).play;
    if (play === 'warn' && warned < 0) warned = at;
    if (play === 'swat') swatted = at;
  }
  assert.ok(warned > 0 && swatted - warned >= 1500 && swatted - warned < 1600, `${warned} → ${swatted}`);
  const calmed = new Petting();
  let at = 0, warnedAt = -1;
  for (; at < 14_000 && warnedAt < 0; at += 16) if (calmed.stroke('cheeks', 12, at, playful).play === 'warn') warnedAt = at;
  // The hand stops for a moment, then strokes on: no swat, and a slow blink first.
  assert.equal(calmed.afterglow(at + 400), true);
  assert.ok(!pet(calmed, 'cheeks', 1600, playful, at + 600).seen.includes('swat'));
});

test('a new cat purrs more slowly, and pulls away after a long petting', () => {
  const shy = { ...calm, band: 'new' as const };
  const familiar = pet(new Petting(), 'head', 2000);
  const fresh = pet(new Petting(), 'head', 2000, shy);
  assert.ok(Math.abs(fresh.purr - familiar.purr / 2) < .02, `${fresh.purr} vs ${familiar.purr}`);
  const petting = new Petting();
  const moods: string[] = [];
  let shied = 0;
  for (let at = 0; at < 12_000; at += 16) {
    const touch = petting.stroke('head', 12, at, shy);
    if (touch.play === 'shy') shied++;
    if (at > 8100 && touch.mood) moods.push(touch.mood);
  }
  assert.equal(shied, 1);
  assert.ok(moods.length >= 3 && moods.every(mood => mood === 'tooMuch'), moods.join());
  assert.ok(petting.purr(12_000) < pet(new Petting(), 'head', 8000, shy).purr, 'the purr fades once it has had enough');
});

test('a close cat purrs at the first touch and rolls over for a belly rub', () => {
  const close = { ...calm, band: 'close' as const };
  assert.ok(pet(new Petting(), 'head', 100, close).seen.includes('purr'));
  assert.ok(!pet(new Petting(), 'head', 100).seen.includes('purr'));
  const rolled = pet(new Petting(), 'chest', 7000, close).seen;
  assert.ok(rolled.includes('flop') && !rolled.includes('grab'), rolled.join());
  const playfulClose = pet(new Petting(), 'chest', 7000, { ...playful, band: 'close' }).seen;
  assert.ok(playfulClose.indexOf('flop') < playfulClose.indexOf('grab'), playfulClose.join());
  assert.ok(!pet(new Petting(), 'chest', 7000, playful).seen.includes('flop'), 'only a close cat');
  assert.ok(!pet(new Petting(), 'chest', 7000, { ...calm, band: 'new' }).seen.includes('flop'));
});

test('frantic petting does less for the bond than steady petting', () => {
  const moods = (step: number) => {
    const petting = new Petting();
    const seen = new Set<string>();
    for (let at = 0; at < 3000; at += 16) { const mood = petting.stroke('head', step, at, calm).mood; if (mood) seen.add(mood); }
    return [...seen];
  };
  assert.deepEqual(moods(6), ['pet']);
  assert.ok(moods(20).includes('roughPet'));
});

test('with Reduce motion the purr builds the same and nothing moves', () => {
  const still = { ...playful, still: true };
  const moved = ['swat', 'warn', 'knead', 'grab', 'flop', 'flick', 'bat', 'shy'];
  for (const zone of ['cheeks', 'chest', 'tail', 'paws'] as Zone[]) {
    const { seen } = pet(new Petting(), zone, 14_000, { ...still, band: 'close' });
    assert.ok(!seen.some(play => moved.includes(play)), `${zone}: ${seen.join()}`);
  }
  assert.equal(pet(new Petting(), 'head', 2000, still).purr, pet(new Petting(), 'head', 2000, playful).purr);
});

test('key presses alone build a purr', () => {
  const petting = new Petting();
  let at = 0;
  for (let press = 0; press < 3; press++) {
    for (const end = at + KEY_STROKE.lasts; at < end; at += KEY_STROKE.every) petting.stroke('head', KEY_STROKE.distance, at, calm);
  }
  assert.ok(petting.purr(at) > .6, String(petting.purr(at)));
});
