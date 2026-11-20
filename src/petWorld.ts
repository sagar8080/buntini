import { number, t, tn } from './i18n.ts';
import type { PetMotion } from './types';

/* Pure decision logic for the roaming pet: where it can stand, what it does next,
   and when a gentle nudge is worth showing. usePetWorld.ts wires it to the window. */

export interface Rect { x: number; y: number; width: number; height: number }
export interface World {
  idleSeconds?: number | null;
  app?: { name: string; category: string } | null;
  frontWindow?: { number: number; rect: Rect } | null;
  workArea: Rect;
  pet: Rect;
  fullscreen: boolean;
  meeting: boolean;
  health?: { batteryPercent?: number | null; charging: boolean; cpuPercent: number; diskFreePercent: number } | null;
  perched: boolean;
}
export interface Surface { y: number; minX: number; maxX: number; perch?: number }
/** `dash`: a burst of zoomies, three times the usual speed. */
export interface Move { kind: 'walk' | 'jump' | 'fall'; x: number; y: number; perch?: number; dash?: boolean }

/** Space between the window's bottom edge and the pet's paws. */
export const footGap = (scale: number) => 16 * scale;

export function ground(world: World, scale: number): Surface {
  const { workArea: area, pet } = world;
  return { y: area.y + area.height - pet.height + footGap(scale), minX: area.x, maxX: area.x + area.width - pet.width };
}

/** The top edge of the window the user is working in, if the pet fits on it. */
export function windowTop(world: World, scale: number): Surface | null {
  const front = world.frontWindow;
  if (!front || world.fullscreen) return null;
  const { rect } = front;
  const y = rect.y - world.pet.height + footGap(scale);
  const half = world.pet.width / 2;
  const minX = rect.x + 24 - half, maxX = rect.x + rect.width - 24 - half;
  if (y < world.workArea.y || rect.width < 260 || maxX - minX < 40) return null;
  return { y, minX: Math.max(minX, world.workArea.x - half / 2), maxX: Math.min(maxX, world.workArea.x + world.workArea.width - world.pet.width), perch: front.number };
}

export function standingOn(world: World, scale: number): 'ground' | 'window' | 'air' {
  if (world.perched) return 'window';
  const floor = ground(world, scale);
  if (Math.abs(world.pet.y - floor.y) < 6) return 'ground';
  const top = windowTop(world, scale);
  if (top && Math.abs(world.pet.y - top.y) < 6 && world.pet.x >= top.minX - 2 && world.pet.x <= top.maxX + 2) return 'window';
  return 'air';
}

/** Where a pet that was dropped or lost its perch lands: the window below it, else the floor. */
export function landing(world: World, scale: number): Move | null {
  const floor = ground(world, scale);
  const top = windowTop(world, scale);
  const x = Math.min(Math.max(world.pet.x, floor.minX), floor.maxX);
  if (top && world.pet.y <= top.y + 4 && world.pet.x >= top.minX && world.pet.x <= top.maxX) {
    return Math.abs(world.pet.y - top.y) < 3 ? { kind: 'walk', x: world.pet.x, y: top.y, perch: top.perch } : { kind: 'fall', x: world.pet.x, y: top.y, perch: top.perch };
  }
  if (Math.abs(world.pet.y - floor.y) < 3 && Math.abs(world.pet.x - x) < 1) return null;
  return { kind: world.pet.y < floor.y - 3 ? 'fall' : 'walk', x, y: floor.y };
}

export const speeds: Record<PetMotion, number> = { calm: 55, playful: 95, sleepy: 40 };
const pauses: Record<PetMotion, [number, number]> = { calm: [9000, 20000], playful: [5000, 12000], sleepy: [16000, 32000] };

export function nextRoamDelay(motion: PetMotion, random = Math.random) {
  const [low, high] = pauses[motion];
  return low + random() * (high - low);
}

const clamp = (value: number, low: number, high: number) => Math.min(Math.max(value, low), high);

/** What the pet stands on now: the window it's perched on, or the floor. */
function underfoot(world: World, scale: number, place: 'ground' | 'window'): Surface {
  const top = windowTop(world, scale);
  return place === 'window' && top ? top : ground(world, scale);
}

/** Perched on a window that is no longer the one in front, whose edges it can't see. */
const strandedOn = (world: World, scale: number, place: 'ground' | 'window') => {
  const top = windowTop(world, scale);
  return place === 'window' && !(top && Math.abs(world.pet.y - top.y) < 6);
};

