// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { groupShared, setGroup, sharedSentence, shareGroups } from './cloudShares.ts';

const group = (id: string) => shareGroups.find(item => item.id === id)!;

test('a group is ticked only when all of it is shared, and ticking sets all of it', () => {
  assert.equal(groupShared(group('personal'), ['mail', 'calendar']), false, 'partly shared reads as off');
  const shared = setGroup(group('personal'), ['mail', 'files'], true);
  assert.deepEqual([...shared].sort(), ['calendar', 'contacts', 'files', 'mail', 'notes']);
  assert.equal(groupShared(group('personal'), shared), true);
  assert.deepEqual(setGroup(group('personal'), shared, false), ['files'], 'other kinds stay');
});

test('every group names kinds Settings → Privacy knows, each once', () => {
  const kinds = shareGroups.flatMap(item => item.shares);
  assert.equal(new Set(kinds).size, kinds.length);
  for (const kind of kinds) assert.ok(['activity', 'system', 'files', 'history', 'memory', 'mail', 'calendar', 'notes', 'music', 'browser', 'selection', 'contacts', 'shortcuts', 'tools'].includes(kind), kind);
});

test('the choice reads in the user’s words', () => {
  assert.equal(sharedSentence([]), 'It sees nothing personal.');
  assert.equal(sharedSentence(['files']), 'It may see my files.');
  assert.equal(sharedSentence(setGroup(group('doing'), ['files'], true)), 'It may see my files and what I’m doing on the Mac.');
});
