import { useEffect, useState, type ReactNode } from 'react';
import { CalendarDays, Check, ChevronDown, CreditCard, FileText, GitMerge, GitPullRequest, Globe2, KeyRound, LoaderCircle, Send, Triangle, Workflow } from 'lucide-react';
import { errorMessage } from '../bridge';
import { t } from '../i18n.ts';
import type { Settings } from '../types';
import { saveServiceKey, serviceStatuses, showService, unwatchServices } from './api';
import { saveRequest, services, statusLine, type ServiceId, type ServiceInfo, type ServiceStatus } from './model';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void };

const icons: Record<ServiceId, ReactNode> = {
  github: <GitPullRequest size={18} />,
  gitlab: <GitMerge size={18} />,
  gitea: <GitPullRequest size={18} />,
  vercel: <Triangle size={18} />,
  stripe: <CreditCard size={18} />,
  n8n: <Workflow size={18} />,
  notion: <FileText size={18} />,
  calcom: <CalendarDays size={18} />,
  resend: <Send size={18} />,
};

/** How often the section asks again while on screen; each ask keeps the services watched. */
const REFRESH_MS = 30_000;

/** Developer services in Settings → Connections: a key each, and a host for the self-hosted ones. */
export default function ServicesSection({ draft, update, onError }: Props) {
  const [statuses, setStatuses] = useState<ServiceStatus[]>([]);
  useEffect(() => {
    let live = true;
    const unwatch = () => { void unwatchServices().catch(() => {}); };
    // Asking keeps the services polled, so only a section someone can see asks.
    const load = () => {
      if (document.visibilityState !== 'visible') return;
      void serviceStatuses().then(next => { if (live) setStatuses(next); }).catch(() => {});
    };
    const visibility = () => { if (document.visibilityState === 'visible') load(); else unwatch(); };
    load();
    const timer = setInterval(load, REFRESH_MS);
    document.addEventListener('visibilitychange', visibility);
    return () => { live = false; clearInterval(timer); document.removeEventListener('visibilitychange', visibility); unwatch(); };
  }, []);
  return <section className="settings-section">
    {services.map(info => <ServiceRow key={info.id} info={info} status={statuses.find(status => status.id === info.id)} draft={draft} update={update} onError={onError} onStatuses={setStatuses} />)}
  </section>;
}

function ServiceRow({ info, status, draft, update, onError, onStatuses }: { info: ServiceInfo; status?: ServiceStatus; draft: Settings; update: Props['update']; onError: Props['onError']; onStatuses: (next: ServiceStatus[]) => void }) {
  const [open, setOpen] = useState(false);
  const [key, setKey] = useState('');
  const savedHost = info.host ? draft.services[info.host] : '';
  const [host, setHost] = useState(savedHost);
  const [saving, setSaving] = useState(false);
  const connected = !!status?.configured;
  const problem = connected && status && ['rejected', 'failing', 'limited', 'budget'].includes(status.health) ? status.message : null;
  const hostChanged = !!info.host && host.trim() !== savedHost;
  const request = saveRequest(info, key, host, savedHost, connected);

  async function save() {
    if (!request) return;
    setSaving(true);
    try {
      if (request instanceof Error) throw request;
      // The host is saved with the key, in one step, so the key never reaches the old host.
      onStatuses(await saveServiceKey(info.id, request.key, request.host));
      if (info.host && request.host !== undefined) {
        update('services', { ...draft.services, [info.host]: request.host });
        setHost(request.host);
      }
      setKey('');
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setSaving(false);
    }
  }

  async function disconnect() {
    setSaving(true);
    try {
      onStatuses(await saveServiceKey(info.id, ''));
      setHost(savedHost);
      setKey('');
    } catch (error) {
      onError(errorMessage(error));
    } finally {
      setSaving(false);
    }
  }

  async function show() {
    try { onStatuses(await showService(info.id)); } catch (error) { onError(errorMessage(error)); }
  }

  return <div className={`service-row ${open ? 'is-open' : ''}`}>
    <button type="button" className="service-summary" aria-expanded={open} onClick={() => setOpen(!open)}>
      {icons[info.id]}
      <span><strong>{info.name}</strong><small>{statusLine(info, status)}</small></span>
      <em className={connected && status?.health !== 'rejected' ? 'ready' : ''}>{connected ? <><Check size={11} />{t('settings.connected')}</> : t('settings.connect')}</em>
      <ChevronDown size={14} />
    </button>
    {open && <div className="service-body">
      {info.host && <div className="key-input"><Globe2 size={14} /><input type="text" autoComplete="off" spellCheck={false} aria-label={t('settings.services.address', { service: info.name })} placeholder={info.hostPlaceholder} value={host} onChange={event => setHost(event.target.value)} /></div>}
      <div className="key-input"><KeyRound size={14} /><input type="password" autoComplete="off" spellCheck={false} aria-label={t('settings.services.key', { service: info.name })} placeholder={connected && !hostChanged ? t('settings.keySaved') : info.placeholder} value={key} onChange={event => setKey(event.target.value)} /></div>
      {problem && <p className="saved-search-error" role="status">{problem}</p>}
      <div className="service-actions">
        <button type="button" className="secondary-button" disabled={!request || saving} onClick={() => void save()}>{saving ? <LoaderCircle size={13} className="spin" /> : t('common.save')}</button>
        {connected && status?.hidden && <button type="button" className="text-button" onClick={() => void show()}>{t('settings.services.showOnIsland')}</button>}
        {connected && <button type="button" className="text-button danger" disabled={saving} onClick={() => void disconnect()}>{t('settings.disconnect')}</button>}
      </div>
    </div>}
  </div>;
}
