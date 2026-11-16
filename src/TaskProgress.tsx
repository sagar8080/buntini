import { stillOpen } from './taskRecovery';
import { locale, t, tn } from './i18n.ts';
import type { BackgroundStatus, HeadsUp, PendingApprovalSummary, TaskProgress } from './types';

const when = (value: string) => new Date(value).toLocaleString(locale());

/** Saved proposals from any conversation. Review opens the sheet; nothing is approved from here.
    Stop ends one that can't be reviewed any more (say, after switching brains). */
export function WaitingForYou({ items, disabled, onReview, onStop }: { items: PendingApprovalSummary[]; disabled: boolean; onReview: (approvalId: string) => void; onStop: (taskId: string) => void }) {
  if (!items.length) return null;
  return <section className="waiting-for-you" aria-label={t('tasks.waiting')}><div className="section-label">{t('tasks.waiting')}</div>
    {items.map(item => <article key={item.approvalId}><div><strong>{item.goal}</strong><small>{item.description} · {when(item.createdAt)}</small></div><button className="text-button" disabled={disabled} onClick={() => onReview(item.approvalId)}>{t('tasks.review')}</button><button className="text-button" disabled={disabled} aria-label={t('tasks.stopNamed', { goal: item.goal })} onClick={() => onStop(item.taskId)}>{t('common.stop')}</button></article>)}
  </section>;
}

/** Heads-ups from this Mac's own data. Open starts a conversation with the details; nothing is changed from here. */
export function HeadsUps({ items, disabled, onOpen, onDismiss }: { items: HeadsUp[]; disabled: boolean; onOpen: (id: string) => void; onDismiss: (id: string) => void }) {
  if (!items.length) return null;
  return <section className="waiting-for-you heads-ups" aria-label={t('tasks.headsUps')}><div className="section-label">{t('tasks.headsUp')}</div>
    {items.map(item => <article key={item.id}><div><strong>{item.title}</strong><small>{item.detail}</small></div><button className="text-button" disabled={disabled} onClick={() => onOpen(item.id)}>{t('common.open')}</button><button className="text-button" disabled={disabled} aria-label={t('tasks.dismissNamed', { title: item.title })} onClick={() => onDismiss(item.id)}>{t('common.dismiss')}</button></article>)}
  </section>;
}

const labels: Record<string, string> = { complete: 'tasks.status.complete', ready: 'tasks.status.ready', pending: 'tasks.status.pending', blocked: 'tasks.status.blocked', waiting: 'tasks.status.waiting', running: 'tasks.status.running', succeeded: 'tasks.status.succeeded', failed: 'tasks.status.failed', superseded: 'tasks.status.superseded', unknown: 'tasks.status.unknown', cancelled: 'tasks.status.cancelled' };
const statusLabel = (status: string) => labels[status] ? t(labels[status]) : undefined;

// Steps in plain words; anything else reads as its words with the first one capitalized.
const steps: Record<string, string> = {
  read_mail: 'tasks.step.readMail', read_calendar: 'tasks.step.readCalendar', read_reminders: 'tasks.step.readReminders', find_notes: 'tasks.step.findNotes', search_files: 'tasks.step.searchFiles',
  search_memory: 'tasks.step.searchMemory', read_page: 'tasks.step.readPage', read_source: 'tasks.step.readSource', read_evidence: 'tasks.step.readEvidence',
  add_reminder: 'tasks.step.addReminder', add_calendar_event: 'tasks.step.addCalendarEvent', create_note: 'tasks.step.createNote', draft_email: 'tasks.step.draftEmail', reply_email: 'tasks.step.replyEmail', send_message: 'tasks.step.sendMessage', start_call: 'tasks.step.startCall', set_timer: 'tasks.step.setTimer', change_mac_setting: 'tasks.step.changeMacSetting',
  open_link: 'tasks.step.openLink', open_app: 'tasks.step.openApp', run_shortcut: 'tasks.step.runShortcut', create_saved_search: 'tasks.step.createSavedSearch', create_price_watch: 'tasks.step.createPriceWatch', add_to_cart: 'tasks.step.addToCart', create_routine: 'tasks.step.createRoutine', create_goal: 'tasks.step.createGoal',
  complete_reminder: 'tasks.step.completeReminder', reschedule_reminder: 'tasks.step.rescheduleReminder', update_calendar_event: 'tasks.step.updateCalendarEvent', append_to_note: 'tasks.step.appendToNote', list_shortcuts: 'tasks.step.listShortcuts',
};
export const stepLabel = (tool: string) => tool.startsWith('mcp_') ? t('tasks.step.mcp') : steps[tool] ? t(steps[tool]) : (words => words.charAt(0).toUpperCase() + words.slice(1))(tool.replaceAll('_', ' '));

