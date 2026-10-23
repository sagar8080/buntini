import { useState, type ReactNode } from 'react';
import { Check, ChevronDown, Globe2, KeyRound, LoaderCircle, Plane, ShoppingBag } from 'lucide-react';
import SearchBudget from './SearchBudget';
import { errorMessage } from './bridge';
import type { Diagnostics, KeyProvider, Settings } from './types';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onKey: (provider: KeyProvider, key: string) => Promise<void>; onError: (error: string) => void };
const marketplaces = [['EBAY_US', 'United States'], ['EBAY_GB', 'United Kingdom'], ['EBAY_DE', 'Germany'], ['EBAY_FR', 'France'], ['EBAY_IT', 'Italy'], ['EBAY_ES', 'Spain'], ['EBAY_AU', 'Australia'], ['EBAY_CA', 'Canada']];

/** One row per service: status at a glance, keys only when you open it. */
function Service({ icon, name, detail, connected, children }: { icon: ReactNode; name: string; detail: string; connected: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  return <div className={`service-row ${open ? 'is-open' : ''}`}>
    <button type="button" className="service-summary" aria-expanded={open} onClick={() => setOpen(!open)}>{icon}<span><strong>{name}</strong><small>{detail}</small></span><em className={connected ? 'ready' : ''}>{connected ? <><Check size={11} />Connected</> : 'Connect'}</em><ChevronDown size={14} /></button>
    {open && <div className="service-body">{children}</div>}
  </div>;
}

function KeyField({ label, placeholder, secret = true, value, onChange }: { label: string; placeholder: string; secret?: boolean; value: string; onChange: (value: string) => void }) {
  return <div className="key-input"><KeyRound size={14} /><input type={secret ? 'password' : 'text'} autoComplete="off" spellCheck={false} aria-label={label} placeholder={placeholder} value={value} onChange={event => onChange(event.target.value)} /></div>;
}

/** The Connections tab: web search, eBay and flights, and lookup usage. */
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
    <button type="button" className="secondary-button" disabled={!ready || saving === provider} onClick={() => void save(provider, key, reset)}>{saving === provider ? <LoaderCircle size={13} className="spin" /> : 'Save'}</button>
    {connected && <button type="button" className="text-button danger" onClick={() => void save(provider, '', () => {})}>Disconnect</button>}
  </div>;
  return <>
    <section className="settings-section">
      <Service icon={<Globe2 size={18} />} name="Search API fallback" detail={draft.searchProvider === 'brave' ? 'Brave Search' : 'Tavily'} connected={searchSaved}>
        <div className="chip-row" role="radiogroup" aria-label="Fallback search provider">{(['brave', 'tavily'] as const).map(id => <button type="button" key={id} role="radio" aria-checked={draft.searchProvider === id} className={draft.searchProvider === id ? 'active' : ''} onClick={() => { update('searchProvider', id); setSearchKey(''); }}>{id === 'brave' ? 'Brave Search' : 'Tavily'}</button>)}</div>
        <p className="field-help">Chat research searches in the browser first. This optional key is used when browser research is insufficient, and for saved searches.</p>
        <KeyField label="Search API key" placeholder={searchSaved ? 'Key saved · paste to replace' : 'API key'} value={searchKey} onChange={setSearchKey} />
        {actions(draft.searchProvider, !!searchKey.trim(), searchKey.trim(), () => setSearchKey(''), searchSaved)}
      </Service>
      <Service icon={<ShoppingBag size={18} />} name="eBay" detail="Listings and prices" connected={!!diagnostics.ebayConfigured}>
        <KeyField label="eBay client ID" secret={false} placeholder="Client ID (App ID)" value={clientId} onChange={setClientId} />
        <KeyField label="eBay client secret" placeholder="Client secret (Cert ID)" value={clientSecret} onChange={setClientSecret} />
        <label className="inline-field"><span>Marketplace</span><select value={draft.ebayMarketplace} onChange={event => update('ebayMarketplace', event.target.value)}>{marketplaces.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label>
        {actions('ebay', !!clientId.trim() && !!clientSecret.trim(), JSON.stringify({ clientId: clientId.trim(), clientSecret: clientSecret.trim() }), () => { setClientId(''); setClientSecret(''); }, !!diagnostics.ebayConfigured)}
      </Service>
      <Service icon={<Plane size={18} />} name="Flights" detail="Duffel · search only" connected={!!diagnostics.duffelConfigured}>
        <KeyField label="Duffel access token" placeholder="duffel_test_… or duffel_live_…" value={token} onChange={setToken} />
        {actions('duffel', !!token.trim(), token.trim(), () => setToken(''), !!diagnostics.duffelConfigured)}
      </Service>
      <p className="field-help">Weather, exchange rates, Wikipedia and places work without a key. Buntini asks before every request.</p>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Usage</h3></div>
      <SearchBudget draft={draft} update={update} diagnostics={diagnostics} />
    </section>
  </>;
}
