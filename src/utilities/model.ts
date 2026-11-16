// Shared shapes for timers, captures, shortcuts and updates (stream F, Phase 3). No bridge import, so the bridge can use them.

/** Saved under `settings.utilities`; mirrors `utilities::Settings` in Rust. */
export type UtilitiesSettings = Record<string, never>;
export const defaults: UtilitiesSettings = {};

/** What `utilities_call` accepts; mirrors `utilities::Op`. */
export type UtilitiesOp = { op: 'ping' };

/** What the browser preview answers for `utilities_call`. */
export function preview(op: UtilitiesOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error('Open the desktop app to use this.');
}
