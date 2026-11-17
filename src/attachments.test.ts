// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { addFiles, addWindow, brainKey, chipLabel, forBrain, isPicture, maxAttached, removeAttached } from './attachments.ts';
import type { Attached } from './types';

test('files are attached once each, up to the limit, and only by full path', () => {
  let attached = addFiles([], ['/Users/me/lease.pdf', '/Users/me/lease.pdf', 'relative.txt'], 'apple');
  assert.deepEqual(attached, [{ kind: 'file', path: '/Users/me/lease.pdf', brain: 'apple' }]);
  attached = addFiles(attached, Array.from({ length: 10 }, (_, index) => `/tmp/${index}.txt`), 'apple');
  assert.equal(attached.length, maxAttached);
  assert.equal(chipLabel(attached[0]), 'lease.pdf');
});

test('a window replaces the one attached before and can be removed', () => {
  const first = { kind: 'window', id: 'a', app: 'Notes', title: 'Flights to Lisbon', brain: 'apple' } as const;
  const second = { kind: 'window', id: 'b', app: 'TextEdit', title: '', brain: 'apple' } as const;
  let attached: Attached[] = addFiles([], ['/tmp/a.txt'], 'apple');
  attached = addWindow(attached, first).next;
  const swapped = addWindow(attached, second);
  assert.equal(swapped.replaced?.id, 'a');
  assert.deepEqual(swapped.next.map(chipLabel), ['a.txt', 'TextEdit']);
  assert.deepEqual(removeAttached(swapped.next, { ...second }).map(chipLabel), ['a.txt']);
  assert.deepEqual(removeAttached(swapped.next, { kind: 'file', path: '/tmp/a.txt', brain: 'apple' }).map(chipLabel), ['TextEdit']);
});

test('pictures are told apart by their name', () => {
  assert.ok(isPicture('/tmp/IMG_0001.HEIC') && isPicture('/tmp/shot.png'));
  assert.ok(!isPicture('/tmp/report.pdf'));
});

test('what was attached goes only to the brain it was attached under', () => {
  const base = { modelBackend: 'apple', cloudProvider: 'chatgpt', cloudUrl: '' } as const;
  assert.equal(brainKey(base), 'apple');
  assert.equal(brainKey({ ...base, modelBackend: 'cloud' }), 'cloud:chatgpt');
  assert.equal(brainKey({ modelBackend: 'cloud', cloudProvider: 'custom', cloudUrl: ' 127.0.0.1:11434 ' }), 'cloud:custom:127.0.0.1:11434');
  const attached: Attached[] = [...addFiles([], ['/tmp/a.txt'], 'apple'), { kind: 'window', id: 'w', app: 'Notes', title: 'Plan', brain: 'apple' }];
  const moved = forBrain(attached, brainKey({ ...base, modelBackend: 'cloud' }));
  assert.deepEqual(moved.kept, [], 'Buntini moved to ChatGPT: nothing attached under Apple Intelligence goes there');
  assert.equal(moved.dropped.length, 2);
  assert.deepEqual(forBrain(attached, 'apple').kept, attached);
});
