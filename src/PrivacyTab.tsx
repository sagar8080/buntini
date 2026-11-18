import type { ReactNode } from 'react';
import { ChevronRight, Folder, FolderPlus, ShieldCheck, X } from 'lucide-react';
import Toggle from './Toggle';
import MacUpdates from './MacUpdates';
import { t } from './i18n.ts';
import { cloudProviders } from './brain.ts';
import type { Memory, Settings } from './types';

const shares: { id: string; labelKey: string }[] = [
  { id: 'activity', labelKey: 'settings.privacy.share.activity' },
  { id: 'system', labelKey: 'settings.privacy.share.system' },
  { id: 'files', labelKey: 'settings.privacy.share.files' },
  { id: 'history', labelKey: 'settings.privacy.share.history' },
  { id: 'memory', labelKey: 'settings.privacy.share.memory' },
  { id: 'mail', labelKey: 'settings.privacy.share.mail' },
  { id: 'calendar', labelKey: 'settings.privacy.share.calendar' },
  { id: 'notes', labelKey: 'settings.privacy.share.notes' },
  { id: 'music', labelKey: 'settings.privacy.share.music' },
  { id: 'photos', labelKey: 'settings.privacy.share.photos' },
  { id: 'browser', labelKey: 'settings.privacy.share.browser' },
  { id: 'selection', labelKey: 'settings.privacy.share.selection' },
  { id: 'contacts', labelKey: 'settings.privacy.share.contacts' },
  { id: 'shortcuts', labelKey: 'settings.privacy.share.shortcuts' },
  { id: 'tools', labelKey: 'settings.privacy.share.tools' },
  { id: 'dictation', labelKey: 'settings.privacy.share.dictation' },
];
type Props = {
  draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; roots: string[];
  onAddFolder: () => Promise<void>; onRemoveFolder: (path: string) => Promise<void>; onAudit: () => void; onError: (error: string) => void; children?: ReactNode;
  memories: Memory[]; onForget: (id: number) => Promise<void>; onPolicy?: () => void;
};

/** The Privacy tab: what Buntini notices, which folders it can read, and what cloud models may see. */
export default function PrivacyTab({ draft, update, roots, onAddFolder, onRemoveFolder, onAudit, onError, children, memories, onForget, onPolicy }: Props) {
  const toggle = (id: string, on: boolean) => update('externalShares', on ? [...new Set([...draft.externalShares, id])] : draft.externalShares.filter(item => item !== id));
  // The brains asked once whether they may work questions out: the answer, to change.
  const asked = [...draft.workoutBrains, ...draft.workoutDeclined];
  const workOut = (brain: string, on: boolean) => {
    const others = (list: string[]) => list.filter(item => item !== brain);
    update('workoutBrains', on ? [...others(draft.workoutBrains), brain] : others(draft.workoutBrains));
    update('workoutDeclined', on ? others(draft.workoutDeclined) : [...others(draft.workoutDeclined), brain]);
  };
  return <>
    <section className="settings-section">
      <Toggle label={t('settings.privacy.notice')} detail={t('settings.privacy.noticeDetail')} checked={draft.petAwareness} onChange={value => update('petAwareness', value)} />
      <Toggle label={t('settings.privacy.checkUpdates')} detail={t('settings.privacy.checkUpdatesDetail')} checked={draft.island.updates} onChange={value => update('island', { ...draft.island, updates: value })} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.privacy.folders')}</h3></div>
      {roots.map(path => <div className="root-item" key={path}><Folder size={14} /><span title={path}>{path}</span><button type="button" aria-label={t('settings.privacy.removeAccessTo', { path })} title={t('settings.privacy.removeAccess')} onClick={() => void onRemoveFolder(path)}><X size={14} /></button></div>)}
      <div className="row-actions"><button type="button" className="secondary-button" onClick={() => void onAddFolder()}><FolderPlus size={14} /> {t('settings.privacy.addFolder')}</button></div>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.privacy.memory')}</h3></div>
      {memories.length ? memories.map(memory => <div className="root-item" key={memory.id}><span title={memory.fact}>{memory.fact}</span><button type="button" aria-label={t('settings.privacy.forgetFact', { fact: memory.fact })} title={t('common.forget')} onClick={() => void onForget(memory.id)}><X size={14} /></button></div>)
        : <p className="field-help">{t('settings.privacy.nothingYet')}</p>}
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.privacy.cloudMaySee')}</h3></div>
      {shares.map(item => <Toggle key={item.id} label={t(item.labelKey)} checked={draft.externalShares.includes(item.id)} onChange={value => toggle(item.id, value)} />)}
      {asked.map(brain => <Toggle key={`workout-${brain}`} label={t('settings.privacy.workOut', { brain: cloudProviders.find(item => item.id === brain)?.name ?? brain })} checked={draft.workoutBrains.includes(brain)} onChange={value => workOut(brain, value)} />)}
    </section>
    {children}
    <MacUpdates onError={onError} />
    <button type="button" className="audit-button" onClick={onAudit}><ShieldCheck size={14} /><span>{t('settings.privacy.activityLog')}</span><ChevronRight size={14} /></button>
    {onPolicy && <button type="button" className="audit-button" onClick={onPolicy}><ShieldCheck size={14} /><span>{t('settings.privacy.policy')}</span><ChevronRight size={14} /></button>}
  </>;
}
