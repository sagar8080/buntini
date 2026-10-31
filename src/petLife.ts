import type { PetMotion } from './types';

export type PetAction =
  | 'look' | 'slowblink' | 'tilt' | 'ear' | 'tailcurl' | 'groom' | 'stretch' | 'yawn' | 'wiggle' | 'nod'
  | 'bat' | 'pounce' | 'nudge' | 'mischief'
  | 'stalk' | 'hunt' | 'knead' | 'loaf'
  | 'love' | 'purr' | 'happy' | 'sad';

/** How long each one-shot action class stays on the pet, in ms. Purr is held while petting. */
export const actionDuration: Record<PetAction, number> = {
  look: 2600, slowblink: 1800, tilt: 1600, ear: 650, tailcurl: 1400, groom: 2600, stretch: 1900,
  yawn: 2000, wiggle: 1500, nod: 2400, bat: 2200, pounce: 1700, nudge: 1300, love: 1600, purr: 900, happy: 1400, sad: 1800,
  mischief: 2800, stalk: 1500, hunt: 900, knead: 2800, loaf: 7000,
};

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
  calm: { look: 4, slowblink: 3, tilt: 2, ear: 2, nudge: 2, loaf: 2, tailcurl: 1, groom: 1, stretch: 1, bat: 1, yawn: 1 },
  // A playful cat keeps going back to its ball of wool, a different way each time.
  // Now and then it knocks the ball far away and looks straight at you.
  playful: { bat: 3, look: 3, wiggle: 3, pounce: 2, nudge: 2, tilt: 2, tailcurl: 2, ear: 2, mischief: 1, groom: 1, stretch: 1, slowblink: 1 },
  // A sleepy cat is dozing while idle, so it only stirs, and now and then paws at the ball.
  sleepy: { nod: 3, loaf: 3, ear: 2, yawn: 2, tailcurl: 1, nudge: 1 },
};
// Other characters have no ears, paws or pupils to animate separately.
const otherIdle: Record<PetMotion, Weights> = {
  calm: { slowblink: 3, tilt: 2, stretch: 1 },
  playful: { tilt: 2, wiggle: 3, stretch: 1, slowblink: 1 },
  sleepy: { slowblink: 2, stretch: 1 },
};
const idleDelay: Record<PetMotion, [number, number]> = { calm: [7000, 13000], playful: [3500, 7500], sleepy: [8000, 15000] };

export function pickIdleAction(motion: PetMotion, cat: boolean, random = Math.random): PetAction {
  const weights = Object.entries((cat ? catIdle : otherIdle)[motion]) as [PetAction, number][];
  let roll = random() * weights.reduce((sum, [, weight]) => sum + weight, 0);
  for (const [action, weight] of weights) {
    roll -= weight;
    if (roll < 0) return action;
  }
  return weights[weights.length - 1][0];
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
export function nextHabitDelay(motion: PetMotion, played: PetAction | null, random = Math.random): number {
  return (played ? actionDuration[played] : 0) + nextIdleDelay(motion, random, isToyAction(played));
}

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
    and pounces once it stops nearby, like prey that froze. Sleepy cats can't be bothered. */
export class Hunter {
  private interest = 0;
  private lastPounce = -Infinity;
  private last: { x: number; y: number; at: number } | null = null;
  /** `dx`/`dy` are the pointer's offset from the cat's face, in CSS px. */
  moved(dx: number, dy: number, at: number) {
    if (this.last) {
      const seconds = Math.max(.016, (at - this.last.at) / 1000);
      const speed = Math.hypot(dx - this.last.x, dy - this.last.y) / seconds;
      const near = Math.hypot(dx, dy) < 260;
      this.interest = near && speed > 500 ? Math.min(1, this.interest + .35) : this.interest * .85;
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
}
