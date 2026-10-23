// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { taskRecovery, missingTaskFields } from './taskRecovery.ts';
import type { TaskProgress, TaskRecord } from './types';

const progress = (state: string, canResume?: boolean): TaskProgress => ({ id: 'task', goal: 'Read calendar then add a reminder', state, canResume, updatedAt: '', steps: [] });

test('failed reads expose continuation only with backend retry eligibility', () => {
  assert.deepEqual(taskRecovery('error', progress('failed', true)), { canContinue: true, retryRead: true, canReview: false });
  for (const saved of [undefined, progress('failed'), progress('failed', false), progress('unknown', true)]) {
    assert.equal(taskRecovery('error', saved).canContinue, false);
  }
});

test('unknown, cancelled, completed and working tasks never expose retry controls', () => {
  for (const state of ['unknown', 'cancelled', 'completed', 'running'] as TaskRecord['state'][]) {
    assert.deepEqual(taskRecovery(state, progress('failed', true)), { canContinue: false, retryRead: false, canReview: false });
  }
  assert.equal(taskRecovery('paused', progress('paused', false)).canContinue, false);
  assert.equal(taskRecovery('paused', progress('paused', true)).canContinue, true);
});

test('pending approvals retain review and do not become read retries', () => {
  assert.deepEqual(taskRecovery('waiting_action', progress('waiting_action', false)), { canContinue: false, retryRead: false, canReview: true });
  assert.deepEqual(taskRecovery('waiting_online', progress('waiting_online', false)), { canContinue: false, retryRead: false, canReview: true });
});

test('clarification targets missing inputs on unfinished actions only', () => {
  const saved = progress('paused', true);
  saved.contract = { version: 1, originalRequest: '', currentRequest: '', revision: 1, intentStatus: 'conservative_bootstrap', constraints: [], provenance: {}, revisions: [],
    outcomes: ['complete', 'ready', 'waiting'].map((status, i) => ({ id: `outcome-${i}`, kind: 'write', description: '', acceptedTools: [], resources: [], dependencies: [], status, requiredArguments: {}, requiredFields: [], completedBy: [], sourceRevision: 1 })),
    unresolvedFields: [
      { outcomeId: 'outcome-0', field: 'title', reason: '', status: 'missing' },
      { outcomeId: 'outcome-1', field: 'due', reason: '', status: 'missing' },
      { outcomeId: 'outcome-1', field: 'title', reason: '', status: 'unverified' },
      { outcomeId: 'outcome-2', field: 'body', reason: '', status: 'missing' },
      { outcomeId: 'outcome-1', field: 'unsupported', reason: '', status: 'missing' },
    ] };
  const fields = missingTaskFields(saved);
  assert.equal(fields.length, 1);
  assert.equal(fields[0].label, 'Due date and time');
  assert.equal(fields[0].outcomeId, 'outcome-1');
  assert.equal(fields[0].date, true);
  assert.deepEqual(missingTaskFields(), []);
});
