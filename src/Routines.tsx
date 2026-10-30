import { useState } from 'react';
import { Check, ChevronRight, LoaderCircle, Play, Repeat, Square, Target, X } from 'lucide-react';
import type { Goal, Routine } from './types';
import { errorMessage } from './bridge';

/** "Weekdays at 8:00 AM". */
function scheduleLabel(daysValue: string, time: string) {
  const [hour, minute] = time.split(':').map(Number);
  const at = new Date(2000, 0, 1, hour, minute).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' });
  const days = daysValue === 'every_day' ? 'Every day' : daysValue === 'weekdays' ? 'Weekdays' : daysValue === 'weekends' ? 'Weekends' : `${daysValue.charAt(0).toUpperCase()}${daysValue.slice(1)}s`;
  return `${days} at ${at}`;
}

/** "Weekdays at 8:00 AM"; the morning brief runs when you first open Buntini instead. */
export function routineSchedule(routine: Routine) {
  if (routine.builtIn) return 'Every day, when you first open me';
  return scheduleLabel(routine.days, routine.time);
}

/** "Weekdays at 9:00 AM until Fri, Nov 13". */
export function goalSchedule(goal: Goal) {
  const [year, month, day] = goal.until.split('-').map(Number);
  const until = new Date(year, month - 1, day).toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric' });
  return `${scheduleLabel(goal.days, goal.time)} until ${until}`;
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

/** Goals are made from chat; here they can be opened, paused, marked done or deleted. A finished goal says why it ended. */
export function Goals({ goals, onAction, onOpen, onError }: { goals: Goal[]; onAction: (command: string, args: Record<string, unknown>) => Promise<unknown>; onOpen: (taskId: string) => void; onError: (message: string) => void }) {
  const [working, setWorking] = useState('');
  async function act(id: string, action: () => Promise<unknown>) { setWorking(id); try { await action(); } catch (error) { onError(errorMessage(error)); } finally { setWorking(''); } }
  return <section className="saved-searches routines goals"><div className="section-label"><Target size={11} /> GOALS</div>
    {goals.map(goal => {
      const open = goal.state === 'active' || goal.state === 'paused';
      return <article className={`saved-search ${goal.state === 'active' ? '' : 'paused'}`} key={goal.id}>
        <div className="saved-search-heading"><Target size={14} /><strong>{goal.aim}</strong><button className="icon-button" aria-label={`Delete goal: ${goal.aim}`} disabled={!!working} onClick={() => void act(goal.id, () => onAction('delete_goal', { id: goal.id }))}><X size={12} /></button></div>
        <div className="saved-search-info"><span>{goal.state === 'active' ? <><span className="status-dot" />{goalSchedule(goal)}</> : goal.state === 'paused' ? 'Paused' : goal.state === 'done' ? 'Done' : 'Ended'}</span>
          {open && <button className="text-button" disabled={!!working} onClick={() => void act(goal.id, () => onAction('update_goal', { id: goal.id, action: goal.state === 'active' ? 'pause' : 'resume' }))}>{working === goal.id ? <LoaderCircle size={10} className="spin" /> : goal.state === 'active' ? <Square size={9} /> : <Play size={10} />}{goal.state === 'active' ? 'Pause' : 'Resume'}</button>}
          {open && <button className="text-button" disabled={!!working} onClick={() => void act(goal.id, () => onAction('update_goal', { id: goal.id, action: 'done' }))}><Check size={10} />Done</button>}
          {goal.lastTaskId && <button className="text-button" onClick={() => onOpen(goal.lastTaskId!)}>Latest <ChevronRight size={10} /></button>}
        </div>
        {open ? goal.note && <p className="saved-search-error">{goal.note}</p> : goal.outcome && <p className="saved-search-empty">{goal.outcome}</p>}
      </article>;
    })}
    {goals.length === 0 && <p className="saved-search-empty">Ask me to help with something over a few days, like “Follow up with Ana until she replies”.</p>}
  </section>;
}
