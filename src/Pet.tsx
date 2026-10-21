import { useId, useRef } from 'react';
import Cat from './Cat';
import { usePetLife, type PetReaction } from './usePetLife';
import type { PetLook, PetState, Settings } from './types';

const defaultLook: PetLook = { coat: 'ginger', eyes: 'green', accessory: 'bell', motion: 'playful', accent: 'ember' };

const sideTails = [
  'M83 141C57 150 24 149 10 130C6 125 6 119 8 114C12 125 23 129 34 126L28 121C51 119 71 128 83 141Z',
  'M80 135C50 132 19 114 8 88C5 82 5 77 7 72C14 84 25 91 36 90L30 83C55 90 72 111 80 135Z',
  'M83 130C56 113 29 86 21 55C19 47 19 41 22 36C25 49 33 60 44 64L41 55C62 71 78 102 83 130Z',
  'M87 128C68 103 49 66 47 30C46 22 48 16 52 12C50 28 58 40 69 47L69 37C82 63 89 101 87 128Z',
];

const tailTips = [
  'M10 130C6 125 6 119 8 114C12 125 23 129 34 126L28 121L40 123C30 134 20 137 10 130Z',
  'M8 88C5 82 5 77 7 72C14 84 25 91 36 90L30 83L42 90C34 101 20 101 8 88Z',
  'M21 55C19 47 19 41 22 36C25 49 33 60 44 64L41 55L50 65C46 76 31 75 21 55Z',
  'M47 30C46 22 48 16 52 12C50 28 58 40 69 47L69 37L74 49C64 57 51 51 47 30Z',
];

