// Disposable UI fixture, available only from the Vite development preview.
// It never invokes Tauri, opens an app, or represents device acceptance.
import type { BackgroundStatus, TaskProgress, TaskRecord } from './types';
export const reliabilityPreview = import.meta.env.DEV && new URLSearchParams(location.search).has('reliability-preview');
const createdAt = '2026-10-22T12:00:00Z';
export const previewReliabilityTasks: TaskRecord[] = [{
  id: 'preview-journey', prompt: 'Preview fixture: save my trip plan and remind me to buy the train ticket.', answer: 'The trip note is verified. Your task is saved and waiting for approval of the reminder.', state: 'waiting_action', createdAt, sources: [], files: [],
  handoff: 'action', action: { id: 'preview-intent', tool: 'add_reminder', title: 'Add this reminder?', note: 'Preview only. Added to your default list in Reminders.', fields: [{ key: 'title', label: 'Reminder', value: 'Buy train ticket', kind: 'text' }, { key: 'due', label: 'When', value: '2026-10-23 09:00', kind: 'when' }] },
}, { id: 'preview-unknown', prompt: 'Preview fixture: save a packing note.', answer: 'The helper stopped after creating the note. Its outcome is unknown; check Notes before making another request.', state: 'unknown', createdAt: '2026-10-22T12:01:00Z', sources: [], files: [] }, {
  id: 'preview-clarification', prompt: 'Preview fixture: save my trip note, then remind me to book the train tomorrow.', answer: 'The trip note is saved. Choose the reminder’s date and time to continue.', state: 'paused', createdAt, sources: [], files: [],
}];
export const previewReliabilityProgress: TaskProgress[] = [{ id: 'preview-journey', goal: previewReliabilityTasks[0].prompt, state: 'waiting_action', updatedAt: createdAt, steps: [
  { id: 'preview-note', tool: 'create_note', arguments: { title: 'Trip plan', body: 'Take the train.' }, state: 'succeeded', approvedAt: createdAt, evidence: { receipt: { outcome: 'succeeded', objectId: 'fixture-note-123', message: 'Saved Trip plan in Notes.', evidence: { readBack: { title: 'Trip plan', body: 'Take the train.' } } } } },
  { id: 'preview-intent', tool: 'add_reminder', arguments: { title: 'Buy train ticket', due: '2026-10-23 09:00' }, state: 'waiting' },
] }, { id: 'preview-clarification', goal: previewReliabilityTasks[2].prompt, state: 'paused', canResume: true, updatedAt: createdAt,
  steps: [{ id: 'saved-note', tool: 'create_note', arguments: { title: 'Trip', body: 'Take the train' }, state: 'succeeded', approvedAt: createdAt, evidence: { receipt: { outcome: 'succeeded', objectId: 'fixture-note' } } }],
  contract: { version: 1, originalRequest: previewReliabilityTasks[2].prompt, currentRequest: previewReliabilityTasks[2].prompt, revision: 1, intentStatus: 'conservative_bootstrap', constraints: [], revisions: [],
    outcomes: [
      { id: 'outcome-1', kind: 'write', description: 'Save my trip note', tool: 'create_note', acceptedTools: ['create_note'], resources: ['notes'], dependencies: [], status: 'complete', requiredArguments: {}, requiredFields: ['title', 'body'], completedBy: ['saved-note'], sourceRevision: 1 },
      { id: 'outcome-2', kind: 'write', description: 'Remind me to book the train', tool: 'add_reminder', acceptedTools: ['add_reminder'], resources: ['reminders'], dependencies: ['outcome-1'], status: 'ready', requiredArguments: { title: 'Book the train' }, requiredFields: ['title', 'due'], completedBy: [], sourceRevision: 1 },
    ], unresolvedFields: [{ outcomeId: 'outcome-2', field: 'due', status: 'missing', reason: 'Choose when the reminder is due.' }],
    provenance: { 'saved-note': { title: { value: 'Trip', origin: 'user_literal', approved: true, verified: true }, body: { value: 'Take the train', origin: 'user_literal', approved: true, verified: true } } },
  },
}];
export const previewBackground: BackgroundStatus = { executionMode: 'app_open_and_awake', limitation: 'Buntini checks while it is running and your Mac is awake. Overdue work is checked once when Buntini returns.', timers: [{ id: 'preview-timer', seconds: 600, dueAt: '2026-10-23T12:10:00Z', state: 'pending', lateSeconds: 0 }], monitors: [{ id: 'preview-monitor', kind: 'saved-search', label: 'Train fare updates', enabled: true, nextRunAt: '2026-10-23T13:00:00Z', lastSuccessAt: createdAt, overdueSeconds: 0, lastDeliveryError: 'Notifications are disabled in System Settings.' }] };
