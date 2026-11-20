import { useState, type ReactNode } from 'react';
import { Check, ChevronDown, Globe2, KeyRound, LoaderCircle, Plane, ShoppingBag } from 'lucide-react';
import SearchBudget from './SearchBudget';
import AddedTools from './AddedTools';
import ServicesSection from './services/ServicesSection';
import { errorMessage } from './bridge';
import { locale, t } from './i18n.ts';
import type { Diagnostics, KeyProvider, Settings } from './types';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onKey: (provider: KeyProvider, key: string) => Promise<void>; onError: (error: string) => void };
const marketplaces = [['EBAY_US', 'US', 'United States'], ['EBAY_GB', 'GB', 'United Kingdom'], ['EBAY_DE', 'DE', 'Germany'], ['EBAY_FR', 'FR', 'France'], ['EBAY_IT', 'IT', 'Italy'], ['EBAY_ES', 'ES', 'Spain'], ['EBAY_AU', 'AU', 'Australia'], ['EBAY_CA', 'CA', 'Canada']];

/** A country by its region code: English from the list, any other language from the system's own names. */
function country(region: string, english: string): string {
  const code = locale();
  if (!code) return english;
  try { return new Intl.DisplayNames([code], { type: 'region' }).of(region) ?? english; } catch { return english; }
}

/** One row per service: status at a glance, keys only when you open it. */
function Service({ icon, name, detail, connected, children }: { icon: ReactNode; name: string; detail: string; connected: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return <div className={`service-row ${open ? 'is-open' : ''}`}>
    <button type="button" className="service-summary" aria-expanded={open} onClick={() => setOpen(!open)}>{icon}<span><strong>{name}</strong><small>{detail}</small></span><em className={connected ? 'ready' : ''}>{connected ? <><Check size={11} />{t('settings.connected')}</> : t('settings.connect')}</em><ChevronDown size={14} /></button>
    {open && <div className="service-body">{children}</div>}
  </div>;
}

function KeyField({ label, placeholder, secret = true, value, onChange }: { label: string; placeholder: string; secret?: boolean; value: string; onChange: (value: string) => void }) {
  return <div className="key-input"><KeyRound size={14} /><input type={secret ? 'password' : 'text'} autoComplete="off" spellCheck={false} aria-label={label} placeholder={placeholder} value={value} onChange={event => onChange(event.target.value)} /></div>;
}

/** The Connections tab: web search, eBay, flights, developer services and tools you added. Search usage shows once a key is in. */
export default function Connections({ draft, update, diagnostics, onKey, onError }: Props) {
  const [searchKey, setSearchKey] = useState('');
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [token, setToken] = useState('');
  const [saving, setSaving] = useState('');
  const searchSaved = draft.searchProvider === 'brave' ? diagnostics.braveConfigured : diagnostics.tavilyConfigured;
  async function save(provider: KeyProvider, key: string, reset: () => void) {
    setSaving(provider);
    try { await onKey(provider, key); reset(); } catch (error) { onError(errorMessage(error)); } finally { setSaving(''); }
  }
  const actions = (provider: KeyProvider, ready: boolean, key: string, reset: () => void, connected: boolean) => <div className="service-actions">
    <button type="button" className="secondary-button" disabled={!ready || saving === provider} onClick={() => void save(provider, key, reset)}>{saving === provider ? <LoaderCircle size={13} className="spin" /> : t('common.save')}</button>
    {connected && <button type="button" className="text-button danger" onClick={() => void save(provider, '', () => {})}>{t('settings.disconnect')}</button>}
  </div>;
  return <>
    <section className="settings-section">
      <Service icon={<Globe2 size={18} />} name={t('settings.connections.webSearch')} detail={draft.searchProvider === 'brave' ? 'Brave Search' : 'Tavily'} connected={searchSaved}>
        <div className="chip-row" role="radiogroup" aria-label={t('settings.connections.searchProvider')}>{(['brave', 'tavily'] as const).map(id => <button type="button" key={id} role="radio" aria-checked={draft.searchProvider === id} className={draft.searchProvider === id ? 'active' : ''} onClick={() => { update('searchProvider', id); setSearchKey(''); }}>{id === 'brave' ? 'Brave Search' : 'Tavily'}</button>)}</div>
        <KeyField label={t('settings.connections.searchKey')} placeholder={searchSaved ? t('settings.keySaved') : t('settings.apiKey')} value={searchKey} onChange={setSearchKey} />
        {actions(draft.searchProvider, !!searchKey.trim(), searchKey.trim(), () => setSearchKey(''), searchSaved)}
        {searchSaved && <SearchBudget draft={draft} update={update} diagnostics={diagnostics} />}
      </Service>
      <Service icon={<ShoppingBag size={18} />} name="eBay" detail={t('settings.connections.ebayDetail')} connected={!!diagnostics.ebayConfigured}>
        <KeyField label={t('settings.connections.ebayClientId')} secret={false} placeholder={t('settings.connections.clientIdHint')} value={clientId} onChange={setClientId} />
        <KeyField label={t('settings.connections.ebaySecret')} placeholder={t('settings.connections.secretHint')} value={clientSecret} onChange={setClientSecret} />
        <label className="inline-field"><span>{t('settings.connections.marketplace')}</span><select value={draft.ebayMarketplace} onChange={event => update('ebayMarketplace', event.target.value)}>{marketplaces.map(([id, region, english]) => <option key={id} value={id}>{country(region, english)}</option>)}</select></label>
        {actions('ebay', !!clientId.trim() && !!clientSecret.trim(), JSON.stringify({ clientId: clientId.trim(), clientSecret: clientSecret.trim() }), () => { setClientId(''); setClientSecret(''); }, !!diagnostics.ebayConfigured)}
      </Service>
      <Service icon={<Plane size={18} />} name={t('settings.connections.flights')} detail={t('settings.connections.flightsDetail')} connected={!!diagnostics.duffelConfigured}>
        <KeyField label={t('settings.connections.duffelToken')} placeholder="duffel_live_…" value={token} onChange={setToken} />
        {actions('duffel', !!token.trim(), token.trim(), () => setToken(''), !!diagnostics.duffelConfigured)}
      </Service>
    </section>
    <ServicesSection draft={draft} update={update} onError={onError} />
    <AddedTools onError={onError} />
  </>;
}