/** Picks the pet's next move, or null to stay put. `cursorX` lets a playful pet chase the
    pointer it's watching. */
export function chooseMove(world: World, scale: number, motion: PetMotion, random = Math.random, cursorX?: number): Move | null {
  const place = standingOn(world, scale);
  if (place === 'air') return landing(world, scale);
  const floor = ground(world, scale);
  const top = windowTop(world, scale);
  const surface = underfoot(world, scale, place);
  const userActive = (world.idleSeconds ?? 0) < 60;
  const options: [weight: number, move: () => Move | null][] = [
    [motion === 'calm' ? 5 : motion === 'sleepy' ? 7 : 3, () => null],
    [motion === 'playful' ? 5 : 4, () => {
      const step = (80 + random() * 320) * (random() < .5 ? -1 : 1);
      return { kind: 'walk', x: clamp(world.pet.x + step, surface.minX, surface.maxX), y: surface.y, perch: surface.perch };
    }],
  ];
  // Keeping the user company: walk under the window they are working in, then hop up.
  // Jumps stay short sideways so they read as a leap, never a slide.
  if (place === 'ground' && top && userActive) options.push([motion === 'sleepy' ? 1 : 3, () => {
    const x = clamp(world.pet.x, top.minX, top.maxX);
    return Math.abs(x - world.pet.x) > 40 ? { kind: 'walk', x, y: floor.y } : { kind: 'jump', x, y: top.y, perch: top.perch };
  }]);
  if (place === 'window') options.push([1.5, () => ({ kind: 'jump', x: clamp(world.pet.x + (random() < .5 ? -120 : 120), floor.minX, floor.maxX), y: floor.y })]);
  if (motion === 'playful' && cursorX !== undefined) options.push([2, () => ({ kind: 'walk', x: clamp(cursorX - world.pet.width / 2, surface.minX, surface.maxX), y: surface.y, perch: surface.perch })]);
  const total = options.reduce((sum, [weight]) => sum + weight, 0);
  let roll = random() * total;
  for (const [weight, move] of options) {
    roll -= weight;
    if (roll < 0) {
      const next = move();
      return next && Math.abs(next.x - world.pet.x) < 8 && Math.abs(next.y - world.pet.y) < 2 ? null : next;
    }
  }
  return null;
}

/** A burst of zoomies along where it stands (petDay.ts decides when): a sprint to the far
    end, then most of the way back. */
export function zoomies(world: World, scale: number, random = Math.random): Move[] {
  const place = standingOn(world, scale);
  if (place === 'air' || strandedOn(world, scale, place)) return [];
  const surface = underfoot(world, scale, place);
  const far = world.pet.x - surface.minX > surface.maxX - world.pet.x ? surface.minX : surface.maxX;
  const there = clamp(far + (random() - .5) * 60, surface.minX, surface.maxX);
  const back = clamp(there + (world.pet.x - there) * (.5 + random() * .4), surface.minX, surface.maxX);
  if (Math.abs(there - world.pet.x) < 120) return [];
  return [there, back].map((x): Move => ({ kind: 'walk', x, y: surface.y, perch: surface.perch, dash: true }));
}

/** A few steps off from you, when it's had enough fuss: 160 to 300 px away from the pointer
    (`pointerX`, the screen x) along where it stands, or the other way when cornered. */
export function walkAway(world: World, scale: number, pointerX?: number, random = Math.random): Move | null {
  const place = standingOn(world, scale);
  if (place === 'air' || strandedOn(world, scale, place)) return null;
  const surface = underfoot(world, scale, place);
  const centre = world.pet.x + world.pet.width / 2;
  const away = pointerX === undefined ? (random() < .5 ? -1 : 1) : pointerX > centre ? -1 : 1;
  const step = 160 + random() * 140;
  let x = clamp(world.pet.x + away * step, surface.minX, surface.maxX);
  if (Math.abs(x - world.pet.x) < 60) x = clamp(world.pet.x - away * step, surface.minX, surface.maxX);
  return Math.abs(x - world.pet.x) < 60 ? null : { kind: 'walk', x, y: surface.y, perch: surface.perch };
}

