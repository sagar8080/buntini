// A small Markdown reader for chat replies: paragraphs, bullet and numbered lists,
// headings, bold, italics, code and https links. It builds plain data (never HTML), so
// nothing a model writes can inject markup, and half-finished streamed text stays readable.

export interface Span { text: string; bold?: boolean; italic?: boolean; code?: boolean; href?: string }
export type Block =
  | { kind: 'paragraph'; lines: Span[][] }
  | { kind: 'heading'; spans: Span[] }
  | { kind: 'list'; ordered: boolean; items: Span[][] };

const BULLET = /^\s*[-*•]\s+(.*)$/;
const NUMBER = /^\s*\d+[.)]\s+(.*)$/;
const HEADING = /^\s*#{1,6}\s+(.*)$/;

/** Inline styles: **bold**, *italic* or _italic_, `code`, [label](https://…). */
export function inline(text: string): Span[] {
  const spans: Span[] = [];
  const pattern = /(\*\*([^*]+)\*\*|__([^_]+)__|`([^`]+)`|\[([^\]]+)\]\((https:\/\/[^\s)]+)\)|\*([^*\s][^*]*)\*|(?<![\w])_([^_\s][^_]*)_(?![\w]))/g;
  let last = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index! > last) spans.push({ text: text.slice(last, match.index) });
    if (match[2] ?? match[3]) spans.push({ text: (match[2] ?? match[3])!, bold: true });
    else if (match[4]) spans.push({ text: match[4], code: true });
    else if (match[5]) spans.push({ text: match[5], href: match[6] });
    else spans.push({ text: (match[7] ?? match[8])!, italic: true });
    last = match.index! + match[0].length;
  }
  if (last < text.length) spans.push({ text: text.slice(last) });
  return spans;
}

export function parse(text: string): Block[] {
  const blocks: Block[] = [];
  for (const line of text.replace(/\r/g, '').split('\n')) {
    const previous = blocks[blocks.length - 1];
    const bullet = line.match(BULLET), number = bullet ? null : line.match(NUMBER), heading = line.match(HEADING);
    if (!line.trim()) { blocks.push({ kind: 'paragraph', lines: [] }); continue; }
    if (heading) { blocks.push({ kind: 'heading', spans: inline(heading[1]) }); continue; }
    if (bullet || number) {
      const ordered = !!number, item = inline((bullet ?? number)![1]);
      if (previous?.kind === 'list' && previous.ordered === ordered) previous.items.push(item);
      else blocks.push({ kind: 'list', ordered, items: [item] });
      continue;
    }
    if (previous?.kind === 'paragraph') previous.lines.push(inline(line.trim()));
    else blocks.push({ kind: 'paragraph', lines: [inline(line.trim())] });
  }
  return blocks.filter(block => block.kind !== 'paragraph' || block.lines.length > 0);
}
