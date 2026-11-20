// Buntini's words in the person's language. The catalogs are locales/<code>.json, the same
// files Rust reads (src-tauri/src/i18n.rs); locales.ts loads them into the app.
//
// - Keys are dotted, `area.name`. A key missing from a catalog falls back to English, then to
//   the key itself.
// - Placeholders are `{name}`. `tn` picks a plural form (`key.one`, `key.few`, …, falling back
//   to `key.other`) with Intl.PluralRules, and writes `{count}` with Intl.NumberFormat.

import englishCatalog from '../locales/en.json' with { type: 'json' };

export type Catalog = Record<string, string>;
export type Values = Record<string, string | number>;

/** The source of every word, and what a missing translation falls back to. */
export const englishWords: Catalog = englishCatalog;

/** The languages Buntini speaks, as Coucou does, each named in itself. */
export const languages = [
  { code: 'en', name: 'English' },
  { code: 'zh-Hans', name: '简体中文' },
  { code: 'zh-Hant', name: '繁體中文' },
  { code: 'hi', name: 'हिन्दी' },
  { code: 'es', name: 'Español' },
  { code: 'ar', name: 'العربية' },
  { code: 'fr', name: 'Français' },
  { code: 'bn', name: 'বাংলা' },
  { code: 'pt-BR', name: 'Português (Brasil)' },
  { code: 'ru', name: 'Русский' },
  { code: 'id', name: 'Bahasa Indonesia' },
  { code: 'de', name: 'Deutsch' },
  { code: 'tr', name: 'Türkçe' },
  { code: 'th', name: 'ไทย' },
  { code: 'ko', name: '한국어' },
  { code: 'ja', name: '日本語' },
] as const;

const codes: readonly string[] = languages.map(item => item.code);

let english: Catalog = englishWords;
let current: Catalog = englishWords;
let active = 'en';
const listeners = new Set<() => void>();

/** The language Buntini speaks now. */
export const language = () => active;

/** For Intl and toLocale…String: undefined in English, so English keeps the Mac's own formats. */
export const locale = () => (active === 'en' ? undefined : active);

/** Written right to left. */
export const rtl = (code: string) => code === 'ar';

/** Speaks `code` with its catalog, English underneath. */
export function use(code: string, catalog: Catalog, englishCatalog: Catalog) {
  const same = active === code && current === catalog;
  english = englishCatalog;
  current = catalog;
  active = code;
  if (!same) listeners.forEach(listener => listener());
}

/** Called whenever the language changes. */
export function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

/** The language Buntini has for a BCP 47 tag such as `de-AT`, `zh-Hant-TW` or `pt-PT`. */
export function matching(tag: string): string | undefined {
  const clean = tag.replace(/_/g, '-');
  const exact = codes.find(code => code.toLowerCase() === clean.toLowerCase());
  if (exact) return exact;
  const [base, ...rest] = clean.toLowerCase().split('-');
  if (base === 'zh') {
    const traditional = rest.includes('hant') || (!rest.includes('hans') && rest.some(part => ['tw', 'hk', 'mo'].includes(part)));
    return traditional ? 'zh-Hant' : 'zh-Hans';
  }
  if (base === 'pt') return 'pt-BR';
  if (base === 'in') return 'id';
  return codes.find(code => code === base);
}

/** The language picked in Settings, else the first of `preferred` Buntini has, else English. */
export function resolve(chosen: string, preferred: readonly string[]): string {
  if (codes.includes(chosen)) return chosen;
  for (const tag of preferred) {
    const found = matching(tag);
    if (found) return found;
  }
  return 'en';
}

function fill(template: string, values: Values = {}): string {
  return template.replace(/\{([A-Za-z0-9_]+)\}/g, (whole, name: string) => (name in values ? String(values[name]) : whole));
}

/** The text for `key`, its `{name}` placeholders filled from `values`. */
export function t(key: string, values?: Values): string {
  return fill(current[key] ?? english[key] ?? key, values);
}

/** Numbers as people write them in the language now. */
export function number(value: number, options?: Intl.NumberFormatOptions): string {
  try { return new Intl.NumberFormat(locale(), options).format(value); } catch { return String(value); }
}

/** The plural form of `count` in `code`. */
export function plural(code: string, count: number): Intl.LDMLPluralRule {
  try { return new Intl.PluralRules(code).select(count); } catch { return count === 1 ? 'one' : 'other'; }
}

/** `key` in the plural form `count` takes, with `{count}` and the other placeholders filled. */
export function tn(key: string, count: number, values?: Values): string {
  const form = `${key}.${plural(active, count)}`;
  const template = current[form] ?? current[`${key}.other`] ?? english[`${key}.${plural('en', count)}`] ?? english[`${key}.other`] ?? key;
  return fill(template, { ...values, count: number(count) });
}
