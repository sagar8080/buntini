// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { addFiles, addWindow, chipLabel, isPicture, maxAttached, removeAttached } from './attachments.ts';
import type { Attached } from './types';

test('files are attached once each, up to the limit, and only by full path', () => {
  let attached = addFiles([], ['/Users/me/lease.pdf', '/Users/me/lease.pdf', 'relative.txt']);
  assert.deepEqual(attached, [{ kind: 'file', path: '/Users/me/lease.pdf' }]);
  attached = addFiles(attached, Array.from({ length: 10 }, (_, index) => `/tmp/${index}.txt`));
  assert.equal(attached.length, maxAttached);
  assert.equal(chipLabel(attached[0]), 'lease.pdf');
});

test('a window replaces the one attached before and can be removed', () => {
  const first = { kind: 'window', id: 'a', app: 'Safari', title: 'Flights to Lisbon' } as const;
  const second = { kind: 'window', id: 'b', app: 'TextEdit', title: '' } as const;
  let attached: Attached[] = addFiles([], ['/tmp/a.txt']);
  attached = addWindow(attached, first).next;
  const swapped = addWindow(attached, second);
  assert.equal(swapped.replaced?.id, 'a');
  assert.deepEqual(swapped.next.map(chipLabel), ['a.txt', 'TextEdit']);
  assert.deepEqual(removeAttached(swapped.next, { ...second }).map(chipLabel), ['a.txt']);
  assert.deepEqual(removeAttached(swapped.next, { kind: 'file', path: '/tmp/a.txt' }).map(chipLabel), ['TextEdit']);
});

test('pictures are told apart by their name', () => {
  assert.ok(isPicture('/tmp/IMG_0001.HEIC') && isPicture('/tmp/shot.png'));
  assert.ok(!isPicture('/tmp/report.pdf'));
});
