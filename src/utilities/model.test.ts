// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { held, keycaps, preview, searchSettings, settingsIndex, shortcutFrom, type ShortcutRow } from './model.ts';

const press = (code: string, modifiers: Partial<Record<'metaKey' | 'altKey' | 'ctrlKey' | 'shiftKey', boolean>> = {}) =>
  ({ code, metaKey: false, altKey: false, ctrlKey: false, shiftKey: false, ...modifiers });

test('a key press becomes a shortcut that reads as macOS writes it', () => {
  assert.deepEqual(shortcutFrom(press('Digit2', { metaKey: true, shiftKey: true })), { keyCode: 19, modifiers: ['shift', 'command'], label: '⇧⌘2' });
  assert.deepEqual(shortcutFrom(press('KeyF', { ctrlKey: true, altKey: true })), { keyCode: 3, modifiers: ['control', 'option'], label: '⌃⌥F' });
  assert.deepEqual(shortcutFrom(press('F5')), { keyCode: 96, modifiers: [], label: 'F5' }, 'a function key on its own');
  assert.equal(shortcutFrom(press('ArrowUp', { ctrlKey: true }))?.label, '⌃↑');
  assert.equal(shortcutFrom(press('ShiftLeft', { shiftKey: true })), null, 'a modifier alone keeps listening');
  // A bare letter is passed on, so Buntini can say what it needs.
  assert.deepEqual(shortcutFrom(press('KeyA')), { keyCode: 0, modifiers: [], label: 'A' });
});

test('held modifiers and keycaps read in the Mac’s order', () => {
  assert.equal(held({ metaKey: true, altKey: true, ctrlKey: true, shiftKey: true }), '⌃⌥⇧⌘');
  assert.deepEqual(keycaps('⇧⌘Space'), ['⇧', '⌘', 'Space']);
  assert.deepEqual(keycaps('F5'), ['F5']);
  assert.deepEqual(keycaps('⌃⌥'), ['⌃', '⌥']);
});

const first = (query: string) => searchSettings(query)[0]?.label;

test('settings are found by their label', () => {
  assert.equal(first('Less motion'), 'Less motion');
  assert.equal(first('open at'), 'Open at login');
  assert.equal(first('screensh'), 'Screenshot');
  assert.equal(first('monthly'), 'Monthly cap');
  assert.equal(first('folders'), 'Folders');
});

test('settings are found by the words people use', () => {
  assert.equal(first('dark mode'), 'Appearance');
  assert.equal(first('pomodoro'), 'Focus timer');
  assert.equal(first('hotkeys'), 'Shortcuts');
  assert.equal(first('wake word'), 'Hey Buntini');
  assert.equal(first('startup'), 'Open at login');
  assert.equal(first('ollama'), 'Server address');
  assert.equal(first('mcp'), 'Tools you added');
  assert.equal(first('notifications'), 'Desktop notifications');
  assert.equal(first('reduce motion'), 'Less motion');
  assert.equal(first('Claude Code'), 'Coding agents');
});

test('every word must match, and nonsense finds nothing', () => {
  assert.deepEqual(searchSettings('   '), []);
  assert.deepEqual(searchSettings('zzqx'), []);
  assert.deepEqual(searchSettings('dark zzqx'), []);
  assert.ok(searchSettings('s').length <= 6, 'a short query still shows a few');
});

test('each tab has settings and no label is listed twice on one tab', () => {
  for (const tab of ['buntini', 'brain', 'apps', 'connections', 'privacy']) assert.ok(settingsIndex.some(entry => entry.tab === tab), tab);
  const seen = new Set(settingsIndex.map(entry => `${entry.tab}:${entry.label}`));
  assert.equal(seen.size, settingsIndex.length);
});

test('the preview keeps shortcuts and refuses what Buntini would', () => {
  const rows = preview({ op: 'shortcuts' }) as ShortcutRow[];
  assert.deepEqual(rows.map(row => row.shortcut?.label), ['⇧⌘Space', '⌃⌥Space', '⌃⌥A', '⌃⌥]', '⌃⌥M', '⇧⌘2', '⇧⌘S', '⌃⌥F']);
  assert.throws(() => preview({ op: 'setShortcut', id: 'focus', shortcut: { keyCode: 3, modifiers: ['shift'], label: '⇧F' } }), /Add ⌘, ⌃ or ⌥/);
  assert.throws(() => preview({ op: 'setShortcut', id: 'focus', shortcut: { keyCode: 19, modifiers: ['command', 'shift'], label: '⇧⌘2' } }), /Screenshot uses it/);
  const next = preview({ op: 'setShortcut', id: 'focus', shortcut: { keyCode: 3, modifiers: ['control', 'option', 'command'], label: '⌃⌥⌘F' } }) as ShortcutRow[];
  assert.equal(next.find(row => row.id === 'focus')?.shortcut?.label, '⌃⌥⌘F');
  assert.equal((preview({ op: 'setShortcut', id: 'mute', shortcut: null }) as ShortcutRow[]).find(row => row.id === 'mute')?.shortcut, null);
  assert.equal((preview({ op: 'resetShortcuts' }) as ShortcutRow[]).find(row => row.id === 'focus')?.shortcut?.label, '⌃⌥F');
});
