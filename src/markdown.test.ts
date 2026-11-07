// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { inline, parse, speakable, withoutTracking } from './markdown.ts';

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
  const split = parse('1. **Tea**\n\nBrew a pot.\n\n2. **Books**\n\nRead a little.').filter(block => block.kind === 'list');
  assert.deepEqual(split.map(block => block.kind === 'list' ? block.start : 0), [1, 2], 'a split list keeps counting');
  assert.ok(closing.kind === 'paragraph' && closing.lines.length === 2);
  assert.deepEqual(parse('\n\n'), []);
});

test('a web citation becomes its own span, without the brackets or the tracking tag', () => {
  assert.deepEqual(inline('Rates rose. ([reuters.com](https://www.reuters.com/a?utm_source=openai))'), [
    { text: 'Rates rose. ' }, { text: 'reuters.com', cite: true, href: 'https://www.reuters.com/a' },
  ]);
  assert.deepEqual(inline('Two ([a.com](https://a.com/x), [b.com](https://b.com/y)) here').map(span => span.cite ? span.text : ''), ['', 'a.com', 'b.com', '']);
  assert.deepEqual(inline('Old ([a.org](http://a.org/x))')[1], { text: 'a.org', cite: true }, 'only https opens');
  assert.deepEqual(inline('(see [the docs](https://example.com) too)').map(span => !!span.cite), [false, false, false], 'a link in brackets is an ordinary link');
  assert.equal(withoutTracking('https://a.com/x?id=1&utm_source=openai'), 'https://a.com/x?id=1');
  assert.equal(withoutTracking('https://a.com/x?utm_source=news'), 'https://a.com/x?utm_source=news');
  assert.equal(withoutTracking('not a url'), 'not a url');
});

test('styles hold each other, and links keep their brackets and escapes straight', () => {
  assert.deepEqual(inline('**[Dune](https://a.com/dune)** is out'), [{ text: 'Dune', bold: true, href: 'https://a.com/dune' }, { text: ' is out' }]);
  assert.deepEqual(inline('**bold and *italic***').map(span => [span.text, !!span.bold, !!span.italic]), [['bold and ', true, false], ['italic', true, true]]);
  assert.deepEqual(inline('[Heat](https://en.wikipedia.org/wiki/Heat_(1995_film)) ran')[0], { text: 'Heat', href: 'https://en.wikipedia.org/wiki/Heat_(1995_film)' });
  assert.deepEqual(inline('It costs \\$1,200'), [{ text: 'It costs $1,200' }]);
});

test('a table reads as a table, whether or not it has outer pipes', () => {
  const [table] = parse('| Model | Price | Notes |\n|:---|---:|---|\n| **A** | $1 | [link](https://a.com) |\n| B | $2 |\n\nAfter');
  if (table.kind !== 'table') throw new Error('expected a table');
  assert.deepEqual(table.align, ['left', 'right', undefined]);
  assert.equal(table.header.length, 3);
  assert.equal(table.rows.length, 2);
  assert.ok(table.rows[0][0][0].bold && table.rows[0][2][0].href === 'https://a.com');
  assert.deepEqual(table.rows[1][2], [], 'a short row is filled out');
  assert.equal(parse('a | b\n---|---\n1 | 2')[0].kind, 'table');
  assert.deepEqual(parse('| a | b |').map(block => block.kind), ['paragraph'], 'no divider yet, no table yet');
  assert.deepEqual(parse('one | two\n\n---').map(block => block.kind), ['paragraph', 'rule']);
  const [cellPipe] = parse('| a |\n|---|\n| x \\| y |');
  assert.ok(cellPipe.kind === 'table' && cellPipe.rows[0][0][0].text === 'x | y');
});

test('the rest of a long answer keeps its shape', () => {
  assert.deepEqual(parse('Before\n\n---\n\n> Said\n> twice\n\n```py\nprint(1)\n\nprint(2)\n```\nAfter').map(block => block.kind), ['paragraph', 'rule', 'quote', 'code', 'paragraph']);
  const [code] = parse('```\nnot **bold**\n\nstill code');
  assert.ok(code.kind === 'code' && code.text === 'not **bold**\n\nstill code', 'an unclosed block is still code');
  const [gapped] = parse('- one\n\n- two\n- three');
  assert.ok(gapped.kind === 'list' && gapped.items.length === 3, 'a blank line between items keeps one list');
  const [nested] = parse('- one\n  - inner\n    - deepest\n      - deeper still\n- two');
  if (nested.kind !== 'list') throw new Error('expected a list');
  assert.deepEqual(nested.depths, [0, 1, 2, 2, 0]);
  const [wrapped] = parse('1. **Tea**\n   Brew a pot.');
  assert.ok(wrapped.kind === 'list' && wrapped.items[0][0].bold && wrapped.items[0].at(-1)!.text === 'Brew a pot.');
  assert.deepEqual(parse('**Bottom line**\nIt works.').map(block => block.kind), ['heading', 'paragraph']);
  assert.deepEqual(parse('Intro\n**Not a heading**\nmore').map(block => block.kind), ['paragraph'], 'bold inside a paragraph stays there');
  assert.deepEqual(parse('**Yes.**').map(block => block.kind), ['paragraph'], 'a bold reply alone is the reply');
});

test('a citation still arriving stays out of sight until it is whole', () => {
  for (const partial of ['Rates rose ([reuters', 'Rates rose ([reuters.com](https://www.reu', 'Rates rose ([reuters.com]']) {
    const [block] = parse(partial);
    if (block.kind !== 'paragraph') throw new Error('expected a paragraph');
    assert.deepEqual(block.lines[0], [{ text: 'Rates rose' }], partial);
  }
});

test('reading aloud says the words only', () => {
  assert.equal(speakable('**Rates** rose. ([reuters.com](https://reuters.com/a?utm_source=openai)) See [the report](https://a.com/b).'), 'Rates rose. See the report.');
  assert.equal(speakable('# Summary\n\n- One\n- Two\n\n---\n\n```\ncode\n```\nDone'), 'Summary. One. Two. Done.');
  assert.equal(speakable('| Model | Price | Size |\n|---|---|---|\n| A | $1 | 2 |\n| B | $3 | 4 |'), 'A: Price $1, Size 2. B: Price $3, Size 4.');
  assert.equal(speakable('Plain words, 2 * 3 is 6.'), 'Plain words, 2 * 3 is 6.');
  assert.equal(speakable('|---|---|'), '');
});
