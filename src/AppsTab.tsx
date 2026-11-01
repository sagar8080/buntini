import { useEffect, useState } from 'react';
import Toggle from './Toggle';
import { command, errorMessage } from './bridge';
import type { AppList, Settings } from './types';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void };

/** The Apps tab: which apps on this Mac Buntini may use. Nothing is allowed until you say so. */
export default function AppsTab({ draft, update, onError }: Props) {
  const [list, setList] = useState<AppList | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [assisting, setAssisting] = useState<boolean | null>(null);
  useEffect(() => { command<AppList>('app_list').then(setList).catch(error => onError(errorMessage(error))); }, []);
  // Accessibility lets Buntini press buttons and read pages in other apps; macOS may take a
  // moment to report a new permission, so look again when the window comes back into focus.
  useEffect(() => {
    const look = () => void command<boolean>('accessibility', { ask: false }).then(setAssisting).catch(() => setAssisting(false));
    look();
    window.addEventListener('focus', look);
    return () => window.removeEventListener('focus', look);
  }, []);
  async function toggle(id: string, on: boolean) {
    update('apps', on ? [...new Set([...draft.apps, id])] : draft.apps.filter(item => item !== id));
    if (!on) { setNotes(previous => ({ ...previous, [id]: '' })); return; }
    // Calendar and Reminders ask macOS right away, so the prompt shows up here.
    try { const note = await command<string>('check_app', { id }); setNotes(previous => ({ ...previous, [id]: note })); }
    catch (error) { setNotes(previous => ({ ...previous, [id]: errorMessage(error) })); }
  }
  if (!list) return <section className="settings-section"><p className="field-help">Loading…</p></section>;
  return <>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Apps Buntini can use</h3></div>
      {list.apps.map(app => <div key={app.id} className="app-permission">
        <Toggle label={app.name} detail={app.installed ? app.can.join(' · ') : 'Not on this Mac'} checked={app.installed && draft.apps.includes(app.id)} disabled={!app.installed} onChange={value => void toggle(app.id, value)} />
        {app.note && app.installed && <p className="field-help app-note">{app.note}</p>}
        {notes[app.id] && <p className="field-help">{notes[app.id]}</p>}
      </div>)}
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Accessibility</h3><span className={`status-dot ${assisting ? '' : 'amber'}`} /></div>
      <p className="field-help">{assisting ? 'Allowed' : 'Spotify, browser pages, screen lock'}</p>
      {!assisting && <button type="button" className="secondary-button" onClick={() => void command<boolean>('accessibility', { ask: true }).then(setAssisting)}>Allow in System Settings…</button>}
    </section>
    {list.others.length > 0 && <section className="settings-section">
      <details className="disclosure"><summary>Other apps ({list.others.length})</summary>
        <p className="other-apps">{list.others.join(' · ')}</p>
      </details>
    </section>}
  </>;
}
