import { useState } from 'react';
import { CalendarPlus, Bell, ExternalLink, Mail, NotebookPen, Play, AppWindow } from 'lucide-react';
import ModalSheet from './ModalSheet';

export interface ActionField { key: string; label: string; value: string; kind: 'text' | 'long' | 'when' }
export interface ActionProposal { id: string; tool: string; title: string; fields: ActionField[]; note: string }

const icons: Record<string, typeof Bell> = { add_reminder: Bell, add_calendar_event: CalendarPlus, create_note: NotebookPen, draft_email: Mail, run_shortcut: Play, open_app: AppWindow, open_link: ExternalLink };
// The sheet edits "YYYY-MM-DD HH:MM"; the browser's picker wants "YYYY-MM-DDTHH:MM".
const toPicker = (value: string) => value.replace(' ', 'T');
const fromPicker = (value: string) => value.replace('T', ' ').slice(0, 16);

/** Shows exactly what an action will do, lets the user fix any detail, and runs only on Allow. */
export default function ActionSheet({ proposal, running, error, onCancel, onAllow }: { proposal: ActionProposal; running: boolean; error?: string; onCancel: () => void; onAllow: (args: Record<string, string | number>) => void }) {
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(proposal.fields.map(field => [field.key, field.value])));
  const Icon = icons[proposal.tool] || Play;
  const allow = () => onAllow(Object.fromEntries(Object.entries(values).filter(([, value]) => value.trim() !== '').map(([key, value]) => [key, key === 'minutes' ? Number(value) : value])));
  return <ModalSheet className="action-sheet" labelledBy="action-title">
    <Icon size={22} /><h3 id="action-title">{proposal.title}</h3>
    <form className="action-fields" onSubmit={event => { event.preventDefault(); if (!running) allow(); }}>
      {proposal.fields.map(field => <label key={field.key}><span>{field.label}</span>
        {field.kind === 'long' ? <textarea disabled={running} rows={4} value={values[field.key]} onChange={event => setValues(previous => ({ ...previous, [field.key]: event.target.value }))} />
          : field.kind === 'when' ? <input disabled={running} type="datetime-local" value={toPicker(values[field.key])} onChange={event => setValues(previous => ({ ...previous, [field.key]: fromPicker(event.target.value) }))} />
          : <input disabled={running} type={field.key === 'minutes' ? 'number' : 'text'} min={field.key === 'minutes' ? 5 : undefined} max={field.key === 'minutes' ? 1440 : undefined} step={field.key === 'minutes' ? 1 : undefined} value={values[field.key]} onChange={event => setValues(previous => ({ ...previous, [field.key]: event.target.value }))} placeholder={field.key === 'to' || field.key === 'location' ? 'Optional' : undefined} />}
      </label>)}
    </form>
    {proposal.note && <small>{proposal.note}</small>}
    {error && <p role="alert">{error}</p>}
    <div><button className="secondary-button" disabled={running} onClick={onCancel}>Cancel</button><button className="primary-button" disabled={running} autoFocus onClick={allow}>{running ? 'Working…' : 'Allow'}</button></div>
  </ModalSheet>;
}
