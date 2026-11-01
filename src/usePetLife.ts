import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import { desktop } from './bridge';
import { actionDuration, ASK_GAP, forCharacter, gazeToward, Hunter, isToyAction, nearness, nextBlinkDelay, nextDreamDelay, nextEarSwivel, nextHabit, nextHabitDelay, nextSaccade, randomGlance, toyPlay, type PetAction } from './petLife';
import { drift, faceFor, feel, memory, moodPace, Pokes, proudOrHappy, remembered, startingMood, type Face, type Mood, type MoodEvent } from './petMood';
import type { PetMotion, PetState } from './types';

export interface PetReaction { kind: PetAction; id: number }

// Reactions play through a change of state (a reply starting, say); habits don't.
const reactions = new Set<PetAction>(['love', 'purr', 'happy', 'sad', 'proud', 'bashful', 'curious', 'excited', 'startle', 'grumpy', 'wave', 'swat']);
// What a reaction says happened, for the mood.
const reactionFeeling: Partial<Record<PetAction, MoodEvent>> = { happy: 'done', proud: 'done', sad: 'failed', bashful: 'praise', love: 'praise', excited: 'userBack', wave: 'userBack' };
const MOOD_KEY = 'buntini-cat-mood';
const forgetters = new Set<() => void>();

/** Delete everything: the cat forgets its bond and mood, both the saved copy and the one
    it's feeling now. */
export function forgetMood() {
  try { localStorage.removeItem(MOOD_KEY); } catch { /* Nothing was saved. */ }
  for (const forget of forgetters) forget();
}
const stillPreferred = () => document.documentElement.dataset.motion === 'reduced' || window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* Gives a pet its small signs of life: randomized blinks, idle habits picked by its
   motion personality and its mood, eyes that follow the cursor, petting, clicks and task
   reactions. Gaze is written straight to CSS variables so cursor tracking never re-renders;
   the mood re-renders only when the face it shows changes. */
/** `paused`: travelling, napping or being carried, when habits wait. `remember`: the
    desktop cat, whose mood and bond carry over to the next launch. */
export function usePetLife(ref: RefObject<HTMLDivElement | null>, { alive, state, paused = false, motion, cat, reaction, asleep = false, held = false, remember = false }: { alive: boolean; state: PetState; paused?: boolean; motion: PetMotion; cat: boolean; reaction?: PetReaction; asleep?: boolean; held?: boolean; remember?: boolean }) {
  const [initialMood] = useState<Mood>(() => {
    const hour = new Date().getHours();
    if (!remember) return startingMood(motion, hour);
    try { return remembered(localStorage.getItem(MOOD_KEY), motion, hour, Date.now()); } catch { return startingMood(motion, hour); }
  });
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
  const live = useRef({ state, paused, motion, cat, asleep });
  live.current = { state, paused, motion, cat, asleep };

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
    const then = next === 'askpet' ? () => play('ear') : next === 'startle' && mood.current.annoyance > .25 ? () => play('grumpy') : () => {
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

  // Time passes: feelings drift back to the cat's usual self, and boredom creeps in while
  // you're around but busy with other things.
  useEffect(() => {
    if (!alive) return;
    // Saved every 30 seconds and as the window goes away, not on every tick.
    const save = () => { if (remember) try { localStorage.setItem(MOOD_KEY, memory(mood.current, Date.now())); } catch { /* A mood that isn't saved starts fresh next time. */ } };
    let ticks = 0;
    const timer = window.setInterval(() => {
      const { motion: personality, asleep: napping } = live.current;
      mood.current = drift(mood.current, 5, { motion: personality, hour: new Date().getHours(), userAround: performance.now() - lastMove.current < 60_000, asleep: napping });
      setFace(faceFor(mood.current));
      if (++ticks % 6 === 0) save();
    }, 5000);
    window.addEventListener('pagehide', save);
    return () => { window.clearInterval(timer); window.removeEventListener('pagehide', save); save(); };
  }, [alive, remember]);

  useEffect(() => {
    if (!remember) return;
    const forget = () => { mood.current = startingMood(live.current.motion, new Date().getHours()); setFace(faceFor(mood.current)); };
    forgetters.add(forget);
    return () => { forgetters.delete(forget); };
  }, [remember]);

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
          played = nextHabit(motion, cat, { stretch: now - lastStretch.current, ask: now - lastAsk.current, userAround: now - lastMove.current < 60_000 }, Math.random, faceFor(mood.current));
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
    let travel = 0;
    let windowStart = 0;
    let purringSince = 0;
    let pettingSince = 0;
    let lastFelt = 0;
    let swatted = false;
    let inside = false;
    let still = 0;
    let lastBunt = -Infinity;
    let last: { x: number; y: number } | null = null;
    // A hand held still over the cat gets a head rub against it.
    const settle = (event: PointerEvent) => {
      window.clearTimeout(still);
      const rect = element.getBoundingClientRect();
      const side = event.clientX < rect.left + rect.width / 2 ? -1 : 1;
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
      if (event.buttons || stillPreferred()) return;
      inside = true;
      settle(event);
      const now = performance.now();
      if (now - windowStart > 1400) { windowStart = now; travel = 0; }
      if (last) travel += Math.hypot(event.clientX - last.x, event.clientY - last.y);
      last = { x: event.clientX, y: event.clientY };
      // A few seconds of steady petting and it starts making biscuits.
      if (current.current === 'knead' || current.current === 'swat' || current.current === 'bunt') return;
      // The pet it asked for.
      if (current.current === 'askpet') { play('love'); moodTo('pet'); return; }
      if (current.current === 'purr' || travel > 140) {
        if (!purringSince) purringSince = now;
        if (!pettingSince) pettingSince = now;
        if (now - lastFelt > 1000) { lastFelt = now; moodTo('pet'); }
        // Too much of a good thing: a playful (or cross) cat swats the hand, once a session.
        if (!swatted && live.current.cat && now - pettingSince > 12_000 && (live.current.motion === 'playful' || mood.current.annoyance > .3)) {
          swatted = true; purringSince = 0;
          play('swat'); moodTo('overpet');
          return;
        }
        if (now - purringSince > 3000 && live.current.cat) { purringSince = 0; play('knead'); return; }
        play('purr');
      }
    };
    const leave = () => { inside = false; window.clearTimeout(still); last = null; travel = 0; purringSince = 0; pettingSince = 0; swatted = false; };
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
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('click', click);
    return () => { window.clearTimeout(still); element.removeEventListener('pointermove', move); element.removeEventListener('pointerleave', leave); element.removeEventListener('click', click); };
  }, [alive, ref, play, moodTo]);

  // Waking from a nap: a long stretch, and it's pleased to see you.
  const wasAsleep = useRef(asleep);
  useEffect(() => {
    if (alive && wasAsleep.current && !asleep && current.current !== 'startle' && !stillPreferred()) { moodTo('userBack'); play('stretch'); }
    wasAsleep.current = asleep;
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
