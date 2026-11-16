// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { added, defaults, mailboxName, preview, ruleLine, ruleReady, sameMailbox } from './model.ts';

test('a word is added once whatever its case, and blanks are not added', () => {
  assert.deepEqual(added(['Lottery'], ' lottery '), ['Lottery']);
  assert.deepEqual(added(['Lottery'], 'prize'), ['Lottery', 'prize']);
  assert.deepEqual(added([], '   '), []);
});

test('a rule reads as one line and is ready only as Rust accepts it', () => {
  const to = { account: '', path: 'Newsletters' };
  assert.equal(ruleLine({ id: 'r', field: 'domain', value: 'news.example.com', action: 'move', to, enabled: true }), 'Domain “news.example.com” → To Newsletters');
  assert.equal(ruleLine({ id: 'r', field: 'subject', value: 'receipt', action: 'read', to: null, enabled: true }), 'Subject “receipt” → Read');
  assert.equal(ruleReady({ field: 'sender', value: 'x', action: 'move', to, enabled: true }), true);
  assert.equal(ruleReady({ field: 'sender', value: 'x', action: 'move', to: null, enabled: true }), false, 'a move needs a mailbox');
  assert.equal(ruleReady({ field: 'sender', value: 'x', action: 'flag', to, enabled: true }), false, 'only a move has a mailbox');
  assert.equal(ruleReady({ field: 'sender', value: '  ', action: 'flag', to: null, enabled: true }), false);
  assert.equal(ruleReady({ field: 'sender', value: 'a\nb', action: 'flag', to: null, enabled: true }), false);
});

test('mailboxes read with their account and compare without case', () => {
  assert.equal(mailboxName({ account: 'iCloud', path: 'Archive/2026' }), 'Archive/2026 · iCloud');
  assert.equal(mailboxName({ account: '', path: 'Receipts' }), 'Receipts');
  assert.ok(sameMailbox({ account: 'iCloud', path: 'INBOX' }, { account: 'iCloud', path: 'Inbox' }));
  assert.ok(!sameMailbox({ account: 'iCloud', path: 'INBOX' }, { account: '', path: 'INBOX' }));
});

test('the browser preview reads but never changes mail', () => {
  assert.deepEqual(defaults, { rules: [], banned: [], vips: [] });
  assert.equal((preview({ op: 'list', filter: 'unread' }) as { filter: string }).filter, 'unread');
  assert.throws(() => preview({ op: 'approve', id: 'x' }));
  assert.throws(() => preview({ op: 'junk', id: '14' }));
});
