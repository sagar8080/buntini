// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { greeting, mergeTurns, threadItems } from './thread.ts';
import type { TaskRecord } from './types';

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
});
