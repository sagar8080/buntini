import { useEffect, useRef, useState } from 'react';
import { CalendarDays, Check, ChevronRight, LoaderCircle, Mail, X } from 'lucide-react';
import { command, errorMessage } from './bridge';
import { connectPane, connectRows, type ConnectRow } from './setupSteps';
import type { AppConnection, AppList, AppStatus } from './types';

const DISMISSED = 'buntini-connect-dismissed';
const icons: Record<string, typeof Mail> = { mail: Mail, calendar: CalendarDays };
const detail = (row: ConnectRow) => row.state === 'ready' ? 'Connected' : row.state === 'noAccount' ? 'Add an account' : row.state === 'denied' ? 'Allow in System Settings' : row.can.join(' · ');

function wasDismissed() {
  try { return localStorage.getItem(DISMISSED) === '1'; } catch { return false; }
}

type Props = {
  allowed: string[]; disabled: boolean;
  onAllow: (id: string) => Promise<void>; onConnected: () => void;
  onPane: (pane: string) => void; onError: (message: string) => void;
};

/** Mail and Calendar in one tap, on an empty chat, until they're connected or waved away. */
export default function ConnectApps({ allowed, disabled, onAllow, onConnected, onPane, onError }: Props) {
  const [apps, setApps] = useState<AppStatus[]>([]);
  const [found, setFound] = useState<Record<string, AppConnection | null>>({});
  const [working, setWorking] = useState('');
  const [dismissed, setDismissed] = useState(wasDismissed);
  useEffect(() => { command<AppList>('app_list').then(list => setApps(list.apps)).catch(error => onError(errorMessage(error))); }, []);

  async function connect(id: string) {
    setWorking(id);
    try {
      await onAllow(id);
      const connection = await command<AppConnection>('connect_app', { id });
      setFound(previous => ({ ...previous, [id]: connection }));
      if (connection === 'ready') onConnected();
    } catch (error) {
      setFound(previous => ({ ...previous, [id]: null }));
      onError(errorMessage(error));
    } finally { setWorking(''); }
  }
  // Back from System Settings, look again at whatever sent the person there.
  const waiting = useRef<string[]>([]);
  waiting.current = Object.keys(found).filter(id => found[id] === 'noAccount' || found[id] === 'denied');
  useEffect(() => {
    const look = () => {
      for (const id of waiting.current) void command<AppConnection>('connect_app', { id }).then(connection => {
        setFound(previous => ({ ...previous, [id]: connection }));
        if (connection === 'ready') onConnected();
      }).catch(() => {});
    };
    window.addEventListener('focus', look);
    return () => window.removeEventListener('focus', look);
  }, []);

  const rows = connectRows(apps, allowed, found);
  // Checks stay after connecting here; next time, a card with nothing to do doesn't come back.
  if (dismissed || !rows.length || (rows.every(row => row.state === 'ready') && !Object.keys(found).length)) return null;
  function dismiss() {
    setDismissed(true);
    try { localStorage.setItem(DISMISSED, '1'); } catch { /* It comes back next time. */ }
  }
  return <section className="connect-apps" aria-label="Connect your apps">
    <header><h2>Connect your apps</h2><button type="button" aria-label="Dismiss" onClick={dismiss}><X size={13} /></button></header>
    {rows.map(row => {
      const Icon = icons[row.id] ?? Mail, pane = connectPane(row), busy = working === row.id;
      return <button type="button" key={row.id} aria-busy={busy} disabled={row.state === 'ready' || busy || (disabled && !pane)} onClick={() => { if (pane) onPane(pane); else void connect(row.id); }}>
        <i className={`connect-icon is-${row.id}`}><Icon /></i>
        <span><strong>{row.name}</strong><small>{detail(row)}</small></span>
        {busy ? <LoaderCircle size={14} className="spin" /> : row.state === 'ready' ? <Check size={14} className="connect-done" /> : <ChevronRight size={14} />}
      </button>;
    })}
  </section>;
}
