import { useState } from 'react';
import { BookOpen, CloudSun, Coins, ExternalLink, Globe2, KeyRound, MapPin, Plane, ShieldCheck, ShoppingBag } from 'lucide-react';
import { errorMessage } from './bridge';
import type { Diagnostics, ResultCard, Settings, WebApproval } from './types';

const titles: Record<string, string> = {
  web_search: 'Search the web?', weather: 'Check the weather?', currency: 'Get exchange rates?', wikipedia: 'Look it up on Wikipedia?',
  places: 'Find nearby places?', ebay_search: 'Search eBay listings?', flight_search: 'Search flights?',
};
const icons = { weather: CloudSun, currency: Coins, article: BookOpen, place: MapPin, listing: ShoppingBag, flight: Plane };

/** Shows exactly what one approved request will send, and to whom, before anything leaves the Mac. */
export function ApprovalSheet({ approval, approving, onCancel, onAllow, onWebInstead }: { approval: WebApproval; approving: boolean; onCancel: () => void; onAllow: () => void; onWebInstead: () => void }) {
  const web = approval.tool === 'web_search';
  return <div className="approval-backdrop"><section className="approval-sheet" role="alertdialog" aria-modal="true" aria-labelledby="approval-title">
    <ShieldCheck size={23} /><h3 id="approval-title">{titles[approval.tool] || 'Send this request?'}</h3>
    <p>{approval.summary}. {web ? 'Public result pages may also be retrieved for a sourced answer.' : 'Only these details are sent.'}</p>
    <dl className="approval-sends"><div><dt>Sent to</dt><dd>{approval.service} <span>{approval.host}</span></dd></div>{approval.sends.map(field => <div key={field.label}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>
    {approval.note && <p className="approval-note">{approval.note}</p>}
    <small>{approval.usesKey ? `Uses your saved ${approval.service} key and counts toward your monthly request cap. ` : 'No account or key needed. '}Private files and conversation history stay on this Mac. This permission expires after one use.</small>
    <div>{!web && <button className="text-button" disabled={approving} onClick={onWebInstead}>Web search instead</button>}<button className="secondary-button" disabled={approving} onClick={onCancel}>Cancel</button><button className="primary-button" disabled={approving} autoFocus onClick={onAllow}>{approving ? 'Starting…' : 'Allow once'}</button></div>
  </section></div>;
}

/** Numbered cards matching the [n] citations in the answer. */
export function ResultCards({ cards, onOpen }: { cards: ResultCard[]; onOpen: (url: string) => void }) {
  return <div className="result-cards"><div className="section-label">LIVE RESULTS <span>{cards.length}</span></div>
    {cards.map((card, index) => {
      const Icon = icons[card.kind] || Globe2;
      return <article key={`${card.title}-${index}`} className={`result-card result-card--${card.kind}`}>
        <header><span className="source-number">{index + 1}</span><Icon size={15} /><div><strong>{card.title}</strong>{card.subtitle && <small>{card.subtitle}</small>}</div>
          {card.url && <a href={card.url} target="_blank" rel="noreferrer" aria-label={`Open ${card.title}`} onClick={event => { event.preventDefault(); onOpen(card.url!); }}><ExternalLink size={13} /></a>}</header>
        {card.fields.length > 0 && <dl>{card.fields.map(field => <div key={field.label}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>}
        <footer>{card.attribution}</footer>
      </article>;
    })}
  </div>;
}

const marketplaces = [['EBAY_US', 'United States'], ['EBAY_GB', 'United Kingdom'], ['EBAY_DE', 'Germany'], ['EBAY_FR', 'France'], ['EBAY_IT', 'Italy'], ['EBAY_ES', 'Spain'], ['EBAY_AU', 'Australia'], ['EBAY_CA', 'Canada']];

/** Optional services that need the user's own keys. Keys go straight to the macOS Keychain. */
export function ConnectedServices({ draft, update, diagnostics, onKey, onError }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onKey: (provider: 'ebay' | 'duffel', key: string) => Promise<void>; onError: (error: string) => void }) {
  const [clientId, setClientId] = useState('');
  const [clientSecret, setClientSecret] = useState('');
  const [token, setToken] = useState('');
  const [saving, setSaving] = useState('');
  const save = async (provider: 'ebay' | 'duffel', key: string, reset: () => void) => {
    setSaving(provider);
    try { await onKey(provider, key); reset(); } catch (error) { onError(errorMessage(error)); } finally { setSaving(''); }
  };
  return <section className="settings-section"><div className="settings-section-heading"><Globe2 size={15} /><h3>Live lookups</h3></div>
    <p>Weather (Open-Meteo), exchange rates (ECB), Wikipedia and nearby places (OpenStreetMap) need no key. Every request asks you first and shows exactly what is sent.</p>
    <div className="service"><div className="service-heading"><ShoppingBag size={14} /><strong>eBay listings</strong><span className="label-status">{diagnostics.ebayConfigured ? 'Saved in Keychain' : 'Not connected'}</span></div>
      <label>Client ID (App ID)<div className="key-input"><KeyRound size={13} /><input autoComplete="off" spellCheck={false} value={clientId} onChange={event => setClientId(event.target.value)} placeholder={diagnostics.ebayConfigured ? 'Saved · enter to replace' : 'YourName-App-PRD-…'} /></div></label>
      <label>Client secret (Cert ID)<div className="key-input"><KeyRound size={13} /><input type="password" autoComplete="off" value={clientSecret} onChange={event => setClientSecret(event.target.value)} placeholder="PRD-…" /></div></label>
      <label>Marketplace<select value={draft.ebayMarketplace} onChange={event => update('ebayMarketplace', event.target.value)}>{marketplaces.map(([id, label]) => <option key={id} value={id}>{label}</option>)}</select></label>
      <div className="service-actions"><button type="button" className="secondary-button" disabled={!clientId.trim() || !clientSecret.trim() || saving === 'ebay'} onClick={() => void save('ebay', JSON.stringify({ clientId: clientId.trim(), clientSecret: clientSecret.trim() }), () => { setClientId(''); setClientSecret(''); })}>{saving === 'ebay' ? 'Saving…' : 'Save eBay keys'}</button>{diagnostics.ebayConfigured && <button type="button" className="text-button danger" onClick={() => void save('ebay', '', () => {})}>Remove</button>}</div>
      <div className="field-help">Create a free app at developer.ebay.com. Sandbox keys (with “SBX”) search the eBay sandbox.</div>
    </div>
    <div className="service"><div className="service-heading"><Plane size={14} /><strong>Flights via Duffel</strong><span className="label-status">{diagnostics.duffelConfigured ? 'Saved in Keychain' : 'Not connected'}</span></div>
      <label>Access token<div className="key-input"><KeyRound size={13} /><input type="password" autoComplete="off" value={token} onChange={event => setToken(event.target.value)} placeholder={diagnostics.duffelConfigured ? '••••••••••••' : 'duffel_test_… or duffel_live_…'} /></div></label>
      <div className="service-actions"><button type="button" className="secondary-button" disabled={!token.trim() || saving === 'duffel'} onClick={() => void save('duffel', token.trim(), () => setToken(''))}>{saving === 'duffel' ? 'Saving…' : 'Save token'}</button>{diagnostics.duffelConfigured && <button type="button" className="text-button danger" onClick={() => void save('duffel', '', () => {})}>Remove</button>}</div>
      <div className="field-help">Search only. Buntini never books, holds, or pays for flights. Test tokens return sample airline data.</div>
    </div>
  </section>;
}
