// Buntini's look: theme presets (each with a light and a dark version) plus a few
// adjustments. `appearance` turns settings into the attributes and CSS variables the
// stylesheet reads, so the rules live in one place and can be tested.
import type { Settings } from './types';

export interface Palette { panel: string; accent: string }
export const presets: { id: Settings['themePreset']; label: string; light: Palette; dark: Palette }[] = [
  { id: 'classic', label: 'Classic', light: { panel: '#f6f7f9', accent: '#c2561c' }, dark: { panel: '#121419', accent: '#ff9a55' } },
  { id: 'midnight', label: 'Midnight', light: { panel: '#f4f5fb', accent: '#4f5fd1' }, dark: { panel: '#171a2b', accent: '#8c9bff' } },
  { id: 'paper', label: 'Paper', light: { panel: '#f7f4ec', accent: '#3d5a80' }, dark: { panel: '#22201c', accent: '#9db4d6' } },
  { id: 'sakura', label: 'Sakura', light: { panel: '#fdf5f7', accent: '#d4577a' }, dark: { panel: '#2a2226', accent: '#f08aa7' } },
  { id: 'forest', label: 'Forest', light: { panel: '#f3f6f1', accent: '#4f7d4a' }, dark: { panel: '#1d231e', accent: '#8fc286' } },
  { id: 'ocean', label: 'Ocean', light: { panel: '#f2f7f9', accent: '#2f7f9a' }, dark: { panel: '#182326', accent: '#6cc3dc' } },
];

// Fixed accents; "preset" follows the theme and "custom" uses accentColor.
export const accentChoices: { id: Settings['accent']; label: string; color?: string }[] = [
  { id: 'preset', label: 'Theme' },
  { id: 'ember', label: 'Ember', color: '#e9985d' },
  { id: 'ocean', label: 'Ocean', color: '#71bacb' },
  { id: 'violet', label: 'Violet', color: '#bc92df' },
  { id: 'sage', label: 'Sage', color: '#9db98b' },
  { id: 'custom', label: 'Custom' },
];

export const isHex = (value: string) => /^#[0-9a-f]{6}$/i.test(value);

/** The accent as a colour, for the interface and the cat's accessory. */
export function accentHex(settings: Pick<Settings, 'accent' | 'accentColor' | 'themePreset'>, dark: boolean): string {
  if (settings.accent === 'custom' && isHex(settings.accentColor)) return settings.accentColor;
  const fixed = accentChoices.find(choice => choice.id === settings.accent)?.color;
  if (fixed) return fixed;
  const preset = presets.find(item => item.id === settings.themePreset) ?? presets[0];
  return (dark ? preset.dark : preset.light).accent;
}

const textScale = { small: .92, medium: 1, large: 1.12 } as const;
export const panelScale = { compact: .9, standard: 1, large: 1.15 } as const;

/** What the root element needs: data attributes for the stylesheet and inline variables. */
export function appearance(settings: Settings, systemDark: boolean): { attributes: Record<string, string>; variables: Record<string, string> } {
  const dark = settings.theme === 'dark' || (settings.theme === 'system' && systemDark);
  const variables: Record<string, string> = { '--text-scale': String(textScale[settings.textSize] ?? 1) };
  // The theme's own accent comes from the stylesheet; any other choice overrides it.
  if (settings.accent !== 'preset') {
    const accent = accentHex(settings, dark);
    variables['--accent'] = accent;
    variables['--accent-soft'] = `color-mix(in srgb, ${accent} ${dark ? 24 : 16}%, transparent)`;
  }
  return {
    attributes: { mode: dark ? 'dark' : 'light', preset: settings.themePreset, density: settings.density, font: settings.chatFont, corners: settings.corners },
    variables,
  };
}

/** The idle status line: the user's own words, or one in Buntini's voice. */
export function idleStatus(settings: Pick<Settings, 'statusText' | 'petMotion'>, asleep: boolean, hour: number): string {
  if (settings.statusText.trim()) return settings.statusText.trim();
  if (asleep) return 'Napping';
  if (hour >= 23 || hour < 5) return 'Up late with you';
  return { calm: 'Right here', playful: 'Purring nearby', sleepy: 'Half awake' }[settings.petMotion] ?? 'Right here';
}
