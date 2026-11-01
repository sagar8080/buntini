import type { PetAction } from './petLife.ts';
import type { PetMotion } from './types';

/* The cat's mood: five feelings between 0 and 1 that drift back to where its personality
   and the time of day put them, and move when things happen to it. The mood picks its
   resting face and tilts which habits it reaches for. Underneath sits its bond with you,
   which grows over days of petting and kind words, fades over weeks, and makes it a
   happier cat to be around. Pure logic; usePetLife.ts feeds it. */

export interface Mood { energy: number; affection: number; boredom: number; annoyance: number; excitement: number; bond: number }
export type Feeling = keyof Mood;
/** What the cat looks like between habits. */
export type Face = 'neutral' | 'content' | 'drowsy' | 'bored' | 'grumpy' | 'excited';
export type MoodEvent =
  | 'pet' | 'click' | 'poke' | 'overpet' | 'held' | 'woken'
  | 'play' | 'chase' | 'userBack' | 'talk' | 'praise' | 'done' | 'failed';

const clamp = (value: number) => Math.min(1, Math.max(0, value));

/** How lively a cat is at this hour: up with the morning, flagging late at night. */
export function energyAt(hour: number): number {
  if (hour >= 6 && hour < 11) return .72;
  if (hour >= 11 && hour < 18) return .62;
  if (hour >= 18 && hour < 22) return .52;
  return .28;
}

const restingEnergy: Record<PetMotion, number> = { calm: 0, playful: .12, sleepy: -.18 };

export function startingMood(motion: PetMotion, hour: number): Mood {
  return { energy: clamp(energyAt(hour) + restingEnergy[motion]), affection: .5, boredom: 0, annoyance: 0, excitement: motion === 'playful' ? .2 : 0, bond: 0 };
}

// Each event nudges a few feelings. Playful cats find being picked up fun; others don't.
const nudges: Record<MoodEvent, Partial<Mood>> = {
  pet: { affection: .03, boredom: -.2, annoyance: -.04, bond: .002 },
  click: { affection: .05, boredom: -.3, excitement: .08, bond: .002 },
  poke: { annoyance: .28, boredom: -.3, affection: -.02, bond: -.004 },
  overpet: { annoyance: .35, excitement: .1 },
  held: { annoyance: .12, boredom: -.4 },
  woken: { annoyance: .3, energy: .1 },
  play: { energy: -.03, excitement: .12, boredom: -.15 },
  chase: { excitement: .3, boredom: -.3 },
  userBack: { excitement: .45, affection: .06, boredom: -.5, bond: .003 },
  talk: { boredom: -.5, affection: .02 },
  praise: { affection: .15, excitement: .2, annoyance: -.3, bond: .01 },
  done: { excitement: .25, affection: .04 },
  failed: { excitement: -.2, affection: .02 },
};

export function feel(mood: Mood, event: MoodEvent, motion: PetMotion): Mood {
  const change = event === 'held' && motion === 'playful' ? { excitement: .25, boredom: -.4 } : nudges[event];
  const next = { ...mood };
  for (const [feeling, amount] of Object.entries(change) as [Feeling, number][]) next[feeling] = clamp(next[feeling] + amount);
  return next;
}

const BOND_FADES = 21 * 86_400;

/** Lets `seconds` pass. Annoyance and excitement fade in a minute or two, energy settles
    toward the hour's, a cat with you around but left alone slowly gets bored, and
    affection settles higher the closer the bond. */
export function drift(mood: Mood, seconds: number, { motion, hour, userAround, asleep }: { motion: PetMotion; hour: number; userAround: boolean; asleep: boolean }): Mood {
  const toward = (value: number, target: number, perSecond: number) => value + (target - value) * Math.min(1, perSecond * seconds);
  const energyTarget = asleep ? 1 : clamp(energyAt(hour) + restingEnergy[motion]);
  // Ten minutes ignored for a playful cat, fifteen for a calm one, before it's bored.
  const boredRate = motion === 'playful' ? 1 / 600 : motion === 'calm' ? 1 / 900 : 1 / 1500;
  return {
    energy: clamp(toward(mood.energy, energyTarget, asleep ? 1 / 300 : 1 / 600)),
    affection: clamp(toward(mood.affection, .42 + mood.bond * .33, 1 / 1800)),
    boredom: clamp(asleep || !userAround ? mood.boredom * Math.max(0, 1 - seconds / 120) : mood.boredom + seconds * boredRate),
    annoyance: clamp(mood.annoyance * Math.max(0, 1 - seconds / 45)),
    excitement: clamp(mood.excitement * Math.max(0, 1 - seconds / 60)),
    bond: clamp(mood.bond * Math.max(0, 1 - seconds / BOND_FADES)),
  };
}

/** The mood saved when Buntini last ran, brought up to now. Time apart passes as a nap:
    it wakes rested and over any crossness, with its bond (a little faded) intact. */
