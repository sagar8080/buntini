import { useEffect, useState } from 'react';
import { FileText, LoaderCircle, Plus, Undo2, X } from 'lucide-react';
import { errorMessage } from '../bridge';
import { approve, deny, dropPlan, mailboxes as readMailboxes, mailLog, makePlan, pickPdf, planned, proposeSort, proposeUndo, runPlan } from './api';
import { added, mailboxName, ruleLine, ruleReady, type Mailbox, type MailDone, type MailLogRow, type MailPlan, type MailRule, type MailSettings, type MailWaiting, type RuleAction, type RuleField } from './model';
import './mail.css';

type Props = { value: MailSettings; onChange: (next: MailSettings) => void; onError: (error: string) => void };

function Words({ label, placeholder, list, onChange }: { label: string; placeholder: string; list: string[]; onChange: (next: string[]) => void }) {
  const [entry, setEntry] = useState('');
  return <div className="mail-words">
    <label>{label}</label>
    <div className="mail-chips">
      {list.map(item => <span key={item} className="mail-chip">{item}<button type="button" aria-label={`Remove ${item}`} onClick={() => onChange(list.filter(other => other !== item))}><X size={11} /></button></span>)}
    </div>
    <form className="mail-add" onSubmit={event => { event.preventDefault(); onChange(added(list, entry)); setEntry(''); }}>
      <input aria-label={label} value={entry} onChange={event => setEntry(event.target.value)} placeholder={placeholder} maxLength={200} />
      <button type="submit" className="secondary-button" disabled={!entry.trim()}><Plus size={12} />Add</button>
    </form>
  </div>;
}

/** Waiting for Allow: what will change, as data. */
function Pending({ waiting, working, onAllow, onDeny }: { waiting: MailWaiting; working: boolean; onAllow: () => void; onDeny: () => void }) {
  return <div className="inline-approval mail-pending" role="group" aria-label={waiting.title}>
    <strong>{waiting.title}</strong>
    <ul>{waiting.lines.map((line, index) => <li key={index}>{line}</li>)}</ul>
    <div><button type="button" className="text-button" onClick={onDeny} disabled={working}>Deny</button><button type="button" className="primary-button" onClick={onAllow} disabled={working}>{working && <LoaderCircle size={12} className="spin" />}Allow</button></div>
  </div>;
}

function Result({ done }: { done: MailDone }) {
  return <ul className="mail-result">
    {done.changed.map((line, index) => <li key={`c${index}`}>{line}</li>)}
    {done.failed.map((line, index) => <li key={`f${index}`} className="mail-problem">{line}</li>)}
  </ul>;
}

