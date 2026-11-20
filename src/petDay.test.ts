// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { Company, Curiosity, handsBusy, loseInterest, napAfter, napDue, napLength, Regard, settleRoutine, sleepiness, wakeRoutine, zoomiesChance, zoomiesGap } from './petDay.ts';

const MINUTE = 60_000;
const sequence = (...values: number[]) => () => values.length ? values.shift()! : .5;
const middle = () => .5;

test('a cat dozes through the early afternoon and the small hours, and is up at dawn and dusk', () => {
  assert.ok(sleepiness(14) > sleepiness(10) && sleepiness(3) > sleepiness(14));
  assert.ok(sleepiness(6) < sleepiness(10) && sleepiness(19) < sleepiness(16));
  for (let hour = 0; hour < 24; hour++) assert.ok(sleepiness(hour) > 0 && sleepiness(hour) < 1, String(hour));
});

test('it naps sooner and longer at a sleepy hour, as a sleepy cat, or when tired', () => {
  const up = (hour: number, motion: 'calm' | 'playful' | 'sleepy' = 'calm', energy = .6) => napAfter(hour, motion, energy, middle) / MINUTE;
  assert.ok(up(14) < up(19), 'sooner after lunch than at dusk');
  assert.ok(up(14, 'sleepy') < up(14) && up(14) < up(14, 'playful'));
  assert.ok(up(14, 'calm', .1) < up(14, 'calm', .9), 'sooner when tired');
  for (const hour of [3, 6, 14, 19]) for (const motion of ['calm', 'playful', 'sleepy'] as const) {
    const low = napAfter(hour, motion, .6, () => 0) / MINUTE, high = napAfter(hour, motion, .6, () => .999999) / MINUTE;
    assert.ok(low >= 10 && high <= 120 && high > low, `${hour} ${motion}: ${low}–${high} minutes awake`);
  }
  const nap = (hour: number, motion: 'calm' | 'playful' | 'sleepy' = 'calm') => napLength(hour, motion, middle) / MINUTE;
  assert.ok(nap(14) > nap(19) && nap(3) > nap(14));
  assert.ok(nap(14, 'sleepy') > nap(14) && nap(14) > nap(14, 'playful'));
  assert.ok(nap(19, 'playful') >= 3 && napLength(3, 'sleepy', () => .999999) <= 30 * MINUTE, 'between 3 and 30 minutes');
});

test('a nap waits until nobody has touched it for two minutes and the pointer has moved off', () => {
  assert.ok(napDue(10 * MINUTE, 9 * MINUTE, 0, 0));
  assert.ok(!napDue(8 * MINUTE, 9 * MINUTE, 0, 0), 'not due yet');
  assert.ok(!napDue(10 * MINUTE, 9 * MINUTE, 9 * MINUTE, 0), 'petted a minute ago');
  assert.ok(!napDue(10 * MINUTE, 9 * MINUTE, 0, .5), 'the pointer is hovering by it');
});

test('it kneads and turns round before a nap, and wakes with a stretch, a yawn and a wash', () => {
  assert.deepEqual(settleRoutine(false), ['knead', 'circle']);
  assert.deepEqual(settleRoutine(true), [], 'with Less motion it just lies down');
  assert.deepEqual(wakeRoutine(true), ['stretch', 'yawn', 'groom']);
  assert.deepEqual(wakeRoutine(false), ['stretch'], 'woken by you, just the stretch');
});

test('zoomies come mostly in the evening and after a good sleep, never with Less motion', () => {
  const base: Parameters<typeof zoomiesChance>[0] = { hour: 20, motion: 'playful', energy: .6, excitement: 0, sinceWake: 60 * MINUTE, slept: 0, sinceLast: 120 * MINUTE, still: false };
  const chance = (changes: Partial<typeof base> = {}) => zoomiesChance({ ...base, ...changes });
  assert.ok(chance() > 5 * chance({ hour: 13 }), 'evening, not midday');
  assert.ok(chance({ hour: 6 }) > 5 * chance({ hour: 13 }), 'a little at dawn');
  assert.ok(chance({ hour: 13, sinceWake: MINUTE, slept: 20 * MINUTE }) > .3, 'just up from a good nap');
  assert.ok(chance({ hour: 13, sinceWake: MINUTE, slept: 2 * MINUTE }) < .01, 'a doze is not a good sleep');
  assert.ok(chance({ excitement: .8 }) > chance() && chance({ energy: 1 }) > chance({ energy: .2 }));
  assert.ok(chance() > chance({ motion: 'calm' }) && chance({ motion: 'calm' }) > chance({ motion: 'sleepy' }));
  assert.equal(chance({ still: true, sinceWake: 0, slept: 60 * MINUTE }), 0, 'Less motion');
  assert.equal(chance({ sinceLast: 5 * MINUTE }), 0, 'not straight after the last burst');
  assert.ok(zoomiesGap.sleepy > zoomiesGap.calm && zoomiesGap.calm > zoomiesGap.playful);
  assert.equal(chance({ motion: 'calm', sinceLast: 30 * MINUTE }), 0, 'a calm cat waits longer between bursts');
  assert.ok(chance({ sinceWake: 0, slept: 60 * MINUTE, excitement: 1, energy: 1 }) <= .6);
});

