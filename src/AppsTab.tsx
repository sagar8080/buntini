import { useEffect, useState } from 'react';
import Toggle from './Toggle';
import { command, errorMessage } from './bridge';
import type { AppList, CodingAgent, Settings } from './types';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void };

/** The Apps tab: which apps on this Mac Buntini may use. Nothing is allowed until you say so. */
export default function AppsTab({ draft, update, onError }: Props) {
  const [list, setList] = useState<AppList | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [assisting, setAssisting] = useState<boolean | null>(null);
  const [agents, setAgents] = useState<CodingAgent[]>([]);
  const [connecting, setConnecting] = useState('');
  useEffect(() => { command<AppList>('app_list').then(setList).catch(error => onError(errorMessage(error))); }, []);
  // Claude Code and Codex: on writes Buntini's hooks into the agent's own settings, off takes them out.
  useEffect(() => { command<CodingAgent[]>('coding_agents').then(setAgents).catch(() => {}); }, []);
  async function connect(id: string, on: boolean) {
    setConnecting(id);
    try { const next = await command<CodingAgent>('connect_coding_agent', { id, on }); setAgents(previous => previous.map(agent => agent.id === id ? next : agent)); }
    catch (error) { onError(errorMessage(error)); }
    finally { setConnecting(''); }
  }
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
    {agents.some(agent => agent.available) && <section className="settings-section">
      <div className="settings-section-heading"><h3>Coding agents</h3></div>
      {agents.filter(agent => agent.available).map(agent => <div key={agent.id} className="app-permission">
        <Toggle label={agent.name} detail={agent.id === 'claude' ? 'Approve, deny and answer from the cat · Know when it’s done' : 'Approve and deny from the cat · Know when it’s done'} checked={agent.connected} disabled={connecting === agent.id} onChange={on => void connect(agent.id, on)} />
        {agent.problem && <p className="field-help">{agent.problem}</p>}
        {agent.id === 'codex' && agent.connected && !agent.heard && <p className="field-help app-note">In Codex, type /hooks and trust Buntini’s hooks.</p>}
      </div>)}
    </section>}
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