/** The next step toward sitting by you after a long while alone: under the window you're
    working in, up onto it, and along it to just below the pointer; on the floor below the
    pointer when there's no window to sit on. Null once it's there. */
export function approach(world: World, scale: number, pointerX: number): Move | null {
  const place = standingOn(world, scale);
  if (place === 'air') return landing(world, scale);
  const floor = ground(world, scale);
  const top = windowTop(world, scale);
  const below = (surface: Surface) => clamp(pointerX - world.pet.width / 2, surface.minX, surface.maxX);
  // On some other window: down to the floor first.
  if (strandedOn(world, scale, place)) return { kind: 'jump', x: clamp(world.pet.x, floor.minX, floor.maxX), y: floor.y };
  if (place === 'window' && top) {
    const x = below(top);
    return Math.abs(x - world.pet.x) < 30 ? null : { kind: 'walk', x, y: top.y, perch: top.perch };
  }
  if (top) {
    const x = clamp(below(top), floor.minX, floor.maxX);
    return Math.abs(x - world.pet.x) > 40 ? { kind: 'walk', x, y: floor.y } : { kind: 'jump', x: clamp(world.pet.x, top.minX, top.maxX), y: top.y, perch: top.perch };
  }
  const x = below(floor);
  return Math.abs(x - world.pet.x) < 30 ? null : { kind: 'walk', x, y: floor.y };
}

export type NudgeAction = 'checkMac' | 'lookUp' | 'findFile' | 'focus' | 'snooze' | 'remindLate' | 'dismiss';
export interface Nudge { id: string; kind: string; text: string; actions: { label: string; action: NudgeAction }[]; ttl: number }

const MINUTE = 60_000;

/** Decides when a gentle nudge is worth it. Keeps its own memory of streaks and cooldowns. */
export class NudgeRules {
  private lastNudge = -Infinity;
  private shown = new Map<string, number>();
  private activeSince: number | null = null;
  private awaySince: number | null = null;
  private breakDueAt: number | null = null;
  private focusEndsAt: number | null = null;
  private focusQuiet = false;
  private lateRemindAt: number | null = null;
  private appSince: { category: string; at: number } | null = null;
  private hotSince: number | null = null;
  private batteryWarned = false;

  private readonly breakAfter: number;
  private readonly gap: number;

  constructor(breakAfter = 50 * MINUTE, gap = 10 * MINUTE) {
    this.breakAfter = breakAfter;
    this.gap = gap;
  }

  /** `quiet`: the island's focus timer runs it and says when it ends. */
  startFocus(now: number, minutes = 25, quiet = false) { this.focusEndsAt = now + minutes * MINUTE; this.focusQuiet = quiet; }
  snoozeBreak(now: number, minutes = 15) { this.breakDueAt = now + minutes * MINUTE; }
  remindLate(now: number, minutes = 30) { this.lateRemindAt = now + minutes * MINUTE; }
  get focusing() { return this.focusEndsAt !== null; }

  private ready(kind: string, now: number, cooldown: number, urgent = false) {
    return (urgent || now - this.lastNudge >= this.gap) && now - (this.shown.get(kind) ?? -Infinity) >= cooldown;
  }
  private make(kind: string, now: number, text: string, actions: Nudge['actions'], ttl = 25_000): Nudge {
    this.lastNudge = now;
    this.shown.set(kind, now);
    return { id: `${kind}-${now}`, kind, text, actions, ttl };
  }

