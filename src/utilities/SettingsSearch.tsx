import { useEffect, useRef, useState } from 'react';
import { Search, X } from 'lucide-react';
import { searchSettings, tabNames, type SettingEntry, type SettingsTab } from './model';
import './utilities.css';

const plain = (text: string) => text.toLowerCase().replace(/[’']/g, '').replace(/\s+/g, ' ').trim();

/** The row a setting's label belongs to, for the highlight. */
function rowOf(element: Element): Element[] {
  if (element.matches('h3')) return [element.closest('.settings-section') ?? element];
  if (element.matches('.setting-label')) return [element, element.nextElementSibling].filter((item): item is Element => !!item);
  return [element.closest('.toggle-row, .shortcut-row, .service-row, .key-input, .range-row, .inline-field, .voice-row, .profile-card, .audit-button, .app-permission, .flow-model, .row-actions, .chip-row, .character-options') ?? element];
}

/** What a setting's label says: a group's name, or the text before a toggle's detail. */
function textOf(element: Element): string {
  if (element.hasAttribute('aria-label') && !element.matches('button')) return element.getAttribute('aria-label') ?? '';
  if (element.matches('.toggle-row > span')) return element.firstChild?.textContent ?? '';
  return element.textContent ?? '';
}

/** Finds a setting on the open tab by its label and shows it, highlighted. */
function reveal(panel: Element, label: string): boolean {
  const wanted = plain(label);
  const candidates = panel.querySelectorAll('h3, .setting-label, .toggle-row > span, .shortcut-name, .service-summary strong, .range-row > span, .inline-field > span, .audit-button > span, summary, button, label > span, [aria-label]');
  let found: Element | null = null;
  for (const element of candidates) {
    const text = plain(textOf(element));
    // "Monthly cap ($)" is found as "Monthly cap".
    if (text === wanted || (text.startsWith(wanted) && /^[\s(]/.test(text.slice(wanted.length)))) { found = element; break; }
  }
  if (!found) return false;
  const details = found.closest('details');
  if (details && !details.open) details.open = true;
  const rows = rowOf(found);
  rows[0].scrollIntoView({ block: 'center', behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
  for (const row of rows) {
    row.classList.remove('setting-found');
    void (row as HTMLElement).offsetWidth;
    row.classList.add('setting-found');
    window.setTimeout(() => row.classList.remove('setting-found'), 2200);
  }
  const focusable = (found.matches('input, select, textarea, button') ? found : rows[0].querySelector('input, select, textarea, button')) as HTMLElement | null;
  focusable?.focus({ preventScroll: true });
  return true;
}

/** A search field above the tabs: any setting by its label or the words people use for it. */
export default function SettingsSearch({ onTab }: { onTab: (tab: SettingsTab) => void }) {
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(0);
  const field = useRef<HTMLInputElement>(null);
  const results = searchSettings(query);

  // ⌘F searches, as in every Mac app.
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      if (event.metaKey && !event.altKey && !event.ctrlKey && event.key.toLowerCase() === 'f') {
        event.preventDefault();
        field.current?.focus();
        field.current?.select();
      }
    };
    window.addEventListener('keydown', listener);
    return () => window.removeEventListener('keydown', listener);
  }, []);

  function go(entry: SettingEntry) {
    onTab(entry.tab);
    setQuery('');
    field.current?.blur();
    // The tab draws first; some parts load a moment later.
    const started = performance.now();
    const look = () => {
      const panel = document.querySelector('.settings-page [role=tabpanel]');
      if (panel && reveal(panel, entry.label)) return;
      if (performance.now() - started < 1500) window.setTimeout(look, 60);
    };
    window.requestAnimationFrame(look);
  }

  return <div className="settings-search" role="search">
    <Search size={13} aria-hidden="true" />
    <input ref={field} type="search" value={query} placeholder="Search settings" aria-label="Search settings" aria-expanded={results.length > 0} aria-controls="settings-search-results" aria-activedescendant={results.length ? `settings-result-${active}` : undefined} role="combobox" autoComplete="off" spellCheck={false}
      onChange={event => { setQuery(event.target.value); setActive(0); }}
      onKeyDown={event => {
        if (event.key === 'ArrowDown' && results.length) { event.preventDefault(); setActive((active + 1) % results.length); }
        else if (event.key === 'ArrowUp' && results.length) { event.preventDefault(); setActive((active - 1 + results.length) % results.length); }
        else if (event.key === 'Enter' && results[active]) { event.preventDefault(); go(results[active]); }
        else if (event.key === 'Escape' && query) { event.preventDefault(); event.stopPropagation(); setQuery(''); }
      }} />
    {query && <button type="button" className="settings-search-clear" aria-label="Clear" onClick={() => { setQuery(''); field.current?.focus(); }}><X size={12} /></button>}
    {query.trim() && <div className="settings-search-results" id="settings-search-results" role="listbox">
      {results.length
        ? results.map((entry, index) => <button type="button" role="option" id={`settings-result-${index}`} aria-selected={index === active} key={`${entry.tab}-${entry.label}`} className={index === active ? 'active' : ''} onMouseEnter={() => setActive(index)} onMouseDown={event => event.preventDefault()} onClick={() => go(entry)}>
            <span>{entry.label}</span><small>{tabNames[entry.tab]}</small>
          </button>)
        : <p className="settings-search-empty">No matches</p>}
    </div>}
  </div>;
}
