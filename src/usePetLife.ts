import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import { desktop } from './bridge';
import { actionDuration, gazeToward, Hunter, isToyAction, nextBlinkDelay, nextHabitDelay, pickIdleAction, randomGlance, toyPlay, type PetAction } from './petLife';
import type { PetMotion, PetState } from './types';

export interface PetReaction { kind: PetAction; id: number }

const reactions = new Set<PetAction>(['love', 'purr', 'happy', 'sad']);
const stillPreferred = () => document.documentElement.dataset.motion === 'reduced' || window.matchMedia('(prefers-reduced-motion: reduce)').matches;

/* Gives a pet its small signs of life: randomized blinks, idle habits picked by its
   motion personality, eyes that follow the cursor, petting, clicks and task reactions.
   Gaze is written straight to CSS variables so cursor tracking never re-renders. */
export function usePetLife(ref: RefObject<HTMLDivElement | null>, { alive, state, motion, cat, reaction }: { alive: boolean; state: PetState; motion: PetMotion; cat: boolean; reaction?: PetReaction }) {
  const [action, setAction] = useState<PetAction | null>(null);
  const [blinking, setBlinking] = useState(false);
  const current = useRef<PetAction | null>(null);
  const endTimer = useRef(0);
  const pointerGaze = useRef({ x: 0, y: 0 });
  const lastMove = useRef(0);
  const glance = useRef<{ x: number; y: number } | null>(null);
  const hunter = useRef(new Hunter());
  const lastTyping = useRef(0);
  const live = useRef({ state, motion, cat });
  live.current = { state, motion, cat };

  const applyGaze = useCallback(() => {
    const element = ref.current;
    if (!element) return;
    // While you type in the panel (up and to its left), the cat watches the words appear.
    const now = performance.now();
    const gaze = stillPreferred() ? { x: 0, y: 0 } : now - lastTyping.current < 1500 ? { x: -.9, y: -.75 } : now - lastMove.current < 3500 ? pointerGaze.current : glance.current ?? { x: 0, y: 0 };
    element.style.setProperty('--gx', gaze.x.toFixed(3));
    element.style.setProperty('--gy', gaze.y.toFixed(3));
  }, [ref]);

  const stop = useCallback(() => {
    window.clearTimeout(endTimer.current);
    current.current = null;
    glance.current = null;
    setAction(null);
    applyGaze();
  }, [applyGaze]);

  const play = useCallback((next: PetAction) => {
    window.clearTimeout(endTimer.current);
    current.current = next;
    setAction(next);
    if (next === 'look') { glance.current = randomGlance(); applyGaze(); }
    // Each play with the ball has its own strength, read by the toy animations.
    if (isToyAction(next) && ref.current) {
      const { roll, lift } = next === 'mischief' ? { roll: 2.4, lift: 1.2 } : toyPlay();
      ref.current.style.setProperty('--toy-roll', roll.toFixed(2));
      ref.current.style.setProperty('--toy-lift', lift.toFixed(2));
    }
    endTimer.current = window.setTimeout(stop, actionDuration[next]);
  }, [applyGaze, ref, stop]);

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

  useEffect(() => {
    if (!alive) return;
    if (state !== 'idle') {
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
          played = pickIdleAction(motion, cat);
          play(played);
        }
        schedule();
      }, nextHabitDelay(motion, played));
    };
    schedule();
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [alive, state, motion, cat, play, stop]);

  useEffect(() => {
    if (!alive) return;
    const follow = (x: number, y: number) => {
      const element = ref.current;
      if (!element) return;
      const rect = element.getBoundingClientRect();
      const dx = x - (rect.left + rect.width / 2), dy = y - (rect.top + rect.height * .45);
      pointerGaze.current = gazeToward(dx, dy);
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

  // Hunting: a pointer that darted about nearby and then froze gets stalked and pounced on.
  useEffect(() => {
    if (!alive) return;
    const timer = window.setInterval(() => {
      const { state: now, motion: mood, cat: isCat } = live.current;
      if (!isCat || now !== 'idle' || current.current || document.hidden || stillPreferred()) return;
      const side = hunter.current.ready(performance.now(), mood);
      if (!side) return;
      ref.current?.style.setProperty('--hunt-dir', String(side));
      play('stalk');
      window.setTimeout(() => { if (current.current === 'stalk' || current.current === null) play('hunt'); }, actionDuration.stalk);
    }, 200);
    return () => window.clearInterval(timer);
  }, [alive, play, ref]);

  // Typing in the panel turns its eyes and ears to the words, with the odd ear twitch.
  useEffect(() => {
    if (!alive) return;
    const typed = (event: KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.key.length > 1 || stillPreferred()) return;
      const now = performance.now();
      const burst = now - lastTyping.current < 1500;
      lastTyping.current = now;
      applyGaze();
      if (burst && !current.current && live.current.cat && Math.random() < .04) play('ear');
      window.setTimeout(applyGaze, 1600);
    };
    window.addEventListener('keydown', typed);
    return () => window.removeEventListener('keydown', typed);
  }, [alive, applyGaze, play]);

  useEffect(() => {
    const element = ref.current;
    if (!alive || !element) return;
    let travel = 0;
    let windowStart = 0;
    let purringSince = 0;
    let last: { x: number; y: number } | null = null;
    // Stroking back and forth over the pet (without dragging it) makes it purr.
    const move = (event: PointerEvent) => {
      if (event.buttons || stillPreferred()) return;
      const now = performance.now();
      if (now - windowStart > 1400) { windowStart = now; travel = 0; }
      if (last) travel += Math.hypot(event.clientX - last.x, event.clientY - last.y);
      last = { x: event.clientX, y: event.clientY };
      // A few seconds of steady petting and it starts making biscuits.
      if (current.current === 'knead') return;
      if (current.current === 'purr' || travel > 140) {
        if (!purringSince) purringSince = now;
        if (now - purringSince > 3000 && live.current.cat) { purringSince = 0; play('knead'); return; }
        play('purr');
      }
    };
    const leave = () => { last = null; travel = 0; purringSince = 0; };
    const click = () => { if (!stillPreferred() && current.current !== 'purr') play('love'); };
    element.addEventListener('pointermove', move);
    element.addEventListener('pointerleave', leave);
    element.addEventListener('click', click);
    return () => { element.removeEventListener('pointermove', move); element.removeEventListener('pointerleave', leave); element.removeEventListener('click', click); };
  }, [alive, ref, play]);

  // Keyed on the reaction id so the same outcome twice in a row still plays.
  const reactionId = reaction?.id;
  const reactionKind = reaction?.kind;
  useEffect(() => {
    if (alive && reactionKind && !stillPreferred()) play(reactionKind);
  }, [alive, reactionId, reactionKind, play]);

  return { action, blinking };
}
