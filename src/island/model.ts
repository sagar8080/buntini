// Shared shapes for the island at the notch and its cat (stream A). No bridge import, so the bridge can use them.
import { t } from '../i18n.ts';

/** Saved under `settings.island`; mirrors `island::Settings` in Rust. */
export interface IslandSettings {
  /** Where the island sits: set by dragging the cat, never in a settings page. */
  placement: 'auto' | 'topLeft' | 'topRight' | 'bottomLeft' | 'bottomRight';
  /** Look once a day for a newer Buntini. */
  updates: boolean;
}
export const defaults: IslandSettings = { placement: 'auto', updates: false };

/** What `island_call` accepts; mirrors `island::Op`. `busy`: a question typed in the island wasn't asked. */
export type IslandOp = { op: 'ping' } | { op: 'busy' };

/** What the browser preview answers for `island_call`. */
export function preview(op: IslandOp): unknown {
  if (op.op === 'ping') return true;
  if (op.op === 'busy') return null;
  throw new Error(t('common.desktopOnly'));
}
