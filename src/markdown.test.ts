// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { inline, parse } from './markdown.ts';

test('inline styles become spans, never markup', () => {
  assert.deepEqual(inline('You are in **Finder** with `cmd` and *style*.'), [
    { text: 'You are in ' }, { text: 'Finder', bold: true }, { text: ' with ' }, { text: 'cmd', code: true }, { text: ' and ' }, { text: 'style', italic: true }, { text: '.' },
  ]);
  assert.deepEqual(inline('See [the docs](https://example.com/a) now'), [{ text: 'See ' }, { text: 'the docs', href: 'https://example.com/a' }, { text: ' now' }]);
  assert.deepEqual(inline('[bad](javascript:alert(1)) <b>x</b>'), [{ text: '[bad](javascript:alert(1)) <b>x</b>' }], 'only https links; HTML stays text');
  assert.deepEqual(inline('snake_case_name and 2 * 3 * 4'), [{ text: 'snake_case_name and 2 * 3 * 4' }]);
  assert.deepEqual(inline('half **finished'), [{ text: 'half **finished' }], 'streamed text stays readable');
});

test('replies split into paragraphs, lists and headings', () => {
  const blocks = parse('Here is a summary:\n\n- **Time:** 22:41\n- **App:** Finder\n  \n1. First\n2. Second\n### Done\nLast line\nwraps here');
  assert.deepEqual(blocks.map(block => block.kind), ['paragraph', 'list', 'list', 'heading', 'paragraph']);
  const [intro, bullets, numbers, , closing] = blocks;
  assert.equal(intro.kind === 'paragraph' && intro.lines.length, 1);
  assert.ok(bullets.kind === 'list' && !bullets.ordered && bullets.items.length === 2 && bullets.items[0][0].bold);
  assert.ok(numbers.kind === 'list' && numbers.ordered && numbers.items.length === 2);
  assert.ok(closing.kind === 'paragraph' && closing.lines.length === 2);
  assert.deepEqual(parse('\n\n'), []);
});
