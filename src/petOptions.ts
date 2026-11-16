import { accentHex } from './themes.ts';
import { t } from './i18n.ts';
import type { CatAccessory, CatCoat, CatEyes, PetLook, Settings } from './types';

// Each label is read in the language spoken when it shows.
export const characters: { id: Settings['avatar']; readonly label: string }[] = [
  { id: 'cat', get label() { return t('surface.character.cat'); } },
  { id: 'orb', get label() { return t('surface.character.orb'); } },
];
export const coats: { id: CatCoat; readonly label: string }[] = [
  { id: 'ginger', get label() { return t('surface.coat.ginger'); } },
  { id: 'gray', get label() { return t('surface.coat.gray'); } },
  { id: 'black', get label() { return t('surface.coat.black'); } },
  { id: 'white', get label() { return t('surface.coat.white'); } },
  { id: 'tuxedo', get label() { return t('surface.coat.tuxedo'); } },
  { id: 'calico', get label() { return t('surface.coat.calico'); } },
  { id: 'siamese', get label() { return t('surface.coat.siamese'); } },
];
export const eyeColors: Record<Exclude<CatEyes, 'odd'>, string> = { amber: '#F2B33D', green: '#86C46B', blue: '#6DB6E8', copper: '#D9772B' };
export const eyes: { id: CatEyes; readonly label: string }[] = [
  { id: 'amber', get label() { return t('surface.eyes.amber'); } },
  { id: 'green', get label() { return t('surface.eyes.green'); } },
  { id: 'blue', get label() { return t('surface.eyes.blue'); } },
  { id: 'copper', get label() { return t('surface.eyes.copper'); } },
  { id: 'odd', get label() { return t('surface.eyes.odd'); } },
];
export const accessories: { id: CatAccessory; readonly label: string }[] = [
  { id: 'none', get label() { return t('surface.accessory.none'); } },
  { id: 'bell', get label() { return t('surface.accessory.bell'); } },
  { id: 'bow', get label() { return t('surface.accessory.bow'); } },
  { id: 'bandana', get label() { return t('surface.accessory.bandana'); } },
  { id: 'hat', get label() { return t('surface.accessory.hat'); } },
  { id: 'flowers', get label() { return t('surface.accessory.flowers'); } },
];

// The accessory takes the accent colour (the theme's bright accent when it follows the theme).
export const lookFrom = (settings: Settings): PetLook => ({ coat: settings.catCoat, eyes: settings.catEyes, accessory: settings.catAccessory, motion: settings.petMotion, accent: accentHex(settings, true) });
