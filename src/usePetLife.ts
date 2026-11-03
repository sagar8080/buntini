import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import { command, desktop } from './bridge';
import { actionDuration, ASK_GAP, forCharacter, gazeToward, Hunter, isToyAction, nearness, nextBlinkDelay, nextDreamDelay, nextEarSwivel, nextHabit, nextHabitDelay, nextSaccade, randomGlance, toyPlay, type PetAction } from './petLife';
import { askPace, bandFor, drift, faceFor, feel, moodPace, Pokes, proudOrHappy, remembered, restored, startingMood, type Face, type Mood, type MoodEvent } from './petMood';
import { KEY_STROKE, Petting, tapsDue, tiltFor, zoneAt, type Favourite, type Zone } from './petTouch';
import { PurrSound } from './purrSound';
import type { PetMemory, PetMotion, PetState } from './types';

export interface PetReaction { kind: PetAction; id: number }

// Reactions play through a change of state (a reply starting, say); habits don't.
const reactions = new Set<PetAction>(['love', 'purr', 'happy', 'sad', 'proud', 'bashful', 'curious', 'excited', 'startle', 'grumpy', 'wave', 'swat']);
// What a reaction says happened, for the mood.
const reactionFeeling: Partial<Record<PetAction, MoodEvent>> = { happy: 'done', proud: 'done', sad: 'failed', bashful: 'praise', love: 'praise', excited: 'userBack', wave: 'userBack' };
// Where earlier versions kept the mood; it moves into the app's store once.
const MOOD_KEY = 'buntini-cat-mood';
// Petting reactions that play out before a stroke can start another.
const pettingReactions = new Set<PetAction | null>(['knead', 'swat', 'bunt', 'flick', 'grab', 'shy', 'flop', 'warn']);
// ...except pulling away, a warning and the swat it warned of, which cut in on anything.
const cutsIn = new Set<PetAction>(['shy', 'warn', 'swat']);
const forgetters = new Set<() => void>();
// One purr for the desktop cat, made in the page; see purrSound.ts.
const purrSound = new PurrSound(() => { try { return new AudioContext(); } catch { return null; } });

/** Delete everything: the store has erased the cat's memory already. This forgets the
    mood it's feeling now, and a new cat joins. */
export function forgetMood() {
  try { localStorage.removeItem(MOOD_KEY); } catch { /* Nothing was saved. */ }
  for (const forget of forgetters) forget();
}
/** Asks the desktop cat to be petted for a second, from the keyboard or VoiceOver. */
export const petFromKeyboard = () => window.dispatchEvent(new Event('buntini-pet'));
/** What the desktop cat says about being petted, for the screen-reader announcement. */
export type PetFeeling = 'purring' | 'kneading';
const stillPreferred = () => document.documentElement.dataset.motion === 'reduced' || window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* Gives a pet its small signs of life: randomized blinks, idle habits picked by its
   motion personality and its mood, eyes that follow the cursor, petting, clicks and task
   reactions. Gaze is written straight to CSS variables so cursor tracking never re-renders;
   the mood re-renders only when the face it shows changes. */
/** `paused`: travelling, napping or being carried, when habits wait. `remember`: the
    desktop cat, whose mood and bond carry over to the next launch. */
