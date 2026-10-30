import { useState, type ReactNode } from 'react';
import { Check, Cloud, Cpu, KeyRound, LoaderCircle } from 'lucide-react';
import IntelligenceMark from './IntelligenceMark';
import LocalModel from './LocalModel';
import ChatGPTAccount from './ChatGPTAccount';
import { errorMessage } from './bridge';
import { appleStatus, cloudProviders, friendlyStatus } from './brain.ts';
import type { CloudProvider, Diagnostics, KeyProvider, ModelBackend, Settings } from './types';

export { appleStatus, backendLabel, cloudProviders } from './brain.ts';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onError: (error: string) => void; onKey: (provider: KeyProvider, key: string) => Promise<void>; onRefresh: () => Promise<void>; onNotice: (message: string) => void };

/** The Brain tab: which model answers. Buntini picks the model, prices and voice parts itself. */
export default function BrainSettings({ draft, update, diagnostics, onError, onKey, onRefresh, onNotice }: Props) {
  const apple = appleStatus(diagnostics.appleIntelligence);
  const provider = cloudProviders.find(item => item.id === draft.cloudProvider) || cloudProviders[0];
  const option = (id: ModelBackend, icon: ReactNode, title: string, detail: string, badge?: ReactNode) => <button type="button" role="radio" aria-checked={draft.modelBackend === id} className={draft.modelBackend === id ? 'active' : ''} onClick={() => update('modelBackend', id)}>{icon}<span><strong>{title}</strong><small>{detail}</small></span>{badge}</button>;
  return <>
    <section className="settings-section">
      <div className="brain-options" role="radiogroup" aria-label="Brain">
        {option('apple', <IntelligenceMark size={18} />, 'Apple Intelligence', 'Private and built in', <em className={apple.ready ? 'ready' : ''}>{apple.ready && <Check size={11} />}{apple.label}</em>)}
        {option('mlx', <Cpu size={18} />, 'Local model', 'Qwen or Gemma, run by MLX')}
        {option('cloud', <Cloud size={18} />, 'Cloud model', provider.id === 'chatgpt' ? 'Your own ChatGPT plan' : `${provider.name} with your key`, diagnostics.cloudKeys?.includes(provider.id) ? <em className="ready"><Check size={11} />{provider.id === 'chatgpt' ? 'Signed in' : 'Key saved'}</em> : undefined)}
      </div>
      {draft.modelBackend === 'apple' && apple.advice && <p className="field-help">{apple.advice}</p>}
      {draft.modelBackend === 'mlx' && <LocalModel draft={draft} update={update} onError={onError} />}
      {draft.modelBackend === 'cloud' && <CloudSettings draft={draft} update={update} diagnostics={diagnostics} onError={onError} onKey={onKey} onRefresh={onRefresh} onNotice={onNotice} />}
      {/* Only a brain that can't answer yet needs a word here. */}
      {!diagnostics.modelReady && <p className="worker-status"><span className="status-dot amber" />{friendlyStatus(diagnostics.workerStatus, diagnostics.modelReady, draft.modelBackend)}</p>}
    </section>
  </>;
}

function CloudSettings({ draft, update, diagnostics, onError, onKey, onRefresh, onNotice }: Props) {
  const [key, setKey] = useState('');
  const [saving, setSaving] = useState(false);
  const provider = cloudProviders.find(item => item.id === draft.cloudProvider) || cloudProviders[0];
  const saved = !!diagnostics.cloudKeys?.includes(provider.id);
  const spend = diagnostics.cloudSpendUsd ?? 0;
  // Only an earlier version let people name a model; offer the way back to the suggested one.
  const named = draft.cloudModel.trim() && draft.cloudModel.trim() !== provider.model ? draft.cloudModel.trim() : '';
  const ratio = draft.cloudBudgetUsd > 0 ? Math.min(1, spend / draft.cloudBudgetUsd) : 1;
  const pick = (id: CloudProvider) => {
    const next = cloudProviders.find(item => item.id === id)!;
    update('cloudProvider', id); update('cloudModel', ''); update('cloudInputPrice', next.input); update('cloudOutputPrice', next.output); setKey('');
  };
  async function save(value: string) {
    setSaving(true);
    try { await onKey(provider.id, value); setKey(''); } catch (error) { onError(errorMessage(error)); } finally { setSaving(false); }
  }
  const providers = <div className="chip-row" role="radiogroup" aria-label="Cloud provider">{cloudProviders.map(item => <button type="button" key={item.id} role="radio" aria-checked={item.id === provider.id} className={item.id === provider.id ? 'active' : ''} onClick={() => pick(item.id)}>{item.name}</button>)}</div>;
  if (provider.id === 'chatgpt') return <div className="cloud-settings">
    {providers}
    <ChatGPTAccount account={diagnostics.chatgptAccount} onChange={onRefresh} onError={onError} onNotice={onNotice} />
    {draft.cloudModel.trim() && <div className="row-actions"><span className="field-help">Using {draft.cloudModel.trim()}, the model you picked earlier.</span><button type="button" className="text-button" onClick={() => update('cloudModel', '')}>Use your plan’s default</button></div>}
    <p className="field-help">It sees only what you allow in Privacy.</p>
  </div>;
  return <div className="cloud-settings">
    {providers}
    <div className="key-input"><KeyRound size={14} /><input type="password" autoComplete="off" value={key} onChange={event => setKey(event.target.value)} placeholder={saved ? 'Key saved · paste to replace' : `${provider.name} API key`} aria-label={`${provider.name} API key`} /><button type="button" disabled={!key.trim() || saving} onClick={() => void save(key.trim())}>{saving ? <LoaderCircle size={13} className="spin" /> : 'Save'}</button></div>
    {saved && <button type="button" className="text-button danger" onClick={() => void save('')}>Remove key</button>}
    <div className="usage-meter">
      <div><span>This month</span><strong>${spend.toFixed(2)} of ${draft.cloudBudgetUsd.toFixed(2)}</strong></div>
      <div className={`usage-track ${ratio >= .8 ? 'is-high' : ''}`} role="meter" aria-valuemin={0} aria-valuemax={draft.cloudBudgetUsd} aria-valuenow={spend} aria-label="Cloud spending this month"><i style={{ width: `${ratio * 100}%` }} /><b style={{ left: '80%' }} /></div>
    </div>
    <label className="inline-field"><span>Monthly cap ($)</span><input aria-label="Monthly cloud spending cap in USD" type="number" min="0" max="10000" step="0.5" value={draft.cloudBudgetUsd} onChange={event => update('cloudBudgetUsd', Number(event.target.value))} /></label>
    {named && <div className="row-actions"><span className="field-help">Using {named}, the model you named earlier.</span><button type="button" className="text-button" onClick={() => { update('cloudModel', ''); update('cloudInputPrice', provider.input); update('cloudOutputPrice', provider.output); }}>Use the suggested model</button></div>}
    <p className="field-help">Spending is estimated from {provider.name}’s list prices; their invoice is final. It sees only what you allow in Privacy.</p>
  </div>;
}
