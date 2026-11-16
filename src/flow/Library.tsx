import { useCallback, useEffect, useState } from 'react';
import { CircleStop, Copy, Mic, Trash2 } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { desktop, errorMessage } from '../bridge';
import { callFlow } from './api';
import { speak, useLanguage } from '../locales';
import { minutes, type Dictation, type FlowStatus, type LiveLine, type Note, type Stats } from './model';
import './flow.css';

type Tab = 'notes' | 'history' | 'stats';
const tabs: { id: Tab; label: string }[] = [{ id: 'notes', label: 'Notes' }, { id: 'history', label: 'History' }, { id: 'stats', label: 'Statistics' }];

const when = (iso: string) => {
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? '' : date.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' });
};

/** The Library window: call notes, dictation history and statistics. */
export default function Library() {
  useLanguage();
  const [tab, setTab] = useState<Tab>('notes');
  const [error, setError] = useState('');
  // The language Buntini speaks, and its changes from Settings.
  useEffect(() => {
    if (!desktop) return;
    void callFlow<string>({ op: 'language' }).then(code => speak(code)).catch(() => {});
    const spoken = listen<string>('language', event => { void speak(event.payload); });
    return () => { void spoken.then(stop => stop()); };
  }, []);
  useEffect(() => {
    const dark = matchMedia('(prefers-color-scheme: dark)');
    const follow = () => { document.documentElement.dataset.mode = dark.matches ? 'dark' : 'light'; };
    follow();
    dark.addEventListener('change', follow);
    return () => dark.removeEventListener('change', follow);
  }, []);
  return <div className="library">
    <header className="library-header">
      <h1>Library</h1>
      <div className="library-tabs" role="tablist">{tabs.map(item => <button key={item.id} type="button" role="tab" aria-selected={tab === item.id} className={tab === item.id ? 'active' : ''} onClick={() => setTab(item.id)}>{item.label}</button>)}</div>
    </header>
    {error && <div className="library-error" role="alert">{error}<button type="button" onClick={() => setError('')}>×</button></div>}
    <main className="library-body">
      {tab === 'notes' && <Notes onError={message => setError(message)} />}
      {tab === 'history' && <History onError={message => setError(message)} />}
      {tab === 'stats' && <Statistics />}
    </main>
  </div>;
}

function useRefresh(event: string, load: () => void) {
  useEffect(() => {
    load();
    if (!desktop) return;
    const subscription = listen(event, () => load());
    return () => { void subscription.then(stop => stop()); };
  }, [event, load]);
}