/** Mail under Settings → Apps: VIPs, banned words and rules; sorting, a plan, and every change with Undo. */
export default function MailSection({ value, onChange, onError }: Props) {
  const [boxes, setBoxes] = useState<Mailbox[]>([]);
  const [log, setLog] = useState<MailLogRow[]>([]);
  const [waiting, setWaiting] = useState<MailWaiting | null>(null);
  const [plan, setPlan] = useState<MailPlan | null>(null);
  const [done, setDone] = useState<MailDone | null>(null);
  const [working, setWorking] = useState('');
  const [request, setRequest] = useState('');
  const [pdf, setPdf] = useState<string | null>(null);
  const [rule, setRule] = useState<Omit<MailRule, 'id'>>({ field: 'sender', value: '', action: 'move', to: null, enabled: true });

  const refreshLog = () => mailLog().then(setLog).catch(() => {});
  useEffect(() => {
    readMailboxes().then(setBoxes).catch(() => {});
    planned().then(setPlan).catch(() => {});
    void refreshLog();
  }, []);

  async function run<T>(name: string, work: () => Promise<T>): Promise<T | undefined> {
    setWorking(name);
    try { return await work(); }
    catch (error) { onError(errorMessage(error)); return undefined; }
    finally { setWorking(''); }
  }
  async function settle(work: () => Promise<MailDone>) {
    const result = await run('allow', work);
    if (result) setDone(result);
    setWaiting(null);
    void refreshLog();
  }

  function addRule() {
    if (!ruleReady(rule)) return;
    const next: MailRule = { ...rule, value: rule.value.trim(), to: rule.action === 'move' ? rule.to : null, id: crypto.randomUUID() };
    onChange({ ...value, rules: [...value.rules, next] });
    setRule({ ...rule, value: '' });
  }
  const pickBox = (key: string) => setRule({ ...rule, to: boxes.find(box => `${box.account}\t${box.path}` === key) ?? null });

  return <>
    <section className="settings-section mail-settings">
      <div className="settings-section-heading"><h3>Mail</h3></div>
      <Words label="VIPs" placeholder="name@example.com" list={value.vips} onChange={vips => onChange({ ...value, vips })} />
      <Words label="Banned words" placeholder="lottery" list={value.banned} onChange={banned => onChange({ ...value, banned })} />
      <label>Rules</label>
      {value.rules.map(item => <div key={item.id} className="mail-rule">
        <input type="checkbox" aria-label={`${ruleLine(item)} on`} checked={item.enabled} onChange={event => onChange({ ...value, rules: value.rules.map(other => other.id === item.id ? { ...other, enabled: event.target.checked } : other) })} />
        <span>{ruleLine(item)}</span>
        <button type="button" className="text-button" aria-label={`Remove ${ruleLine(item)}`} onClick={() => onChange({ ...value, rules: value.rules.filter(other => other.id !== item.id) })}><X size={12} /></button>
      </div>)}
      <form className="mail-rule-form" onSubmit={event => { event.preventDefault(); addRule(); }}>
        <select aria-label="Match" value={rule.field} onChange={event => setRule({ ...rule, field: event.target.value as RuleField })}>
          <option value="sender">From</option><option value="domain">Domain</option><option value="subject">Subject</option>
        </select>
        <input aria-label="Contains" value={rule.value} onChange={event => setRule({ ...rule, value: event.target.value })} placeholder={rule.field === 'domain' ? 'example.com' : rule.field === 'sender' ? 'newsletter@' : 'receipt'} maxLength={200} />
        <select aria-label="Then" value={rule.action} onChange={event => setRule({ ...rule, action: event.target.value as RuleAction, to: event.target.value === 'move' ? rule.to : null })}>
          <option value="move">Move to</option><option value="flag">Flag</option><option value="read">Mark read</option><option value="junk">Junk</option>
        </select>
        {rule.action === 'move' && <select aria-label="Mailbox" value={rule.to ? `${rule.to.account}\t${rule.to.path}` : ''} onChange={event => pickBox(event.target.value)}>
          <option value="" disabled>Mailbox</option>
          {boxes.map(box => <option key={`${box.account}\t${box.path}`} value={`${box.account}\t${box.path}`}>{mailboxName(box)}</option>)}
        </select>}
        <button type="submit" className="secondary-button" disabled={!ruleReady(rule)}><Plus size={12} />Add</button>
      </form>
      <div className="mail-actions">
        <button type="button" className="secondary-button" disabled={!!working} onClick={() => void run('sort', async () => { const next = await proposeSort(); setDone(next ? null : { changed: ['Nothing to sort'], failed: [] }); setWaiting(next); })}>{working === 'sort' && <LoaderCircle size={12} className="spin" />}Sort</button>
      </div>
      {waiting && <Pending waiting={waiting} working={working === 'allow'} onAllow={() => void settle(() => approve(waiting.id))} onDeny={() => { void deny(waiting.id); setWaiting(null); }} />}
      {done && <Result done={done} />}
    </section>

    <section className="settings-section mail-settings">
      <div className="settings-section-heading"><h3>Mail plan</h3></div>
      <textarea className="mail-request" aria-label="Mail plan request" value={request} onChange={event => setRequest(event.target.value)} placeholder="Flag my boss’s email and send each person their page of the PDF" maxLength={2000} rows={3} />
      <div className="mail-actions">
        <button type="button" className="secondary-button" disabled={!!working} onClick={() => void run('pdf', async () => setPdf(await pickPdf() ?? pdf))}><FileText size={12} />{pdf ? pdf.split('/').pop() : 'PDF'}</button>
        {pdf && <button type="button" className="text-button" aria-label="Remove PDF" onClick={() => setPdf(null)}><X size={12} /></button>}
        <button type="button" className="primary-button" disabled={!!working || !request.trim()} onClick={() => void run('plan', async () => { setDone(null); setPlan(await makePlan(request, pdf)); })}>{working === 'plan' && <LoaderCircle size={12} className="spin" />}Plan</button>
      </div>
      {plan && <div className="inline-approval mail-pending" role="group" aria-label="Mail plan">
        <strong>{plan.summary || 'Plan'}</strong>
        <ul>{plan.lines.map((line, index) => <li key={index}>{line}</li>)}</ul>
        {plan.problems.length > 0 && <ul className="mail-left-out">{plan.problems.map((line, index) => <li key={index}>{line}</li>)}</ul>}
        <div>
          <button type="button" className="text-button" disabled={!!working} onClick={() => { void dropPlan(plan.id); setPlan(null); }}>Discard</button>
          <button type="button" className="primary-button" disabled={!!working} onClick={() => void run('run', async () => { const result = await runPlan(plan.id); setPlan(null); setDone(result); void refreshLog(); })}>{working === 'run' && <LoaderCircle size={12} className="spin" />}Run</button>
        </div>
      </div>}
    </section>

    {log.length > 0 && <section className="settings-section mail-settings">
      <div className="settings-section-heading"><h3>Mail changes</h3></div>
      <ul className="mail-log">
        {log.map(entry => <li key={entry.id} className={entry.undone ? 'undone' : ''}>
          <span>{entry.label}</span>
          <time dateTime={new Date(entry.at * 1000).toISOString()}>{new Date(entry.at * 1000).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })}</time>
          {entry.undoable && <button type="button" className="text-button" disabled={!!working} onClick={() => void run('undo', async () => { setDone(null); setWaiting(await proposeUndo(entry.id)); })}><Undo2 size={12} />Undo</button>}
        </li>)}
      </ul>
    </section>}
  </>;
}
