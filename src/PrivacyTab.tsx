import type { ReactNode } from 'react';
import { ChevronRight, Folder, FolderPlus, ShieldCheck, X } from 'lucide-react';
import Toggle from './Toggle';
import FileIndexSettings from './FileIndexSettings';
import type { Settings } from './types';

const shares: { id: string; label: string }[] = [
  { id: 'activity', label: 'What you’re doing' },
  { id: 'system', label: 'Mac health' },
  { id: 'files', label: 'Your files' },
  { id: 'history', label: 'Earlier messages' },
];
type Props = {
  draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; roots: string[];
  onAddFolder: () => Promise<void>; onRemoveFolder: (path: string) => Promise<void>; onAudit: () => void; onError: (error: string) => void; children?: ReactNode;
};

/** The Privacy tab: what Buntini notices, which folders it can read, and what cloud models may see. */
export default function PrivacyTab({ draft, update, roots, onAddFolder, onRemoveFolder, onAudit, onError, children }: Props) {
  const toggle = (id: string, on: boolean) => update('externalShares', on ? [...new Set([...draft.externalShares, id])] : draft.externalShares.filter(item => item !== id));
  return <>
    <section className="settings-section">
      <Toggle label="Notice what I’m doing" detail="App names and times, on this Mac only" checked={draft.petAwareness} onChange={value => update('petAwareness', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Folders</h3><span className="count-badge">{roots.length}</span></div>
      {roots.map(path => <div className="root-item" key={path}><Folder size={14} /><span title={path}>{path}</span><button type="button" aria-label={`Remove access to ${path}`} title="Remove access" onClick={() => void onRemoveFolder(path)}><X size={14} /></button></div>)}
      <div className="row-actions"><button type="button" className="secondary-button" onClick={() => void onAddFolder()}><FolderPlus size={14} /> Add folder</button><FileIndexSettings roots={roots} onError={onError} /></div>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Cloud models may see</h3></div>
      {shares.map(item => <Toggle key={item.id} label={item.label} checked={draft.externalShares.includes(item.id)} onChange={value => toggle(item.id, value)} />)}
      <p className="field-help">Your messages always go to a cloud model. Everything else stays here unless you turn it on.</p>
    </section>
    {children}
    <button type="button" className="audit-button" onClick={onAudit}><ShieldCheck size={14} /><span>Activity log</span><ChevronRight size={14} /></button>
  </>;
}
