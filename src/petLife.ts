import { moodWeights, type Face } from './petMood.ts';
import type { PetMotion } from './types';

export type PetAction =
  | 'look' | 'slowblink' | 'tilt' | 'ear' | 'tailcurl' | 'groom' | 'stretch' | 'yawn' | 'wiggle' | 'nod'
  | 'bat' | 'pounce' | 'nudge' | 'mischief'
  | 'stalk' | 'hunt' | 'knead' | 'loaf'
  | 'wink' | 'smile' | 'lick' | 'askpet'
  | 'sigh' | 'wave' | 'blep' | 'tailchase' | 'sneeze' | 'chatter' | 'bunt' | 'swat' | 'dream'
  | 'love' | 'purr' | 'happy' | 'sad' | 'proud' | 'bashful' | 'curious' | 'excited' | 'startle' | 'grumpy';

/** How long each one-shot action class stays on the pet, in ms. Purr is held while petting. */
export const actionDuration: Record<PetAction, number> = {
  look: 2600, slowblink: 1800, tilt: 1600, ear: 650, tailcurl: 1400, groom: 2600, stretch: 2600,
  yawn: 2000, wiggle: 1500, nod: 2400, bat: 2200, pounce: 1700, nudge: 1300, love: 1600, purr: 900, happy: 1400, sad: 1800,
  mischief: 2800, stalk: 1500, hunt: 900, knead: 2800, loaf: 7000,
  wink: 1100, smile: 2000, lick: 2600, askpet: 9000,
  sigh: 2400, wave: 2000, blep: 3200, tailchase: 2600, sneeze: 1300, chatter: 2000, bunt: 2600, swat: 900, dream: 1400,
  proud: 2200, bashful: 2400, curious: 2200, excited: 1800, startle: 1200, grumpy: 2600,
};

/** Actions only a cat can act out; other characters fall back to a plain reaction. */
export const catOnly: ReadonlySet<PetAction> = new Set(['bat', 'pounce', 'nudge', 'mischief', 'stalk', 'hunt', 'knead', 'loaf', 'groom', 'lick', 'askpet', 'sigh', 'wave', 'blep', 'tailchase', 'sneeze', 'chatter', 'bunt', 'swat', 'dream', 'bashful', 'grumpy', 'curious', 'startle', 'proud', 'excited', 'wink', 'smile']);
export const forCharacter = (action: PetAction, cat: boolean): PetAction => cat || !catOnly.has(action) ? action : ['grumpy', 'startle', 'curious'].includes(action) ? 'tilt' : 'happy';

/** Ways of playing with the ball of wool: two swipes, a pounce, a lazy nudge. */
export const toyActions: readonly PetAction[] = ['bat', 'pounce', 'nudge', 'mischief'];
export const isToyAction = (action: PetAction | null) => !!action && toyActions.includes(action);

/** How hard this play is: how far the ball rolls (0.55–1.45 of the usual) and how high the
    paw lifts (0.75–1.25), so no two plays look the same. */
export function toyPlay(random = Math.random): { roll: number; lift: number } {
  return { roll: .55 + random() * .9, lift: .75 + random() * .5 };
}

type Weights = Partial<Record<PetAction, number>>;
const catIdle: Record<PetMotion, Weights> = {
  calm: { look: 4, slowblink: 3, tilt: 2, ear: 2, nudge: 2, loaf: 2, smile: 2, tailcurl: 1, groom: 1, lick: 1, wink: 1, stretch: 1, bat: 1, blep: .5, sneeze: .4, yawn: 1 },
  // A playful cat keeps going back to its ball of wool, a different way each time.
  // Now and then it knocks the ball far away and looks straight at you.
  // It chases its own tail, and sticks its tongue out for no reason at all.
  playful: { bat: 3, look: 3, wiggle: 3, pounce: 2, nudge: 2, tilt: 2, tailcurl: 2, ear: 2, wink: 2, tailchase: 2, mischief: 1, smile: 1, groom: 1, lick: 1, stretch: 1, slowblink: 1, blep: 1, sneeze: .4 },
  // A sleepy cat is dozing while idle, so it only stirs, and now and then paws at the ball.
  sleepy: { nod: 3, loaf: 3, ear: 2, yawn: 2, tailcurl: 1, nudge: 1, lick: 1 },
};
// Other characters have no ears, paws or pupils to animate separately.
const otherIdle: Record<PetMotion, Weights> = {
  calm: { slowblink: 3, tilt: 2, stretch: 1 },
  playful: { tilt: 2, wiggle: 3, stretch: 1, slowblink: 1 },
  sleepy: { slowblink: 2, stretch: 1 },
};
const idleDelay: Record<PetMotion, [number, number]> = { calm: [7000, 13000], playful: [3500, 7500], sleepy: [8000, 15000] };

/** A habit picked by personality and, for a cat, tilted by the face its mood is wearing. */
export function pickIdleAction(motion: PetMotion, cat: boolean, random = Math.random, face?: Face): PetAction {
  const weights = Object.entries(cat && face ? moodWeights(catIdle[motion], face) : (cat ? catIdle : otherIdle)[motion]) as [PetAction, number][];
  let roll = random() * weights.reduce((sum, [, weight]) => sum + weight, 0);
  for (const [action, weight] of weights) {
    roll -= weight;
    if (roll < 0) return action;
  }
  return weights[weights.length - 1][0];
}

const MINUTE = 60_000;
/** A cat stretches every so often whatever else it's doing. */
export const STRETCH_EVERY = 10 * MINUTE;
/** And asks to be petted now and then, not more than this often. */
export const ASK_GAP = 12 * MINUTE;

