import type { CatAccessory, CatCoat, CatEyes, PetLook, PetMotion, Settings } from './types';

export const characters: { id: Settings['avatar']; label: string }[] = [{ id: 'cat', label: 'Cat' }, { id: 'kyuubi', label: 'Nine tails' }, { id: 'fox', label: 'Fox' }, { id: 'orb', label: 'Orbit' }];
export const accents: { id: Settings['accent']; label: string; color: string }[] = [{ id: 'ember', label: 'Ember', color: '#e9985d' }, { id: 'ocean', label: 'Ocean', color: '#71bacb' }, { id: 'violet', label: 'Violet', color: '#bc92df' }, { id: 'sage', label: 'Sage', color: '#9db98b' }];
export const coats: { id: CatCoat; label: string }[] = [{ id: 'ginger', label: 'Ginger tabby' }, { id: 'gray', label: 'Silver tabby' }, { id: 'black', label: 'Midnight' }, { id: 'white', label: 'Snowball' }, { id: 'tuxedo', label: 'Tuxedo' }, { id: 'calico', label: 'Calico' }, { id: 'siamese', label: 'Siamese' }];
export const eyeColors: Record<Exclude<CatEyes, 'odd'>, string> = { amber: '#F2B33D', green: '#86C46B', blue: '#6DB6E8', copper: '#D9772B' };
export const eyes: { id: CatEyes; label: string }[] = [{ id: 'amber', label: 'Amber' }, { id: 'green', label: 'Green' }, { id: 'blue', label: 'Blue' }, { id: 'copper', label: 'Copper' }, { id: 'odd', label: 'Odd-eyed' }];
export const accessories: { id: CatAccessory; label: string }[] = [{ id: 'none', label: 'None' }, { id: 'bell', label: 'Bell collar' }, { id: 'bow', label: 'Bow' }, { id: 'bandana', label: 'Bandana' }, { id: 'hat', label: 'Top hat' }, { id: 'flowers', label: 'Flowers' }];
export const motions: { id: PetMotion; label: string; detail: string }[] = [{ id: 'calm', label: 'Calm', detail: 'Slow breaths and a lazy tail.' }, { id: 'playful', label: 'Playful', detail: 'Tail swishes, ear twitches, and the odd little hop.' }, { id: 'sleepy', label: 'Sleepy', detail: 'Dozes when idle and wakes up when you need it.' }];

export const lookFrom = (settings: Settings): PetLook => ({ coat: settings.catCoat, eyes: settings.catEyes, accessory: settings.catAccessory, motion: settings.petMotion, accent: settings.accent });
export const accentColor = (id: Settings['accent']) => accents.find(item => item.id === id)?.color ?? accents[0].color;
