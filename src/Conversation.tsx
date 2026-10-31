import { useState, type RefObject } from 'react';
import { Check, Copy, ExternalLink, FileText, Laptop, LoaderCircle, Square, Volume2, VolumeX } from 'lucide-react';
import Pet from './Pet';
import { ResultCards } from './Online';
import { backendLabel } from './BrainSettings';
import { lookFrom } from './petOptions';
import { ProgressDetails } from './TaskProgress';
import { taskRecovery, missingTaskFields } from './taskRecovery';
import { parse, type Span } from './markdown';
import { fileSize, relativeTime, safeHostname } from './format';
import type { ModelBackend, Settings, TaskRecord, TaskProgress, TaskFieldUpdate } from './types';

type Props = {
  turns: TaskRecord[]; live: TaskRecord | null; busy: boolean; activity: string;
  settings?: Settings; focusId: string; speaking: boolean; scroller: RefObject<HTMLDivElement | null>;
  canLoadEarlier: boolean; onLoadEarlier: () => Promise<void>;
  onOpen: (url: string) => void; onSpeak: (text: string) => void; onStopSpeaking: () => void;
  progress?: TaskProgress[]; onContinue: (task: TaskRecord, guidance?: string, fields?: TaskFieldUpdate[]) => void; onCancel: (task: TaskRecord) => void;
};

