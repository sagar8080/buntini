// Buntini's look: theme presets (each with a light and a dark version) plus a few
// adjustments. `appearance` turns settings into the attributes and CSS variables the
// stylesheet reads, so the rules live in one place and can be tested.
import type { Settings } from './types';

export interface Palette { panel: string; accent: string }
// Each preset's colours are also in styles.css; themes.test.ts checks the two agree and that
// text and accents there are readable (4.5:1, WCAG AA for small text).
export const presets: { id: Settings['themePreset']; label: string; light: Palette; dark: Palette }[] = [
  { id: 'classic', label: 'Classic', light: { panel: '#fbf7f2', accent: '#a9491a' }, dark: { panel: '#1a1512', accent: '#ff9e62' } },
  { id: 'midnight', label: 'Midnight', light: { panel: '#f4f5fb', accent: '#4d5dcd' }, dark: { panel: '#171a2b', accent: '#8c9bff' } },
  { id: 'paper', label: 'Paper', light: { panel: '#f7f4ec', accent: '#3d5a80' }, dark: { panel: '#22201c', accent: '#9db4d6' } },
  { id: 'sakura', label: 'Sakura', light: { panel: '#fdf5f7', accent: '#aa4662' }, dark: { panel: '#2a2226', accent: '#f08aa7' } },
  { id: 'forest', label: 'Forest', light: { panel: '#f3f6f1', accent: '#487243' }, dark: { panel: '#1d231e', accent: '#8fc286' } },
  { id: 'ocean', label: 'Ocean', light: { panel: '#f2f7f9', accent: '#2a728b' }, dark: { panel: '#182326', accent: '#6cc3dc' } },
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

const channels = (hex: string) => [1, 3, 5].map(at => parseInt(hex.slice(at, at + 2), 16));
const toHex = (values: number[]) => `#${values.map(value => Math.round(Math.min(255, Math.max(0, value))).toString(16).padStart(2, '0')).join('')}`;
/** `amount` of the way from colour `a` to colour `b`. */
export const mix = (a: string, b: string, amount: number) => { const from = channels(a), to = channels(b); return toHex(from.map((value, index) => value + (to[index] - value) * amount)); };

/** WCAG relative luminance of a #rrggbb colour. */
export function luminance(hex: string): number {
  const [r, g, b] = channels(hex).map(value => { const c = value / 255; return c <= .04045 ? c / 12.92 : ((c + .055) / 1.055) ** 2.4; });
  return .2126 * r + .7152 * g + .0722 * b;
}
export function contrast(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (light + .05) / (dark + .05);
}

/** Text on the accent (the send button, your bubbles): white or near-black, whichever reads better. */
export const onAccent = (accent: string) => contrast('#ffffff', accent) >= contrast('#15171c', accent) ? '#ffffff' : '#15171c';

/** The accent darkened (light mode) or lightened (dark mode) just enough to read at 4.5:1 as
    text on the panel and on its own soft tint, so any colour a user picks stays legible. */
export function readableAccent(accent: string, panel: string, dark: boolean): string {
  const toward = dark ? '#ffffff' : '#000000';
  for (let step = 0; step <= 50; step++) {
    const candidate = mix(accent, toward, step / 50);
    const soft = mix(panel, candidate, dark ? .24 : .16);
    if (contrast(candidate, panel) >= 4.5 && contrast(candidate, soft) >= 4.5) return candidate;
  }
  return toward;
}

const textScale = { small: .92, medium: 1, large: 1.12 } as const;
export const panelScale = { compact: .9, standard: 1, large: 1.15 } as const;

/** What the root element needs: data attributes for the stylesheet and inline variables. */
export function appearance(settings: Settings, systemDark: boolean): { attributes: Record<string, string>; variables: Record<string, string> } {
  const dark = settings.theme === 'dark' || (settings.theme === 'system' && systemDark);
  const variables: Record<string, string> = { '--text-scale': String(textScale[settings.textSize] ?? 1) };
  const preset = presets.find(item => item.id === settings.themePreset) ?? presets[0];
  const palette = dark ? preset.dark : preset.light;
  // The theme's own accent comes from the stylesheet; any other choice overrides it, made readable.
  let accent = palette.accent;
  if (settings.accent !== 'preset') {
    accent = readableAccent(accentHex(settings, dark), palette.panel, dark);
    variables['--accent'] = accent;
    variables['--accent-soft'] = `color-mix(in srgb, ${accent} ${dark ? 24 : 16}%, transparent)`;
  }
  variables['--on-accent'] = onAccent(accent);
  return {
    attributes: { mode: dark ? 'dark' : 'light', preset: settings.themePreset, density: settings.density, font: settings.chatFont, corners: settings.corners },
    variables,
  };
}

/** Whether to keep still: the Reduce motion setting, or macOS's own. */
export const stillMotion = (reduced?: boolean) => !!reduced || (typeof window !== 'undefined' && !!window.matchMedia?.('(prefers-reduced-motion: reduce)').matches);

/** The idle status line: the user's own words, or one in Buntini's voice. */
export function idleStatus(settings: Pick<Settings, 'statusText' | 'petMotion'>, asleep: boolean, hour: number): string {
  if (settings.statusText.trim()) return settings.statusText.trim();
  if (asleep) return 'Napping';
  if (hour >= 23 || hour < 5) return 'Up late with you';
  return { calm: 'Right here', playful: 'Purring nearby', sleepy: 'Half awake' }[settings.petMotion] ?? 'Right here';
}
