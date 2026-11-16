// Shared shapes for coding agents beyond requests and hooks (stream B). No bridge import, so the bridge can use them.

/** Saved under `settings.agents`; mirrors `agents::Settings` in Rust. */
export type AgentsSettings = Record<string, never>;
export const defaults: AgentsSettings = {};

/** What `agents_call` accepts; mirrors `agents::Op`. */
export type AgentsOp = { op: 'ping' };

/** What the browser preview answers for `agents_call`. */
export function preview(op: AgentsOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error('Open the desktop app to use this.');
}
