import { useState, type RefObject } from 'react';
import { Check, Copy, ExternalLink, FileText, Laptop, LoaderCircle, Square, Volume2, VolumeX } from 'lucide-react';
import Pet from './Pet';
import { ResultCards } from './Online';
import { backendLabel } from './BrainSettings';
import { lookFrom } from './petOptions';
import { threadItems } from './thread';
import { fileSize, relativeTime, safeHostname } from './format';
import type { ModelBackend, Settings, TaskRecord } from './types';

type Props = {
  turns: TaskRecord[]; live: TaskRecord | null; boundary?: string | null; busy: boolean; activity: string;
  settings?: Settings; focusId: string; speaking: boolean; scroller: RefObject<HTMLDivElement | null>;
  canLoadEarlier: boolean; onLoadEarlier: () => Promise<void>;
  onOpen: (url: string) => void; onSpeak: (text: string) => void; onStopSpeaking: () => void;
};

/** The ongoing chat: your messages on the right, Buntini's replies (with any cards and sources) on the left. */
export default function Conversation({ turns, live, boundary, busy, activity, settings, focusId, speaking, scroller, canLoadEarlier, onLoadEarlier, onOpen, onSpeak, onStopSpeaking }: Props) {
  const [reading, setReading] = useState('');
  const [copied, setCopied] = useState('');
  const [loading, setLoading] = useState(false);
  async function loadEarlier() {
    const element = scroller.current;
    const before = element?.scrollHeight ?? 0;
    setLoading(true);
    try { await onLoadEarlier(); } finally { setLoading(false); }
    // Keep the reader's place while older turns appear above.
    requestAnimationFrame(() => { if (element) { element.style.scrollBehavior = 'auto'; element.scrollTop += element.scrollHeight - before; element.style.scrollBehavior = ''; } });
  }
  const turn = (item: TaskRecord, isLive: boolean) => {
    const done = !isLive && item.state === 'completed';
    const working = isLive && busy;
    return <article className={`turn ${focusId && focusId === item.id ? 'is-focused' : ''}`} id={item.id ? `turn-${item.id}` : undefined} key={item.id || 'live'}>
      <div className="bubble-row is-you"><div className="bubble bubble-you">{item.prompt}</div></div>
      <div className="bubble-row is-buntini">
        <span className="bubble-avatar"><Pet state={working ? 'thinking' : 'idle'} avatar={settings?.avatar || 'cat'} look={settings ? lookFrom(settings) : undefined} /></span>
        <div className={`bubble bubble-buntini ${item.state === 'error' ? 'is-error' : ''}`}>
          {item.answer && item.state !== 'error' && <div className="answer-text">{item.answer}</div>}
          {working && !item.answer && <div className="typing" role="status" aria-label={activity || 'Thinking'}><i /><i /><i /><span>{activity}</span></div>}
          {item.system && <SystemCard snapshot={item.system} />}
          {!!item.cards?.length && <ResultCards cards={item.cards} onOpen={onOpen} />}
          {item.files.length > 0 && <div className="file-results">{item.files.map(file => <article className="file-result" key={file.path}><FileText size={18} /><div><strong>{file.name}</strong><span title={file.path}>{file.path}</span><small>{fileSize(file.size)} · {relativeTime(file.modifiedAt)}</small>{file.excerpt && <p>{file.excerpt}</p>}</div></article>)}</div>}
          {item.sources.length > 0 && <div className="sources"><div className="section-label">SOURCES <span>{item.sources.length}</span></div>{item.sources.map((source, index) => <a key={`${source.url}-${index}`} className="source" href={source.url} target="_blank" rel="noreferrer" onClick={event => { event.preventDefault(); onOpen(source.url); }}><span className="source-number">{index + 1}</span><div><strong>{source.title}</strong><small>{safeHostname(source.url)} · {relativeTime(source.retrievedAt)}</small></div><ExternalLink size={13} /></a>)}</div>}
          {!!item.prices?.length && <div className="product-comparison"><div className="section-label">PUBLISHED OFFERS</div>{item.prices.map((offer, index) => <article key={`${offer.url}-${index}`}><strong>{offer.product}</strong><div><b>{offer.currency} {offer.price}{offer.isAggregate && offer.highPrice ? `–${offer.highPrice}` : ''}</b><span>{offer.isAggregate ? 'Published range' : 'Published offer'}</span></div>{offer.seller && <small>Seller: {offer.seller}</small>}{offer.availability && <small>{offer.availability.replace(/^https?:\/\/schema.org\//, '')}</small>}<button className="text-button" onClick={() => onOpen(offer.url)}><ExternalLink size={10} /> {safeHostname(offer.url)}</button></article>)}<p>Page-reported offers, observed at retrieval. Seller reliability requires separate evidence.</p></div>}
          {!!item.research?.length && <details className="research-evidence"><summary>Inspect retrieved evidence · {item.research.length} pages</summary>{item.research.map((content, index) => <article key={`${content.url}-${index}`}><strong>{content.title || safeHostname(content.url)}</strong><small>{safeHostname(content.url)} · {relativeTime(content.retrievedAt)}{content.truncated ? ' · excerpt' : ''}</small><p>{content.text}</p></article>)}</details>}
          {working && !!item.answer && <div className="activity" role="status"><LoaderCircle size={12} className="spin" /><span>{activity || 'Still going'}</span></div>}
          {item.state === 'cancelled' && <div className="muted-note"><Square size={11} /> Stopped. You’re in control.</div>}
          {item.state === 'error' && !working && <p className="bubble-error">{item.answer || 'This one didn’t finish. Please try again.'}</p>}
        </div>
      </div>
      {done && <div className="bubble-meta"><span title={new Date(item.createdAt).toLocaleString()}><Check size={10} /> {relativeTime(item.createdAt)}{item.backend ? ` · ${backendLabel[item.backend as ModelBackend] || item.backend}` : ''}</span>
        <button className="text-button" onClick={() => { if (speaking && reading === item.id) onStopSpeaking(); else { setReading(item.id); onSpeak(item.answer); } }}>{speaking && reading === item.id ? <VolumeX size={11} /> : <Volume2 size={11} />}{speaking && reading === item.id ? 'Stop' : 'Read aloud'}</button>
        <button className="text-button" onClick={() => { void navigator.clipboard?.writeText(item.answer).then(() => { setCopied(item.id); setTimeout(() => setCopied(current => current === item.id ? '' : current), 1500); }); }}>{copied === item.id ? <Check size={11} /> : <Copy size={11} />}{copied === item.id ? 'Copied' : 'Copy'}</button>
      </div>}
    </article>;
  };
  return <div className="thread">
    {canLoadEarlier && <button className="load-earlier" onClick={() => void loadEarlier()} disabled={loading}>{loading ? <LoaderCircle size={11} className="spin" /> : null}{loading ? 'Loading…' : 'Load earlier'}</button>}
    {threadItems(turns, boundary).map(item => item.kind === 'topic' ? <div className="topic-divider" key={`topic-${item.at}`} title="Earlier messages are no longer sent to the model"><span>New topic</span></div> : turn(item.turn, false))}
    {live && turn(live, true)}
  </div>;
}

export function SystemCard({ snapshot }: { snapshot: Record<string, unknown> }) {
  const memoryUsed = Number(snapshot.memoryUsed || 0), memoryTotal = Number(snapshot.memoryTotal || 1);
  const cpu = Number(snapshot.cpuPercent || 0), memoryPercent = memoryUsed / memoryTotal * 100;
  return <div className="system-card"><div className="system-card-heading"><Laptop size={13} /><span>{String(snapshot.os || 'Your Mac')}</span><small>Live snapshot</small></div><div className="system-metrics"><div><span>CPU usage</span><strong>{cpu.toFixed(1)}<small>%</small></strong><div className="metric-track"><i style={{ width: `${Math.min(100, Math.max(0, cpu))}%` }} /></div></div><div><span>Memory in use</span><strong>{(memoryUsed / 1073741824).toFixed(1)}<small> / {(memoryTotal / 1073741824).toFixed(0)} GB</small></strong><div className="metric-track"><i style={{ width: `${Math.min(100, Math.max(0, memoryPercent))}%` }} /></div></div></div><p>A moment in time, measured on your Mac.</p></div>;
}
