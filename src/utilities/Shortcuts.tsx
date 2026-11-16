import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import { errorMessage } from '../bridge';
import { callUtilities } from './api';
import { held, keycaps, shortcutFrom, type Shortcut, type ShortcutRow } from './model';
import { t } from '../i18n.ts';
import './utilities.css';

const same = (a: Shortcut | null, b: Shortcut | null) => (a?.label ?? '') === (b?.label ?? '');

/** The shortcuts you can change, each with a key recorder. A combination macOS, every app,
    dictation or another shortcut already uses is refused with the reason. */
export default function Shortcuts({ onError }: { onError: (message: string) => void }) {
  const [rows, setRows] = useState<ShortcutRow[] | null>(null);
  const [recording, setRecording] = useState<string | null>(null);
  const [holding, setHolding] = useState('');
  const [problem, setProblem] = useState<{ id: string; text: string } | null>(null);

  useEffect(() => {
    void callUtilities<ShortcutRow[]>({ op: 'shortcuts' }).then(setRows).catch(error => onError(errorMessage(error)));
  }, []);

  async function choose(id: string, shortcut: Shortcut | null) {
    try {
      setRows(await callUtilities<ShortcutRow[]>({ op: 'setShortcut', id, shortcut }));
      setProblem(null);
      setRecording(null);
    } catch (error) {
      setProblem({ id, text: errorMessage(error) });
    }
  }

  // While recording, Buntini's own shortcuts stand aside so the keys reach the recorder.
  useEffect(() => {
    if (!recording) return;
    void callUtilities({ op: 'recording', on: true }).catch(() => {});
    setHolding('');
    const down = (event: KeyboardEvent) => {
      // Nothing else hears these keys: not Esc closing Settings, not ⌘Q.
      event.preventDefault();
      event.stopImmediatePropagation();
      setHolding(held(event));
      if (event.key === 'Escape') { setRecording(null); setProblem(null); return; }
      if ((event.key === 'Backspace' || event.key === 'Delete') && !held(event)) { void choose(recording, null); return; }
      if (event.repeat) return;
      const shortcut = shortcutFrom(event);
      if (shortcut) void choose(recording, shortcut);
    };
    const up = (event: KeyboardEvent) => setHolding(held(event));
    const away = () => setRecording(null);
    window.addEventListener('keydown', down, true);
    window.addEventListener('keyup', up, true);
    window.addEventListener('blur', away);
    return () => {
      window.removeEventListener('keydown', down, true);
      window.removeEventListener('keyup', up, true);
      window.removeEventListener('blur', away);
      void callUtilities({ op: 'recording', on: false }).catch(() => {});
    };
  }, [recording]);

  if (!rows) return null;
  const changed = rows.some(row => !same(row.shortcut, row.default));
  return <section className="settings-section shortcuts">
    <div className="settings-section-heading"><h3>{t('settings.shortcuts.title')}</h3>{changed && <button type="button" className="text-button" onClick={() => void callUtilities<ShortcutRow[]>({ op: 'resetShortcuts' }).then(next => { setRows(next); setProblem(null); }).catch(error => onError(errorMessage(error)))}>{t('settings.shortcuts.restore')}</button>}</div>
    {rows.map(row => {
      const live = recording === row.id;
      const wrong = problem?.id === row.id ? problem.text : row.problem;
      return <div key={row.id} className={`shortcut-row ${wrong ? 'has-problem' : ''}`}>
        <span className="shortcut-name">{row.label}</span>
        <button type="button" className={`shortcut-key ${live ? 'is-recording' : ''}`} aria-label={t('settings.shortcuts.current', { action: row.label, keys: row.shortcut?.label ?? t('settings.shortcuts.none') })} aria-pressed={live} onClick={() => { setProblem(null); setRecording(live ? null : row.id); }}>
          {live
            ? holding ? keycaps(holding).map(cap => <kbd key={cap}>{cap}</kbd>) : <span className="shortcut-hint">{t('settings.shortcuts.pressKeys')}</span>
            : row.shortcut ? keycaps(row.shortcut.label).map(cap => <kbd key={cap}>{cap}</kbd>) : <span className="shortcut-hint">—</span>}
        </button>
        <button type="button" className="icon-button shortcut-clear" aria-label={t('settings.shortcuts.clear', { action: row.label })} title={t('settings.shortcuts.noneTitle')} disabled={!row.shortcut || live} onClick={() => void choose(row.id, null)}><X size={12} /></button>
        {wrong && <small className="shortcut-problem" role="status">{wrong}</small>}
      </div>;
    })}
  </section>;
}