export function usePetLife(ref: RefObject<HTMLDivElement | null>, { alive, state, paused = false, motion, cat, reaction, asleep = false, held = false, remember = false }: { alive: boolean; state: PetState; paused?: boolean; motion: PetMotion; cat: boolean; reaction?: PetReaction; asleep?: boolean; held?: boolean; remember?: boolean }) {
  // Until the store answers, any mood an earlier version left in this browser.
  const [initialMood] = useState<Mood>(() => {
    const hour = new Date().getHours();
    if (!remember) return startingMood(motion, hour);
    try { return remembered(localStorage.getItem(MOOD_KEY), motion, hour, Date.now()); } catch { return startingMood(motion, hour); }
  });
  // Which cat this is and its favourite spot, once the store has answered.
  const known = useRef<{ id: string; favourite: Favourite } | null>(null);
  const [action, setAction] = useState<PetAction | null>(null);
  const [blinking, setBlinking] = useState(false);
  const [face, setFace] = useState<Face>(() => faceFor(initialMood));
  const current = useRef<PetAction | null>(null);
  const endTimer = useRef(0);
  const pointerGaze = useRef({ x: 0, y: 0 });
  const lastMove = useRef(0);
  const glance = useRef<{ x: number; y: number } | null>(null);
  const hunter = useRef(new Hunter());
  const lastTyping = useRef(0);
  const lastStretch = useRef(performance.now());
  // The first request for a pet comes a few minutes in, not straight away.
  const lastAsk = useRef(performance.now() - ASK_GAP + 4 * 60_000);
  const near = useRef(0);
  const mood = useRef<Mood>(initialMood);
  // A feeling that arrived while another was playing, or while the cat was on the move.
  const pending = useRef<{ kind: PetAction; at: number } | null>(null);
  // What it settles into after the slow blink that follows a good purr.
  const settleAfter = useRef<PetAction | null>(null);
  const live = useRef({ state, paused, motion, cat, asleep, remember });
  live.current = { state, paused, motion, cat, asleep, remember };

  const moodTo = useCallback((event: MoodEvent) => {
    mood.current = feel(mood.current, event, live.current.motion);
    setFace(faceFor(mood.current));
  }, []);

  const applyGaze = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    // While you type in the panel (up and to its left), the cat watches the words appear.
    const now = performance.now();
    const gaze = stillPreferred() ? { x: 0, y: 0 } : now - lastTyping.current < 1500 ? { x: -.9, y: -.75 } : now - lastMove.current < 3500 ? pointerGaze.current : glance.current ?? { x: 0, y: 0 };
    element.style.setProperty('--gx', gaze.x.toFixed(3));
    element.style.setProperty('--gy', gaze.y.toFixed(3));
    element.style.setProperty('--near', !stillPreferred() && now - lastMove.current < 3500 ? near.current.toFixed(2) : '0');
  }, [ref]);

  const stop = useCallback(() => {
    window.clearTimeout(endTimer.current);
    current.current = null;
    glance.current = null;
    setAction(null);
    applyGaze();
  }, [applyGaze]);

  // The waiting feeling, if it's still fresh and the cat has settled.
  const takePending = () => {
    const waiting = pending.current;
    if (!waiting || (live.current.paused && !live.current.asleep)) return null;
    pending.current = null;
    return performance.now() - waiting.at < 6000 ? waiting.kind : null;
  };

  const play = useCallback((wanted: PetAction) => {
    const next = forCharacter(wanted, live.current.cat);
    window.clearTimeout(endTimer.current);
    current.current = next;
    setAction(next);
    if (next === 'look') { glance.current = randomGlance(); applyGaze(); }
    if (next === 'stretch') lastStretch.current = performance.now();
    if (next === 'askpet') lastAsk.current = performance.now();
    if (isToyAction(next) || next === 'tailchase') moodTo('play');
    // Each play with the ball has its own strength, read by the toy animations.
    if (isToyAction(next) && ref.current) {
      const { roll, lift } = next === 'mischief' ? { roll: 2.4, lift: 1.2 } : toyPlay();
      ref.current.style.setProperty('--toy-roll', roll.toFixed(2));
      ref.current.style.setProperty('--toy-lift', lift.toFixed(2));
    }
    // Asked for a pet and got none: a little flick of the ear, and it lets it go. Poked
    // awake, it shakes off the fright and grumbles.
    const settle = next === 'slowblink' ? settleAfter.current : null;
    settleAfter.current = null;
    const then = next === 'askpet' ? () => play('ear') : next === 'startle' && mood.current.annoyance > .25 ? () => play('grumpy') : settle ? () => play(settle) : () => {
      const waiting = takePending();
      if (waiting) play(waiting); else stop();
    };
    endTimer.current = window.setTimeout(then, actionDuration[next]);
  }, [applyGaze, moodTo, ref, stop]);

  // Feelings take turns: one that arrives while another plays waits for it to finish, and
  // one that arrives while the cat walks or is carried waits until it settles, so you see
  // both. A fright never waits.
  const show = useCallback((kind: PetAction) => {
    const playing = current.current;
    const moving = live.current.paused && !live.current.asleep;
    if (kind !== 'startle' && (moving || (playing && playing !== 'purr' && reactions.has(playing)))) { pending.current = { kind, at: performance.now() }; return; }
    play(kind);
  }, [play]);
  useEffect(() => {
    if (!alive || paused || current.current) return;
    const waiting = takePending();
    if (waiting) play(waiting);
  }, [alive, paused, play]);

  useEffect(() => () => window.clearTimeout(endTimer.current), []);

  useEffect(() => {
    if (!alive) return;
    let cancelled = false;
    let timer = 0;
    const wait = (ms: number) => new Promise<void>(resolve => { timer = window.setTimeout(resolve, ms); });
    void (async () => {
      while (!cancelled) {
        await wait(nextBlinkDelay(motion));
        if (cancelled || document.hidden || stillPreferred()) continue;
        for (let blinks = Math.random() < .18 ? 2 : 1; blinks > 0 && !cancelled; blinks--) {
          setBlinking(true);
          await wait(130);
          setBlinking(false);
          if (blinks > 1) await wait(170);
        }
      }
    })();
    return () => { cancelled = true; window.clearTimeout(timer); setBlinking(false); };
  }, [alive, motion]);

  const save = useCallback(async () => {
    const cat = known.current;
    if (!remember || !cat) return;
    await command('save_pet_memory', { id: cat.id, mood: mood.current, savedAt: Date.now() });
  }, [remember]);

  // The desktop cat's memory lives in the app's store, so export has it and Delete
  // everything erases it. A mood from an earlier version moves there once.
  useEffect(() => {
    if (!remember) return;
    let cancelled = false;
    const load = async () => {
      const saved = await command<PetMemory>('pet_memory');
      if (cancelled) return;
      let legacy: string | null = null;
      try { legacy = localStorage.getItem(MOOD_KEY); } catch { /* Nothing to move. */ }
      const hour = new Date().getHours();
      if (saved.mood) mood.current = restored(saved.mood, saved.savedAt, live.current.motion, hour, Date.now());
      else if (legacy) mood.current = remembered(legacy, live.current.motion, hour, Date.now());
      setFace(faceFor(mood.current));
      known.current = { id: saved.id, favourite: saved.favourite };
      if (!saved.mood && legacy) await save();
      try { localStorage.removeItem(MOOD_KEY); } catch { /* Already gone. */ }
    };
    // A cat whose memory can't be reached starts fresh and isn't saved.
    const start = () => void load().catch(() => {});
    start();
    const forget = () => { known.current = null; mood.current = startingMood(live.current.motion, new Date().getHours()); setFace(faceFor(mood.current)); start(); };
    forgetters.add(forget);
    return () => { cancelled = true; forgetters.delete(forget); };
  }, [remember, save]);

  // Time passes: feelings drift back to the cat's usual self, and boredom creeps in while
  // you're around but busy with other things.
  useEffect(() => {
    if (!alive) return;
    // Saved every 30 seconds and as the window goes away, not on every tick.
    const keep = () => void save().catch(() => { /* Saved next time. */ });
    let ticks = 0;
    const timer = window.setInterval(() => {
      const { motion: personality, asleep: napping } = live.current;
      mood.current = drift(mood.current, 5, { motion: personality, hour: new Date().getHours(), userAround: performance.now() - lastMove.current < 60_000, asleep: napping });
      setFace(faceFor(mood.current));
      if (++ticks % 6 === 0) keep();
    }, 5000);
    window.addEventListener('pagehide', keep);
    return () => { window.clearInterval(timer); window.removeEventListener('pagehide', keep); keep(); };
  }, [alive, save]);

  // Between habits: eyes that wander when nothing is moving, and ears that swivel to small
  // sounds. Both are CSS variables, so none of it re-renders.
  useEffect(() => {
    if (!alive) return;
    let eyes = 0, ears = 0, back = 0;
    const resting = () => !document.hidden && !stillPreferred() && !current.current && live.current.state === 'idle' && !live.current.paused;
    const wander = () => {
      const { delay, gaze } = nextSaccade();
      eyes = window.setTimeout(() => {
        const now = performance.now();
        if (resting() && now - lastMove.current > 3500 && now - lastTyping.current > 1500) { glance.current = gaze; applyGaze(); }
        wander();
      }, delay);
    };
    const swivel = () => {
      const { delay, side, angle, hold } = nextEarSwivel();
      ears = window.setTimeout(() => {
        const element = ref.current;
        if (element && live.current.cat && resting()) {
          element.style.setProperty(`--ear-${side}`, angle.toFixed(1));
          back = window.setTimeout(() => element.style.setProperty(`--ear-${side}`, '0'), hold);
        }
        swivel();
      }, delay);
    };
    wander();
    swivel();
    return () => { window.clearTimeout(eyes); window.clearTimeout(ears); window.clearTimeout(back); };
  }, [alive, applyGaze, ref]);

  // Asleep, it dreams: an ear flicks, a paw paddles, the tail twitches.
  useEffect(() => {
    if (!alive || !asleep || !cat) return;
    let timer = 0;
    const doze = () => {
      timer = window.setTimeout(() => {
        if (!document.hidden && !stillPreferred() && !current.current) play('dream');
        doze();
      }, nextDreamDelay());
    };
    doze();
    return () => window.clearTimeout(timer);
  }, [alive, asleep, cat, play]);

  useEffect(() => {
    if (!alive) return;
    if (state !== 'idle' || paused) {
      // You're talking to it: that's company.
      if (state !== 'idle') moodTo('talk');
      if (current.current && !reactions.has(current.current)) stop();
      return;
    }
    let cancelled = false;
    let timer = 0;
    let played: PetAction | null = null;
    const schedule = () => {
      timer = window.setTimeout(() => {
        if (cancelled) return;
        played = null;
        if (!document.hidden && !stillPreferred() && !current.current) {
          const now = performance.now();
          played = nextHabit(motion, cat, { stretch: now - lastStretch.current, ask: (now - lastAsk.current) * askPace[bandFor(mood.current.bond)], userAround: now - lastMove.current < 60_000 }, Math.random, faceFor(mood.current));
          play(played);
        }
        schedule();
      }, nextHabitDelay(motion, played, Math.random, moodPace(mood.current)));
    };
    schedule();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [alive, state, paused, motion, cat, play, stop, moodTo]);

  useEffect(() => {
    if (!alive) return;
    const follow = (x: number, y: number) => {
      const element = ref.current;
      if (!element) return;
      const rect = element.getBoundingClientRect();
      const dx = x - (rect.left + rect.width / 2), dy = y - (rect.top + rect.height * .45);
      pointerGaze.current = gazeToward(dx, dy);
      near.current = nearness(dx, dy);
      lastMove.current = performance.now();
      hunter.current.moved(dx, dy, lastMove.current);
      applyGaze();
    };
    if (!desktop) {
      const move = (event: PointerEvent) => follow(event.clientX, event.clientY);
      window.addEventListener('pointermove', move);
      const timer = window.setInterval(applyGaze, 500);
      return () => { window.removeEventListener('pointermove', move); window.clearInterval(timer); };
    }
    // The companion window is tiny, so the desktop app follows the cursor anywhere on screen.
    // Positions stay in this process; nothing is recorded.
    const appWindow = getCurrentWindow();
    let origin = { x: 0, y: 0 };
    let previous = '';
    void appWindow.outerPosition().then(position => { origin = position; }).catch(() => {});
    const unlisten = appWindow.onMoved(({ payload }) => { origin = payload; });
    const timer = window.setInterval(async () => {
      if (document.hidden || stillPreferred()) return applyGaze();
      try {
        const cursor = await cursorPosition();
        const key = `${cursor.x},${cursor.y},${origin.x},${origin.y}`;
        if (key === previous) return applyGaze();
        previous = key;
        const scale = window.devicePixelRatio || 1;
        follow((cursor.x - origin.x) / scale, (cursor.y - origin.y) / scale);
      } catch { /* Cursor tracking is decorative; the pet simply looks ahead. */ }
    }, 150);
    return () => { window.clearInterval(timer); void unlisten.then(off => off()).catch(() => {}); };
  }, [alive, ref, applyGaze]);

  // Hunting: a pointer that darted about nearby and then froze gets stalked and pounced on;
  // one darting about out of reach gets chattered at.
  useEffect(() => {
    if (!alive) return;
    const timer = window.setInterval(() => {
      const { state: now, paused: away, motion: personality, cat: isCat } = live.current;
      if (!isCat || now !== 'idle' || away || current.current || document.hidden || stillPreferred()) return;
      const at = performance.now();
      const side = hunter.current.ready(at, personality);
      if (!side) {
        if (hunter.current.chatters(at, personality)) { play('chatter'); moodTo('chase'); }
        return;
      }
      ref.current?.style.setProperty('--hunt-dir', String(side));
      moodTo('chase');
      play('stalk');
      window.setTimeout(() => { if (current.current === 'stalk' || current.current === null) play('hunt'); }, actionDuration.stalk);
    }, 200);
    return () => window.clearInterval(timer);
  }, [alive, play, moodTo, ref]);

  // Typing in the panel turns its eyes and ears to the words, with the odd ear twitch.
  useEffect(() => {
    if (!alive) return;
    const typed = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.key.length > 1 || stillPreferred()) return;
      const now = performance.now();
      const burst = now - lastTyping.current < 1500;
      lastTyping.current = now;
      applyGaze();
      if (!burst) moodTo('talk');
      if (burst && !current.current && live.current.cat && Math.random() < .04) play('ear');
      window.setTimeout(applyGaze, 1600);
    };
    window.addEventListener('keydown', typed);
    return () => window.removeEventListener('keydown', typed);
  }, [alive, applyGaze, play, moodTo]);

  useEffect(() => {
    const element = ref.current;
    if (!alive || !element) return;
    const pokes = new Pokes();
    const petting = new Petting();
    let inside = false;
    let still = 0;
    let ticker = 0;
    let lastStroke = 0;
    let lastTick = 0;
    let purringSince = 0;
    let lean = 0;
    // Whether the purr may be heard, asked of the app every 10 seconds while purring.
    const hearing = { allowed: false, asked: -Infinity };
    let lastBunt = -Infinity;
    let last: { x: number; y: number } | null = null;
    const context = () => ({ cat: live.current.cat, motion: live.current.motion, annoyance: mood.current.annoyance, favourite: known.current?.favourite ?? null, band: bandFor(mood.current.bond), still: stillPreferred() });
    // Said once each petting, when the purr starts and when kneading starts.
    const said = new Set<PetFeeling>();
    const say = (feeling: PetFeeling) => {
      if (!live.current.remember || said.has(feeling)) return;
      said.add(feeling);
      window.dispatchEvent(new CustomEvent<PetFeeling>('buntini-pet-feeling', { detail: feeling }));
    };
    // A point on the page in the cat's own 180 × 180 drawing, and which side of it.
    const where = (clientX: number, clientY: number) => {
      const rect = (element.querySelector('svg') ?? element).getBoundingClientRect();
      return { x: (clientX - rect.left) / rect.width * 180, y: (clientY - rect.top) / rect.height * 180, side: clientX < rect.left + rect.width / 2 ? -1 : 1 };
    };
    const zoneUnder = (point: { x: number; y: number }): Zone | null => live.current.cat ? zoneAt(point.x, point.y) : 'head';
    // While a purr lasts: its level for the CSS, the purr held until it fades, and a slow
    // blink once the strokes stop after a good one.
    const showPurr = (level: number, now: number) => {
      element.style.setProperty('--purr', level.toFixed(3));
      const purring = level > .02;
      // The breathing animation starts over when the purr starts; taps keep time with it.
      if (purring && !purringSince) purringSince = now;
      if (!purring) purringSince = 0;
      element.toggleAttribute('data-purring', purring);
      element.toggleAttribute('data-purr-high', level > .6);
    };
    // The desktop cat's purr can be felt on a trackpad and heard, quietly.
    const feelAndHear = (level: number, now: number) => {
      if (!live.current.remember) return;
      if (level > 0 && now - hearing.asked > 10_000) {
        hearing.asked = now;
        void command<boolean>('pet_sound_allowed').then(allowed => { hearing.allowed = allowed; }).catch(() => { hearing.allowed = false; });
      }
      purrSound.set(level, hearing.allowed && live.current.state === 'idle', now);
      const taps = purringSince && now - lastStroke < 400 ? tapsDue(purringSince, lastTick, now, level) : 0;
      if (taps) void command('pet_purr_tap', { double: taps === 2 }).catch(() => {});
    };
    const tick = () => {
      const now = performance.now();
      const level = petting.purr(now);
      showPurr(level, now);
      feelAndHear(level, now);
      lastTick = now;
      if (now - lastStroke > 600) { delete element.dataset.touch; delete element.dataset.ruffle; element.removeAttribute('data-favourite'); }
      // Asked only once it's free to, so a knead still playing doesn't use up the blink.
      const free = live.current.cat && live.current.state === 'idle' && !live.current.paused && (current.current === 'purr' || !current.current);
      if (free && petting.afterglow(now)) {
        settleAfter.current = live.current.motion === 'sleepy' ? 'loaf' : 'tailcurl';
        play('slowblink');
      } else if (current.current === 'purr' && level < .12) stop();
      if (level < .12) said.clear();
      if (!level && now - lastStroke > 1200 && purrSound.resting(now)) { window.clearInterval(ticker); ticker = 0; }
    };
    const holdPurr = () => {
      if (current.current === 'purr') return;
      window.clearTimeout(endTimer.current);
      current.current = 'purr';
      setAction('purr');
      say('purring');
    };
    const stroke = (zone: Zone, distance: number, point: { x: number; y: number; side: number }, angle: number) => {
      const now = performance.now();
      const touch = petting.stroke(zone, distance, now, context());
      lastStroke = now;
      if (!ticker) { lastTick = now; ticker = window.setInterval(tick, 100); }
      showPurr(touch.purr, now);
      const still = stillPreferred();
      // Where it's touched shapes the pose (an arched back, a lifted chin), so not with
      // Reduce motion. Its favourite spot makes it blush.
      if (still) delete element.dataset.touch; else element.dataset.touch = zone;
      element.toggleAttribute('data-favourite', zone === known.current?.favourite);
      // It leans its head into the hand, more as the purr builds.
      element.style.setProperty('--tilt', `${still ? 0 : tiltFor(zone, touch.purr, point.side < 0 ? -1 : 1).toFixed(1)}deg`);
      // The fur parts where the hand goes, and smooths back after.
      element.style.setProperty('--ruffle-x', `${point.x.toFixed(1)}px`);
      element.style.setProperty('--ruffle-y', `${point.y.toFixed(1)}px`);
      element.style.setProperty('--ruffle-angle', `${(angle * 180 / Math.PI).toFixed(0)}deg`);
      element.dataset.ruffle = '';
      if (touch.mood) moodTo(touch.mood);
      // The pet it asked for.
      if (current.current === 'askpet' && touch.mood === 'pet') { play('love'); return; }
      if (!touch.play || (pettingReactions.has(current.current) && !cutsIn.has(touch.play))) return;
      if (touch.play === 'knead') say('kneading');
      if (touch.play === 'purr') holdPurr(); else play(touch.play);
    };
    // A hand held still over the cat gets a head rub against it.
    const settle = (event: PointerEvent) => {
      window.clearTimeout(still);
      const { side } = where(event.clientX, event.clientY);
      still = window.setTimeout(() => {
        if (!inside || current.current || !live.current.cat || live.current.state !== 'idle' || live.current.paused || stillPreferred() || performance.now() - lastBunt < 20_000) return;
        lastBunt = performance.now();
        element.style.setProperty('--bunt-dir', String(side));
        play('bunt');
        moodTo('pet');
      }, 1300);
    };
    // Stroking back and forth over the pet (without dragging it) makes it purr.
    const move = (event: PointerEvent) => {
      if (event.buttons) return;
      inside = true;
      settle(event);
      const previous = last;
      last = { x: event.clientX, y: event.clientY };
      if (!previous) return;
      const point = where(event.clientX, event.clientY);
      const zone = zoneUnder(point);
      const dx = event.clientX - previous.x, dy = event.clientY - previous.y;
      if (zone) stroke(zone, Math.hypot(dx, dy), point, Math.atan2(dy, dx));
    };
    // Two fingers stroking across the cat on a trackpad pet it too. A pinch is not a pet.
    const wheel = (event: WheelEvent) => {
      if (event.ctrlKey) return;
      event.preventDefault();
      const point = where(event.clientX, event.clientY);
      const zone = zoneUnder(point);
      const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? 400 : 1;
      if (zone) stroke(zone, Math.hypot(event.deltaX, event.deltaY) * unit, point, Math.atan2(event.deltaY, event.deltaX));
    };
    const leave = () => {
      inside = false;
      window.clearTimeout(still);
      // Mid-purr, it leans after the hand for a moment as it goes.
      const exit = last && where(last.x, last.y);
      if (petting.leave(performance.now()) && exit && live.current.cat && !live.current.paused && !stillPreferred()) {
        element.style.setProperty('--lean-side', String(exit.side));
        element.dataset.lean = '';
        window.clearTimeout(lean);
        lean = window.setTimeout(() => { delete element.dataset.lean; }, 400);
      }
      last = null;
    };
    const click = () => {
      if (stillPreferred()) return;
      const poked = pokes.click(performance.now()) === 'poke';
      // Woken by a click: a jump, and a grumble if it was cross already.
      if (live.current.asleep || current.current === 'loaf') { moodTo('woken'); play('startle'); return; }
      if (poked) { moodTo('poke'); play(faceFor(mood.current) === 'grumpy' && live.current.cat ? 'swat' : 'grumpy'); return; }
      if (current.current === 'purr') return;
      moodTo('click');
      play(faceFor(mood.current) === 'grumpy' ? 'grumpy' : 'love');
    };
    // Audio may start only after a click or key press in Buntini.
    // From the keyboard or VoiceOver, each press is a second of steady head strokes.
    let keyed = 0, keyedUntil = 0;
    const petByKey = () => {
      keyedUntil = performance.now() + KEY_STROKE.lasts;
      if (keyed) return;
      keyed = window.setInterval(() => {
        if (performance.now() > keyedUntil) { window.clearInterval(keyed); keyed = 0; return; }
        stroke('head', KEY_STROKE.distance, { x: 90, y: 72, side: 1 }, 0);
      }, KEY_STROKE.every);
    };
    if (live.current.remember) window.addEventListener('buntini-pet', petByKey);
    const unlock = () => purrSound.unlock();
    if (live.current.remember) { window.addEventListener('pointerdown', unlock, true); window.addEventListener('keydown', unlock, true); }
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('wheel', wheel, { passive: false });
    element.addEventListener('click', click);
    return () => {
      window.clearTimeout(still);
      window.clearTimeout(lean);
      window.clearInterval(ticker);
      if (live.current.remember) purrSound.set(0, false, performance.now());
      window.removeEventListener('pointerdown', unlock, true);
      window.removeEventListener('keydown', unlock, true);
      window.removeEventListener('buntini-pet', petByKey);
      window.clearInterval(keyed);
      element.removeEventListener('pointermove', move);
      element.removeEventListener('pointerleave', leave);
      element.removeEventListener('wheel', wheel);
      element.removeEventListener('click', click);
    };
  }, [alive, ref, play, stop, moodTo]);

  // Waking from a nap: a long stretch, and it's pleased to see you.
  // A cat close to you, back with you after an hour or more, rubs against you too.
  const wasAsleep = useRef(asleep);
  const asleepSince = useRef(0);
  useEffect(() => {
    let timer = 0;
    if (asleep && !wasAsleep.current) asleepSince.current = performance.now();
    if (alive && wasAsleep.current && !asleep && current.current !== 'startle' && !stillPreferred()) {
      const missed = bandFor(mood.current.bond) === 'close' && performance.now() - asleepSince.current >= 3_600_000;
      moodTo('userBack');
      play('stretch');
      if (missed && live.current.cat) timer = window.setTimeout(() => { if (!current.current) play('bunt'); }, actionDuration.stretch + 100);
    }
    wasAsleep.current = asleep;
    return () => window.clearTimeout(timer);
  }, [alive, asleep, play, moodTo]);

  // Picked up and put down: a playful cat wants to go again, a cross one grumbles. The
  // feeling shows once it has landed.
  const wasHeld = useRef(held);
  useEffect(() => {
    let timer = 0;
    if (alive && held && !wasHeld.current) { stop(); moodTo('held'); }
    if (alive && !held && wasHeld.current && !stillPreferred()) {
      const feeling = live.current.motion === 'playful' ? 'excited' : mood.current.annoyance > .45 ? 'grumpy' : 'curious';
      timer = window.setTimeout(() => show(feeling), 250);
    }
    wasHeld.current = held;
    return () => window.clearTimeout(timer);
  }, [alive, held, show, stop, moodTo]);

  // Keyed on the reaction id so the same outcome twice in a row still plays. A job well
  // done makes a lively cat proud, and a calmer one simply happy.
  const reactionId = reaction?.id;
  const reactionKind = reaction?.kind;
  useEffect(() => {
    if (!alive || !reactionKind || stillPreferred()) return;
    const feeling = reactionFeeling[reactionKind];
    if (feeling) moodTo(feeling);
    show(reactionKind === 'happy' ? proudOrHappy(mood.current) : reactionKind);
  }, [alive, reactionId, reactionKind, show, moodTo]);

  return { action, blinking, face };
}
