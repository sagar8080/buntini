import { useState } from 'react';
import { Check, ChevronDown, ExternalLink, LoaderCircle, Plus, RefreshCw, Tag, X } from 'lucide-react';
import type { PriceWatch } from './types';
import { errorMessage } from './bridge';
import { locale, number, t } from './i18n.ts';

function money(price: string, currency: string) {
  try { return new Intl.NumberFormat(locale(), { style: 'currency', currency }).format(Number(price)); } catch { return `${currency} ${price}`; }
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
  return <section className="saved-searches price-watches"><div className="section-label"><Tag size={11} /> {t('watches.title')} <button className="text-button" onClick={() => setAdding(!adding)}><Plus size={12} /> {t('watches.track')}</button></div>
    {adding && <form className="saved-search-form price-watch-form" onSubmit={event => { event.preventDefault(); void act('new', async () => { await onAction('save_price_watch', { url: url.trim(), label: label.trim(), currency, threshold, intervalMinutes }); setAdding(false); setUrl(''); setLabel(''); setThreshold(''); }); }}>
      <input aria-label={t('watches.url')} type="url" value={url} onChange={event => setUrl(event.target.value)} placeholder="https://store.example/product" required maxLength={4096} />
      <input aria-label={t('watches.name')} value={label} onChange={event => setLabel(event.target.value)} placeholder={t('watches.namePlaceholder')} required maxLength={80} />
      <div><select aria-label={t('watches.currency')} value={currency} onChange={event => setCurrency(event.target.value)}>{['USD', 'EUR', 'GBP', 'CAD', 'AUD', 'INR', 'JPY'].map(item => <option key={item}>{item}</option>)}</select><input aria-label={t('watches.target')} type="number" min="0.01" step="0.01" value={threshold} onChange={event => setThreshold(event.target.value)} placeholder={t('watches.target')} required /></div>
      <div><select aria-label={t('watches.frequency')} value={intervalMinutes} onChange={event => setIntervalMinutes(Number(event.target.value))}><option value={15}>{t('watches.every15Minutes')}</option><option value={60}>{t('watches.everyHour')}</option><option value={360}>{t('watches.every6Hours')}</option><option value={1440}>{t('watches.daily')}</option></select><button type="submit" className="primary-button" disabled={!!working}>{working === 'new' ? <LoaderCircle size={12} className="spin" /> : <Check size={12} />} {t('watches.save')}</button></div>
    </form>}
    {watches.map(watch => <article className={`saved-search ${watch.enabled ? '' : 'paused'}`} key={watch.id}><div className="saved-search-heading"><Tag size={14} /><strong>{watch.label}</strong><button className="icon-button" aria-label={t('watches.delete', { name: watch.label })} disabled={!!working} onClick={() => void act(watch.id, () => onAction('delete_price_watch', { id: watch.id }))}><X size={12} /></button></div>
      <div className="price-watch-value"><strong>{watch.lastPrice ? money(watch.lastPrice, watch.currency) : t('watches.noPrice')}</strong><span>{t('watches.targetAt', { price: money(watch.threshold, watch.currency) })}</span></div>
      <div className="saved-search-info"><span>{watch.enabled ? t('watches.watching') : t('common.paused')} · {watch.intervalMinutes >= 1440 ? t('watches.daily') : t('watches.everyHours', { hours: number(watch.intervalMinutes / 60) })}</span><button className="text-button" disabled={!!working} onClick={() => void act(watch.id, () => onAction('update_price_watch', { id: watch.id, enabled: !watch.enabled, intervalMinutes: watch.intervalMinutes }))}>{watch.enabled ? t('common.pause') : t('common.resume')}</button><button className="text-button" disabled={!!working} onClick={() => void act(watch.id, () => onAction('run_price_watch', { id: watch.id }))}>{working === watch.id ? <LoaderCircle size={10} className="spin" /> : <RefreshCw size={10} />} {t('watches.checkNow')}</button></div>
      {watch.lastError && <p className="saved-search-error">{watch.lastError}</p>}
      <button className="text-button price-history-toggle" onClick={() => setExpanded(expanded === watch.id ? '' : watch.id)}><ChevronDown size={11} /> {expanded === watch.id ? t('watches.hideHistory') : t('watches.viewHistory')}</button>
      {expanded === watch.id && <div className="price-history">{watch.history.length ? <table><thead><tr><th>{t('watches.observed')}</th><th>{t('watches.price')}</th></tr></thead><tbody>{watch.history.slice(-8).reverse().map((point, index) => <tr key={`${point.observedAt}-${index}`}><td>{new Date(point.observedAt).toLocaleString(locale(), { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' })}</td><td>{money(point.price, point.currency)}</td></tr>)}</tbody></table> : <p>{t('common.noneYet')}</p>}<button className="text-button" onClick={() => void act(watch.id, () => onAction('open_source', { url: watch.url }))}><ExternalLink size={10} /> {t('watches.openPage')}</button></div>}
    </article>)}
  </section>;
}