export default function Pet({ state, small = false, avatar = 'cat', look = defaultLook, alive = false, reaction }: { state: PetState; small?: boolean; avatar?: Settings['avatar']; look?: PetLook; alive?: boolean; reaction?: PetReaction }) {
  const gradientId = useId().replaceAll(':', '');
  const element = useRef<HTMLDivElement>(null);
  const { action, blinking } = usePetLife(element, { alive, state, motion: look.motion, cat: avatar === 'cat', reaction });
  const curious = state === 'listening' || state === 'approval';
  const focused = state === 'thinking' || state === 'working';

  const classes = ['pet', `pet--${state}`, `pet--motion-${look.motion}`, avatar === 'cat' && 'pet--cat', alive && 'pet--alive', action && `pet--act-${action}`, blinking && 'pet--blink', small && 'pet--small'];
  return <div ref={element} className={classes.filter(Boolean).join(' ')} aria-hidden="true">
    <div className="pet-aura" />
    {avatar === 'orb' ? <Orb state={state} id={gradientId} /> : avatar === 'cat' ? <Cat id={gradientId} look={look} /> : <svg viewBox="0 0 180 180" fill="none" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id={`${gradientId}-tail`} x1="90" y1="22" x2="90" y2="147" gradientUnits="userSpaceOnUse">
          <stop stopColor="#FFA83F" /><stop offset=".54" stopColor="#ED8127" /><stop offset="1" stopColor="#B74C20" />
        </linearGradient>
        <linearGradient id={`${gradientId}-fur`} x1="73" y1="70" x2="108" y2="151" gradientUnits="userSpaceOnUse">
          <stop stopColor="#FFB44D" /><stop offset=".5" stopColor="#F38A2F" /><stop offset="1" stopColor="#D36322" />
        </linearGradient>
        <linearGradient id={`${gradientId}-cream`} x1="90" y1="103" x2="90" y2="151" gradientUnits="userSpaceOnUse">
          <stop stopColor="#FFE5AE" /><stop offset="1" stopColor="#F4BC72" />
        </linearGradient>
        <linearGradient id={`${gradientId}-ear`} x1="60" y1="55" x2="65" y2="85" gradientUnits="userSpaceOnUse">
          <stop stopColor="#32231F" /><stop offset="1" stopColor="#663824" />
        </linearGradient>
      </defs>

      <ellipse className="pet-shadow" cx="90" cy="165" rx="35" ry="5" fill="#241811" fillOpacity=".2" />
      <g className="pet-body">
        {avatar === 'fox' && <g><path d="M108 148C155 153 176 122 158 91C149 75 139 74 135 62C129 80 132 94 139 105C149 122 133 135 108 134Z" fill={`url(#${gradientId}-tail)`} stroke="#863C20" strokeWidth="2" /><path d="M135 62C129 80 132 94 139 105L147 110L151 100L157 99C149 80 140 77 135 62Z" fill="#FFE2AD" /></g>}
        {avatar === 'kyuubi' && <g>
        <g stroke="#863C20" strokeWidth="1.5" strokeLinejoin="round">
          {sideTails.map((path, index) => <path key={`left-${index}`} d={path} fill={`url(#${gradientId}-tail)`} />)}
          <g transform="translate(180 0) scale(-1 1)">
            {sideTails.map((path, index) => <path key={`right-${index}`} d={path} fill={`url(#${gradientId}-tail)`} />)}
          </g>
          <path d="M90 128C79 101 77 57 86 22C87 16 89 11 92 7C97 17 98 27 96 38L102 31C109 65 102 102 90 128Z" fill={`url(#${gradientId}-tail)`} />
        </g>
        <g fill="#553021">
          {tailTips.map((path, index) => <path key={`left-tip-${index}`} d={path} />)}
          <g transform="translate(180 0) scale(-1 1)">
            {tailTips.map((path, index) => <path key={`right-tip-${index}`} d={path} />)}
          </g>
          <path d="M86 22C87 16 89 11 92 7C97 17 98 27 96 38L102 31L103 43C94 48 86 39 86 22Z" />
        </g>
        <g stroke="#FFC16A" strokeOpacity=".65" strokeWidth="2" strokeLinecap="round">
          <path d="M21 137C39 143 59 140 72 138M16 100C29 114 48 124 66 126M31 69C42 89 56 105 72 118M56 42C61 67 70 88 81 108M92 49C91 69 92 84 91 98" />
          <path d="M159 137C141 143 121 140 108 138M164 100C151 114 132 124 114 126M149 69C138 89 124 105 108 118M124 42C119 67 110 88 99 108" />
        </g>

        </g>}
        {/* A small seated body keeps the broad tail fan light and legible. */}
        <path d="M71 116C63 126 61 141 66 153C71 163 109 163 114 153C119 141 117 126 109 116Z" fill={`url(#${gradientId}-fur)`} stroke="#873D23" strokeWidth="1.8" />
        <path d="M76 123L90 132L104 123L101 138L106 135L101 148L90 155L79 148L74 135L79 138Z" fill={`url(#${gradientId}-cream)`} />
        <path d="M69 140C65 146 65 153 70 155L80 155C84 153 84 148 80 146M111 140C115 146 115 153 110 155L100 155C96 153 96 148 100 146" fill="#EA822B" stroke="#873D23" strokeWidth="1.6" strokeLinecap="round" />
        <path d="M69 153L70 157H80L81 153M99 153L100 157H110L111 153" fill="#4C2C21" />

        {/* Pointed ears, cheek tufts and a pale snout give a distinct fox profile. */}
        <path d="M58 88C53 78 49 63 51 51C63 55 74 64 79 75M101 75C106 64 117 55 129 51C131 63 127 78 122 88" fill={`url(#${gradientId}-fur)`} stroke="#873D23" strokeWidth="1.8" strokeLinejoin="round" />
        <path d="M58 77C54 67 53 60 54 56C62 61 68 66 72 73L64 72L65 80ZM122 77C126 67 127 60 126 56C118 61 112 66 108 73L116 72L115 80Z" fill={`url(#${gradientId}-ear)`} />
        <path d="M62 79C71 71 80 69 90 73C100 69 109 71 118 79L129 92L124 94L132 103L122 103C118 119 106 130 90 130C74 130 62 119 58 103L48 103L56 94L51 92Z" fill={`url(#${gradientId}-fur)`} stroke="#873D23" strokeWidth="1.8" strokeLinejoin="round" />
        <path d="M90 75L94 87L90 94L86 87Z" fill="#673621" />
        <path d="M61 104C68 100 76 102 84 109L90 115L96 109C104 102 112 100 119 104C116 118 103 126 90 126C77 126 64 118 61 104Z" fill={`url(#${gradientId}-cream)`} />
        <g className="pet-eyes">
          <path d="M64 94C69 88 76 88 82 95C76 101 69 100 64 94ZM98 95C104 88 111 88 116 94C111 100 104 101 98 95Z" fill="#FFE2A0" stroke="#3A2820" strokeWidth="2" strokeLinejoin="round" />
          <ellipse cx="74" cy="94" rx="2.6" ry="4.1" fill="#AD4B21" /><ellipse cx="106" cy="94" rx="2.6" ry="4.1" fill="#AD4B21" />
          <path d="M74 91V97M106 91V97" stroke="#30231D" strokeWidth="1.5" strokeLinecap="round" />
          <circle cx="73" cy="92" r="1" fill="#FFF4D8" /><circle cx="105" cy="92" r="1" fill="#FFF4D8" />
        </g>
        <g stroke="#62341F" strokeWidth="2.4" strokeLinecap="round">
          <path d={curious ? 'M64 84L78 85M102 85L115 81' : focused ? 'M64 86L79 90M101 90L116 86' : 'M64 85L78 88M102 88L116 85'} />
        </g>
        <g stroke="#7B3E23" strokeWidth="1.7" strokeLinecap="round">
          <path d="M58 99L68 102M58 106L67 108M61 113L69 113M122 99L112 102M122 106L113 108M119 113L111 113" />
        </g>
        <path d="M85 108C88 106 92 106 95 108L90 113Z" fill="#3B2820" stroke="#3B2820" strokeWidth="1.5" strokeLinejoin="round" />
        <path className="pet-mouth" d="M90 113V117M81 117C84 122 88 121 90 117C92 121 96 122 99 117" stroke="#653521" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />
        <path className="pet-speaking-mouth" d="M85 117Q90 122 95 117C95 126 85 126 85 117Z" fill="#633124" stroke="#633124" strokeWidth="1.3" />
        <path d="M70 77C75 74 79 73 83 74" stroke="#FFD18B" strokeWidth="2.2" strokeLinecap="round" />
      </g>
    </svg>}
    <span className="pet-orbit orbit-one" /><span className="pet-orbit orbit-two" />
  </div>;
}


