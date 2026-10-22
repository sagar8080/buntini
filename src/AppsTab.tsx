import { useEffect, useState } from 'react';
import Toggle from './Toggle';
import { command, errorMessage } from './bridge';
import type { AppList, Settings } from './types';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void };

/** The Apps tab: which apps on this Mac Buntini may use. Nothing is allowed until you say so. */
export default function AppsTab({ draft, update, onError }: Props) {
  const [list, setList] = useState<AppList | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});
  useEffect(() => { command<AppList>('app_list').then(setList).catch(error => onError(errorMessage(error))); }, []);
  async function toggle(id: string, on: boolean) {
    update('apps', on ? [...new Set([...draft.apps, id])] : draft.apps.filter(item => item !== id));
    if (!on) { setNotes(previous => ({ ...previous, [id]: '' })); return; }
    // Calendar and Reminders ask macOS right away, so the prompt shows up here.
    try { const note = await command<string>('check_app', { id }); setNotes(previous => ({ ...previous, [id]: note })); }
    catch (error) { setNotes(previous => ({ ...previous, [id]: errorMessage(error) })); }
  }
  if (!list) return <section className="settings-section"><p className="field-help">Looking at your apps…</p></section>;
  return <>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Apps Buntini can use</h3></div>
      <p className="field-help">Everything happens on this Mac, with no accounts. Buntini adds or drafts things only after you approve each one.</p>
      {list.apps.map(app => <div key={app.id} className="app-permission">
        <Toggle label={app.name} detail={app.installed ? app.can.join(' · ') : 'Not on this Mac'} checked={app.installed && draft.apps.includes(app.id)} disabled={!app.installed} onChange={value => void toggle(app.id, value)} />
        {notes[app.id] && <p className="field-help">{notes[app.id]}</p>}
      </div>)}
    </section>
    {list.others.length > 0 && <section className="settings-section">
      <details className="disclosure"><summary>{list.others.length} other apps Buntini can open for you</summary>
        <p className="field-help">Ask “open Figma” and Buntini checks with you first. It doesn’t look inside these apps.</p>
        <p className="other-apps">{list.others.join(' · ')}</p>
      </details>
    </section>}
  </>;
}
