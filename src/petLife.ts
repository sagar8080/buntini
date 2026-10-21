import type { PetMotion } from './types';

export type PetAction =
  | 'look' | 'slowblink' | 'tilt' | 'ear' | 'tailcurl' | 'groom' | 'stretch' | 'yawn' | 'wiggle' | 'nod'
  | 'love' | 'purr' | 'happy' | 'sad';

/** How long each one-shot action class stays on the pet, in ms. Purr is held while petting. */
export const actionDuration: Record<PetAction, number> = {
  look: 2600, slowblink: 1800, tilt: 1600, ear: 650, tailcurl: 1400, groom: 2600, stretch: 1900,
  yawn: 2000, wiggle: 1500, nod: 2400, love: 1600, purr: 900, happy: 1400, sad: 1800,
};

type Weights = Partial<Record<PetAction, number>>;
const catIdle: Record<PetMotion, Weights> = {
  calm: { look: 4, slowblink: 3, tilt: 2, ear: 2, tailcurl: 1, groom: 1, stretch: 1, yawn: 1 },
  playful: { look: 3, wiggle: 3, tilt: 2, tailcurl: 2, ear: 2, groom: 1, stretch: 1, slowblink: 1 },
  // A sleepy cat is dozing while idle, so it only stirs.
  sleepy: { nod: 3, ear: 2, yawn: 2, tailcurl: 1 },
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

export function nextIdleDelay(motion: PetMotion, random = Math.random): number {
  const [low, high] = idleDelay[motion];
  return low + random() * (high - low);
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
