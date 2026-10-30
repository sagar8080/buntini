import type { BackgroundStatus, PendingApprovalSummary, TaskProgress } from './types';

/** Saved proposals from any conversation. Review opens the sheet; nothing is approved from here. */
export function WaitingForYou({ items, disabled, onReview }: { items: PendingApprovalSummary[]; disabled: boolean; onReview: (approvalId: string) => void }) {
  if (!items.length) return null;
  return <section className="waiting-for-you" aria-label="Waiting for you"><div className="section-label">WAITING FOR YOU <span>{items.length}</span></div>
    {items.map(item => <article key={item.approvalId}><div><strong>{item.goal}</strong><small>{item.description} · {new Date(item.createdAt).toLocaleString()}</small></div><button className="text-button" disabled={disabled} onClick={() => onReview(item.approvalId)}>Review</button></article>)}
  </section>;
}

const labels: Record<string, string> = { complete: 'Complete', ready: 'Next', pending: 'Still to do', blocked: 'Needs attention', waiting: 'Waiting for approval', running: 'In progress', succeeded: 'Verified', failed: 'Failed', superseded: 'Resolved by a later read', unknown: 'Outcome unknown', cancelled: 'Cancelled' };

// Steps in plain words; anything else reads as its words with the first one capitalized.
const steps: Record<string, string> = {
  read_mail: 'Read your email', read_calendar: 'Read your calendar', read_reminders: 'Read your reminders', find_notes: 'Find notes', search_files: 'Search your files',
  search_memory: 'Look through what I remember', read_page: 'Read the page', read_source: 'Read a source', read_evidence: 'Check what was found',
  add_reminder: 'Add a reminder', add_calendar_event: 'Add a calendar event', create_note: 'Write a note', draft_email: 'Draft an email', set_timer: 'Set a timer',
  open_link: 'Open a link', open_app: 'Open an app', run_shortcut: 'Run a shortcut', create_saved_search: 'Save a search', create_price_watch: 'Watch a price',
};
export const stepLabel = (tool: string) => steps[tool] ?? (words => words.charAt(0).toUpperCase() + words.slice(1))(tool.replaceAll('_', ' '));

export function ProgressDetails({ progress }: { progress?: TaskProgress }) {
  if (!progress || (!progress.steps.length && !progress.contract?.outcomes.length)) return null;
  const contract = progress.contract;
  return <details className="research-evidence task-progress"><summary>Task progress · {contract?.outcomes.length ? `${contract.outcomes.filter(outcome => outcome.status === 'complete').length}/${contract.outcomes.length} requested results complete` : `${progress.steps.filter(step => step.state === 'succeeded').length}/${progress.steps.length} steps completed`}</summary>
    {!!contract?.outcomes.length && <><p>Requested results</p><ol>{contract.outcomes.map(outcome => <li key={outcome.id}><strong>{outcome.description}</strong><span> · {labels[outcome.status] || (outcome.status === 'pending' ? 'Still to do' : outcome.status.replaceAll('_', ' '))}</span></li>)}</ol></>}
    {!!contract?.unresolvedFields.length && <ul>{contract.unresolvedFields.filter(field => field.status === 'missing').map(field => <li key={`${field.outcomeId}-${field.field}`}>{field.reason}</li>)}</ul>}
    <ol>{progress.steps.map(step => <li key={step.id}><strong>{stepLabel(step.tool)}</strong><span> · {labels[step.state] || step.state}</span>
      {step.approvedAt && <small>Approved {new Date(step.approvedAt).toLocaleString()}</small>}
      <details><summary>Show details</summary><pre>{JSON.stringify({ 'What was used': step.arguments, 'Where each detail came from': contract?.provenance[step.id], 'What came back': step.evidence }, null, 2)}</pre></details>
    </li>)}</ol>
  </details>;
}

export function BackgroundWork({ status, onCancel }: { status: BackgroundStatus | null; onCancel: (id: string) => void }) {
  if (!status) return null;
  const active = status.timers.filter(timer => timer.state === 'pending').length + status.monitors.filter(monitor => monitor.enabled).length;
  return <details className="background-work"><summary>Scheduled work · {active} active</summary><p>{status.limitation}</p>
    {status.timers.map(timer => <article key={timer.id}><strong>{timer.seconds % 60 === 0 ? `${timer.seconds / 60} minute` : `${timer.seconds} second`} timer</strong><span>{timer.state.replaceAll('_', ' ')} · due {new Date(timer.dueAt).toLocaleString()}</span>{timer.lateSeconds > 0 && <small>Delivered or checked {Math.ceil(timer.lateSeconds / 60)} minutes late.</small>}{timer.error && <p role="status">{timer.error}</p>}{timer.state === 'pending' && <button className="text-button" onClick={() => onCancel(timer.id)}>Cancel timer</button>}</article>)}
    {status.monitors.map(monitor => <article key={`${monitor.kind}-${monitor.id}`}><strong>{monitor.label}</strong><span>{monitor.enabled ? `Next check ${new Date(monitor.nextRunAt).toLocaleString()}` : 'Paused'}</span><small>{monitor.lastSuccessAt ? `Last success ${new Date(monitor.lastSuccessAt).toLocaleString()}` : 'No successful check yet'}{monitor.overdueSeconds > 0 ? ' · Overdue; will check when available' : ''}</small>{monitor.lastRun?.error && <p>{monitor.lastRun.error}</p>}{monitor.lastDeliveryError && <p role="status">Notification: {monitor.lastDeliveryError}</p>}</article>)}
    {!status.timers.length && !status.monitors.length && <p>No timers or scheduled checks yet.</p>}
  </details>;
}