export function remembered(saved: string | null, motion: PetMotion, hour: number, now: number): Mood {
  const fresh = startingMood(motion, hour);
  try {
    const { mood, at } = JSON.parse(saved ?? '');
    if (!mood || typeof at !== 'number') return fresh;
    const restored = { ...fresh };
    for (const feeling of Object.keys(fresh) as Feeling[]) if (typeof mood[feeling] === 'number') restored[feeling] = clamp(mood[feeling]);
    const apart = Math.max(0, (now - at) / 1000);
    return { ...drift(restored, Math.min(apart, 6 * 3600), { motion, hour, userAround: false, asleep: apart > 600 }), bond: clamp(restored.bond * Math.max(0, 1 - apart / BOND_FADES)) };
  } catch { return fresh; }
}

export const memory = (mood: Mood, now: number) => JSON.stringify({ mood, at: now });

/** The resting face, strongest feeling first. */
export function faceFor(mood: Mood): Face {
  if (mood.annoyance > .5) return 'grumpy';
  if (mood.excitement > .55) return 'excited';
  if (mood.energy < .3) return 'drowsy';
  if (mood.boredom > .6) return 'bored';
  if (mood.affection > .7) return 'content';
  return 'neutral';
}

/** Which face goes with a task's outcome or a word from you: a proud cat after a job done
    when it's feeling lively, a plain happy one otherwise. */
export function proudOrHappy(mood: Mood, random = Math.random): 'proud' | 'happy' {
  return random() < .35 + mood.excitement * .4 ? 'proud' : 'happy';
}

type Weights = Partial<Record<PetAction, number>>;
// How each face tilts the habits: a bored cat goes looking for attention, a grumpy one
// grooms to calm down, a drowsy one yawns and loafs, an excited one plays.
const tilt: Record<Face, Weights> = {
  neutral: {},
  content: { slowblink: 3, smile: 2, loaf: 1.5, knead: 1.5, bat: .7, pounce: .7 },
  drowsy: { yawn: 3, nod: 3, loaf: 2, slowblink: 2, stretch: 1.5, bat: .3, pounce: .2, mischief: .2, tailchase: .2, wiggle: .3 },
  bored: { sigh: 3, mischief: 2.5, wave: 2, loaf: 2, blep: 1.5, look: 1.5 },
  grumpy: { groom: 3, ear: 2, lick: 2, smile: 0, wink: 0, blep: 0, bat: .3, pounce: .3 },
  excited: { pounce: 2.5, wiggle: 2.5, tailchase: 2.5, bat: 2, mischief: 1.5, slowblink: .5, loaf: .2 },
};

/** Scales a personality's habit weights by the mood's face. A habit the personality
    doesn't have joins in only when the face asks for more of it (a bored sigh). */
export function moodWeights(weights: Weights, face: Face): Weights {
  const extra = tilt[face];
  const scaled: Weights = {};
  for (const action of new Set([...Object.keys(weights), ...Object.keys(extra)] as PetAction[])) {
    const base = weights[action] ?? ((extra[action] ?? 0) > 1 ? 1 : 0);
    const value = base * (extra[action] ?? 1);
    if (value > 0) scaled[action] = value;
  }
  return scaled;
}

/** Habits come sooner from an excited or lively cat and later from a drowsy one. */
export function moodPace(mood: Mood): number {
  return Math.min(1.5, Math.max(.55, 1.35 - mood.energy * .5 - mood.excitement * .45));
}

/** Counts clicks on the cat: one is affection, a quick run of them is poking. */
export class Pokes {
  private times: number[] = [];
  click(at: number): 'click' | 'poke' {
    this.times = [...this.times.filter(time => at - time < 2500), at];
    return this.times.length >= 3 ? 'poke' : 'click';
  }
}

// Your own words, read on the Mac: a greeting gets a wave, kindness a bashful blush, fondness
// hearts, a call of "pspsps" an excited cat, and harsh words drooping ears.
const harsh = /\b(bad (cat|kitty|buntini)|shut up|stupid|useless|you suck|hate you)\b/i;
const fond = /\b(love (you|u|ya)|good (kitty|cat)|who['’]?s a good)\b/i;
const kind = /\b(thanks?|thank you|thx|good (job|work|girl|boy)|well done|nice (one|work)|great job|you['’]?re (the best|great|amazing|cute|adorable)|so cute|cutie)\b/i;
const called = /\bps(ps)+\b/i;
const greeting = /^\s*(hi|hello|hey|hiya|howdy|yo|good (morning|afternoon|evening)|morning)\b/i;

export function feelingFor(words: string): PetAction | null {
  if (harsh.test(words)) return 'sad';
  if (fond.test(words)) return 'love';
  if (kind.test(words)) return 'bashful';
  if (called.test(words)) return 'excited';
  if (greeting.test(words)) return 'wave';
  return null;
}
