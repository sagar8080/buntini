// Shared shapes for developer services (stream D). No bridge import, so the bridge can use them.

/** Saved under `settings.services`; mirrors `services::Settings` in Rust. */
export type ServicesSettings = Record<string, never>;
export const defaults: ServicesSettings = {};

/** What `services_call` accepts; mirrors `services::Op`. */
export type ServicesOp = { op: 'ping' };

/** What the browser preview answers for `services_call`. */
export function preview(op: ServicesOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error('Open the desktop app to use this.');
}
