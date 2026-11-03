import type { PetAction } from './petLife.ts';
import type { MoodEvent } from './petMood.ts';
import type { PetMotion } from './types';

/* Petting: which part of the cat a stroke lands on, and how its purr builds while you pet
   it and fades after you stop. A purr is a level from 0 to 1, not a one-off. Pure logic;
   usePetLife.ts feeds it pointer and two-finger strokes. */

export type Zone = 'ears' | 'cheeks' | 'chin' | 'head' | 'back' | 'chest' | 'tail' | 'paws';
export type Favourite = 'cheeks' | 'chin' | 'ears';

/** The part of the seated cat under a point in its 180 × 180 drawing (Cat.tsx). */
export function zoneAt(x: number, y: number): Zone | null {
  if (y >= 149 && y <= 165 && x >= 55 && x <= 125) return 'paws';
  if (x >= 123 && y > 114 && y <= 164 && x <= 160) return 'tail';
  if (y >= 40 && y < 70 && x >= 50 && x <= 130 && (x < 80 || x > 100)) return 'ears';
  if (((x - 90) / 44) ** 2 + ((y - 93) / 36) ** 2 <= 1) {
    if (y >= 113 && Math.abs(x - 90) <= 16) return 'chin';
    if (y >= 97 && Math.abs(x - 90) > 16) return 'cheeks';
    return 'head';
  }
  if (y >= 122 && y <= 164 && x >= 56 && x <= 124) return Math.abs(x - 90) <= 16 ? 'chest' : 'back';
  return null;
}

/** How fast each part makes the purr build. Cats like their faces rubbed best. */
const rise: Record<Zone, number> = { ears: 1.5, cheeks: 1.5, chin: 1.5, head: 1, back: .8, chest: .6, tail: 0, paws: 0 };
/** Purr gained per second of steady stroking on the head, and lost per second after. */
export const PURR_RISE = .25;
export const PURR_FALL = .1;
/** Strokes are measured over this window; slower than STROKING px/s is a resting hand. */
const WINDOW = 600;
const STROKING = 100;
const KNEAD_AT = .8;
const KNEAD_AFTER = 2000;
const GRAB_AFTER = 4000;
const SWAT_AFTER = 12_000;

/** One breath of a purring cat, in ms; the CSS breathing (pet-purr-breath) matches it. */
export const BREATH = 2400;

/** Trackpad taps due between `from` and `to` for a purr whose breathing began at `since`:
    one at the start of each breath once the purr is over 0.3, and a second close behind
    it over 0.7. */
export function tapsDue(since: number, from: number, to: number, level: number): 0 | 1 | 2 {
  if (level <= .3 || to <= from || Math.floor((to - since) / BREATH) <= Math.floor((from - since) / BREATH)) return 0;
  return level > .7 ? 2 : 1;
}

/** How far the head tips toward the side being stroked, in degrees: most for the cheeks
    and ears, and growing with the purr. */
const tilt: Record<Zone, number> = { ears: 8, cheeks: 8, head: 5, chin: 3, back: 2, chest: 2, tail: 0, paws: 0 };
export function tiltFor(zone: Zone, level: number, side: -1 | 1): number {
  return tilt[zone] * Math.min(1, level / .5) * side;
}

export interface PetContext { cat: boolean; motion: PetMotion; annoyance: number; favourite: Favourite | null }
export interface Touch { purr: number; play: PetAction | null; mood: MoodEvent | null }

export class Petting {
  private samples: { at: number; distance: number }[] = [];
  private level = 0;
  private updated = 0;
  private lastStroke = -Infinity;
  private strokingSince = 0;
  private zoneRun: { zone: Zone; since: number } | null = null;
  private highSince = 0;
  private lastFelt = -Infinity;
  private peak = 0;
  private swatted = false;
  private cooldowns = new Map<PetAction, number>();

  /** The purr now, faded for any time without strokes. */
  purr(at: number): number {
    this.fade(at);
    return this.level;
  }

