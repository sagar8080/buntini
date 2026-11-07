// A small Markdown reader for chat replies: paragraphs, lists, headings, tables, quotes,
// code, emphasis, https links and web citations. It builds plain data (never HTML), so
// nothing a model writes can inject markup, and half-finished streamed text stays readable.

export interface Span { text: string; bold?: boolean; italic?: boolean; code?: boolean; href?: string; cite?: boolean }
export type Align = 'left' | 'center' | 'right' | undefined;
export type Block =
  | { kind: 'paragraph'; lines: Span[][] }
  | { kind: 'heading'; spans: Span[] }
  | { kind: 'list'; ordered: boolean; items: Span[][]; depths: number[]; start?: number }
  | { kind: 'table'; header: Span[][]; align: Align[]; rows: Span[][][] }
  | { kind: 'quote'; lines: Span[][] }
  | { kind: 'code'; text: string }
  | { kind: 'rule' };

const BULLET = /^(\s*)[-*•]\s+(.*)$/;
const NUMBER = /^(\s*)(\d+)[.)]\s+(.*)$/;
const HEADING = /^\s*#{1,6}\s+(.*)$/;
const FENCE = /^\s*(```|~~~)/;
const RULE = /^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/;
const QUOTE = /^\s*>\s?(.*)$/;
const TABLE_DIVIDER = /^\s*\|?\s*:?-+:?\s*(?:\|\s*:?-+:?\s*)*\|?\s*$/;
const ONLY_BOLD = /^\s*(?:\*\*[^*]+\*\*|__[^_]+__):?\s*$/;
/** A citation still arriving at the end of the text; it shows once it's whole. */
const UNFINISHED_CITATION = /\s*\(\[[^\]\n]*(?:\](?:\([^\s)]*)?)?$/;

const URL = String.raw`https?:\/\/(?:[^\s()]|\([^\s()]*\))+`;
const LINK = String.raw`\[([^\]]+)\]\((${URL})\)`;
const INLINE = new RegExp([
  String.raw`\((?<cited>(?:\[[^\]]+\]\(${URL}\)[,;\s]*)+)\)`,
  String.raw`\*\*\*(?=\S)(?<both>.+?)(?<=\S)\*\*\*`,
  String.raw`\*\*(?=\S)(?<bold>.+?)(?<=\S)\*\*(?!\*)`,
  String.raw`__(?=\S)(?<bolder>.+?)(?<=\S)__`,
  '`(?<code>[^`]+)`',
  String.raw`\[(?<label>[^\]]+)\]\((?<url>${URL})\)`,
  String.raw`\*(?<star>[^*\s][^*]*)\*`,
  String.raw`(?<![\w])_(?<under>[^_\s][^_]*)_(?![\w])`,
].join('|'), 'g');

