// Loads the catalogs into the app: English always, another language when it's spoken.
import { useSyncExternalStore } from 'react';
import { englishWords as english, language, resolve, rtl, subscribe, use, type Catalog } from './i18n.ts';

const others = import.meta.glob<Catalog>(['../locales/*.json', '!../locales/en.json'], { import: 'default' });
const REMEMBERED = 'buntini-language';
/** The language the Mac prefers, from Rust (WebKit only knows the app's own). */
let system = 'en';
let wanted = '';

/** Speaks `code`: its catalog, the page's language and direction. */
export async function speak(code: string): Promise<void> {
  wanted = code;
  const path = `../locales/${code}.json`;
  const load = path in others ? others[path] : undefined;
  const catalog = load ? await load().catch(() => english) : english;
  // A later choice won while this one loaded.
  if (wanted !== code) return;
  use(load ? code : 'en', catalog, english);
  const root = document.documentElement;
  root.lang = language();
  root.dir = rtl(language()) ? 'rtl' : 'ltr';
  try { localStorage.setItem(REMEMBERED, language()); } catch { /* Remembering it only spares a flash of English. */ }
}

/** Follows Settings: a language, or empty for the Mac's. `mac` is the Mac's when known. */
export function follow(chosen: string, mac?: string): Promise<void> {
  if (mac) system = mac;
  return speak(resolve(chosen, [system]));
}

/** The language last spoken, for the first paint. */
export function remembered(): string {
  try { return localStorage.getItem(REMEMBERED) || 'en'; } catch { return 'en'; }
}

/** Re-renders the component when the language changes. */
export function useLanguage(): string {
  return useSyncExternalStore(subscribe, language);
}