/** The ongoing chat: your messages on the right, Buntini's replies (with any cards and sources) on the left. */
export default function Conversation({ turns, live, busy, activity, settings, focusId, speaking, scroller, canLoadEarlier, onLoadEarlier, onOpen, onSpeak, onStopSpeaking, progress, onContinue, onCancel }: Props) {
  const [reading, setReading] = useState('');
  const [copied, setCopied] = useState('');
  const [loading, setLoading] = useState(false);
  const [clarifications, setClarifications] = useState<Record<string, string>>({});
  const [fieldValues, setFieldValues] = useState<Record<string, string>>({});
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
    const savedProgress = progress?.find(flow => flow.id === item.id);
    const recovery = taskRecovery(item.state, savedProgress);
    const missing = missingTaskFields(savedProgress);
    const continueWithDetails = () => {
      const updates: Record<string, TaskFieldUpdate> = {};
      for (const field of missing) {
        const value = fieldValues[`${item.id}:${field.key}`]?.trim();
        if (!value) continue;
        const update = updates[field.outcomeId] ??= { outcomeId: field.outcomeId, fields: {} };
        update.fields[field.field] = field.date ? value.replace('T', ' ') : value;
      }
      onContinue(item, clarifications[item.id], Object.values(updates));
    };
    return <article className={`turn ${focusId && focusId === item.id ? 'is-focused' : ''}`} id={item.id ? `turn-${item.id}` : undefined} key={item.id || 'live'}>
      {item.prompt && <div className="bubble-row is-you"><div className="bubble bubble-you">{item.prompt}</div></div>}
      <div className="bubble-row is-buntini">
        <span className="bubble-avatar"><Pet state={working ? 'thinking' : 'idle'} avatar={settings?.avatar || 'cat'} look={settings ? lookFrom(settings) : undefined} /></span>
        <div className={`bubble bubble-buntini ${item.state === 'error' ? 'is-error' : ''}`}>
          {item.answer && item.state !== 'error' && <Reply text={item.answer} onOpen={onOpen} />}
          {working && !item.answer && <div className="typing" role="status" aria-label={activity || 'Thinking'}><i /><i /><i /><span>{activity}</span></div>}
          {item.system && <SystemCard snapshot={item.system} />}
          {!!item.cards?.length && <ResultCards cards={item.cards} onOpen={onOpen} />}
          {item.files.length > 0 && <div className="file-results">{item.files.map(file => <article className="file-result" key={file.path}><FileText size={18} /><div><strong>{file.name}</strong><span title={file.path}>{file.path}</span><small>{fileSize(file.size)} · {relativeTime(file.modifiedAt)}</small>{file.excerpt && <p>{file.excerpt}</p>}</div></article>)}</div>}
          {item.sources.length > 0 && <div className="sources"><div className="section-label">Sources</div>{item.sources.map((source, index) => <a key={`${source.url}-${index}`} className="source" href={source.url} target="_blank" rel="noreferrer" onClick={event => { event.preventDefault(); onOpen(source.url); }}><span className="source-number">{index + 1}</span><div><strong>{source.title}</strong><small>{safeHostname(source.url)} · {relativeTime(source.retrievedAt)}</small></div><ExternalLink size={13} /></a>)}</div>}
          {!!item.prices?.length && <div className="product-comparison"><div className="section-label">Published offers</div>{item.prices.map((offer, index) => <article key={`${offer.url}-${index}`}><strong>{offer.product}</strong><div><b>{offer.currency} {offer.price}{offer.isAggregate && offer.highPrice ? `–${offer.highPrice}` : ''}</b><span>{offer.isAggregate ? 'Published range' : 'Published offer'}</span></div>{offer.seller && <small>Seller: {offer.seller}</small>}{offer.availability && <small>{offer.availability.replace(/^https?:\/\/schema.org\//, '')}</small>}<button className="text-button" onClick={() => onOpen(offer.url)}><ExternalLink size={10} /> {safeHostname(offer.url)}</button></article>)}<p>Page-reported offers, observed at retrieval. Seller reliability requires separate evidence.</p></div>}
          {!!item.research?.length && <details className="research-evidence"><summary>Inspect retrieved evidence · {item.research.length} pages</summary>{item.research.map((content, index) => <article key={`${content.url}-${index}`}><strong>{content.title || safeHostname(content.url)}</strong><small>{safeHostname(content.url)} · {relativeTime(content.retrievedAt)}{content.truncated ? ' · excerpt' : ''}</small><p>{content.text}</p></article>)}</details>}
          {working && !!item.answer && <div className="activity" role="status"><LoaderCircle size={12} className="spin" /><span>{activity || 'Still going'}</span></div>}
          {item.state === 'cancelled' && !item.answer && <div className="muted-note"><Square size={11} /> Stopped. You’re in control.</div>}
          <ProgressDetails progress={savedProgress} />
          {!working && recovery.canContinue && missing.map(field => <label className="task-clarification" key={field.key}>{field.label}{field.field === 'body'
            ? <textarea aria-label={field.label} value={fieldValues[`${item.id}:${field.key}`] || ''} maxLength={8000} disabled={busy} onChange={event => setFieldValues(previous => ({ ...previous, [`${item.id}:${field.key}`]: event.target.value }))} />
            : <input aria-label={field.label} type={field.date ? 'datetime-local' : 'text'} value={fieldValues[`${item.id}:${field.key}`] || ''} maxLength={8000} disabled={busy} onChange={event => setFieldValues(previous => ({ ...previous, [`${item.id}:${field.key}`]: event.target.value }))} />}</label>)}
          {!working && recovery.canContinue && <label className="task-clarification">Add details for this task<textarea aria-label="Additional task details" value={clarifications[item.id] || ''} maxLength={4000} disabled={busy} onChange={event => setClarifications(previous => ({ ...previous, [item.id]: event.target.value }))} placeholder="For example: tomorrow at 3 pm" /></label>}
          {!working && (recovery.canContinue || recovery.canReview) && <div className="task-controls"><button disabled={busy} onClick={continueWithDetails}>{recovery.canReview ? 'Review next step' : recovery.retryRead ? 'Try again' : 'Continue task'}</button><button disabled={busy} className="text-button" onClick={() => onCancel(item)}>Stop task</button></div>}
          {item.state === 'unknown' && <p role="status">Check the destination app before making another request. This task will not repeat an uncertain action.</p>}
          {item.state === 'error' && !working && <p className="bubble-error">{item.answer || 'This one didn’t finish. Please try again.'}</p>}
        </div>
      </div>
      {done && <div className="bubble-meta"><span title={new Date(item.createdAt).toLocaleString()}><Check size={10} /> {relativeTime(item.createdAt)}{item.backend === 'brief' ? ' · Morning brief' : item.backend === 'nudge' ? ' · Heads-up' : item.backend ? ` · ${backendLabel[item.backend as ModelBackend] || item.backend}` : ''}</span>
        <button className="text-button" onClick={() => { if (speaking && reading === item.id) onStopSpeaking(); else { setReading(item.id); onSpeak(item.answer); } }}>{speaking && reading === item.id ? <VolumeX size={11} /> : <Volume2 size={11} />}{speaking && reading === item.id ? 'Stop' : 'Read aloud'}</button>
        <button className="text-button" onClick={() => { void navigator.clipboard?.writeText(item.answer).then(() => { setCopied(item.id); setTimeout(() => setCopied(current => current === item.id ? '' : current), 1500); }); }}>{copied === item.id ? <Check size={11} /> : <Copy size={11} />}{copied === item.id ? 'Copied' : 'Copy'}</button>
      </div>}
    </article>;
  };
  return <div className="thread">
    {canLoadEarlier && <button className="load-earlier" onClick={() => void loadEarlier()} disabled={loading}>{loading ? <LoaderCircle size={11} className="spin" /> : null}{loading ? 'Loading…' : 'Load earlier'}</button>}
    {turns.map(item => turn(item, false))}
    {live && turn(live, true)}
  </div>;
}

function Spans({ spans, onOpen }: { spans: Span[]; onOpen: (url: string) => void }) {
  return <>{spans.map((span, index) => span.href ? <a key={index} href={span.href} onClick={event => { event.preventDefault(); onOpen(span.href!); }}>{span.text}</a>
    : span.code ? <code key={index}>{span.text}</code> : span.bold ? <strong key={index}>{span.text}</strong> : span.italic ? <em key={index}>{span.text}</em> : <span key={index}>{span.text}</span>)}</>;
}

/** A reply with its light formatting shown: paragraphs, lists, headings and emphasis. */
function Reply({ text, onOpen }: { text: string; onOpen: (url: string) => void }) {
  return <div className="answer-text">{parse(text).map((block, index) => {
    if (block.kind === 'heading') return <p key={index} className="reply-heading"><Spans spans={block.spans} onOpen={onOpen} /></p>;
    if (block.kind === 'list') {
      const items = block.items.map((item, at) => <li key={at}><Spans spans={item} onOpen={onOpen} /></li>);
      return block.ordered ? <ol key={index}>{items}</ol> : <ul key={index}>{items}</ul>;
    }
    return <p key={index}>{block.lines.map((line, at) => <span key={at}>{at > 0 && <br />}<Spans spans={line} onOpen={onOpen} /></span>)}</p>;
  })}</div>;
}

export function SystemCard({ snapshot }: { snapshot: Record<string, unknown> }) {
  const memoryUsed = Number(snapshot.memoryUsed || 0), memoryTotal = Number(snapshot.memoryTotal || 1);
  const cpu = Number(snapshot.cpuPercent || 0), memoryPercent = memoryUsed / memoryTotal * 100;
  return <div className="system-card"><div className="system-card-heading"><Laptop size={13} /><span>{String(snapshot.os || 'Your Mac')}</span><small>Live snapshot</small></div><div className="system-metrics"><div><span>CPU usage</span><strong>{cpu.toFixed(1)}<small>%</small></strong><div className="metric-track"><i style={{ width: `${Math.min(100, Math.max(0, cpu))}%` }} /></div></div><div><span>Memory in use</span><strong>{(memoryUsed / 1073741824).toFixed(1)}<small> / {(memoryTotal / 1073741824).toFixed(0)} GB</small></strong><div className="metric-track"><i style={{ width: `${Math.min(100, Math.max(0, memoryPercent))}%` }} /></div></div></div><p>A moment in time, measured on your Mac.</p></div>;
}
