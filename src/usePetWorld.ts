import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { cursorPosition, getCurrentWindow } from '@tauri-apps/api/window';
import { command } from './bridge';
import { chooseMove, landing, nextRoamDelay, NudgeRules, speeds, type Move, type Nudge, type NudgeAction, type World } from './petWorld';
import type { PetAction } from './petLife';
import type { PetMotion, PetState } from './types';

type Moving = Move['kind'] | null;
const reactions: Partial<Record<string, PetAction>> = { welcome: 'happy', break: 'stretch', 'focus-done': 'happy', late: 'yawn', battery: 'sad', cpu: 'tilt' };

/* Lets the pet live on the desktop: wander along the floor and the user's active window,
   land when dropped, nap while the user is away, vanish during meetings and full screen,
   and offer the occasional helpful nudge. Signals come from world.rs and never leave
   this window. */
export function usePetWorld({ enabled, roam, aware, nudges, motion, scale, expanded, petState, react, onAction }: {
  enabled: boolean; roam: boolean; aware: boolean; nudges: boolean; motion: PetMotion; scale: number; expanded: boolean; petState: PetState;
  react: (kind: PetAction) => void; onAction: (action: NudgeAction) => void;
}) {
  const [moving, setMoving] = useState<Moving>(null);
  const [facing, setFacing] = useState<-1 | 1>(-1);
  const [landingNow, setLanding] = useState(false);
  const [asleep, setAsleep] = useState(false);
  const [quiet, setQuiet] = useState(false);
  const [nudge, setNudge] = useState<Nudge | null>(null);
  const world = useRef<World | null>(null);
  const movingRef = useRef(false);
  const perched = useRef(false);
  const pendingPerch = useRef<number | undefined>(undefined);
  const rules = useRef(new NudgeRules());
  const live = useRef({ roam, aware, nudges, motion, scale, expanded, petState, asleep, quiet, nudge, react, onAction });
  live.current = { roam, aware, nudges, motion, scale, expanded, petState, asleep, quiet, nudge, react, onAction };

  // Our own position updates can still arrive just after a move ends; they are not drags.
  const echoUntil = useRef(0);
  const fresh = useCallback(async () => {
    const sampled = await command<World | null>('pet_sample').catch(() => null);
    if (sampled) world.current = sampled;
    return sampled ?? world.current;
  }, []);
  const go = useCallback(async (move: Move | null) => {
    if (!move || movingRef.current) return;
    movingRef.current = true;
    perched.current = false;
    pendingPerch.current = move.perch;
    // Turn and get up first, then travel: the pose change plays before the window moves.
    const from = world.current?.pet.x;
    if (from !== undefined && Math.abs(move.x - from) > 1) setFacing(move.x > from ? 1 : -1);
    if (move.kind !== 'fall') {
      setMoving(move.kind);
      await new Promise(resolve => window.setTimeout(resolve, 220));
    }
    if (!movingRef.current || live.current.expanded) { setMoving(null); movingRef.current = false; return; }
    try { await command('pet_move', { x: move.x, y: move.y, kind: move.kind, speed: speeds[live.current.motion] }); }
    catch { movingRef.current = false; setMoving(null); }
  }, []);
  const land = useCallback(async () => {
    if (!live.current.roam || live.current.expanded || movingRef.current) return;
    const current = await fresh();
    if (current) void go(landing(current, live.current.scale));
  }, [go, fresh]);

  useEffect(() => {
    if (!enabled) return;
    void command('pet_world', { roam, aware }).catch(() => {});
  }, [enabled, roam, aware]);
  useEffect(() => () => { if (enabled) void command('pet_world', { roam: false, aware: false }).catch(() => {}); }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    const subscriptions = [
      listen<World>('pet-world', event => {
        const next = event.payload;
        world.current = next;
        const state = live.current;
        setAsleep(state.aware && (next.idleSeconds ?? 0) >= 180 && state.petState === 'idle' && !state.expanded);
        setQuiet(state.aware && !state.expanded && (next.meeting || next.fullscreen));
        if (state.nudges && !state.expanded && !state.nudge && state.petState === 'idle') {
          const shown = rules.current.update(next, Date.now(), new Date().getHours());
          if (shown && !next.meeting && !next.fullscreen) {
            setNudge(shown);
            const reaction = reactions[shown.kind];
            if (reaction) state.react(reaction);
          }
        }
      }),
      listen<{ phase: 'start' | 'end' | 'cancelled'; kind: Move['kind']; direction: number }>('pet-motion', event => {
        const { phase, kind, direction } = event.payload;
        if (phase === 'start') {
          setMoving(kind);
          if (direction) setFacing(direction > 0 ? 1 : -1);
          return;
        }
        movingRef.current = false;
        echoUntil.current = Date.now() + 800;
        setMoving(null);
        if (phase === 'end') {
          if (kind !== 'walk') { setLanding(true); window.setTimeout(() => setLanding(false), 380); }
          const number = pendingPerch.current;
          if (number !== undefined) void command('pet_perch', { number }).then(() => { perched.current = true; }).catch(() => void land());
        }
        pendingPerch.current = undefined;
      }),
      listen('pet-perch-lost', () => { perched.current = false; void land(); }),
    ];
    // A move we did not make (a drag, or the window settling after collapsing) ends in a landing.
    let settle = 0;
    const moved = getCurrentWindow().onMoved(() => {
      if (movingRef.current || perched.current || live.current.expanded || Date.now() < echoUntil.current) return;
      window.clearTimeout(settle);
      settle = window.setTimeout(() => void land(), 900);
    });
    return () => { window.clearTimeout(settle); for (const subscription of [...subscriptions, moved]) void subscription.then(off => off()); };
  }, [enabled, land]);

  // Roaming loop: pause while open, busy, asleep, quiet, talking or already moving.
  useEffect(() => {
    if (!enabled || !roam) return;
    let timer = 0;
    const tick = async () => {
      const state = live.current;
      if (world.current && !state.expanded && !state.quiet && !state.asleep && !state.nudge && state.petState === 'idle' && !movingRef.current && !document.hidden) {
        const current = await fresh();
        let cursorX: number | undefined;
        if (state.motion === 'playful') cursorX = await cursorPosition().then(point => point.x / (window.devicePixelRatio || 1)).catch(() => undefined);
        if (current && !movingRef.current) void go(chooseMove(current, state.scale, state.motion, Math.random, cursorX));
      }
      timer = window.setTimeout(tick, nextRoamDelay(live.current.motion));
    };
    timer = window.setTimeout(tick, 2500);
    return () => window.clearTimeout(timer);
  }, [enabled, roam, go, fresh]);

  useEffect(() => {
    if (!enabled) return;
    if (expanded) { movingRef.current = false; perched.current = false; void command('pet_stop').catch(() => {}); setNudge(null); }
  }, [enabled, expanded]);

  // Invisible and click-through while the user presents, meets or works full screen.
  useEffect(() => {
    if (!enabled) return;
    void getCurrentWindow().setIgnoreCursorEvents(quiet).catch(() => {});
    if (quiet) setNudge(null);
  }, [enabled, quiet]);

  useEffect(() => {
    if (!nudge) return;
    const timer = window.setTimeout(() => setNudge(current => current?.id === nudge.id ? null : current), nudge.ttl);
    return () => window.clearTimeout(timer);
  }, [nudge]);

  const act = useCallback((action: NudgeAction) => {
    const now = Date.now();
    setNudge(null);
    if (action === 'snooze') rules.current.snoozeBreak(now);
    else if (action === 'remindLate') rules.current.remindLate(now);
    else if (action === 'focus') { rules.current.startFocus(now); live.current.react('happy'); }
    else if (action !== 'dismiss') live.current.onAction(action);
  }, []);
  const beforeDrag = useCallback(() => {
    movingRef.current = false;
    perched.current = false;
    void command('pet_stop').catch(() => {});
  }, []);

  return { moving, facing, speed: speeds[motion], landing: landingNow, asleep, quiet, nudge, act, beforeDrag };
}
