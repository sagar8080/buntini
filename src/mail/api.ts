import { command } from '../bridge';
import type { MailOp } from './model';

export const callMail = <T>(op: MailOp) => command<T>('mail_call', { op });
