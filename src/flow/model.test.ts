// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { chordFrom, defaults, minutes, preview } from './model.ts';

const press = (code: string, key: string, modifiers: Partial<Record<'metaKey' | 'altKey' | 'ctrlKey' | 'shiftKey', boolean>> = {}) =>
  ({ code, key, metaKey: false, altKey: false, ctrlKey: false, shiftKey: false, ...modifiers });

test('a chord needs a modifier and reads as macOS writes it', () => {
  assert.deepEqual(chordFrom(press('KeyD', 'd', { ctrlKey: true, altKey: true })), { keyCode: 2, modifiers: ['control', 'option'], label: '⌃⌥D' });
  assert.deepEqual(chordFrom(press('Space', ' ', { metaKey: true, shiftKey: true })), { keyCode: 49, modifiers: ['shift', 'command'], label: '⇧⌘Space' });
  assert.equal(chordFrom(press('KeyD', 'd')), null, 'a bare key would type');
  assert.equal(chordFrom(press('F13', 'F13', { ctrlKey: true })), null, 'a key without a known code');
});

test('call lengths read as minutes and seconds, or hours past one', () => {
  assert.equal(minutes(0), '0:00');
  assert.equal(minutes(75.4), '1:15');
  assert.equal(minutes(3725), '1:02:05');
});

test('dictation starts off, and the preview answers what the Library asks', () => {
  assert.equal(defaults.enabled, false);
  assert.equal(defaults.mute, true);
  assert.equal(preview({ op: 'ping' }), true);
  assert.ok(Array.isArray(preview({ op: 'history' })));
  assert.throws(() => preview({ op: 'downloadModels' }));
});
