import { useEffect, useState } from 'react';
import { CalendarPlus, CalendarClock, Bell, BellRing, ExternalLink, ListChecks, Mail, MessageCircle, NotebookPen, Phone, Play, Plug, AppWindow, Reply, ShoppingCart } from 'lucide-react';
import ModalSheet from './ModalSheet';
import { locale, t } from './i18n.ts';

/** `hidden` is sent back without being shown (an item's id); `fixed` is shown but never edited or sent, such as the email a reply answers, its recipient, or what an item is like now. */
export interface ActionField { key: string; label: string; value: string; kind: 'text' | 'long' | 'when' | 'hidden' | 'fixed' }
/** A value the user can pick for a field, such as one of several matching contacts. `also` sets other fields with it. */
export interface ActionChoice { field: string; label: string; value: string; also?: Record<string, string> }
export interface ActionProposal { id: string; tool: string; title: string; fields: ActionField[]; note: string; choices?: ActionChoice[] }

const icons: Record<string, typeof Bell> = { add_reminder: Bell, add_calendar_event: CalendarPlus, create_note: NotebookPen, draft_email: Mail, reply_email: Reply, send_message: MessageCircle, start_call: Phone, run_shortcut: Play, open_app: AppWindow, open_link: ExternalLink, complete_reminder: ListChecks, reschedule_reminder: BellRing, update_calendar_event: CalendarClock, append_to_note: NotebookPen, add_to_cart: ShoppingCart };
// The sheet edits "YYYY-MM-DD HH:MM"; the browser's picker wants "YYYY-MM-DDTHH:MM".
const toPicker = (value: string) => value.replace(' ', 'T');
const fromPicker = (value: string) => value.replace('T', ' ').slice(0, 16);
/** "2026-10-31 09:00" as "Sat 31 Oct, 9:00". */
const shown = (value: string) => {
  const at = new Date(value.replace(' ', 'T'));
  if (Number.isNaN(at.getTime())) return value;
  const day = at.toLocaleDateString(locale(), { weekday: 'short', day: 'numeric', month: 'short' });
  return value.length > 10 ? t('action.dayAndTime', { day, time: at.toLocaleTimeString(locale(), { hour: 'numeric', minute: '2-digit' }) }) : day;
};
// The button says what happens.
const verbs: Record<string, string> = { add_reminder: 'action.verb.add', add_calendar_event: 'action.verb.add', create_note: 'action.verb.save', draft_email: 'action.verb.draft', reply_email: 'action.verb.draftReply', send_message: 'action.verb.send', start_call: 'action.verb.call', run_shortcut: 'action.verb.run', open_app: 'action.verb.open', open_link: 'action.verb.open', complete_reminder: 'action.verb.markDone', reschedule_reminder: 'action.verb.move', update_calendar_event: 'action.verb.change', append_to_note: 'action.verb.add', create_routine: 'action.verb.setUp', create_goal: 'action.verb.start', remember: 'action.verb.remember', set_timer: 'action.verb.start', change_mac_setting: 'action.verb.change', create_saved_search: 'action.verb.save', create_price_watch: 'action.verb.watch', add_to_cart: 'action.verb.addToCart' };
const verb = (tool: string) => verbs[tool] ? t(verbs[tool]) : t(tool.startsWith('mcp_') ? 'action.verb.run' : 'action.allow');
// What an edit changes from, shown struck through beside its new value.
const before: Record<string, string> = { due: 'was_due', start: 'occurrence' };

/** Shows exactly what an action will do, lets the user fix any detail, and runs only when they confirm. */
export default function ActionSheet({ proposal, running, error, onCancel, onAllow }: { proposal: ActionProposal; running: boolean; error?: string; onCancel: () => void; onAllow: (args: Record<string, string | number>) => void }) {
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(proposal.fields.map(field => [field.key, field.value])));
  // An error answers the details as they were; it goes once the user changes them.
  const [shownError, setShownError] = useState(error);
  useEffect(() => setShownError(error), [error]);
  const Icon = icons[proposal.tool] || (proposal.tool.startsWith('mcp_') ? Plug : Play);
  const sent = new Set(proposal.fields.filter(field => field.kind !== 'fixed').map(field => field.key));
  const allow = () => onAllow(Object.fromEntries(Object.entries(values).filter(([key, value]) => sent.has(key) && value.trim() !== '').map(([key, value]) => [key, key === 'minutes' ? Number(value) : value])));
  const set = (key: string, value: string) => { setShownError(''); setValues(previous => ({ ...previous, [key]: value })); };
  // A pick replaces everything another pick of the same field set, so nothing from the
  // first match (such as its location) stays on the one picked.
  const pick = (choice: ActionChoice) => { setShownError(''); setValues(previous => {
    const cleared = Object.fromEntries((proposal.choices ?? []).filter(other => other.field === choice.field).flatMap(other => Object.keys(other.also ?? {})).map(key => [key, '']));
    return { ...previous, ...cleared, ...choice.also, [choice.field]: choice.value };
  }); };
  const choicesFor = (key: string) => proposal.choices?.filter(choice => choice.field === key) ?? [];
  const picker = (label: string, choices: ActionChoice[], key: string) => <span className="field-choices" role="group" aria-label={t('action.matching', { field: label })}>
    {choices.map(choice => <button key={choice.value} type="button" className="secondary-button" disabled={running} aria-pressed={values[key] === choice.value} onClick={() => pick(choice)}>{choice.label}</button>)}
  </span>;
  return <ModalSheet className="action-sheet" labelledBy="action-title">
    <Icon size={22} /><h3 id="action-title">{proposal.title}</h3>
    <form className="action-fields" onSubmit={event => { event.preventDefault(); if (!running) allow(); }}>
      {proposal.fields.map(field => {
        const choices = choicesFor(field.key);
        if (field.kind === 'hidden') return choices.length > 0 ? <div key={field.key} className="action-pick"><span>{field.label}</span>{picker(field.label, choices, field.key)}</div> : null;
        // The title names the item; its present state and "Done" don't need a row.
        if (field.key === 'now' || field.key === 'after') return null;
        if (field.kind === 'fixed') return <div key={field.key} className="action-fixed"><span>{field.label}</span><output>{values[field.key] || field.value}</output></div>;
        const was = before[field.key] && values[before[field.key]];
        return <label key={field.key}><span>{field.label}</span>
          {was && was !== values[field.key] && <s className="action-was">{shown(was)}</s>}
          {field.kind === 'long' ? <textarea disabled={running} rows={4} value={values[field.key]} onChange={event => set(field.key, event.target.value)} />
            : field.kind === 'when' ? <input disabled={running} type="datetime-local" value={toPicker(values[field.key])} onChange={event => set(field.key, fromPicker(event.target.value))} />
            : <input disabled={running} type={field.key === 'minutes' ? 'number' : 'text'} min={field.key === 'minutes' ? 5 : undefined} max={field.key === 'minutes' ? 1440 : undefined} step={field.key === 'minutes' ? 1 : undefined} value={values[field.key]} onChange={event => set(field.key, event.target.value)} placeholder={field.key === 'to' || field.key === 'location' ? t('action.optional') : undefined} />}
          {choices.length > 0 && picker(field.label, choices, field.key)}
        </label>;
      })}
    </form>
    {shownError && <p role="alert">{shownError}</p>}
    <div><button className="secondary-button" disabled={running} onClick={onCancel}>{t('action.notNow')}</button><button className="primary-button" disabled={running} onClick={allow}>{running ? t('action.working') : verb(proposal.tool)}</button></div>
  </ModalSheet>;
}
