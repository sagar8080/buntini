import type { Attached, Settings } from './types';

/* What sits in the composer to go with the next messages: files, and at most one window.
   They stay until removed or a new topic starts, so follow-up questions see them too. Each
   goes only to the brain it was attached under: when the brain changes, by the person or by
   Buntini, it leaves the composer. */

/** Which brain the settings use, as attach.rs brain_key has it: a server by its address. */
export function brainKey(settings: Pick<Settings, 'modelBackend' | 'cloudProvider' | 'cloudUrl'>): string {
  if (settings.modelBackend !== 'cloud') return settings.modelBackend;
  return settings.cloudProvider === 'custom' ? `cloud:custom:${settings.cloudUrl.trim()}` : `cloud:${settings.cloudProvider}`;
}

/** What may still go with a message to `brain`, and what was attached under another. */
export function forBrain(current: Attached[], brain: string): { kept: Attached[]; dropped: Attached[] } {
  return { kept: current.filter(item => item.brain === brain), dropped: current.filter(item => item.brain !== brain) };
}

/** As many as Buntini takes with one message (attach.rs MAX_ATTACHMENTS). */
export const maxAttached = 6;

/** Adds files, each once, up to the limit, attached under `brain`. */
export function addFiles(current: Attached[], paths: string[], brain: string, limit = maxAttached): Attached[] {
  const next = [...current];
  for (const path of paths) {
    if (!path.startsWith('/') || next.some(item => item.kind === 'file' && item.path === path)) continue;
    if (next.length >= limit) break;
    next.push({ kind: 'file', path, brain });
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
