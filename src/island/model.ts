// Shared shapes for the island at the notch and its cat (stream A). No bridge import, so the bridge can use them.

/** Saved under `settings.island`; mirrors `island::Settings` in Rust. */
export type IslandSettings = Record<string, never>;
export const defaults: IslandSettings = {};

/** What `island_call` accepts; mirrors `island::Op`. */
export type IslandOp = { op: 'ping' };

/** What the browser preview answers for `island_call`. */
export function preview(op: IslandOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error('Open the desktop app to use this.');
}