const unescaped = (text: string) => text.replace(/\\([\\`*_{}[\]()#+\-.!|$~>])/g, '$1');
/** Only https opens; anything else keeps its words and loses the link. */
const linked = (url: string) => url.startsWith('https://') ? { href: withoutTracking(url) } : {};

/** OpenAI tags the links its search cites. The reader's click isn't theirs to report. */
export function withoutTracking(url: string) {
  try {
    const parsed = new globalThis.URL(url);
    if (parsed.searchParams.get('utm_source') !== 'openai') return url;
    parsed.searchParams.delete('utm_source');
    return parsed.toString();
  } catch { return url; }
}

/** Inline styles: **bold**, *italic* or _italic_, `code`, [label](https://…), and a web
 * citation: ([site](https://…)). Styles can hold each other, so a bold link stays a link. */
export function inline(text: string, style: Pick<Span, 'bold' | 'italic'> = {}): Span[] {
  const spans: Span[] = [];
  let last = 0;
  const plain = (words: string) => { if (words) spans.push({ text: unescaped(words), ...style }); };
  for (const match of text.matchAll(INLINE)) {
    const found = match.groups!;
    plain(text.slice(last, match.index));
    if (found.cited) for (const link of found.cited.matchAll(new RegExp(LINK, 'g'))) spans.push({ text: link[1], cite: true, ...linked(link[2]) });
    else if (found.both) spans.push(...inline(found.both, { bold: true, italic: true }));
    else if (found.bold ?? found.bolder) spans.push(...inline((found.bold ?? found.bolder)!, { ...style, bold: true }));
    else if (found.code) spans.push({ text: found.code, code: true, ...style });
    else if (found.label) spans.push(...inline(found.label, style).map(span => ({ ...span, ...linked(found.url) })));
    else spans.push(...inline((found.star ?? found.under)!, { ...style, italic: true }));
    last = match.index! + match[0].length;
  }
  plain(text.slice(last));
  return spans;
}

/** A table row's cells, with the outer pipes gone and `\|` kept as a pipe. */
function cells(line: string) {
  return line.trim().replace(/^\|/, '').replace(/(?<!\\)\|$/, '').split(/(?<!\\)\|/).map(cell => cell.trim().replace(/\\\|/g, '|'));
}
function alignment(cell: string): Align {
  return cell.startsWith(':') ? (cell.endsWith(':') ? 'center' : 'left') : cell.endsWith(':') ? 'right' : undefined;
}

export function parse(text: string): Block[] {
  const blocks: Block[] = [];
  const lines = text.replace(/\r/g, '').replace(UNFINISHED_CITATION, '').split('\n');
  for (let at = 0; at < lines.length; at += 1) {
    const line = lines[at];
    const previous = blocks[blocks.length - 1];
    const fence = line.match(FENCE);
    if (fence) {
      const code: string[] = [];
      for (at += 1; at < lines.length && !lines[at].trim().startsWith(fence[1]); at += 1) code.push(lines[at]);
      blocks.push({ kind: 'code', text: code.join('\n') });
      continue;
    }
    if (!line.trim()) { blocks.push({ kind: 'paragraph', lines: [] }); continue; }
    const divider = lines[at + 1] ?? '';
    if (line.includes('|') && divider.includes('|') && TABLE_DIVIDER.test(divider)) {
      const header = cells(line), align = cells(divider).map(alignment);
      const fit = (row: string[]) => header.map((_, column) => inline(row[column] ?? ''));
      const rows: Span[][][] = [];
      for (at += 2; at < lines.length && lines[at].includes('|'); at += 1) rows.push(fit(cells(lines[at])));
      at -= 1;
      blocks.push({ kind: 'table', header: fit(header), align: header.map((_, column) => align[column]), rows });
      continue;
    }
    if (RULE.test(line)) { blocks.push({ kind: 'rule' }); continue; }
    const heading = line.match(HEADING);
    if (heading) { blocks.push({ kind: 'heading', spans: inline(heading[1]) }); continue; }
    const quote = line.match(QUOTE);
    if (quote) {
      if (previous?.kind === 'quote') previous.lines.push(inline(quote[1]));
      else blocks.push({ kind: 'quote', lines: [inline(quote[1])] });
      continue;
    }
    const bullet = line.match(BULLET), number = bullet ? null : line.match(NUMBER);
    if (bullet || number) {
      const ordered = !!number, item = inline(bullet ? bullet[2] : number![3]);
      const depth = Math.min(2, Math.floor((bullet ? bullet[1] : number![1]).length / 2));
      // A blank line between items doesn't end the list.
      const blank = previous?.kind === 'paragraph' && !previous.lines.length;
      const list = blank ? blocks[blocks.length - 2] : previous;
      if (list?.kind === 'list' && list.ordered === ordered) {
        if (blank) blocks.pop();
        list.items.push(item);
        list.depths.push(depth);
      // A numbered list split by paragraphs ("1. … text … 2. …") keeps counting.
      } else blocks.push({ kind: 'list', ordered, items: [item], depths: [depth], ...(number ? { start: Number(number[2]) } : {}) });
      continue;
    }
    // An indented line belongs to the item above it.
    if (previous?.kind === 'list' && /^\s{2,}\S/.test(line)) {
      previous.items[previous.items.length - 1].push({ text: ' ' }, ...inline(line.trim()));
      continue;
    }
    // A bold line of its own, with more below, is how many replies write a heading.
    const opensSection = !(previous?.kind === 'paragraph' && previous.lines.length) && ONLY_BOLD.test(line) && lines.slice(at + 1).some(next => next.trim());
    if (opensSection) { blocks.push({ kind: 'heading', spans: inline(line.trim().replace(/:$/, '')).map(span => ({ ...span, bold: false })) }); continue; }
    if (previous?.kind === 'paragraph') previous.lines.push(inline(line.trim()));
    else blocks.push({ kind: 'paragraph', lines: [inline(line.trim())] });
  }
  return blocks.filter(block => block.kind !== 'paragraph' || block.lines.length > 0);
}

const said = (spans: Span[]) => spans.filter(span => !span.cite).map(span => span.text).join('').replace(/\s+/g, ' ').replace(/ ([.,;:!?])/g, '$1').trim();
const sentence = (words: string) => !words ? '' : /[.!?:;,]$/.test(words) ? words : `${words}.`;

/** A reply as it should be heard: the words, without Markdown, citations, links or code. */
export function speakable(text: string) {
  const parts = parse(text).flatMap(block => {
    if (block.kind === 'paragraph' || block.kind === 'quote') return block.lines.map(said);
    if (block.kind === 'heading') return [said(block.spans)];
    if (block.kind === 'list') return block.items.map(said);
    if (block.kind === 'table') {
      const names = block.header.map(said);
      // Each row by name, since nobody hearing it can look up at the header.
      return block.rows.map(row => {
        const [first, ...rest] = row.map(said);
        return `${first}: ${rest.map((cell, column) => `${names[column + 1]} ${cell}`).join(', ')}`;
      });
    }
    return [];
  });
  const wanted = parts.filter(part => /[\p{L}\p{N}]/u.test(part));
  return wanted.length > 1 ? wanted.map(sentence).join(' ') : wanted.join('');
}
