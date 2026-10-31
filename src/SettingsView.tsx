import { useEffect, useRef, useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import Customization from './Customization';
import BrainSettings from './BrainSettings';
import Connections from './Connections';
import PrivacyTab from './PrivacyTab';
import AppsTab from './AppsTab';
import type { SettingsSaver, SaveState } from './settingsSave';
import type { Bootstrap, KeyProvider, Settings } from './types';

export type Tab = 'buntini' | 'brain' | 'apps' | 'connections' | 'privacy';
const tabs: { id: Tab; label: string }[] = [{ id: 'buntini', label: 'Buntini' }, { id: 'brain', label: 'Brain' }, { id: 'apps', label: 'Apps' }, { id: 'connections', label: 'Connections' }, { id: 'privacy', label: 'Privacy' }];

type Props = {
  data: Bootstrap; saver: SettingsSaver; initialTab?: Tab; onError: (message: string) => void;
  onAddFolder: () => Promise<void>; onRemoveFolder: (path: string) => Promise<void>;
  onKey: (provider: KeyProvider, key: string) => Promise<void>; onAudit: () => void; onRestartSetup: () => void;
  onPreviewVoice: (voiceId: string, rate: number, text: string) => Promise<void>;
  privacyExtras?: React.ReactNode; onForget: (id: number) => Promise<void>; onPolicy?: () => void;
  onRefresh: () => Promise<void>; onNotice: (message: string) => void;
};

/** Settings in five tabs. Changes save on their own a moment after you stop editing, and right
    away when Settings closes. While a task is running, model and policy changes wait for it. */
export default function SettingsView({ data, saver, initialTab, onError, onAddFolder, onRemoveFolder, onKey, onAudit, onRestartSetup, onPreviewVoice, privacyExtras, onForget, onPolicy, onRefresh, onNotice }: Props) {
  const [tab, setTab] = useState<Tab>(() => { if (initialTab) return initialTab; try { return (localStorage.getItem('buntini-settings-tab') as Tab) || 'buntini'; } catch { return 'buntini'; } });
  // An edit still waiting to save (say, behind a running task) is what the toggles show.
  const [draft, setDraft] = useState(() => saver.pending() ?? data.settings);
  const [status, setStatus] = useState<SaveState>(() => saver.pending() ? saver.state : 'idle');
  const [problem, setProblem] = useState('');
  const [loginProblem, setLoginProblem] = useState('');
  const edited = useRef(false);
  const update = <K extends keyof Settings>(name: K, value: Settings[K]) => { edited.current = true; setDraft(previous => ({ ...previous, [name]: value })); };
  useEffect(() => { try { localStorage.setItem('buntini-settings-tab', tab); } catch { /* Remembering the tab is optional. */ } }, [tab]);
  useEffect(() => saver.subscribe((state, message) => { setStatus(state); setProblem(message); }), [saver]);
  useEffect(() => { if (edited.current) saver.edit(draft); }, [draft]);
  // Open at login is saved as what macOS confirmed: when it refused, the switch says so.
  useEffect(() => saver.subscribeSaved((sent, saved) => {
    if (sent.openAtLogin === saved.openAtLogin) { setLoginProblem(''); return; }
    edited.current = false;
    setDraft(previous => ({ ...previous, openAtLogin: saved.openAtLogin }));
    setLoginProblem(saved.openAtLogin ? 'Still in your login items. Remove me in System Settings → General → Login Items.' : 'macOS didn’t add me to your login items.');
  }), [saver]);
  // Closing Settings (Esc, the header button, tucking the panel away) saves at once.
  useEffect(() => () => {
    void saver.flush();
    if (saver.state === 'waiting') onNotice('Settings save after this task.');
  }, [saver]);
  return <div className="subpage settings-page">
    <div className="settings-top">
      <div className="settings-tabs" role="tablist" aria-label="Settings" onKeyDown={event => {
        // Arrow keys move between tabs, as in native tab bars.
        const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
        if (!step) return;
        event.preventDefault();
        const next = tabs[(tabs.findIndex(item => item.id === tab) + step + tabs.length) % tabs.length];
        setTab(next.id);
        (event.currentTarget.querySelector(`[data-tab="${next.id}"]`) as HTMLButtonElement | null)?.focus();
      }}>{tabs.map(item => <button key={item.id} data-tab={item.id} role="tab" type="button" tabIndex={tab === item.id ? 0 : -1} aria-selected={tab === item.id} className={tab === item.id ? 'active' : ''} onClick={() => setTab(item.id)}>{item.label}</button>)}</div>
      <span className={`save-state ${status}`} role="status">{status === 'saving' ? <><LoaderCircle size={12} className="spin" />Saving</> : status === 'waiting' ? <><LoaderCircle size={12} className="spin" />Saves after this task</> : status === 'saved' ? <><Check size={12} />Saved</> : status === 'error' ? 'Not saved' : ''}</span>
    </div>
    {status === 'error' && problem && <p className="settings-problem" role="alert">{problem}</p>}
    {loginProblem && <p className="settings-problem" role="alert">{loginProblem}</p>}
    <div role="tabpanel" aria-label={tabs.find(item => item.id === tab)?.label}>
      {tab === 'buntini' && <Customization draft={draft} update={update} onError={onError} onPreviewVoice={onPreviewVoice} onRestartSetup={onRestartSetup} />}
      {tab === 'brain' && <BrainSettings draft={draft} update={update} diagnostics={data.diagnostics} onError={onError} onKey={onKey} onRefresh={onRefresh} onNotice={onNotice} />}
      {tab === 'apps' && <AppsTab draft={draft} update={update} onError={onError} />}
      {tab === 'connections' && <Connections draft={draft} update={update} diagnostics={data.diagnostics} onKey={onKey} onError={onError} />}
      {tab === 'privacy' && <PrivacyTab draft={draft} update={update} roots={data.roots} onAddFolder={onAddFolder} onRemoveFolder={onRemoveFolder} onAudit={onAudit} onError={onError} memories={data.memories || []} onForget={onForget} onPolicy={onPolicy}>{privacyExtras}</PrivacyTab>}
    </div>
  </div>;
}
