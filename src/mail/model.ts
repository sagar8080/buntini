// Shared shapes for Mail in the island (stream E). No bridge import, so the bridge can use them.

/** Saved under `settings.mail`; mirrors `mail::Settings` in Rust. */
export type MailSettings = Record<string, never>;
export const defaults: MailSettings = {};

/** What `mail_call` accepts; mirrors `mail::Op`. */
export type MailOp = { op: 'ping' };

/** What the browser preview answers for `mail_call`. */
export function preview(op: MailOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error('Open the desktop app to use this.');
}
