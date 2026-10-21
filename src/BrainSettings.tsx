import { useState } from 'react';
import { Apple, Check, Cloud, Cpu, KeyRound, LoaderCircle, WifiOff } from 'lucide-react';
import SetupChecks from './SetupChecks';
import { errorMessage } from './bridge';
import type { CloudProvider, Diagnostics, KeyProvider, ModelBackend, Settings } from './types';

/** Plain-language Apple Intelligence status from the helper's availability report. */
export function appleStatus(reason?: string): { ready: boolean; label: string; advice?: string } {
  switch (reason) {
    case 'available': return { ready: true, label: 'Ready on this Mac' };
    case 'appleIntelligenceNotEnabled': return { ready: false, label: 'Turned off', advice: 'Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri.' };
    case 'modelNotReady': return { ready: false, label: 'Downloading', advice: 'macOS is still downloading the model. Try again in a few minutes.' };
    case 'deviceNotEligible': return { ready: false, label: 'Not supported', advice: 'This Mac doesn’t support Apple Intelligence.' };
    case 'unsupported': return { ready: false, label: 'Needs macOS 26', advice: 'Apple Intelligence needs macOS 26 on an Apple silicon Mac.' };
    case 'missing': return { ready: false, label: 'Missing', advice: 'The Apple Intelligence helper is missing from this installation.' };
    case 'checking': case undefined: return { ready: false, label: 'Checking…' };
    default: return { ready: false, label: 'Unavailable', advice: 'Apple Intelligence isn’t available right now.' };
  }
}

// Mirrors src-tauri/src/cloud.rs PROVIDERS (suggested models and list prices, October 2026).
export const cloudProviders: { id: CloudProvider; name: string; model: string; input: number; output: number; keys: string }[] = [
  { id: 'anthropic', name: 'Anthropic', model: 'claude-opus-5-5', input: 4, output: 20, keys: 'console.anthropic.com' },
  { id: 'openai', name: 'OpenAI', model: 'gpt-5-mini', input: 0.25, output: 2, keys: 'platform.openai.com' },
  { id: 'gemini', name: 'Google Gemini', model: 'gemini-2.5-flash', input: 0.3, output: 2.5, keys: 'aistudio.google.com' },
];
const shares: { id: string; label: string; detail: string }[] = [
  { id: 'activity', label: 'What you’re doing', detail: 'App names and active times from this Mac, never window contents.' },
  { id: 'system', label: 'Mac health', detail: 'Battery, CPU, memory and storage.' },
  { id: 'files', label: 'Your files', detail: 'File names and short excerpts from the folders you allowed.' },
  { id: 'history', label: 'Earlier conversation', detail: 'So it can follow along. Turns that used data you haven’t shared stay private.' },
];

export const backendLabel: Record<ModelBackend, string> = { apple: 'Apple Intelligence', mlx: 'Local MLX model', cloud: 'Cloud model' };

/** Which model answers: Apple Intelligence, a local MLX model or (later) a cloud model. */
export default function BrainSettings({ draft, update, diagnostics, onError, onKey }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onError: (error: string) => void; onKey: (provider: KeyProvider, key: string) => Promise<void> }) {
  const apple = appleStatus(diagnostics.appleIntelligence);
  const choose = (backend: ModelBackend) => update('modelBackend', backend);
  return <section className="settings-section"><div className="settings-section-heading"><Cpu size={15} /><h3>Brain</h3><span className={`pill ${diagnostics.modelReady ? 'good' : ''}`}>{diagnostics.modelReady ? 'Ready' : 'Needs setup'}</span></div>
    <p>Choose which model answers you. On-device models can use everything on this Mac, because nothing leaves it.</p>
    <div className="brain-options" role="radiogroup" aria-label="Model">
      <button type="button" role="radio" aria-checked={draft.modelBackend === 'apple'} className={draft.modelBackend === 'apple' ? 'active' : ''} onClick={() => choose('apple')}>
        <Apple size={16} /><span><strong>Apple Intelligence</strong><small>Built into macOS · private · fast · no setup</small></span>
        <em className={apple.ready ? 'ready' : ''}>{apple.ready && <Check size={10} />}{apple.label}</em>
      </button>
      <button type="button" role="radio" aria-checked={draft.modelBackend === 'mlx'} className={draft.modelBackend === 'mlx' ? 'active' : ''} onClick={() => choose('mlx')}>
        <Cpu size={16} /><span><strong>Local MLX model</strong><small>Your own downloaded model (for example Qwen) · bigger and slower</small></span>
      </button>
      <button type="button" role="radio" aria-checked={draft.modelBackend === 'cloud'} className={draft.modelBackend === 'cloud' ? 'active' : ''} onClick={() => choose('cloud')}>
        <Cloud size={16} /><span><strong>Cloud model</strong><small>Anthropic, OpenAI or Gemini with your own key · sees only what you share</small></span>
      </button>
    </div>
    {draft.modelBackend === 'apple' && apple.advice && <div className="field-help">{apple.advice}</div>}
    {draft.modelBackend === 'mlx' && <>
      <label>Python interpreter<input value={draft.pythonPath} onChange={event => update('pythonPath', event.target.value)} placeholder="/path/to/venv/bin/python" spellCheck={false} /></label>
      <label>MLX model folder<input value={draft.modelPath} onChange={event => update('modelPath', event.target.value)} placeholder="/path/to/local/model" spellCheck={false} /></label>
      <div className="field-help"><WifiOff size={11} /> Local model files only. Buntini never downloads models.</div>
      <SetupChecks onError={onError} />
    </>}
    {draft.modelBackend === 'cloud' && <CloudSettings draft={draft} update={update} diagnostics={diagnostics} onError={onError} onKey={onKey} />}
    <div className="worker-status"><span className={`status-dot ${diagnostics.modelReady ? '' : 'amber'}`} />{diagnostics.workerStatus}</div>
  </section>;
}

