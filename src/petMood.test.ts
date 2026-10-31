// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { drift, energyAt, faceFor, feel, feelingFor, memory, moodPace, moodWeights, Pokes, proudOrHappy, remembered, startingMood, type Mood } from './petMood.ts';
import { actionDuration, catOnly, forCharacter, Hunter, nearness, nextDreamDelay, nextEarSwivel, nextHabit, nextSaccade, pickIdleAction } from './petLife.ts';

const calm = (changes: Partial<Mood> = {}): Mood => ({ energy: .6, affection: .5, boredom: 0, annoyance: 0, excitement: 0, bond: 0, ...changes });
const sequence = (...values: number[]) => () => values.shift() ?? 0;

test('a cat is livelier in the morning than late at night, and a sleepy one less so', () => {
  assert.ok(energyAt(8) > energyAt(15) && energyAt(15) > energyAt(20) && energyAt(20) > energyAt(2));
  assert.ok(startingMood('sleepy', 10).energy < startingMood('calm', 10).energy);
  assert.ok(startingMood('playful', 10).energy > startingMood('calm', 10).energy);
});

test('poking makes it cross and the crossness fades within a couple of minutes', () => {
  let mood = calm();
  for (let poke = 0; poke < 2; poke++) mood = feel(mood, 'poke', 'calm');
  assert.equal(faceFor(mood), 'grumpy');
  mood = drift(mood, 30, { motion: 'calm', hour: 14, userAround: true, asleep: false });
  assert.ok(mood.annoyance < .56 && mood.annoyance > 0);
  mood = drift(mood, 60, { motion: 'calm', hour: 14, userAround: true, asleep: false });
  assert.equal(mood.annoyance, 0);
  assert.notEqual(faceFor(mood), 'grumpy');
});

test('left alone with you around, it gets bored; company and petting fix that', () => {
  let mood = calm();
  for (let minute = 0; minute < 5; minute++) mood = drift(mood, 60, { motion: 'playful', hour: 14, userAround: true, asleep: false });
  assert.notEqual(faceFor(mood), 'bored', 'not after five minutes');
  for (let minute = 0; minute < 6; minute++) mood = drift(mood, 60, { motion: 'playful', hour: 14, userAround: true, asleep: false });
  assert.equal(faceFor(mood), 'bored');
  assert.equal(faceFor(feel(mood, 'talk', 'playful')), 'neutral');
  assert.ok(feel(mood, 'pet', 'playful').boredom < mood.boredom);
  // Nobody about, or napping: no boredom builds.
  const away = drift(calm({ boredom: .5 }), 120, { motion: 'playful', hour: 14, userAround: false, asleep: false });
  assert.equal(away.boredom, 0);
  // A calm cat takes longer to get bored than a playful one.
  const after = (motion: 'calm' | 'playful') => drift(calm(), 120, { motion, hour: 14, userAround: true, asleep: false }).boredom;
  assert.ok(after('calm') < after('playful'));
});

test('late at night the cat gets drowsy, and a nap brings its energy back', () => {
  let mood = calm();
  for (let minute = 0; minute < 30; minute++) mood = drift(mood, 60, { motion: 'calm', hour: 2, userAround: false, asleep: false });
  assert.equal(faceFor(mood), 'drowsy');
  for (let minute = 0; minute < 20; minute++) mood = drift(mood, 60, { motion: 'calm', hour: 2, userAround: false, asleep: true });
  assert.ok(mood.energy > .9);
});

test('being picked up is fun for a playful cat and a bother for the others', () => {
  assert.ok(feel(calm(), 'held', 'playful').excitement > 0);
  assert.equal(feel(calm(), 'held', 'playful').annoyance, 0);
  assert.ok(feel(calm(), 'held', 'calm').annoyance > 0);
});

test('the strongest feeling sets the face, and affection shows as contentment', () => {
  assert.equal(faceFor(calm()), 'neutral');
  assert.equal(faceFor(calm({ affection: .8 })), 'content');
  assert.equal(faceFor(calm({ excitement: .7, affection: .9 })), 'excited');
  assert.equal(faceFor(calm({ annoyance: .6, excitement: .9 })), 'grumpy');
  assert.equal(faceFor(calm({ energy: .2, boredom: .9 })), 'drowsy');
  for (const event of ['pet', 'click', 'poke', 'overpet', 'held', 'woken', 'play', 'chase', 'userBack', 'talk', 'praise', 'done', 'failed'] as const) {
    for (const value of Object.values(feel(calm({ energy: 1, affection: 1, boredom: 1, annoyance: 1, excitement: 1, bond: 1 }), event, 'calm'))) assert.ok(value >= 0 && value <= 1, event);
  }
});