function Orb({ state, id }: { state: PetState; id: string }) {
  return <svg viewBox="0 0 180 180" fill="none"><defs><radialGradient id={`${id}-orb`} cx=".35" cy=".25" r=".8"><stop stopColor="#FFF4D5" /><stop offset=".45" stopColor="#E6AF6D" /><stop offset="1" stopColor="#8D573B" /></radialGradient></defs><ellipse className="pet-shadow" cx="90" cy="162" rx="31" ry="4" fill="#1C1514" opacity=".18" /><g className="pet-body"><circle cx="90" cy="93" r="53" fill={`url(#${id}-orb)`} stroke="#F6D2A2" strokeWidth="1.5" /><ellipse cx="90" cy="93" rx="67" ry="21" transform="rotate(-28 90 93)" stroke="#D4A576" strokeWidth="2" /><path d="M56 70C61 58 74 51 88 52" stroke="#FFF8E2" strokeWidth="5" strokeLinecap="round" opacity=".65" /><g className="pet-eyes"><rect x="71" y="86" width="6" height="12" rx="3" fill="#563A2F" /><rect x="103" y="86" width="6" height="12" rx="3" fill="#563A2F" /></g><path className="pet-mouth" d="M84 109Q90 114 96 109" stroke="#563A2F" strokeWidth="2.5" strokeLinecap="round" /><ellipse className="pet-speaking-mouth" cx="90" cy="111" rx="4" ry="5" fill="#563A2F" /><circle cx="141" cy="42" r={state === 'working' ? 5 : 3} fill="#F5CF9C" /></g></svg>;
}