  /** Call about once a second. `hour` is the local hour (0–23). */
  update(world: World, now: number, hour: number): Nudge | null {
    const idle = world.idleSeconds ?? 0;
    const away = idle >= 180;
    let welcome: Nudge | null = null;
    if (away) {
      this.awaySince ??= now - idle * 1000;
      this.activeSince = null;
      this.appSince = null;
    } else {
      if (this.awaySince !== null && now - this.awaySince >= 10 * MINUTE && this.ready('welcome', now, 0, true)) {
        const minutes = Math.round((now - this.awaySince) / MINUTE);
        welcome = this.make('welcome', now, minutes >= 60 ? t('pet.nudge.welcomeHours', { hours: number(Math.floor(minutes / 60)), minutes: number(minutes % 60) }) : tn('pet.nudge.welcomeMinutes', minutes), [{ label: t('pet.nudge.hi'), action: 'dismiss' }], 8000);
      }
      this.awaySince = null;
      this.activeSince ??= now;
      this.breakDueAt ??= this.activeSince + this.breakAfter;
    }
    if (welcome) return welcome;
    if (world.meeting || world.fullscreen) return null;

    if (this.focusEndsAt !== null && now >= this.focusEndsAt) {
      this.focusEndsAt = null;
      this.breakDueAt = now + this.breakAfter;
      if (!this.focusQuiet) return this.make('focus-done', now, t('pet.nudge.focusDone'), [{ label: t('pet.nudge.okay'), action: 'dismiss' }, { label: t('pet.nudge.anotherFocus'), action: 'focus' }]);
    }
    const battery = world.health?.batteryPercent;
    if (world.health?.charging || battery == null || battery > 20) this.batteryWarned = false;
    if (battery != null && !world.health?.charging && battery <= 15 && !this.batteryWarned && this.ready('battery', now, 0, battery <= 5)) {
      this.batteryWarned = true;
      return this.make('battery', now, t('pet.nudge.battery', { percent: number(battery) }), [{ label: t('pet.nudge.gotIt'), action: 'dismiss' }]);
    }
    if (away) return null;
    if (this.lateRemindAt !== null && now >= this.lateRemindAt) {
      this.lateRemindAt = null;
      return this.make('late-reminder', now, t('pet.nudge.lateReminder'), [{ label: t('pet.nudge.thanks'), action: 'dismiss' }]);
    }
    if (this.breakDueAt !== null && now >= this.breakDueAt && !this.focusing && this.ready('break', now, 0)) {
      this.breakDueAt = now + this.breakAfter;
      const minutes = Math.round((now - (this.activeSince ?? now)) / MINUTE);
      return this.make('break', now, tn('pet.nudge.break', minutes), [{ label: t('pet.nudge.done'), action: 'dismiss' }, { label: t('pet.nudge.snooze'), action: 'snooze' }]);
    }
    if ((hour >= 23 || hour < 5) && this.ready('late', now, 8 * 60 * MINUTE)) {
      return this.make('late', now, t('pet.nudge.late'), [{ label: t('pet.nudge.remindLate'), action: 'remindLate' }, { label: t('pet.nudge.imOkay'), action: 'dismiss' }]);
    }
    const cpu = world.health?.cpuPercent ?? 0;
    this.hotSince = cpu >= 85 ? this.hotSince ?? now : null;
    if (this.hotSince !== null && now - this.hotSince >= 2 * MINUTE && this.ready('cpu', now, 30 * MINUTE)) {
      return this.make('cpu', now, t('pet.nudge.cpu', { percent: number(Math.round(cpu)) }), [{ label: t('pet.nudge.checkMac'), action: 'checkMac' }, { label: t('pet.nudge.notNow'), action: 'dismiss' }]);
    }
    const free = world.health?.diskFreePercent;
    if (free != null && free < 8 && this.ready('disk', now, 24 * 60 * MINUTE)) {
      return this.make('disk', now, t('pet.nudge.disk', { percent: number(Math.round(free)) }), [{ label: t('pet.nudge.checkMac'), action: 'checkMac' }, { label: t('pet.nudge.later'), action: 'dismiss' }]);
    }
    const category = world.app?.category;
    if (!category || this.appSince?.category !== category) this.appSince = category ? { category, at: now } : null;
    if (this.appSince && now - this.appSince.at >= 3 * MINUTE && this.ready(`app-${category}`, now, 60 * MINUTE)) {
      const name = world.app?.name;
      if (category === 'browser') return this.make('app-browser', now, t('pet.nudge.lookUp'), [{ label: t('pet.nudge.lookUpAction'), action: 'lookUp' }, { label: t('pet.nudge.noThanks'), action: 'dismiss' }]);
      if (category === 'files') return this.make('app-files', now, t('pet.nudge.findFile'), [{ label: t('pet.nudge.findFileAction'), action: 'findFile' }, { label: t('pet.nudge.noThanks'), action: 'dismiss' }]);
      if ((category === 'code' || category === 'writing' || category === 'design') && !this.focusing) {
        return this.make(`app-${category}`, now, name ? t('pet.nudge.deepIn', { app: name }) : t('pet.nudge.deepInApp'), [{ label: t('pet.nudge.startTimer'), action: 'focus' }, { label: t('pet.nudge.noThanks'), action: 'dismiss' }]);
      }
    }
    return null;
  }
}