  /** A stroke of `distance` px over `zone` at time `at` (ms). */
  stroke(zone: Zone, distance: number, at: number, context: PetContext): Touch {
    this.fade(at);
    this.samples = [...this.samples.filter(sample => at - sample.at < WINDOW), { at, distance }];
    const speed = this.samples.reduce((sum, sample) => sum + sample.distance, 0) / (WINDOW / 1000);
    const quiet = { purr: this.level, play: null, mood: null };
    if (zone === 'tail') {
      if (!this.ready('flick', at, 3000)) return quiet;
      this.level = 0;
      this.peak = 0;
      return { purr: 0, play: context.cat ? 'flick' : 'tilt', mood: 'tail' };
    }
    if (zone === 'paws') {
      if (speed < STROKING || !context.cat || !this.ready('bat', at, 4000)) return quiet;
      return { purr: this.level, play: 'bat', mood: 'play' };
    }
    if (speed < STROKING) return quiet;
    if (at - this.lastStroke > 1000) { this.strokingSince = at; this.zoneRun = null; }
    // The first stroke after a pause only starts the count.
    const elapsed = at - this.lastStroke < 200 ? at - this.lastStroke : 0;
    this.lastStroke = at;
    this.zoneRun = this.zoneRun?.zone === zone ? this.zoneRun : { zone, since: at };
    const favourite = context.favourite === zone ? 2 : 1;
    this.level = Math.min(1, this.level + PURR_RISE * rise[zone] * favourite * elapsed / 1000);
    this.updated = at;
    this.peak = Math.max(this.peak, this.level);
    const felt = at - this.lastFelt >= 1000;
    if (felt) this.lastFelt = at;
    const mood: MoodEvent | null = felt ? 'pet' : null;
    if (!context.cat) return { purr: this.level, play: this.level > .15 ? 'purr' : null, mood };
    // Too much of a good thing: a playful (or cross) cat swats the hand, once a session.
    if (!this.swatted && at - this.strokingSince > SWAT_AFTER && (context.motion === 'playful' || context.annoyance > .3)) {
      this.swatted = true;
      this.level = 0;
      this.peak = 0;
      return { purr: 0, play: 'swat', mood: 'overpet' };
    }
    // A long belly rub is an invitation to a playful cat: it grabs your hand and kicks.
    if (zone === 'chest' && context.motion === 'playful' && at - this.zoneRun.since > GRAB_AFTER && this.ready('grab', at, 10_000)) {
      this.level = Math.max(0, this.level - .3);
      this.zoneRun = null;
      return { purr: this.level, play: 'grab', mood: 'play' };
    }
    // Purring hard for a while, it starts making biscuits.
    if (this.level >= KNEAD_AT) {
      if (!this.highSince) this.highSince = at;
      if (at - this.highSince >= KNEAD_AFTER) { this.highSince = 0; return { purr: this.level, play: 'knead', mood }; }
    } else this.highSince = 0;
    return { purr: this.level, play: this.level > .15 ? 'purr' : null, mood };
  }

  /** Once, a second after a good long purr's strokes stop: time for a slow blink. */
  afterglow(at: number): boolean {
    this.fade(at);
    if (this.peak < .5 || at - this.lastStroke < 1000) return false;
    this.peak = 0;
    return true;
  }

  /** The hand left: the next petting starts a new session, with a swat allowed again.
      True when the purr was going well enough for the cat to lean after the hand. */
  leave(at: number): boolean {
    this.samples = [];
    this.swatted = false;
    this.zoneRun = null;
    return this.purr(at) > .4;
  }

  private fade(at: number) {
    if (at - this.lastStroke < 300) return;
    const from = Math.max(this.updated, this.lastStroke + 300);
    if (at > from) this.level = Math.max(0, this.level - PURR_FALL * (at - from) / 1000);
    this.updated = at;
    if (!this.level) this.peak = 0;
  }

  private ready(action: PetAction, at: number, gap: number) {
    if (at - (this.cooldowns.get(action) ?? -Infinity) < gap) return false;
    this.cooldowns.set(action, at);
    return true;
  }
}