test('moods tilt the habits: bored cats seek attention, grumpy ones groom, drowsy ones rest', () => {
  const base = { look: 2, groom: 1, bat: 2, smile: 1 };
  const bored = moodWeights(base, 'bored');
  assert.ok(bored.sigh && bored.wave && bored.mischief, 'attention seeking joins in');
  const grumpy = moodWeights(base, 'grumpy');
  assert.equal(grumpy.smile, undefined, 'no smiles while cross');
  assert.ok(grumpy.groom! > base.groom && grumpy.bat! < base.bat);
  const drowsy = moodWeights(base, 'drowsy');
  assert.ok(drowsy.yawn && drowsy.nod && drowsy.bat! < base.bat);
  assert.equal(drowsy.tailchase, undefined, 'a drowsy cat does not start chasing its tail');
  assert.deepEqual(moodWeights(base, 'neutral'), base);
  // Every habit a mood can ask for has a duration to play out.
  for (const face of ['content', 'drowsy', 'bored', 'grumpy', 'excited'] as const) {
    for (const action of Object.keys(moodWeights({}, face))) assert.ok(action in actionDuration, action);
  }
});

test('a bored cat picks attention-seeking habits and asks for a pet sooner', () => {
  const seen = new Set<string>();
  for (let index = 0; index < 300; index++) seen.add(pickIdleAction('calm', true, sequence(index / 300), 'bored'));
  assert.ok(seen.has('sigh') && seen.has('wave'));
  assert.ok(!pickIdleAction('calm', false, sequence(.5), 'bored').match(/sigh|wave/), 'other characters keep their own habits');
  const since = { stretch: 0, ask: 7 * 60_000, userAround: true };
  assert.equal(nextHabit('calm', true, since, sequence(.4), 'bored'), 'askpet');
  assert.notEqual(nextHabit('calm', true, since, sequence(.4, .5), 'neutral'), 'askpet');
});

test('a lively or excited cat acts sooner, a drowsy one later', () => {
  assert.ok(moodPace(calm({ energy: .9, excitement: .8 })) < moodPace(calm()));
  assert.ok(moodPace(calm({ energy: .1 })) > moodPace(calm()));
  for (const mood of [calm({ energy: 0 }), calm({ energy: 1, excitement: 1 })]) assert.ok(moodPace(mood) >= .55 && moodPace(mood) <= 1.5);
});

test('one click is affection, a quick run of them is poking', () => {
  const pokes = new Pokes();
  assert.equal(pokes.click(0), 'click');
  assert.equal(pokes.click(600), 'click');
  assert.equal(pokes.click(1200), 'poke');
  assert.equal(new Pokes().click(0), 'click');
  const slow = new Pokes();
  for (const at of [0, 3000, 6000]) assert.equal(slow.click(at), 'click');
});

test('a job done makes a lively cat proud more often than a flat one', () => {
  assert.equal(proudOrHappy(calm({ excitement: 1 }), sequence(.7)), 'proud');
  assert.equal(proudOrHappy(calm(), sequence(.7)), 'happy');
});

test('your words show on its face: hello, thanks, fondness, pspsps and harsh words', () => {
  assert.equal(feelingFor('hi buntini'), 'wave');
  assert.equal(feelingFor('Good morning!'), 'wave');
  assert.equal(feelingFor('thanks, that worked'), 'bashful');
  assert.equal(feelingFor("you're the best"), 'bashful');
  assert.equal(feelingFor('you’re the best'), 'bashful', 'with the curly apostrophe macOS types');
  assert.equal(feelingFor('love you'), 'love');
  assert.equal(feelingFor('good kitty'), 'love');
  assert.equal(feelingFor('pspsps'), 'excited');
  assert.equal(feelingFor('shut up'), 'sad');
  assert.equal(feelingFor('what is on my calendar tomorrow'), null);
  assert.equal(feelingFor('find the history of the hippo'), null, 'a word that merely contains a greeting');
  assert.equal(feelingFor('say hi to Sam for me'), null, 'a greeting only counts at the start');
});

test('characters other than the cat get a reaction they can show', () => {
  for (const action of catOnly) {
    const shown = forCharacter(action, false);
    assert.ok(!catOnly.has(shown), `${action} → ${shown}`);
    assert.equal(forCharacter(action, true), action);
  }
  assert.equal(forCharacter('slowblink', false), 'slowblink');
});

