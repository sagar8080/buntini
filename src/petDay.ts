import type { PetAction } from './petLife.ts';
import type { Face } from './petMood.ts';
import type { Band } from './petTouch.ts';
import type { PetMotion } from './types';

/* The cat's day, from what it can notice: the hour, how long it has been up, the pointer,
   and you at your Mac. When it naps and for how long, when it gets the zoomies, how its
   interest in the pointer comes and goes, when it slow-blinks back at you, and when it
   wants room or company. Pure logic; usePetLife.ts and usePetWorld.ts act on it. */

const MINUTE = 60_000;
const clamp01 = (value: number) => Math.min(1, Math.max(0, value));

/** How much a cat wants to sleep at this hour, from 0 to 1. Cats doze through the early
    afternoon and the small hours, and are wide awake at dawn and dusk. */
export function sleepiness(hour: number): number {
  if (hour >= 5 && hour < 8) return .15;
  if (hour >= 8 && hour < 12) return .35;
  if (hour >= 12 && hour < 16) return .7;
  if (hour >= 16 && hour < 18) return .4;
  if (hour >= 18 && hour < 21) return .15;
  if (hour >= 21 && hour < 23) return .5;
  return .85;
}

const awakeFor: Record<PetMotion, number> = { playful: 1.3, calm: 1, sleepy: .7 };
const sleepsFor: Record<PetMotion, number> = { playful: .7, calm: 1, sleepy: 1.4 };

/** How long it stays up before its next nap, in ms: about 25 to 70 minutes for a calm
    cat, less at a sleepy hour, for a sleepy cat or a tired one (`energy`, 0 to 1), and a
    third either way at random. */
export function napAfter(hour: number, motion: PetMotion, energy: number, random = Math.random): number {
  const minutes = (70 - 45 * sleepiness(hour)) * awakeFor[motion] * (.6 + .6 * clamp01(energy));
  return minutes * MINUTE * (.7 + random() * .6);
}

/** How long a nap lasts, in ms: about 5 to 20 minutes, longer at a sleepy hour and for a
    sleepy cat, never under 3 or over 30. */
export function napLength(hour: number, motion: PetMotion, random = Math.random): number {
  const minutes = (5 + 15 * sleepiness(hour)) * sleepsFor[motion] * (.7 + random() * .6);
  return Math.min(30, Math.max(3, minutes)) * MINUTE;
}

/** Whether to settle down for a nap now: one is due, nobody has touched it for two
    minutes, and the pointer isn't hovering by it (`near`, 0 to 1). */
export const napDue = (now: number, dueAt: number, touchedAt: number, near: number) => now >= dueAt && now - touchedAt >= 2 * MINUTE && near < .3;

/** Before a nap it kneads its spot and turns round once. With Less motion it simply lies down. */
export const settleRoutine = (still: boolean): PetAction[] => still ? [] : ['knead', 'circle'];

/** Waking from a nap that ran its course: a long stretch, a yawn and a quick wash. Woken
    by you, just the stretch. */
export const wakeRoutine = (natural: boolean): PetAction[] => natural ? ['stretch', 'yawn', 'groom'] : ['stretch'];

const zoomiesFor: Record<PetMotion, number> = { playful: 1.6, calm: .6, sleepy: .3 };
/** The least time between two bursts of zoomies. */
export const zoomiesGap: Record<PetMotion, number> = { playful: 20 * MINUTE, calm: 45 * MINUTE, sleepy: 90 * MINUTE };

/** The chance of a burst of zoomies on one roaming turn. Cats get them mostly at dusk and in
    the evening, a little at dawn, and right after a good sleep; a lively, excited cat more
    often, a calm or sleepy one less. Never with Less motion (`still`). `sinceWake` and
    `slept` are in ms: how long ago it last woke and how long that sleep was. */
export function zoomiesChance({ hour, motion, energy, excitement, sinceWake, slept, sinceLast, still }: {
  hour: number; motion: PetMotion; energy: number; excitement: number; sinceWake: number; slept: number; sinceLast: number; still: boolean;
}): number {
  if (still || sinceLast < zoomiesGap[motion]) return 0;
  const hourly = hour >= 18 && hour < 23 ? .05 : hour >= 5 && hour < 7 ? .04 : .002;
  const rested = sinceWake < 3 * MINUTE && slept >= 10 * MINUTE ? .3 : 0;
  return Math.min(.6, (hourly * (.5 + clamp01(energy)) * (1 + clamp01(excitement)) + rested) * zoomiesFor[motion]);
}

/** Whether you're typing (or scrolling): there was input in the last second and a half and
    the pointer hasn't moved for two seconds (`pointerStill`, in ms). Only the time since any
    input is known, never what it was. */
export const handsBusy = (idleSeconds: number | null | undefined, pointerStill: number) => idleSeconds != null && idleSeconds < 1.5 && pointerStill >= 2000;

/** The pointer resting near the cat, but not on it, reads as you looking at it. After a
    moment a cat that knows you slow-blinks back; a new one mostly looks away. Once each
    time the pointer comes to rest, and at most every 40 seconds. A pointer parked there
    while you type, or long since, isn't looking. */
