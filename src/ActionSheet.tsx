import { useState } from 'react';
import { CalendarPlus, Bell, ExternalLink, Mail, NotebookPen, Play, AppWindow } from 'lucide-react';

export interface ActionField { key: string; label: string; value: string; kind: 'text' | 'long' | 'when' }
export interface ActionProposal { id: string; tool: string; title: string; fields: ActionField[]; note: string }

const icons: Record<string, typeof Bell> = { add_reminder: Bell, add_calendar_event: CalendarPlus, create_note: NotebookPen, draft_email: Mail, run_shortcut: Play, open_app: AppWindow, open_link: ExternalLink };
// The sheet edits "YYYY-MM-DD HH:MM"; the browser's picker wants "YYYY-MM-DDTHH:MM".
const toPicker = (value: string) => value.replace(' ', 'T');
const fromPicker = (value: string) => value.replace('T', ' ').slice(0, 16);

/** Shows exactly what an action will do, lets the user fix any detail, and runs only on Allow. */
export default function ActionSheet({ proposal, running, onCancel, onAllow }: { proposal: ActionProposal; running: boolean; onCancel: () => void; onAllow: (args: Record<string, string | number>) => void }) {
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(proposal.fields.map(field => [field.key, field.value])));
  const Icon = icons[proposal.tool] || Play;
  const allow = () => onAllow(Object.fromEntries(Object.entries(values).filter(([, value]) => value.trim() !== '').map(([key, value]) => [key, key === 'minutes' ? Number(value) : value])));
  return <div className="approval-backdrop"><section className="approval-sheet action-sheet" role="alertdialog" aria-modal="true" aria-labelledby="action-title">
    <Icon size={22} /><h3 id="action-title">{proposal.title}</h3>
    <form className="action-fields" onSubmit={event => { event.preventDefault(); allow(); }}>
      {proposal.fields.map(field => <label key={field.key}><span>{field.label}</span>
        {field.kind === 'long' ? <textarea rows={4} value={values[field.key]} onChange={event => setValues(previous => ({ ...previous, [field.key]: event.target.value }))} />
          : field.kind === 'when' ? <input type="datetime-local" value={toPicker(values[field.key])} onChange={event => setValues(previous => ({ ...previous, [field.key]: fromPicker(event.target.value) }))} />
          : <input value={values[field.key]} onChange={event => setValues(previous => ({ ...previous, [field.key]: event.target.value }))} placeholder={field.key === 'to' || field.key === 'location' ? 'Optional' : undefined} />}
      </label>)}
    </form>
    {proposal.note && <small>{proposal.note}</small>}
    <div><button className="secondary-button" disabled={running} onClick={onCancel}>Cancel</button><button className="primary-button" disabled={running} autoFocus onClick={allow}>{running ? 'Working…' : 'Allow'}</button></div>
  </section></div>;
}
