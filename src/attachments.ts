import type { Attached } from './types';

/* What sits in the composer to go with the next messages: files, and at most one window.
   They stay until removed or a new topic starts, so follow-up questions see them too. */

/** As many as Buntini takes with one message (attach.rs MAX_ATTACHMENTS). */
export const maxAttached = 6;

/** Adds files, each once, up to the limit. */
export function addFiles(current: Attached[], paths: string[], limit = maxAttached): Attached[] {
  const next = [...current];
  for (const path of paths) {
    if (!path.startsWith('/') || next.some(item => item.kind === 'file' && item.path === path)) continue;
    if (next.length >= limit) break;
    next.push({ kind: 'file', path });
  }
  return next;
}

/** Attaches a window in place of any window attached before. Returns it and the one replaced. */
export function addWindow(current: Attached[], window: Extract<Attached, { kind: 'window' }>): { next: Attached[]; replaced?: Extract<Attached, { kind: 'window' }> } {
  const replaced = current.find((item): item is Extract<Attached, { kind: 'window' }> => item.kind === 'window');
  const rest = current.filter(item => item.kind !== 'window');
  return { next: [...rest, window].slice(-maxAttached), replaced };
}

export function removeAttached(current: Attached[], gone: Attached): Attached[] {
  return current.filter(item => item !== gone && !(item.kind === 'file' && gone.kind === 'file' && item.path === gone.path) && !(item.kind === 'window' && gone.kind === 'window' && item.id === gone.id));
}

/** The chip's words: a file's name, or the window's title (its app's name without one). */
export function chipLabel(item: Attached): string {
  if (item.kind === 'window') return item.title.trim() || item.app;
  return item.path.split('/').filter(Boolean).pop() || item.path;
}

/** Whether a path names a picture, for the chip's icon. */
export function isPicture(path: string): boolean {
  return /\.(png|jpe?g|gif|webp|heic|heif|tiff?|bmp)$/i.test(path);
}