export class Regard {
  private since: number | null = null;
  private answered = false;
  private last = -Infinity;
  /** The pointer moved, ending `distance` px from the cat's face. */
  moved(distance: number, at: number) {
    this.since = distance >= 70 && distance <= 320 ? at : null;
    this.answered = false;
  }
  /** Whether the pointer came to rest near it lately, so it holds your gaze. */
  resting(at: number): boolean {
    return this.since !== null && at - this.since < 12_000;
  }
  answer(at: number, band: Band, typing: boolean, random = Math.random): 'slowblink' | 'look' | null {
    if (this.since === null || this.answered || at - this.last < 40_000) return null;
    const rested = at - this.since;
    // Held while its eyes are still on the pointer (LOOKS).
    const hold = { close: 1500, familiar: 2500, new: 3000 }[band];
    if (rested < hold) return null;
    this.answered = true;
    if (typing || rested > hold + 8000) return null;
    this.last = at;
    const roll = random();
    if (band === 'new') return roll < .3 ? 'slowblink' : 'look';
    return roll < (band === 'close' ? .9 : .6) ? 'slowblink' : null;
  }
}

// How readily each face notices the pointer, how long it keeps watching, and how long it
// then can't be bothered: a bored or excited cat is all eyes, a drowsy one barely looks.
const notice: Record<Face, number> = { neutral: .8, content: .65, drowsy: .35, bored: 1, grumpy: .5, excited: 1 };
const span: Record<Face, number> = { neutral: 1, content: .9, drowsy: .6, bored: 1.5, grumpy: .7, excited: 1.4 };
const lull: Record<Face, number> = { neutral: 1, content: 1.2, drowsy: 1.5, bored: .5, grumpy: 1.3, excited: .5 };
/** A pause this long makes the next movement new again. */
const FORGETS = 8000;
/** The eyes stay on the pointer this long after it last moved. */
export const LOOKS = 3500;

/** Interest in the pointer comes and goes. Something new catches its eye; it watches for 10
    to 20 seconds of movement, gets bored and looks away, and for a while after it can't be
    bothered, unless the pointer comes right up to it. */
export class Curiosity {
  private since: number | null = null;
  private until = 0;
  private ignoring = -Infinity;
  private seen = -Infinity;
  private close = false;
  /** The pointer moved, `distance` px from the cat's face. 'bored', once, as it loses interest. */
  moved(at: number, distance: number, face: Face, random = Math.random): 'watch' | 'ignore' | 'bored' {
    const fresh = at - this.seen > FORGETS;
    this.seen = at;
    this.close = distance < 90;
    if (this.close) return 'watch';
    if (at < this.ignoring) return 'ignore';
    if (this.since === null || fresh) {
      if (random() >= notice[face]) {
        this.since = null;
        this.ignoring = at + 4000;
        return 'ignore';
      }
      this.since = at;
      this.until = at + (10_000 + random() * 10_000) * span[face];
      return 'watch';
    }
    if (at < this.until) return 'watch';
    this.since = null;
    this.ignoring = at + (20_000 + random() * 40_000) * lull[face];
    return 'bored';
  }
  /** Whether its eyes are on the pointer now. */
  watching(at: number): boolean {
    return at - this.seen < LOOKS && (this.close || this.since !== null);
  }
  /** Whether it's interested enough to stalk or chase it. */
  engaged(at: number): boolean {
    return this.since !== null && at - this.seen < FORGETS;
  }
}

/** How it shows it's lost interest: mostly it looks away, sometimes it washes, or flicks an ear. */
export function loseInterest(random = Math.random): 'look' | 'groom' | 'ear' {
  const roll = random();
  return roll < .5 ? 'look' : roll < .8 ? 'groom' : 'ear';
}

/** Fuss and neglect. A cat that isn't close to you yet wants room after a lot of fuss in a
    short while, and any cat left alone long enough while you work comes to find you. */
export class Company {
  private fusses: number[] = [];
  private touched: number;
  private lastRoom = -Infinity;
  private lastVisit: number;
  constructor(now: number) {
    this.touched = now;
    this.lastVisit = now;
  }
  /** A click, a poke, or the start of a petting. */
  fuss(at: number) {
    this.touched = at;
    this.fusses = [...this.fusses.filter(time => at - time < 2 * MINUTE), at];
  }
  /** When it was last touched. */
  get lastTouched() { return this.touched; }
  /** Whether it walks off now: six lots of fuss in two minutes (four for a new cat), from a
      cat that isn't close to you yet, at most every ten minutes. */
  wantsRoom(at: number, band: Band): boolean {
    if (band === 'close' || at - this.lastRoom < 10 * MINUTE) return false;
    if (this.fusses.filter(time => at - time < 2 * MINUTE).length < (band === 'new' ? 4 : 6)) return false;
    this.lastRoom = at;
    this.fusses = [];
    return true;
  }
  /** Whether it comes over now: bored, you busy at your Mac, nobody has touched it for a
      while (12 minutes for a close cat, 20 for a familiar one, 30 for a new one), and no
      visit in the last 40 minutes. */
  wantsCompany(at: number, { boredom, busy, band }: { boredom: number; busy: boolean; band: Band }): boolean {
    const alone = { close: 12, familiar: 20, new: 30 }[band] * MINUTE;
    if (!busy || boredom < .7 || at - this.touched < alone || at - this.lastVisit < 40 * MINUTE) return false;
    this.lastVisit = at;
    return true;
  }
}