test('hands busy: input just now while the pointer stays put', () => {
  assert.ok(handsBusy(.4, 3000));
  assert.ok(!handsBusy(.4, 500), 'moving the pointer');
  assert.ok(!handsBusy(5, 3000), 'nothing for five seconds');
  assert.ok(!handsBusy(null, 3000) && !handsBusy(undefined, 3000), 'not told');
});

test('a pointer resting near the cat gets a slow blink back, sooner from a close cat', () => {
  const regard = new Regard();
  regard.moved(150, 0);
  assert.ok(regard.resting(1000));
  assert.equal(regard.answer(1000, 'close', false, middle), null, 'a moment first');
  assert.equal(regard.answer(1600, 'close', false, middle), 'slowblink');
  assert.equal(regard.answer(2000, 'close', false, middle), null, 'once each rest');
  regard.moved(150, 10_000);
  assert.equal(regard.answer(12_000, 'close', false, middle), null, 'at most every 40 seconds');
  regard.moved(150, 50_000);
  assert.equal(regard.answer(52_000, 'familiar', false, () => .3), null, 'a familiar cat takes a little longer');
  assert.equal(regard.answer(52_600, 'familiar', false, () => .3), 'slowblink');
  regard.moved(150, 100_000);
  assert.equal(regard.answer(104_100, 'new', false, () => .5), 'look', 'a new cat looks away');
});

test('a pointer on the cat, far off, parked while you type or long since, is not looking at it', () => {
  const on = new Regard();
  on.moved(30, 0);
  assert.ok(!on.resting(1000));
  assert.equal(on.answer(3000, 'close', false, middle), null, 'that is a hand on the cat');
  const far = new Regard();
  far.moved(600, 0);
  assert.equal(far.answer(3000, 'close', false, middle), null);
  const typing = new Regard();
  typing.moved(150, 0);
  assert.equal(typing.answer(2000, 'close', true, middle), null);
  assert.equal(typing.answer(3000, 'close', false, middle), null, 'that rest has passed');
  const parked = new Regard();
  parked.moved(150, 0);
  assert.equal(parked.answer(30_000, 'close', false, middle), null);
  assert.ok(!parked.resting(30_000));
});

test('curiosity about the pointer comes and goes', () => {
  const eyes = new Curiosity();
  // Noticed (0.5 < 0.8), watched for 15 s of movement.
  assert.equal(eyes.moved(0, 300, 'neutral', middle), 'watch');
  assert.ok(eyes.watching(1000) && eyes.engaged(1000));
  for (let at = 1000; at < 15_000; at += 1000) assert.equal(eyes.moved(at, 300, 'neutral', middle), 'watch', String(at));
  assert.equal(eyes.moved(15_000, 300, 'neutral', middle), 'bored');
  assert.ok(!eyes.watching(15_100) && !eyes.engaged(15_100));
  // Then it can't be bothered for 40 s, unless the pointer comes right up to it.
  assert.equal(eyes.moved(30_000, 300, 'neutral', middle), 'ignore');
  assert.equal(eyes.moved(31_000, 60, 'neutral', middle), 'watch');
  assert.ok(eyes.watching(31_500) && !eyes.engaged(31_500), 'it looks, but won’t chase');
  assert.equal(eyes.moved(56_000, 300, 'neutral', middle), 'watch', 'interested again');
  assert.ok(!eyes.watching(56_000 + 4000), 'eyes drift off once the pointer stops');
});

