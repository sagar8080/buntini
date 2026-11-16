// Shared shapes for dictation, the pill, call notes and the wake word (stream C). No bridge import, so the bridge can use them.

/** Saved under `settings.flow`; mirrors `flow::Settings` in Rust. */
export type FlowSettings = Record<string, never>;
export const defaults: FlowSettings = {};

/** What `flow_call` accepts; mirrors `flow::Op`. */
export type FlowOp = { op: 'ping' };

/** What the browser preview answers for `flow_call`. */
export function preview(op: FlowOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error('Open the desktop app to use this.');
}
