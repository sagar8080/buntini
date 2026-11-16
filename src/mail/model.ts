// Shared shapes for Mail in the island and the app (stream E). No bridge import, so the bridge can use them.
import { t } from '../i18n.ts';

/** A mailbox by its account's name (empty for On My Mac) and its path; mirrors `mail::Mailbox`. */
export type Mailbox = { account: string; path: string };

export type RuleField = 'sender' | 'domain' | 'subject';
export type RuleAction = 'move' | 'flag' | 'read' | 'junk';
/** Mirrors `mail::rules::Rule`. Only a rule that moves mail has `to`. */
export type MailRule = { id: string; field: RuleField; value: string; action: RuleAction; to?: Mailbox | null; enabled: boolean };

/** Saved under `settings.mail`; mirrors `mail::Settings` in Rust. */
export type MailSettings = { rules: MailRule[]; banned: string[]; vips: string[] };
export const defaults: MailSettings = { rules: [], banned: [], vips: [] };

export type MailFilter = 'all' | 'unread' | 'important' | 'today';
export type MailRow = { id: string; from: string; address: string; subject: string; unread: boolean; flagged: boolean; at: number; mailbox: string };
export type MailListing = { filter: MailFilter; query: string; messages: MailRow[]; unread: number; sortable: number; closed: boolean };
export type MailOpened = { id: string; from: string; subject: string; at: number; to: string[]; attachments: string[]; body: string; cut: boolean };
/** A change waiting for Allow. */
export type MailWaiting = { id: string; title: string; lines: string[] };
export type MailLogRow = { id: string; label: string; at: number; undoable: boolean; undone: boolean };
export type MailPlan = { id: string; summary: string; lines: string[]; problems: string[] };
export type MailDone = { changed: string[]; failed: string[] };

/** What `mail_call` accepts; mirrors `mail::Op`. */
export type MailOp =
  | { op: 'ping' }
  | { op: 'list'; filter?: MailFilter; query?: string }
  | { op: 'open'; id: string }
  | { op: 'mark'; id: string; change: 'read' | 'unread' | 'flag' | 'unflag' }
  | { op: 'junk'; id: string }
  | { op: 'move'; id: string; to: Mailbox }
  | { op: 'reply'; id: string; body: string }
  | { op: 'attach'; files: string[] }
  | { op: 'sort' }
  | { op: 'undo'; entry: string }
  | { op: 'approve'; id: string }
  | { op: 'deny'; id: string }
  | { op: 'log' }
  | { op: 'mailboxes' }
  | { op: 'plan'; request: string; pdf?: string | null }
  | { op: 'planned' }
  | { op: 'runPlan'; id: string }
  | { op: 'dropPlan'; id: string }
  | { op: 'pickPdf' }
  | { op: 'show' };

/** A mailbox as a person reads it. */
export function mailboxName(mailbox: Mailbox): string {
  return mailbox.account ? `${mailbox.path} · ${mailbox.account}` : mailbox.path;
}

export function sameMailbox(a: Mailbox, b: Mailbox): boolean {
  return a.account === b.account && a.path.toLowerCase() === b.path.toLowerCase();
}

/** What a rule does, as one line. */
export function ruleLine(rule: MailRule): string {
  return t('settings.mail.ruleLine', { field: fieldName(rule.field), value: rule.value, action: actionName(rule) });
}

/** What a rule looks at, as the rule form names it. */
export function fieldName(field: RuleField): string {
  switch (field) {
    case 'sender': return t('settings.mail.field.sender');
    case 'domain': return t('settings.mail.field.domain');
    case 'subject': return t('settings.mail.field.subject');
  }
}

function actionName(rule: MailRule): string {
  if (rule.action === 'move' && rule.to) return t('settings.mail.toMailbox', { mailbox: rule.to.path });
  switch (rule.action) {
    case 'move': return t('settings.mail.action.move');
    case 'flag': return t('settings.mail.action.flag');
    case 'read': return t('settings.mail.action.read');
    case 'junk': return t('settings.mail.action.junk');
  }
}

/** A word list with `entry` added once, ignoring case. */
export function added(list: string[], entry: string): string[] {
  const value = entry.trim();
  if (!value || list.some(item => item.toLowerCase() === value.toLowerCase())) return list;
  return [...list, value];
}

/** Whether a rule is complete enough to save, as Rust checks it. */
export function ruleReady(rule: Omit<MailRule, 'id'>): boolean {
  const value = rule.value.trim();
  if (!value || value.length > 200 || /[\u0000-\u001f]/.test(value)) return false;
  return rule.action === 'move' ? !!rule.to?.path : !rule.to;
}

const sample: MailRow[] = [
  { id: '14', from: 'Ana Ruiz', address: 'ana@example.org', subject: 'Dinner on Tuesday?', unread: true, flagged: false, at: Date.now() / 1000 - 600, mailbox: 'INBOX' },
  { id: '15', from: 'Bank Alerts', address: 'alerts@bank.example.com', subject: 'Your statement is ready', unread: false, flagged: true, at: Date.now() / 1000 - 86_400, mailbox: 'INBOX' },
];

/** What the browser preview answers for `mail_call`. */
export function preview(op: MailOp): unknown {
  switch (op.op) {
    case 'ping': return true;
    case 'list': return { filter: op.filter ?? 'all', query: op.query ?? '', messages: sample, unread: 1, sortable: 0, closed: false } satisfies MailListing;
    case 'log': return [{ id: 'e1', label: 'To Newsletters · This week in tech', at: Date.now() / 1000 - 300, undoable: true, undone: false }] satisfies MailLogRow[];
    case 'mailboxes': return [{ account: '', path: 'Newsletters' }, { account: 'iCloud', path: 'Archive' }] satisfies Mailbox[];
    case 'planned': return null;
    case 'pickPdf': return null;
    default: throw new Error(t('common.desktopOnly'));
  }
}
