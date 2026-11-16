import { useEffect, useRef, useState } from 'react';
import { Check, LoaderCircle, SquareTerminal, X } from 'lucide-react';
import { number, t } from './i18n.ts';
import type { AgentAnswer, AgentFinished, AgentRequest } from './types';

/** How long a card shows before a click counts, so one aimed at whatever was under it doesn't. */
const ARMING_MS = 700;

/** What Claude Code or Codex wants to do, or asks. Nothing is allowed until Allow or an answer
    is clicked; "In terminal" leaves it to the agent's own prompt. */
export function AgentCard({ request, more, onAnswer, className = '' }: { request: AgentRequest; more: number; onAnswer: (answer: AgentAnswer) => Promise<void>; className?: string }) {
  const shownAt = useRef(Date.now());
  const [busy, setBusy] = useState(false);
  const [step, setStep] = useState(0);
  const [answers, setAnswers] = useState<Record<string, string | string[]>>({});
  const [picked, setPicked] = useState<string[]>([]);
  useEffect(() => { shownAt.current = Date.now(); setStep(0); setAnswers({}); setPicked([]); }, [request.id]);
  const armed = () => Date.now() - shownAt.current >= ARMING_MS;
  async function answer(next: AgentAnswer) {
    if (!armed() || busy) return;
    setBusy(true);
    try { await onAnswer(next); } finally { setBusy(false); }
  }
  const question = request.questions[step];
  function choose(value: string | string[]) {
    if (!armed()) return;
    const all = { ...answers, [question.question]: value };
    if (step + 1 < request.questions.length) { setAnswers(all); setStep(step + 1); setPicked([]); }
    else void answer({ kind: 'answers', answers: all });
  }
  const toggle = (label: string) => { if (armed()) setPicked(previous => previous.includes(label) ? previous.filter(item => item !== label) : [...previous, label]); };
  const terminal = <button type="button" className="agent-terminal" disabled={busy} onClick={() => void answer({ kind: 'terminal' })}>{t('agentRequests.inTerminal')}</button>;
  return <div className={`agent-card ${className}`} role="alertdialog" aria-label={`${request.agent}${request.project ? ` · ${request.project}` : ''}`}>
    <div className="agent-from"><SquareTerminal size={12} /><span>{request.agent}{request.project && ` · ${request.project}`}</span>{more > 0 && <small>+{more}</small>}</div>
    {question ? <>
      {request.questions.length > 1 && <small className="agent-step">{question.header || `${number(step + 1)} / ${number(request.questions.length)}`}</small>}
      <strong>{question.question}</strong>
      <div className="agent-options">{question.options.map(option => question.multiSelect
        ? <button key={option.label} type="button" title={option.description || undefined} aria-pressed={picked.includes(option.label)} className={picked.includes(option.label) ? 'picked' : ''} disabled={busy} onClick={() => toggle(option.label)}>{option.label}</button>
        : <button key={option.label} type="button" title={option.description || undefined} disabled={busy} onClick={() => choose(option.label)}>{option.label}</button>)}</div>
      <div className="agent-actions">{terminal}{question.multiSelect && <button type="button" className="agent-allow" disabled={busy || !picked.length} onClick={() => choose(picked)}>{step + 1 < request.questions.length ? t('common.next') : t('common.send')}</button>}</div>
    </> : <>
      <strong>{request.title}</strong>
      {request.detail && (request.code ? <pre>{request.detail}</pre> : <p>{request.detail}</p>)}
      <div className="agent-actions">{terminal}<button type="button" disabled={busy} onClick={() => void answer({ kind: 'deny' })}>{request.plan ? t('agentRequests.keepPlanning') : t('agentRequests.deny')}</button>{request.always && <button type="button" disabled={busy} onClick={() => void answer({ kind: 'always' })}>{request.plan ? t('agentRequests.acceptEdits') : t('agentRequests.always')}</button>}<button type="button" className="agent-allow" disabled={busy} onClick={() => void answer({ kind: 'allow' })}>{busy ? <LoaderCircle size={11} className="spin" /> : <Check size={11} />}{request.plan ? t('agentRequests.start') : t('agentRequests.allow')}</button></div>
    </>}
  </div>;
}

/** A long turn finished. */
export function AgentDone({ note, onClose }: { note: AgentFinished; onClose: () => void }) {
  return <div className="agent-card agent-done" role="status">
    <div className="agent-from"><SquareTerminal size={12} /><span>{note.agent}{note.project && ` · ${note.project}`}</span><button type="button" aria-label={t('common.dismiss')} onClick={onClose}><X size={12} /></button></div>
    <strong>{t('common.done')}</strong>
    {note.message && <p>{note.message}</p>}
  </div>;
}
