import { command } from '../bridge';
import type { Mailbox, MailDone, MailFilter, MailListing, MailLogRow, MailOp, MailOpened, MailPlan, MailWaiting } from './model';

export const callMail = <T>(op: MailOp) => command<T>('mail_call', { op });

export const listMail = (filter: MailFilter, query: string) => callMail<MailListing>({ op: 'list', filter, query });
export const openMail = (id: string) => callMail<MailOpened>({ op: 'open', id });
export const mailboxes = () => callMail<Mailbox[]>({ op: 'mailboxes' });
export const mailLog = () => callMail<MailLogRow[]>({ op: 'log' });
/** Every change sorting would make now, waiting for Allow; null when there is nothing to sort. */
export const proposeSort = () => callMail<MailWaiting | null>({ op: 'sort' });
export const proposeUndo = (entry: string) => callMail<MailWaiting>({ op: 'undo', entry });
export const approve = (id: string) => callMail<MailDone>({ op: 'approve', id });
export const deny = (id: string) => callMail<null>({ op: 'deny', id });
export const makePlan = (request: string, pdf: string | null) => callMail<MailPlan>({ op: 'plan', request, pdf });
export const planned = () => callMail<MailPlan | null>({ op: 'planned' });
export const runPlan = (id: string) => callMail<MailDone>({ op: 'runPlan', id });
export const dropPlan = (id: string) => callMail<null>({ op: 'dropPlan', id });
export const pickPdf = () => callMail<string | null>({ op: 'pickPdf' });
