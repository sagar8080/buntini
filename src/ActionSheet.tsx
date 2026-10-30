import { useState } from 'react';
import { CalendarPlus, Bell, ExternalLink, Mail, NotebookPen, Play, AppWindow } from 'lucide-react';
import ModalSheet from './ModalSheet';

export interface ActionField { key: string; label: string; value: string; kind: 'text' | 'long' | 'when' }
/** A value the user can pick for a field, such as one of several matching contacts. */
export interface ActionChoice { field: string; label: string; value: string }
export interface ActionProposal { id: string; tool: string; title: string; fields: ActionField[]; note: string; choices?: ActionChoice[] }

const icons: Record<string, typeof Bell> = { add_reminder: Bell, add_calendar_event: CalendarPlus, create_note: NotebookPen, draft_email: Mail, run_shortcut: Play, open_app: AppWindow, open_link: ExternalLink };
// The sheet edits "YYYY-MM-DD HH:MM"; the browser's picker wants "YYYY-MM-DDTHH:MM".
const toPicker = (value: string) => value.replace(' ', 'T');
const fromPicker = (value: string) => value.replace('T', ' ').slice(0, 16);

/** Shows exactly what an action will do, lets the user fix any detail, and runs only on Allow. */
export default function ActionSheet({ proposal, running, error, onCancel, onAllow }: { proposal: ActionProposal; running: boolean; error?: string; onCancel: () => void; onAllow: (args: Record<string, string | number>) => void }) {
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(proposal.fields.map(field => [field.key, field.value])));
  const Icon = icons[proposal.tool] || Play;
  const allow = () => onAllow(Object.fromEntries(Object.entries(values).filter(([, value]) => value.trim() !== '').map(([key, value]) => [key, key === 'minutes' ? Number(value) : value])));
  const set = (key: string, value: string) => setValues(previous => ({ ...previous, [key]: value }));
  return <ModalSheet className="action-sheet" labelledBy="action-title">
    <Icon size={22} /><h3 id="action-title">{proposal.title}</h3>
    <form className="action-fields" onSubmit={event => { event.preventDefault(); if (!running) allow(); }}>
      {proposal.fields.map(field => {
        const choices = proposal.choices?.filter(choice => choice.field === field.key) ?? [];
        return <label key={field.key}><span>{field.label}</span>
          {field.kind === 'long' ? <textarea disabled={running} rows={4} value={values[field.key]} onChange={event => set(field.key, event.target.value)} />
            : field.kind === 'when' ? <input disabled={running} type="datetime-local" value={toPicker(values[field.key])} onChange={event => set(field.key, fromPicker(event.target.value))} />
            : <input disabled={running} type={field.key === 'minutes' ? 'number' : 'text'} min={field.key === 'minutes' ? 5 : undefined} max={field.key === 'minutes' ? 1440 : undefined} step={field.key === 'minutes' ? 1 : undefined} value={values[field.key]} onChange={event => set(field.key, event.target.value)} placeholder={choices.length ? (field.key === 'to' ? 'Pick one below or type an address' : 'Pick one below') : field.key === 'to' || field.key === 'location' ? 'Optional' : undefined} />}
          {choices.length > 0 && <span className="field-choices" role="group" aria-label={`Matching ${field.label}`}>
            {choices.map(choice => <button key={choice.value} type="button" className="secondary-button" disabled={running} aria-pressed={values[field.key] === choice.value} onClick={() => set(field.key, choice.value)}>{choice.label}</button>)}
          </span>}
        </label>;
      })}
    </form>
    {proposal.choices?.length ? <small>{proposal.tool === 'run_shortcut' ? 'More than one of your shortcuts fits. Pick the one to run.' : 'More than one person in Contacts matches. Pick who this is for.'}</small> : null}
    {proposal.note && <small>{proposal.note}</small>}
    {error && <p role="alert">{error}</p>}
    <div><button className="secondary-button" disabled={running} onClick={onCancel}>Cancel</button><button className="primary-button" disabled={running} autoFocus onClick={allow}>{running ? 'Working…' : 'Allow'}</button></div>
  </ModalSheet>;
}
