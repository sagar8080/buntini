import { useState } from 'react';
import { ChevronRight, LoaderCircle, Play, Repeat, Square, X } from 'lucide-react';
import type { Routine } from './types';
import { errorMessage } from './bridge';

/** "Weekdays at 8:00 AM"; the morning brief runs when you first open Buntini instead. */
export function routineSchedule(routine: Routine) {
  if (routine.builtIn) return 'Every day, when you first open me';
  const [hour, minute] = routine.time.split(':').map(Number);
  const at = new Date(2000, 0, 1, hour, minute).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  const days = routine.days === 'every_day' ? 'Every day' : routine.days === 'weekdays' ? 'Weekdays' : routine.days === 'weekends' ? 'Weekends' : `${routine.days.charAt(0).toUpperCase()}${routine.days.slice(1)}s`;
  return `${days} at ${at}`;
}

/** Routines are made from chat; here they can only be opened, paused or deleted. */
export default function Routines({ routines, onAction, onOpen, onError }: { routines: Routine[]; onAction: (command: string, args: Record<string, unknown>) => Promise<unknown>; onOpen: (taskId: string) => void; onError: (message: string) => void }) {
  const [working, setWorking] = useState('');
  async function act(id: string, action: () => Promise<unknown>) { setWorking(id); try { await action(); } catch (error) { onError(errorMessage(error)); } finally { setWorking(''); } }
  return <section className="saved-searches routines"><div className="section-label"><Repeat size={11} /> ROUTINES</div>
    {routines.map(routine => <article className={`saved-search ${routine.enabled ? '' : 'paused'}`} key={routine.id}>
      <div className="saved-search-heading"><Repeat size={14} /><strong>{routine.task}</strong>{!routine.builtIn && <button className="icon-button" aria-label={`Delete routine: ${routine.task}`} disabled={!!working} onClick={() => void act(routine.id, () => onAction('delete_routine', { id: routine.id }))}><X size={12} /></button>}</div>
      <div className="saved-search-info"><span>{routine.enabled ? <><span className="status-dot" />{routineSchedule(routine)}</> : 'Paused'}</span>
        <button className="text-button" disabled={!!working} onClick={() => void act(routine.id, () => onAction('update_routine', { id: routine.id, enabled: !routine.enabled }))}>{working === routine.id ? <LoaderCircle size={10} className="spin" /> : routine.enabled ? <Square size={9} /> : <Play size={10} />}{routine.enabled ? 'Pause' : 'Resume'}</button>
        {routine.lastTaskId && <button className="text-button" onClick={() => onOpen(routine.lastTaskId!)}>Last result <ChevronRight size={10} /></button>}
      </div>
      {routine.lastError && <p className="saved-search-error">{routine.lastError}</p>}
    </article>)}
    {routines.every(routine => routine.builtIn) && <p className="saved-search-empty">Ask me to do something on a schedule, like “Every weekday at 8, brief me”.</p>}
  </section>;
}