test('a pointer darting about out of reach gets chattered at, once in a while', () => {
  const hunter = new Hunter();
  for (let at = 0, side = 1; at < 1200; at += 100, side = -side) hunter.moved(500 + side * 120, -80, at);
  assert.equal(hunter.chatters(1250, 'playful'), true);
  for (let at = 1300, side = 1; at < 2500; at += 100, side = -side) hunter.moved(500 + side * 120, -80, at);
  assert.equal(hunter.chatters(2550, 'playful'), false, 'not again straight away');
  const sleepy = new Hunter();
  for (let at = 0, side = 1; at < 1200; at += 100, side = -side) sleepy.moved(500 + side * 120, -80, at);
  assert.equal(sleepy.chatters(1250, 'sleepy'), false);
  const close = new Hunter();
  for (let at = 0, side = 1; at < 1200; at += 100, side = -side) close.moved(-140 + side * 40, -20, at);
  assert.equal(close.chatters(1250, 'playful'), false, 'one within reach gets hunted instead');
});

const around = { motion: 'calm' as const, hour: 14, userAround: true, asleep: false };

test('petting and kind words build a bond over days, and a close bond makes a contented cat', () => {
  let mood = calm();
  for (let day = 0; day < 10; day++) {
    for (let stroke = 0; stroke < 15; stroke++) mood = feel(mood, 'pet', 'calm');
    mood = feel(mood, 'praise', 'calm');
    mood = drift(mood, 86_400, around);
  }
  assert.ok(mood.bond > .2 && mood.bond < .5, `a bond grows slowly: ${mood.bond}`);
  assert.ok(feel(calm({ bond: .5 }), 'poke', 'calm').bond < .5, 'poking wears it down a little');
  // Left to settle (and not ignored all the while), a close cat rests more affectionate than a new one.
  const settle = (bond: number) => drift(calm({ bond }), 4 * 3600, { ...around, userAround: false });
  assert.ok(settle(1).affection > .7 && settle(0).affection < .5);
  assert.equal(faceFor(settle(1)), 'content');
  assert.ok(drift(calm({ bond: .8 }), 21 * 86_400, around).bond < .01, 'and fades in three weeks apart');
});

test('the mood carries over to the next launch, rested and no longer cross', () => {
  const before = calm({ bond: .6, annoyance: .9, energy: .2, excitement: .8 });
  const now = 1_000_000_000;
  const after = remembered(memory(before, now - 2 * 3600 * 1000), 'calm', 14, now);
  assert.ok(after.bond > .59 && after.bond <= .6, 'the bond is kept');
  assert.equal(after.annoyance, 0);
  assert.ok(after.energy > .9, 'time apart was a nap');
  assert.ok(after.excitement < .01);
  // A quick restart keeps it as it was.
  const restart = remembered(memory(before, now - 5000), 'calm', 14, now);
  assert.ok(restart.annoyance > .75 && restart.excitement > .7);
  // Nothing saved, or something unreadable: a fresh cat.
  for (const saved of [null, '', 'not json', '{"mood":null,"at":1}', '{"at":"x"}']) assert.deepEqual(remembered(saved, 'playful', 9, now), startingMood('playful', 9));
  assert.deepEqual(remembered('{"mood":{"bond":7,"energy":"lots"},"at":' + now + '}', 'calm', 14, now), { ...startingMood('calm', 14), bond: 1 }, 'out-of-range values are clamped and bad ones ignored');
});

test('between habits the eyes wander, ears swivel and it dreams in its sleep', () => {
  assert.deepEqual(nextSaccade(sequence(0, .1)), { delay: 1800, gaze: null }, 'sometimes back to the middle');
  const aside = nextSaccade(sequence(1, .9, 1, 1));
  assert.ok(aside.gaze && Math.abs(aside.gaze.x) <= .5 && Math.abs(aside.gaze.y) <= .5, 'a small glance aside');
  for (let index = 0; index < 50; index++) {
    const { delay, angle, hold } = nextEarSwivel();
    assert.ok(delay >= 3000 && delay <= 9000 && Math.abs(angle) >= 4 && Math.abs(angle) <= 10 && hold >= 500 && hold <= 1600);
    assert.ok(nextDreamDelay() >= 9000 && nextDreamDelay() <= 21000);
  }
  assert.ok('dream' in actionDuration);
});

test('it perks up as the pointer comes near', () => {
  assert.equal(nearness(0, 0), 1);
  assert.equal(nearness(30, 0), 1);
  assert.equal(nearness(400, 0), 0);
  assert.ok(nearness(100, 60) > nearness(200, 60) && nearness(200, 60) > 0);
});
