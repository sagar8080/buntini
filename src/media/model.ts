// Shared shapes for Music and Spotify (stream F). No bridge import, so the bridge can use them.
import { t } from '../i18n.ts';

/** Saved under `settings.media`; mirrors `media::Settings` in Rust. */
export type MediaSettings = Record<string, never>;
export const defaults: MediaSettings = {};

/** What `media_call` accepts; mirrors `media::Op`. */
export type MediaOp = { op: 'ping' };

/** What the browser preview answers for `media_call`. */
export function preview(op: MediaOp): unknown {
  if (op.op === 'ping') return true;
  throw new Error(t('common.desktopOnly'));
}
