import { useState } from 'react';
import { Check, ChevronDown, ExternalLink, LoaderCircle, Plus, RefreshCw, Tag, X } from 'lucide-react';
import type { PriceWatch } from './types';
import { errorMessage } from './bridge';

function money(price: string, currency: string) {
  try { return new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(Number(price)); } catch { return `${currency} ${price}`; }
}
export default function PriceWatches({ watches, onAction, onError }: { watches: PriceWatch[]; onAction: (command: string, args: Record<string, unknown>) => Promise<unknown>; onError: (error: string) => void }) {
  const [adding, setAdding] = useState(false);
  const [url, setUrl] = useState('');
  const [label, setLabel] = useState('');
  const [currency, setCurrency] = useState('USD');
  const [threshold, setThreshold] = useState('');
  const [intervalMinutes, setIntervalMinutes] = useState(360);
  const [working, setWorking] = useState('');
  const [expanded, setExpanded] = useState('');
  async function act(id: string, action: () => Promise<unknown>) { setWorking(id); try { await action(); } catch (error) { onError(errorMessage(error)); } finally { setWorking(''); } }
  return <section className="saved-searches price-watches"><div className="section-label"><Tag size={11} /> PRICE WATCHES <button className="text-button" onClick={() => setAdding(!adding)}><Plus size={12} /> Track a price</button></div>
    {adding && <form className="saved-search-form price-watch-form" onSubmit={event => { event.preventDefault(); void act('new', async () => { await onAction('save_price_watch', { url: url.trim(), label: label.trim(), currency, threshold, intervalMinutes }); setAdding(false); setUrl(''); setLabel(''); setThreshold(''); }); }}>
      <input aria-label="Product page URL" type="url" value={url} onChange={event => setUrl(event.target.value)} placeholder="https://store.example/product" required maxLength={4096} />
      <input aria-label="Price watch name" value={label} onChange={event => setLabel(event.target.value)} placeholder="Running shoes" required maxLength={80} />
      <div><select aria-label="Currency" value={currency} onChange={event => setCurrency(event.target.value)}>{['USD', 'EUR', 'GBP', 'CAD', 'AUD', 'INR', 'JPY'].map(item => <option key={item}>{item}</option>)}</select><input aria-label="Target price" type="number" min="0.01" step="0.01" value={threshold} onChange={event => setThreshold(event.target.value)} placeholder="Target price" required /></div>
      <div><select aria-label="Price check frequency" value={intervalMinutes} onChange={event => setIntervalMinutes(Number(event.target.value))}><option value={15}>Every 15 minutes</option><option value={60}>Every hour</option><option value={360}>Every 6 hours</option><option value={1440}>Daily</option></select><button type="submit" className="primary-button" disabled={!!working}>{working === 'new' ? <LoaderCircle size={12} className="spin" /> : <Check size={12} />} Save watch</button></div>
    </form>}
    {watches.map(watch => <article className={`saved-search ${watch.enabled ? '' : 'paused'}`} key={watch.id}><div className="saved-search-heading"><Tag size={14} /><strong>{watch.label}</strong><button className="icon-button" aria-label={`Delete price watch ${watch.label}`} disabled={!!working} onClick={() => void act(watch.id, () => onAction('delete_price_watch', { id: watch.id }))}><X size={12} /></button></div>
      <div className="price-watch-value"><strong>{watch.lastPrice ? money(watch.lastPrice, watch.currency) : 'No price yet'}</strong><span>Target ≤ {money(watch.threshold, watch.currency)}</span></div>
      <div className="saved-search-info"><span>{watch.enabled ? 'Watching' : 'Paused'} · {watch.intervalMinutes >= 1440 ? 'Daily' : `${watch.intervalMinutes / 60}h`}</span><button className="text-button" disabled={!!working} onClick={() => void act(watch.id, () => onAction('update_price_watch', { id: watch.id, enabled: !watch.enabled, intervalMinutes: watch.intervalMinutes }))}>{watch.enabled ? 'Pause' : 'Resume'}</button><button className="text-button" disabled={!!working} onClick={() => void act(watch.id, () => onAction('run_price_watch', { id: watch.id }))}>{working === watch.id ? <LoaderCircle size={10} className="spin" /> : <RefreshCw size={10} />} Check now</button></div>
      {watch.lastError && <p className="saved-search-error">{watch.lastError}</p>}
      <button className="text-button price-history-toggle" onClick={() => setExpanded(expanded === watch.id ? '' : watch.id)}><ChevronDown size={11} /> {expanded === watch.id ? 'Hide' : 'View'} price history</button>
      {expanded === watch.id && <div className="price-history">{watch.history.length ? <table><thead><tr><th>Observed</th><th>Price</th></tr></thead><tbody>{watch.history.slice(-8).reverse().map((point, index) => <tr key={`${point.observedAt}-${index}`}><td>{new Date(point.observedAt).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}</td><td>{money(point.price, point.currency)}</td></tr>)}</tbody></table> : <p>None yet</p>}<button className="text-button" onClick={() => void act(watch.id, () => onAction('open_source', { url: watch.url }))}><ExternalLink size={10} /> Open product page</button></div>}
    </article>)}
  </section>;
}
