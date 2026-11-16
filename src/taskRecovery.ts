import type { TaskProgress, TaskRecord } from './types';

/** The backend owns retry eligibility; the UI never infers safety from prose. */
export function taskRecovery(state: TaskRecord['state'], progress?: TaskProgress) {
  const retryRead = state === 'error' && progress?.state === 'failed' && progress.canResume === true;
  const paused = state === 'paused' && progress?.canResume !== false;
  return { canContinue: paused || retryRead, retryRead, canReview: state === 'waiting_action' || state === 'waiting_online' };
}

/** Only unfinished action fields can be clarified; executed actions stay fixed. */
export function missingTaskFields(progress?: TaskProgress) {
  const labels: Record<string, string> = { title: 'Title', body: 'Text', due: 'Due date and time', start: 'Start date and time', to: 'Recipient', subject: 'Subject', name: 'Name', fact: 'What to remember' };
  return (progress?.contract?.unresolvedFields || []).filter(field => field.status === 'missing' && labels[field.field]
    && progress?.contract?.outcomes.some(outcome => outcome.id === field.outcomeId && outcome.kind === 'write' && ['ready', 'pending'].includes(outcome.status)))
    .map(field => ({ ...field, label: labels[field.field], key: `${field.outcomeId}:${field.field}`, date: field.field === 'due' || field.field === 'start' }));
}

/** Whether a task's steps left anything undone. Nothing ran means nothing to show. */
export function stillOpen(progress: TaskProgress) {
  if (!progress.steps.length) return false;
  const outcomes = progress.contract?.outcomes ?? [];
  return outcomes.length ? outcomes.some(outcome => outcome.status !== 'complete') : progress.steps.some(step => step.state !== 'succeeded' && step.state !== 'superseded');
}
