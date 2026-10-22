import { useEffect, useRef, useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import Customization from './Customization';
import BrainSettings from './BrainSettings';
import Connections from './Connections';
import PrivacyTab from './PrivacyTab';
import { errorMessage } from './bridge';
import type { Bootstrap, KeyProvider, Settings } from './types';

type Tab = 'buntini' | 'brain' | 'connections' | 'privacy';
const tabs: { id: Tab; label: string }[] = [{ id: 'buntini', label: 'Buntini' }, { id: 'brain', label: 'Brain' }, { id: 'connections', label: 'Connections' }, { id: 'privacy', label: 'Privacy' }];

type Props = {
  data: Bootstrap; onSave: (settings: Settings) => Promise<void>; onError: (message: string) => void;
  onAddFolder: () => Promise<void>; onRemoveFolder: (path: string) => Promise<void>;
  onKey: (provider: KeyProvider, key: string) => Promise<void>; onAudit: () => void; onRestartSetup: () => void;
  onPreviewVoice: (voiceId: string, rate: number, text: string) => Promise<void>;
  privacyExtras?: React.ReactNode; onForget: (id: number) => Promise<void>;
};

/** Settings in four tabs. Changes save on their own a moment after you stop editing. */
export default function SettingsView({ data, onSave, onError, onAddFolder, onRemoveFolder, onKey, onAudit, onRestartSetup, onPreviewVoice, privacyExtras, onForget }: Props) {
  const [tab, setTab] = useState<Tab>(() => { try { return (localStorage.getItem('buntini-settings-tab') as Tab) || 'buntini'; } catch { return 'buntini'; } });
  const [draft, setDraft] = useState(data.settings);
  const [status, setStatus] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle');
  const [problem, setProblem] = useState('');
  const edited = useRef(false);
  const latest = useRef(0);
  const update = <K extends keyof Settings>(name: K, value: Settings[K]) => { edited.current = true; setDraft(previous => ({ ...previous, [name]: value })); };
  useEffect(() => { try { localStorage.setItem('buntini-settings-tab', tab); } catch { /* Remembering the tab is optional. */ } }, [tab]);
  useEffect(() => {
    if (!edited.current) return;
    setStatus('saving');
    const attempt = ++latest.current;
    const id = window.setTimeout(async () => {
      try { await onSave(draft); if (attempt === latest.current) { setStatus('saved'); setProblem(''); } }
      catch (error) { if (attempt === latest.current) { setStatus('error'); setProblem(errorMessage(error)); } }
    }, 600);
    return () => window.clearTimeout(id);
  }, [draft]);
  return <div className="subpage settings-page">
    <div className="settings-top">
      <div className="settings-tabs" role="tablist" aria-label="Settings">{tabs.map(item => <button key={item.id} role="tab" type="button" aria-selected={tab === item.id} className={tab === item.id ? 'active' : ''} onClick={() => setTab(item.id)}>{item.label}</button>)}</div>
      <span className={`save-state ${status}`} role="status">{status === 'saving' ? <><LoaderCircle size={12} className="spin" />Saving</> : status === 'saved' ? <><Check size={12} />Saved</> : status === 'error' ? 'Not saved' : ''}</span>
    </div>
    {status === 'error' && problem && <p className="settings-problem" role="alert">{problem}</p>}
    <div role="tabpanel" aria-label={tabs.find(item => item.id === tab)?.label}>
      {tab === 'buntini' && <Customization draft={draft} update={update} onError={onError} onPreviewVoice={onPreviewVoice} onRestartSetup={onRestartSetup} />}
      {tab === 'brain' && <BrainSettings draft={draft} update={update} diagnostics={data.diagnostics} onError={onError} onKey={onKey} />}
      {tab === 'connections' && <Connections draft={draft} update={update} diagnostics={data.diagnostics} onKey={onKey} onError={onError} />}
      {tab === 'privacy' && <PrivacyTab draft={draft} update={update} roots={data.roots} onAddFolder={onAddFolder} onRemoveFolder={onRemoveFolder} onAudit={onAudit} onError={onError} memories={data.memories || []} onForget={onForget}>{privacyExtras}</PrivacyTab>}
    </div>
  </div>;
}