function Notes({ onError }: { onError: (message: string) => void }) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [status, setStatus] = useState<FlowStatus | null>(null);
  const [live, setLive] = useState<LiveLine[]>([]);
  const load = useCallback(() => {
    void callFlow<Note[]>({ op: 'notes' }).then(next => { setNotes(next); setSelected(current => current ?? next[0]?.id ?? null); }).catch(error => onError(errorMessage(error)));
    void callFlow<FlowStatus>({ op: 'status' }).then(setStatus).catch(() => {});
  }, [onError]);
  useRefresh('flow-notes', load);
  useEffect(() => {
    if (!desktop) return;
    const subscription = listen<LiveLine>('flow-notes-live', event => setLive(lines => [...lines, event.payload].slice(-200)));
    return () => { void subscription.then(stop => stop()); };
  }, []);
  const recording = status?.recording ?? null;
  const note = notes.find(item => item.id === selected) ?? null;
  const record = () => {
    setLive([]);
    void callFlow({ op: recording ? 'stopNotes' : 'startNotes' }).then(load).catch(error => onError(errorMessage(error)));
  };
  return <div className="library-split">
    <aside className="library-list">
      <button type="button" className={`library-record ${recording ? 'is-recording' : ''}`} onClick={record}>{recording ? <><CircleStop size={15} /> Stop</> : <><Mic size={15} /> Record a call</>}</button>
      {notes.map(item => <button type="button" key={item.id} className={`library-item ${item.id === selected ? 'active' : ''}`} onClick={() => setSelected(item.id)}>
        <strong>{item.title}</strong>
        <span>{when(item.createdAt)}{item.seconds ? ` · ${minutes(item.seconds)}` : ''}{item.state === 'recording' ? ' · Recording' : item.state === 'processing' ? ' · Writing' : item.state === 'failed' ? ' · Not finished' : ''}</span>
      </button>)}
    </aside>
    <section className="library-detail">
      {note?.state === 'recording' && <div className="library-live">{live.map((line, index) => <p key={index}><b>{line.side === 'mic' ? 'You' : 'Them'}</b> {line.text}</p>)}</div>}
      {note && note.state !== 'recording' && <>
        <div className="library-detail-head"><h2>{note.title}</h2><button type="button" className="icon-button" aria-label="Delete this note" title="Delete" onClick={() => void callFlow({ op: 'deleteNote', id: note.id }).then(() => { setSelected(null); load(); }).catch(error => onError(errorMessage(error)))}><Trash2 size={14} /></button></div>
        {note.summary && <p className="library-summary">{note.summary}</p>}
        {note.keyPoints.length > 0 && <><h3>Key points</h3><ul>{note.keyPoints.map((point, index) => <li key={index}>{point}</li>)}</ul></>}
        {note.todos.length > 0 && <><h3>To-dos</h3><ul className="library-todos">{note.todos.map((todo, index) => <li key={index}>{todo}</li>)}</ul></>}
        {note.lines.length > 0 && <><h3>Transcript</h3><div className="library-lines">{note.lines.map((line, index) => <p key={index}><b>{line.speaker ?? ''}</b> {line.text}</p>)}</div></>}
      </>}
    </section>
  </div>;
}

function History({ onError }: { onError: (message: string) => void }) {
  const [items, setItems] = useState<Dictation[]>([]);
  const load = useCallback(() => void callFlow<Dictation[]>({ op: 'history', limit: 200 }).then(setItems).catch(error => onError(errorMessage(error))), [onError]);
  useRefresh('flow-history', load);
  return <div className="library-history">
    {items.map(item => <article key={item.id} className="library-dictation">
      <header><span>{when(item.at)} · {item.app || 'Unknown app'}{item.pasted ? '' : ' · Copied'}</span>
        <button type="button" className="icon-button" aria-label="Copy" title="Copy" onClick={() => void navigator.clipboard.writeText(item.text).catch(() => {})}><Copy size={13} /></button>
        <button type="button" className="icon-button" aria-label="Forget" title="Forget" onClick={() => void callFlow({ op: 'forgetDictation', id: item.id }).then(load).catch(error => onError(errorMessage(error)))}><Trash2 size={13} /></button>
      </header>
      <p>{item.text}</p>
    </article>)}
  </div>;
}

function Statistics() {
  const [stats, setStats] = useState<Stats | null>(null);
  const load = useCallback(() => void callFlow<Stats>({ op: 'stats' }).then(setStats).catch(() => {}), []);
  useRefresh('flow-history', load);
  if (!stats) return null;
  const most = Math.max(1, ...stats.apps.map(app => app.words));
  const tiles: [string, string][] = [
    ['Words', stats.words.toLocaleString()],
    ['Time saved', stats.minutesSaved >= 60 ? `${(stats.minutesSaved / 60).toFixed(1)} h` : `${Math.round(stats.minutesSaved)} min`],
    ['Words a minute', String(Math.round(stats.wordsPerMinute))],
    ['Days in a row', String(stats.streak)],
  ];
  return <div className="library-stats">
    <div className="library-tiles">{tiles.map(([label, value]) => <div className="library-tile" key={label}><strong>{value}</strong><span>{label}</span></div>)}</div>
    {stats.apps.length > 0 && <><h3>Apps</h3>{stats.apps.map(app => <div className="library-bar" key={app.app}><span>{app.app}</span><i><b style={{ width: `${app.words * 100 / most}%` }} /></i><span>{app.words.toLocaleString()}</span></div>)}</>}
  </div>;
}