test('short pauses keep the same spell of interest; a long one makes the pointer new again', () => {
  const eyes = new Curiosity();
  eyes.moved(0, 300, 'neutral', middle);
  assert.equal(eyes.moved(5000, 300, 'neutral', middle), 'watch');
  assert.equal(eyes.moved(12_000, 300, 'neutral', middle), 'watch');
  assert.equal(eyes.moved(15_000, 300, 'neutral', middle), 'bored', 'gaps under 8 s keep counting');
  const fresh = new Curiosity();
  fresh.moved(0, 300, 'neutral', middle);
  assert.equal(fresh.moved(9000, 300, 'neutral', middle), 'watch');
  assert.equal(fresh.moved(15_000, 300, 'neutral', middle), 'watch', 'after a 9 s pause a new spell began');
});

test('a bored cat watches longer and comes back sooner than a drowsy one, which barely looks', () => {
  const spell = (face: 'bored' | 'drowsy' | 'neutral') => {
    const eyes = new Curiosity();
    if (eyes.moved(0, 300, face, () => 0) !== 'watch') return 0;
    let at = 0;
    while (eyes.moved(at += 500, 300, face, () => 0) === 'watch');
    return at;
  };
  assert.ok(spell('bored') > spell('neutral') && spell('neutral') > spell('drowsy'));
  assert.equal(new Curiosity().moved(0, 300, 'drowsy', () => .5), 'ignore', 'a drowsy cat often doesn’t notice');
  assert.equal(new Curiosity().moved(0, 300, 'bored', () => .99), 'watch', 'a bored one always does');
  for (let index = 0; index < 50; index++) assert.ok(['look', 'groom', 'ear'].includes(loseInterest()));
  assert.equal(loseInterest(() => .1), 'look');
});

test('lots of fuss sends a cat that is not close yet off for some room', () => {
  const company = new Company(0);
  for (let index = 0; index < 5; index++) company.fuss(index * 10_000);
  assert.ok(!company.wantsRoom(50_000, 'familiar'), 'five lots is fine');
  company.fuss(55_000);
  assert.ok(company.wantsRoom(56_000, 'familiar'));
  for (let index = 0; index < 6; index++) company.fuss(60_000 + index * 1000);
  assert.ok(!company.wantsRoom(70_000, 'familiar'), 'at most every ten minutes');
  const shy = new Company(0);
  for (let index = 0; index < 4; index++) shy.fuss(index * 1000);
  assert.ok(shy.wantsRoom(5000, 'new'), 'a new cat has had enough sooner');
  const close = new Company(0);
  for (let index = 0; index < 20; index++) close.fuss(index * 1000);
  assert.ok(!close.wantsRoom(21_000, 'close'), 'a close cat never minds');
  const spread = new Company(0);
  for (let index = 0; index < 8; index++) spread.fuss(index * 30_000);
  assert.ok(!spread.wantsRoom(8 * 30_000, 'familiar'), 'fuss spread over minutes is just company');
});

test('left alone long enough while you work, it comes to find you', () => {
  const company = new Company(0);
  const busy = { boredom: .8, busy: true, band: 'familiar' as const };
  assert.ok(!company.wantsCompany(10 * MINUTE, busy), 'not alone long enough yet');
  assert.ok(company.wantsCompany(41 * MINUTE, busy));
  assert.ok(!company.wantsCompany(60 * MINUTE, busy), 'at most every 40 minutes');
  assert.ok(company.wantsCompany(82 * MINUTE, busy));
  const other = new Company(0);
  assert.ok(!other.wantsCompany(50 * MINUTE, { ...busy, busy: false }), 'not while you are away');
  assert.ok(!other.wantsCompany(50 * MINUTE, { ...busy, boredom: .4 }), 'not when it isn’t bored');
  other.fuss(45 * MINUTE);
  assert.equal(other.lastTouched, 45 * MINUTE);
  assert.ok(!other.wantsCompany(50 * MINUTE, busy), 'petted five minutes ago');
  assert.ok(other.wantsCompany(58 * MINUTE, { ...busy, band: 'close' }), 'a close cat misses you after 12 minutes');
  const shy = new Company(0);
  shy.fuss(20 * MINUTE);
  assert.ok(!shy.wantsCompany(45 * MINUTE, { ...busy, band: 'new' }), 'a new cat keeps to itself longer');
  assert.ok(shy.wantsCompany(51 * MINUTE, { ...busy, band: 'new' }));
});
