import { useState, type ReactNode } from 'react';
import { Check, Cloud, Cpu, KeyRound, LoaderCircle, Server, Terminal } from 'lucide-react';
import IntelligenceMark from './IntelligenceMark';
import LocalModel from './LocalModel';
import ChatGPTAccount from './ChatGPTAccount';
import Toggle from './Toggle';
import { groupShared, setGroup, shareGroups } from './cloudShares.ts';
import { command, errorMessage } from './bridge';
import { appleSettingsPane, appleStatus, cloudProviders, friendlyStatus, lmStudioAddress, offeredProviders, offersChatGPT } from './brain.ts';
import { number, t } from './i18n.ts';
import type { CloudProvider, Diagnostics, KeyProvider, ModelBackend, Settings } from './types';

export { appleStatus, backendLabel, cloudProviders } from './brain.ts';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onError: (error: string) => void; onKey: (provider: KeyProvider, key: string) => Promise<void>; onRefresh: () => Promise<void>; onNotice: (message: string) => void };

/** The Brain tab: which model answers. Buntini picks the model, prices and voice parts itself. */
export default function BrainSettings({ draft, update, diagnostics, onError, onKey, onRefresh, onNotice }: Props) {
  const apple = appleStatus(diagnostics.appleIntelligence);
  const provider = cloudProviders.find(item => item.id === draft.cloudProvider) || cloudProviders[0];
  const option = (id: ModelBackend, icon: ReactNode, title: string, detail: string, badge?: ReactNode) => <button type="button" role="radio" aria-checked={draft.modelBackend === id} className={draft.modelBackend === id ? 'active' : ''} onClick={() => { update('modelBackend', id); update('brainAuto', false); }}>{icon}<span><strong>{title}</strong><small>{detail}</small></span>{badge}</button>;
  return <>
    <section className="settings-section">
      <div className="brain-options" role="radiogroup" aria-label={t('settings.brain.title')}>
        {option('apple', <IntelligenceMark size={18} />, 'Apple Intelligence', t('settings.brain.builtIn'), <em className={apple.ready ? 'ready' : ''}>{apple.ready && <Check size={11} />}{apple.label}</em>)}
        {option('mlx', <Cpu size={18} />, t('settings.brain.local'), t('settings.brain.localDetail'))}
        {option('cloud', <Cloud size={18} />, t('brain.backend.cloud'), provider.id === 'chatgpt' ? t('settings.brain.chatgptPlan') : provider.name, diagnostics.cloudKeys?.includes(provider.id) ? <em className="ready"><Check size={11} />{provider.id === 'chatgpt' ? t('settings.brain.signedIn') : t('settings.keySaved')}</em> : undefined)}
      </div>
      {!draft.brainAuto && <button type="button" className="text-button" onClick={() => update('brainAuto', true)}>{t('settings.brain.auto')}</button>}
      {draft.modelBackend === 'apple' && apple.advice && <p className="field-help">{apple.advice}</p>}
      {draft.modelBackend === 'apple' && <OpenAppleSettings reason={diagnostics.appleIntelligence} onError={onError} />}
      {draft.modelBackend === 'apple' && <UseChatGPT reason={diagnostics.appleIntelligence} onUse={() => { update('modelBackend', 'cloud'); update('cloudProvider', 'chatgpt'); update('cloudModel', ''); update('cloudInputPrice', 0); update('cloudOutputPrice', 0); }} />}
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
    // "Only runs on this Mac" was said of one server, not of whatever is picked next.
    if (id !== provider.id) update('cloudOnThisMac', false);
  };
  async function save(value: string) {
    setSaving(true);
    try { await onKey(provider.id, value); setKey(''); } catch (error) { onError(errorMessage(error)); } finally { setSaving(false); }
  }
  const providers = <div className="chip-row" role="radiogroup" aria-label={t('settings.brain.provider')}>{offeredProviders(diagnostics.cliTools, provider.id).map(item => <button type="button" key={item.id} role="radio" aria-checked={item.id === provider.id} className={item.id === provider.id ? 'active' : ''} onClick={() => pick(item.id)}>{item.kind === 'tool' && <Terminal size={11} />}{item.name}</button>)}</div>;
  const shares = <CloudShares name={provider.name} shared={draft.externalShares} onChange={value => update('externalShares', value)} />;
  if (provider.kind === 'server') return <div className="cloud-settings">
    {providers}
    <ServerSettings key={provider.id} draft={draft} update={update} saved={saved} onError={onError} onKey={onKey} />
    {!draft.cloudOnThisMac && shares}
  </div>;
  if (provider.kind === 'tool') return <div className="cloud-settings">
    {providers}
    {!diagnostics.cliTools?.includes(provider.id) && <p className="field-help">{t('settings.brain.notInstalled')}</p>}
    {shares}
  </div>;
  if (provider.id === 'chatgpt') return <div className="cloud-settings">
    {providers}
    <ChatGPTAccount account={diagnostics.chatgptAccount} onChange={onRefresh} onError={onError} onNotice={onNotice} />
    {shares}
    {draft.cloudModel.trim() && <div className="row-actions"><span className="field-help">{t('settings.brain.using', { model: draft.cloudModel.trim() })}</span><button type="button" className="text-button" onClick={() => update('cloudModel', '')}>{t('settings.brain.useDefault')}</button></div>}
  </div>;
  const dollars = (value: number) => number(value, { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return <div className="cloud-settings">
    {providers}
    <div className="key-input"><KeyRound size={14} /><input type="password" autoComplete="off" value={key} onChange={event => setKey(event.target.value)} placeholder={saved ? t('settings.keySaved') : t('settings.brain.apiKeyFor', { provider: provider.name })} aria-label={t('settings.brain.apiKeyFor', { provider: provider.name })} /><button type="button" disabled={!key.trim() || saving} onClick={() => void save(key.trim())}>{saving ? <LoaderCircle size={13} className="spin" /> : t('common.save')}</button></div>
    {saved && <button type="button" className="text-button danger" onClick={() => void save('')}>{t('settings.brain.removeKey')}</button>}
    <div className="usage-meter">
      <div><span>{t('settings.usage.thisMonth')}</span><strong>{t('settings.brain.spend', { spent: dollars(spend), cap: dollars(draft.cloudBudgetUsd) })}</strong></div>
      <div className={`usage-track ${ratio >= .8 ? 'is-high' : ''}`} role="meter" aria-valuemin={0} aria-valuemax={draft.cloudBudgetUsd} aria-valuenow={spend} aria-label={t('settings.brain.spendAria')}><i style={{ width: `${ratio * 100}%` }} /><b style={{ left: '80%' }} /></div>
    </div>
    <label className="inline-field"><span>{t('settings.brain.cap')} ($)</span><input aria-label={t('settings.brain.capAria')} type="number" min="0" max="10000" step="0.5" value={draft.cloudBudgetUsd} onChange={event => update('cloudBudgetUsd', Number(event.target.value))} /></label>
    {shares}
    {named && <div className="row-actions"><span className="field-help">{t('settings.brain.using', { model: named })}</span><button type="button" className="text-button" onClick={() => { update('cloudModel', ''); update('cloudInputPrice', provider.input); update('cloudOutputPrice', provider.output); }}>{t('settings.brain.useDefault')}</button></div>}
  </div>;
}

/** The user's own server: its address, which of its models, and whether it stays on this Mac. */
function ServerSettings({ draft, update, saved, onError, onKey }: Pick<Props, 'draft' | 'update' | 'onError' | 'onKey'> & { saved: boolean }) {
  // LM Studio is always at its own address on this Mac; any other server is the one given.
  const lmStudio = draft.cloudProvider === 'lmstudio';
  const [address, setAddress] = useState(lmStudio ? lmStudioAddress : draft.cloudUrl);
  const [found, setFound] = useState<{ base: string; models: string[]; onThisMac: boolean } | null>(null);
  const [checking, setChecking] = useState(false);
  const [key, setKey] = useState('');
  async function connect() {
    setChecking(true);
    try {
      if (key) { await onKey(lmStudio ? 'lmstudio' : 'custom', key); setKey(''); }
      const server = await command<{ base: string; models: string[]; onThisMac: boolean }>('check_server', { address, provider: draft.cloudProvider });
      setFound(server);
      if (!lmStudio) { setAddress(server.base); update('cloudUrl', server.base); }
      if (!server.onThisMac) update('cloudOnThisMac', false);
      if (!server.models.includes(draft.cloudModel.trim())) update('cloudModel', server.models[0]);
    } catch (error) { setFound(null); onError(errorMessage(error)); } finally { setChecking(false); }
  }
  return <>
    {lmStudio
      ? <div className="row-actions"><button type="button" className="secondary-button" disabled={checking} onClick={() => void connect()}>{checking ? <LoaderCircle size={14} className="spin" /> : <Server size={13} />} {t('settings.connect')}</button></div>
      : <div className="key-input"><Server size={14} /><input autoComplete="off" spellCheck={false} value={address} onChange={event => setAddress(event.target.value)} placeholder="localhost:11434" aria-label={t('settings.brain.serverAddress')} /><button type="button" disabled={checking || !address.trim()} onClick={() => void connect()}>{checking ? <LoaderCircle size={14} className="spin" /> : t('settings.connect')}</button></div>}
    <div className="key-input"><KeyRound size={14} /><input type="password" autoComplete="off" value={key} onChange={event => setKey(event.target.value)} placeholder={saved ? t('settings.keySaved') : t('settings.brain.apiKeyOptional')} aria-label={t('settings.brain.serverKey')} /></div>
    {found && <div className="chip-row" role="radiogroup" aria-label={t('settings.brain.model')}>{found.models.map(model => <button type="button" key={model} role="radio" aria-checked={model === draft.cloudModel.trim()} className={model === draft.cloudModel.trim() ? 'active' : ''} onClick={() => update('cloudModel', model)}>{model}</button>)}</div>}
    {!found && draft.cloudModel.trim() && <span className="field-help">{t('settings.brain.using', { model: draft.cloudModel.trim() })}</span>}
    {(found?.onThisMac || (!found && draft.cloudOnThisMac)) && <Toggle label={t('settings.brain.onThisMac')} checked={draft.cloudOnThisMac} onChange={value => update('cloudOnThisMac', value)} />}
  </>;
}

/** What this cloud brain may see, asked where it is picked. */
export function CloudShares({ name, shared, onChange, setup = false }: { name: string; shared: string[]; onChange: (shared: string[]) => void; setup?: boolean }) {
  return <div className="cloud-shares" role="group" aria-label={t('settings.brain.maySee', { name })}>
    <span className="setup-label cloud-shares-label">{t('settings.brain.maySee', { name })}</span>
    {shareGroups.map(group => setup
      ? <label key={group.id} className="notification-choice cloud-share"><input type="checkbox" checked={groupShared(group, shared)} onChange={event => onChange(setGroup(group, shared, event.target.checked))} /><span>{group.label}</span></label>
      : <Toggle key={group.id} label={group.label} checked={groupShared(group, shared)} onChange={value => onChange(setGroup(group, shared, value))} />)}
  </div>;
}

/** Switches to the user's ChatGPT plan while Apple Intelligence can't answer here. */
export function UseChatGPT({ reason, onUse }: { reason?: string; onUse: () => void }) {
  if (!offersChatGPT(reason)) return null;
  return <button type="button" className="text-button" onClick={onUse}>{t('settings.brain.useChatGPT')}</button>;
}

/** Opens the System Settings pane that can get Apple Intelligence working here. */
export function OpenAppleSettings({ reason, onError }: { reason?: string; onError: (message: string) => void }) {
  const pane = appleSettingsPane(reason);
  if (!pane) return null;
  return <button type="button" className="text-button" onClick={() => void command('open_apple_settings', { pane }).catch(error => onError(errorMessage(error)))}>{t('settings.brain.openSystemSettings')}</button>;
}
