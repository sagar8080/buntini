// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { greeting, invitation, mergeTurns, mergeConversationTurns, threadItems } from './thread.ts';
import type { ConversationPage, TaskRecord } from './types';

const turn = (id: string, createdAt: string, answer = ''): TaskRecord => ({ id, prompt: id, answer, state: 'completed', createdAt, sources: [], files: [] });

test('merged turns stay chronological and replace earlier copies', () => {
  const thread = [turn('a', '2026-10-21T10:00:00Z'), turn('b', '2026-10-21T11:00:00Z', 'draft')];
  const merged = mergeTurns(thread, [turn('b', '2026-10-21T11:00:00Z', 'final'), turn('older', '2026-10-21T09:00:00Z'), turn('', '2026-10-21T12:00:00Z')]);
  assert.deepEqual(merged.map(item => item.id), ['older', 'a', 'b']);
  assert.equal(merged[2].answer, 'final');
});

test('the new topic divider sits before the first turn after the boundary', () => {
  const turns = [turn('a', '2026-10-21T10:00:00Z'), turn('b', '2026-10-21T11:00:00Z')];
  const kinds = (boundary?: string) => threadItems(turns, boundary).map(item => item.kind === 'turn' ? item.turn.id : 'topic');
  assert.deepEqual(kinds(), ['a', 'b']);
  assert.deepEqual(kinds('2026-10-21T10:30:00Z'), ['a', 'topic', 'b']);
  assert.deepEqual(kinds('2026-10-21T12:00:00Z'), ['a', 'b', 'topic'], 'a topic started after the last turn shows at the end');
  assert.deepEqual(kinds('2026-10-21T09:00:00Z'), ['a', 'b'], 'loaded turns are all from the current topic');
  assert.deepEqual(threadItems([], '2026-10-21T09:00:00Z'), []);
});

test('greetings follow the time of day', () => {
  const at = (hour: number) => new Date(2026, 9, 21, hour);
  assert.equal(greeting(at(8), 'Sam'), 'Good morning, Sam.');
  assert.equal(greeting(at(14)), 'Good afternoon.');
  assert.equal(greeting(at(19), '  '), 'Good evening.');
  assert.equal(greeting(at(2)), 'Up late.');
  assert.equal(invitation(at(8)), 'Ready when you are.');
  assert.equal(invitation(at(19)), 'Winding down? I’m all ears.');
  assert.equal(invitation(at(23)), 'I’m here if you need me.');
});

test('opening a saved conversation replaces the current thread and refresh keeps older pages', () => {
  const page: ConversationPage = { conversation: { id: 'trip', title: 'Trip', createdAt: '', updatedAt: '', legacy: false, turnCount: 2 }, turns: [turn('trip-latest', '2026-10-21T12:00:00Z')], nextCursor: 'trip-latest' };
  const switched = mergeConversationTurns('shopping', [turn('shopping-secret', '2026-10-21T13:00:00Z')], page);
  assert.deepEqual(switched.map(turn => turn.id), ['trip-latest']);
  const loaded = mergeTurns(switched, [turn('trip-older', '2026-10-21T11:00:00Z')]);
  const refreshed = mergeConversationTurns('trip', loaded, { ...page, turns: [turn('trip-latest', '2026-10-21T12:00:00Z', 'Verified result')] });
  assert.deepEqual(refreshed.map(turn => turn.id), ['trip-older', 'trip-latest']);
  assert.equal(refreshed[1].answer, 'Verified result');
  assert.deepEqual(mergeConversationTurns('trip', refreshed, { ...page, conversation: { ...page.conversation, id: 'new-topic' }, turns: [] }), []);
});

test('pagination sorts timestamp ties by stable task ID', () => {
  const sameTime = '2026-10-21T12:00:00Z';
  const latest = [turn('turn-003', sameTime), turn('turn-002', sameTime)];
  const earlier = [turn('turn-001', sameTime), turn('turn-000', sameTime)];
  assert.deepEqual(mergeTurns(latest, earlier).map(turn => turn.id), ['turn-000', 'turn-001', 'turn-002', 'turn-003']);
});
