import { useId, useRef, type CSSProperties } from 'react';
import Cat from './Cat';
import { usePetLife, type PetReaction } from './usePetLife';
import type { PetLook, PetState, Settings } from './types';

const defaultLook: PetLook = { coat: 'ginger', eyes: 'green', accessory: 'bell', motion: 'playful', accent: '#e9985d' };

export default function Pet({ state, small = false, avatar = 'cat', look = defaultLook, alive = false, reaction, moving = null, facing = -1, landing = false, asleep = false, held = false, remember = false, speed = 90 }: { state: PetState; small?: boolean; avatar?: Settings['avatar']; look?: PetLook; alive?: boolean; reaction?: PetReaction; moving?: 'walk' | 'jump' | 'fall' | null; facing?: -1 | 1; landing?: boolean; asleep?: boolean; held?: boolean; remember?: boolean; speed?: number }) {
  const gradientId = useId().replaceAll(':', '');
  const element = useRef<HTMLDivElement>(null);
  // Habits wait while the pet is travelling, napping or being carried.
  const { action, blinking, face } = usePetLife(element, { alive, state, paused: !!moving || asleep || held, motion: look.motion, cat: avatar === 'cat', reaction, asleep, held, remember });

  const classes = ['pet', `pet--${state}`, `pet--motion-${look.motion}`, avatar === 'cat' && 'pet--cat', alive && 'pet--alive', action && !moving && `pet--act-${action}`, blinking && 'pet--blink', small && 'pet--small', moving && `pet--${moving}ing`, moving === 'walk' && speed > 200 && 'pet--dashing', facing > 0 && 'pet--facing-right', landing && 'pet--landing', asleep && !moving && 'pet--asleep', held && 'pet--held', face !== 'neutral' && `pet--face-${face}`];
  // One full gait cycle covers about 36 points, so the paws keep pace with the window.
  const stride = `${Math.min(.7, Math.max(.26, 36 / speed)).toFixed(2)}s`;
  return <div ref={element} className={classes.filter(Boolean).join(' ')} style={{ '--stride': stride } as CSSProperties} aria-hidden="true">
    <div className="pet-aura" />
    {avatar === 'orb' ? <Orb state={state} id={gradientId} /> : <Cat id={gradientId} look={look} pose={moving === 'walk' ? 'walk' : moving === 'jump' ? 'leap' : 'sit'} />}
    <span className="pet-orbit orbit-one" /><span className="pet-orbit orbit-two" />
  </div>;
}


// The ring goes behind the orb and comes back round in front of it, below the face.
function Orb({ state, id }: { state: PetState; id: string }) {
  const ring = <ellipse cx="90" cy="93" rx="67" ry="21" transform="rotate(-28 90 93)" />;
  return <svg viewBox="0 0 180 180" fill="none">
    <defs>
      <radialGradient id={`${id}-orb`} cx=".35" cy=".25" r=".8"><stop stopColor="#FFF4D5" /><stop offset=".45" stopColor="#E6AF6D" /><stop offset="1" stopColor="#8D573B" /></radialGradient>
      <radialGradient id={`${id}-orb-bounce`} cx=".72" cy=".92" r=".48"><stop stopColor="#FFD9A8" stopOpacity=".5" /><stop offset="1" stopColor="#FFD9A8" stopOpacity="0" /></radialGradient>
      <radialGradient id={`${id}-orb-shadow`}><stop stopColor="#1C1514" stopOpacity=".3" /><stop offset=".55" stopColor="#1C1514" stopOpacity=".15" /><stop offset="1" stopColor="#1C1514" stopOpacity="0" /></radialGradient>
      <radialGradient id={`${id}-orb-blush`}><stop stopColor="#FF8DA1" stopOpacity=".5" /><stop offset="1" stopColor="#FF8DA1" stopOpacity="0" /></radialGradient>
      <radialGradient id={`${id}-moon`} cx=".35" cy=".3" r=".7"><stop stopColor="#FFF4D5" /><stop offset="1" stopColor="#E0A86A" /></radialGradient>
      <clipPath id={`${id}-ring-front`}><rect x="-110" y="93" width="400" height="200" transform="rotate(-28 90 93)" /></clipPath>
    </defs>
    <ellipse className="pet-shadow" cx="90" cy="162" rx="36" ry="5.5" fill={`url(#${id}-orb-shadow)`} />
    <g className="pet-body">
      <g stroke="#B98A5E" strokeWidth="2">{ring}</g>
      <circle cx="90" cy="93" r="53" fill={`url(#${id}-orb)`} stroke="#F6D2A2" strokeWidth="1.5" />
      <circle cx="90" cy="93" r="53" fill={`url(#${id}-orb-bounce)`} />
      <path d="M56 70C61 58 74 51 88 52" stroke="#FFF8E2" strokeWidth="5" strokeLinecap="round" opacity=".65" />
      <ellipse cx="65" cy="103" rx="7.5" ry="4.4" fill={`url(#${id}-orb-blush)`} /><ellipse cx="115" cy="103" rx="7.5" ry="4.4" fill={`url(#${id}-orb-blush)`} />
      <g className="pet-eyes"><rect x="71" y="86" width="6" height="12" rx="3" fill="#563A2F" /><rect x="103" y="86" width="6" height="12" rx="3" fill="#563A2F" /><circle cx="75" cy="89" r="1.2" fill="#fff" fillOpacity=".85" /><circle cx="107" cy="89" r="1.2" fill="#fff" fillOpacity=".85" /></g>
      <path className="pet-mouth" d="M84 109Q90 114 96 109" stroke="#563A2F" strokeWidth="2.5" strokeLinecap="round" />
      <ellipse className="pet-speaking-mouth" cx="90" cy="111" rx="4" ry="5" fill="#563A2F" />
      <g clipPath={`url(#${id}-ring-front)`} stroke="#D4A576" strokeWidth="2">{ring}</g>
      <circle cx="141" cy="42" r={state === 'working' ? 5 : 3} fill={`url(#${id}-moon)`} />
    </g>
  </svg>;
}