const timerStates: Record<string, string> = { pending: 'tasks.timer.state.pending', claimed: 'tasks.timer.state.claimed', completed: 'tasks.timer.state.completed', cancelled: 'tasks.timer.state.cancelled', delivery_unknown: 'tasks.timer.state.deliveryUnknown', notification_failed: 'tasks.timer.state.notificationFailed' };
const timerState = (state: string) => timerStates[state] ? t(timerStates[state]) : state.replaceAll('_', ' ');

export function ProgressDetails({ progress }: { progress?: TaskProgress }) {
  // A finished answer speaks for itself; this is for what stopped one finishing.
  if (!progress || !stillOpen(progress)) return null;
  const contract = progress.contract;
  return <details className="research-evidence task-progress"><summary>{t('tasks.stillOpen')}</summary>
    {!!contract?.outcomes.length && <><p>{t('tasks.requested')}</p><ol>{contract.outcomes.map(outcome => <li key={outcome.id}><strong>{outcome.description}</strong><span> · {statusLabel(outcome.status) || outcome.status.replaceAll('_', ' ')}</span></li>)}</ol></>}
    {!!contract?.unresolvedFields.length && <ul>{contract.unresolvedFields.filter(field => field.status === 'missing').map(field => <li key={`${field.outcomeId}-${field.field}`}>{field.reason}</li>)}</ul>}
    <ol>{progress.steps.map(step => <li key={step.id}><strong>{stepLabel(step.tool)}</strong><span> · {statusLabel(step.state) || step.state}</span>
      {step.approvedAt && <small>{t('tasks.approvedAt', { when: when(step.approvedAt) })}</small>}
      <details><summary>{t('tasks.showDetails')}</summary><pre>{JSON.stringify({ [t('tasks.details.used')]: step.arguments, [t('tasks.details.sources')]: contract?.provenance[step.id], [t('tasks.details.returned')]: step.evidence }, null, 2)}</pre></details>
    </li>)}</ol>
  </details>;
}

export function BackgroundWork({ status, onCancel }: { status: BackgroundStatus | null; onCancel: (id: string) => void }) {
  if (!status) return null;
  return <details className="background-work"><summary>{t('tasks.scheduled')}</summary>
    {status.timers.map(timer => <article key={timer.id}><strong>{timer.seconds % 60 === 0 ? tn('tasks.timer.minutes', timer.seconds / 60) : tn('tasks.timer.seconds', timer.seconds)}</strong><span>{timerState(timer.state)} · {t('tasks.timer.due', { when: when(timer.dueAt) })}</span>{timer.lateSeconds > 0 && <small>{tn('tasks.timer.late', Math.ceil(timer.lateSeconds / 60))}</small>}{timer.error && <p role="status">{timer.error}</p>}{timer.state === 'pending' && <button className="text-button" onClick={() => onCancel(timer.id)}>{t('tasks.timer.cancel')}</button>}</article>)}
    {status.monitors.map(monitor => <article key={`${monitor.kind}-${monitor.id}`}><strong>{monitor.label}</strong><span>{monitor.enabled ? t('tasks.monitor.nextCheck', { when: when(monitor.nextRunAt) }) : t('common.paused')}</span><small>{monitor.lastSuccessAt ? t('tasks.monitor.lastSuccess', { when: when(monitor.lastSuccessAt) }) : t('tasks.monitor.noSuccess')}{monitor.overdueSeconds > 0 ? ` · ${t('tasks.monitor.overdue')}` : ''}</small>{monitor.lastRun?.error && <p>{monitor.lastRun.error}</p>}{monitor.lastDeliveryError && <p role="status">{t('tasks.monitor.deliveryError', { error: monitor.lastDeliveryError })}</p>}</article>)}
    {!status.timers.length && !status.monitors.length && <p></p>}
  </details>;
}