function CloudSettings({ draft, update, diagnostics, onError, onKey }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onError: (error: string) => void; onKey: (provider: KeyProvider, key: string) => Promise<void> }) {
  const [key, setKey] = useState('');
  const [saving, setSaving] = useState(false);
  const provider = cloudProviders.find(item => item.id === draft.cloudProvider) || cloudProviders[0];
  const saved = !!diagnostics.cloudKeys?.includes(provider.id);
  const spend = diagnostics.cloudSpendUsd ?? 0;
  const ratio = draft.cloudBudgetUsd > 0 ? Math.min(1, spend / draft.cloudBudgetUsd) : 1;
  const pick = (id: CloudProvider) => {
    const next = cloudProviders.find(item => item.id === id)!;
    update('cloudProvider', id); update('cloudModel', ''); update('cloudInputPrice', next.input); update('cloudOutputPrice', next.output); setKey('');
  };
  const toggle = (id: string, on: boolean) => update('externalShares', on ? [...new Set([...draft.externalShares, id])] : draft.externalShares.filter(item => item !== id));
  async function save(value: string) {
    setSaving(true);
    try { await onKey(provider.id, value); setKey(''); } catch (error) { onError(errorMessage(error)); } finally { setSaving(false); }
  }
  return <div className="cloud-settings">
    <label>Provider<select value={provider.id} onChange={event => pick(event.target.value as CloudProvider)}>{cloudProviders.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
    <label>Model<input value={draft.cloudModel} onChange={event => update('cloudModel', event.target.value.trim())} placeholder={provider.model} spellCheck={false} maxLength={100} /></label>
    <label>API key <span className="label-status">{saved ? 'Saved in Keychain' : 'Not connected'}</span><div className="key-input"><KeyRound size={13} /><input type="password" autoComplete="off" value={key} onChange={event => setKey(event.target.value)} placeholder={saved ? '••••••••••••••••' : `Paste your ${provider.name} key`} aria-label={`${provider.name} API key`} /><button type="button" disabled={!key.trim() || saving} onClick={() => void save(key.trim())}>{saving ? <LoaderCircle size={12} className="spin" /> : 'Save key'}</button></div></label>
    <div className="field-help">Create a key at {provider.keys}. It stays in your Mac’s Keychain.{saved && <button type="button" className="text-button danger" onClick={() => void save('')}>Remove</button>}</div>
    <div className="usage-meter">
      <div><span>Spent this month (estimate)</span><strong>${spend.toFixed(2)} of ${draft.cloudBudgetUsd.toFixed(2)}</strong></div>
      <div className={`usage-track ${ratio >= .8 ? 'is-high' : ''}`} role="meter" aria-valuemin={0} aria-valuemax={draft.cloudBudgetUsd} aria-valuenow={spend} aria-label="Cloud spending this month"><i style={{ width: `${ratio * 100}%` }} /><b style={{ left: '80%' }} /></div>
    </div>
    <label className="inline-field"><span>Monthly spending cap<small>Buntini stops using the cloud model when it’s reached</small></span><input aria-label="Monthly cloud spending cap in USD" type="number" min="0" max="10000" step="0.5" value={draft.cloudBudgetUsd} onChange={event => update('cloudBudgetUsd', Number(event.target.value))} required /></label>
    <label className="inline-field"><span>Input price<small>USD per million tokens</small></span><input aria-label="Input price per million tokens" type="number" min="0" max="1000" step="0.01" value={draft.cloudInputPrice} onChange={event => update('cloudInputPrice', Number(event.target.value))} required /></label>
    <label className="inline-field"><span>Output price<small>USD per million tokens, thinking included</small></span><input aria-label="Output price per million tokens" type="number" min="0" max="1000" step="0.01" value={draft.cloudOutputPrice} onChange={event => update('cloudOutputPrice', Number(event.target.value))} required /></label>
    <div className="field-help">Each message reserves a cautious estimate before it’s sent, then settles to what {provider.name} reports. Prices change, so check your provider’s pricing page; their invoice is authoritative.</div>
    <div className="section-label cloud-shares-label">WHAT THIS MODEL MAY SEE</div>
    <p className="cloud-shares-intro">Your messages always go to {provider.name}. Nothing else from this Mac does unless you turn it on here.</p>
    {shares.map(item => <label className="notification-choice cloud-share" key={item.id}><input type="checkbox" checked={draft.externalShares.includes(item.id)} onChange={event => toggle(item.id, event.target.checked)} /><span>{item.label}<small>{item.detail}</small></span></label>)}
  </div>;
}