/** The next habit: a stretch when one is due, sometimes a request for a pet while you're
    around (never from a sleepy cat, and sooner from a bored one), otherwise one picked by
    its personality and mood. */
export function nextHabit(motion: PetMotion, cat: boolean, since: { stretch: number; ask: number; userAround: boolean }, random = Math.random, face?: Face): PetAction {
  const bored = face === 'bored';
  if (cat && motion !== 'sleepy' && since.userAround && since.ask >= (bored ? ASK_GAP / 2 : ASK_GAP) && random() < (bored ? .5 : .3)) return 'askpet';
  if (since.stretch >= STRETCH_EVERY) return 'stretch';
  return pickIdleAction(motion, cat, random, face);
}

/** The wait before the next habit. Right after playing, a cat often comes back for more
    (40% of the time, after about a third of the usual wait), so play comes in random bursts. */
export function nextIdleDelay(motion: PetMotion, random = Math.random, afterPlay = false): number {
  const [low, high] = idleDelay[motion];
  const delay = low + random() * (high - low);
  return afterPlay && motion !== 'sleepy' && random() < .4 ? delay * .35 : delay;
}

/** The wait before the next habit, counted from when this one ends. Counted from its start, a
    quick return after playing would come due while the play is still going and be skipped. */
/** `pace` (from the mood) stretches or shortens the wait. */
export function nextHabitDelay(motion: PetMotion, played: PetAction | null, random = Math.random, pace = 1): number {
  return (played ? actionDuration[played] : 0) + nextIdleDelay(motion, random, isToyAction(played)) * pace;
}

/** Between habits the eyes wander: a small glance aside, or back to the middle, every few seconds. */
export function nextSaccade(random = Math.random): { delay: number; gaze: { x: number; y: number } | null } {
  const delay = 1800 + random() * 3600;
  return random() < .4 ? { delay, gaze: null } : { delay, gaze: { x: (random() * 2 - 1) * .5, y: (random() * 1.2 - .7) * .5 } };
}

/** Now and then one ear swivels toward a sound only the cat heard, holds, and swivels back. */
export function nextEarSwivel(random = Math.random): { delay: number; side: 'left' | 'right'; angle: number; hold: number } {
  return { delay: 3000 + random() * 6000, side: random() < .5 ? 'left' : 'right', angle: (random() < .5 ? -1 : 1) * (4 + random() * 6), hold: 500 + random() * 1100 };
}

/** A sleeping cat dreams every so often: a twitch of an ear, a paw, its tail. */
export const nextDreamDelay = (random = Math.random) => 9000 + random() * 12000;

/** How close the pointer is, from 0 (out of range) to 1 (right on the cat), so it perks up as you come near. */
export const nearness = (dx: number, dy: number) => Math.min(1, Math.max(0, 1 - (Math.hypot(dx, dy) - 40) / 220));

/** Blinks every few seconds; sleepy pets blink less often. */
export function nextBlinkDelay(motion: PetMotion, random = Math.random): number {
  return (motion === 'sleepy' ? 4000 : 2400) + random() * 4200;
}

/** Normalized gaze (-1..1 per axis) from the pet's face toward a point `dx`/`dy` CSS px away. */
export function gazeToward(dx: number, dy: number): { x: number; y: number } {
  const distance = Math.hypot(dx, dy);
  if (distance < 8) return { x: 0, y: 0 };
  const strength = Math.min(1, distance / 160);
  return { x: dx / distance * strength, y: dy / distance * strength * .8 };
}

/** A random place to glance at while nobody is moving the pointer. */
export function randomGlance(random = Math.random): { x: number; y: number } {
  return { x: random() * 2 - 1, y: random() * 1.2 - .7 };
}

/** A cat watching the pointer: it gets interested when the pointer darts about close by,
    and pounces once it stops nearby, like prey that froze. Darting about out of reach, it
    chatters at it the way cats do at birds through a window. Sleepy cats can't be bothered. */
export class Hunter {
  private interest = 0;
  private distant = 0;
  private lastPounce = -Infinity;
  private lastChatter = -Infinity;
  private last: { x: number; y: number; at: number } | null = null;
  /** `dx`/`dy` are the pointer's offset from the cat's face, in CSS px. */
  moved(dx: number, dy: number, at: number) {
    if (this.last) {
      const seconds = Math.max(.016, (at - this.last.at) / 1000);
      const speed = Math.hypot(dx - this.last.x, dy - this.last.y) / seconds;
      const distance = Math.hypot(dx, dy), near = distance < 260;
      this.interest = near && speed > 500 ? Math.min(1, this.interest + .35) : this.interest * .85;
      this.distant = !near && distance < 900 && speed > 700 ? Math.min(1, this.distant + .2) : this.distant * .9;
    }
    this.last = { x: dx, y: dy, at };
  }
  /** Whether to pounce now: interested, the pointer still for half a second and in reach. */
  ready(at: number, motion: PetMotion): -1 | 1 | null {
    if (motion === 'sleepy' || !this.last || this.interest < .7 || at - this.lastPounce < 15000) return null;
    if (at - this.last.at < 500 || Math.hypot(this.last.x, this.last.y) > 200) return null;
    this.interest = 0;
    this.lastPounce = at;
    return this.last.x < 0 ? -1 : 1;
  }
  /** Whether to chatter now: the pointer has been darting about out of reach and still is. */
  chatters(at: number, motion: PetMotion): boolean {
    if (motion === 'sleepy' || !this.last || this.distant < .8 || at - this.last.at > 300 || at - this.lastChatter < 45000) return false;
    this.distant = 0;
    this.lastChatter = at;
    return true;
  }
}
