import type { BackgroundStatus, TaskProgress } from './types';

const labels: Record<string, string> = { complete: 'Complete', ready: 'Next', pending: 'Still to do', blocked: 'Needs attention', waiting: 'Waiting for approval', running: 'In progress', succeeded: 'Verified', failed: 'Failed', superseded: 'Resolved by a later read', unknown: 'Outcome unknown', cancelled: 'Cancelled' };

export function ProgressDetails({ progress }: { progress?: TaskProgress }) {
  if (!progress || (!progress.steps.length && !progress.contract?.outcomes.length)) return null;
  const contract = progress.contract;
  return <details className="research-evidence task-progress"><summary>Task progress · {contract?.outcomes.length ? `${contract.outcomes.filter(outcome => outcome.status === 'complete').length}/${contract.outcomes.length} requested results complete` : `${progress.steps.filter(step => step.state === 'succeeded').length}/${progress.steps.length} steps completed`}</summary>
    {!!contract?.outcomes.length && <><p>Requested results</p><ol>{contract.outcomes.map(outcome => <li key={outcome.id}><strong>{outcome.description}</strong><span> · {labels[outcome.status] || (outcome.status === 'pending' ? 'Still to do' : outcome.status.replaceAll('_', ' '))}</span></li>)}</ol></>}
    {!!contract?.unresolvedFields.length && <ul>{contract.unresolvedFields.filter(field => field.status === 'missing').map(field => <li key={`${field.outcomeId}-${field.field}`}>{field.reason}</li>)}</ul>}
    <ol>{progress.steps.map(step => <li key={step.id}><strong>{step.tool.replaceAll('_', ' ')}</strong><span> · {labels[step.state] || step.state}</span>
      {step.approvedAt && <small>Approved {new Date(step.approvedAt).toLocaleString()}</small>}
      <details><summary>Inspect inputs and result</summary><pre>{JSON.stringify({ inputs: step.arguments, fieldSources: contract?.provenance[step.id], result: step.evidence }, null, 2)}</pre></details>
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
