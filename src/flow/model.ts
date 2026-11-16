// Shared shapes for dictation, the pill, call notes and the wake word (stream C). No bridge import, so the bridge can use them.

export type DictationKey = 'rightOption' | 'rightCommand' | 'rightControl' | 'fn' | 'chord';
export type Chord = { keyCode: number; modifiers: ('command' | 'option' | 'control' | 'shift')[]; label: string };
export type Replacement = { from: string; to: string };
export type Snippet = { trigger: string; text: string };
export type Style = 'asSaid' | 'natural' | 'polished';
export type Mode = 'dictate' | 'prompt' | 'translate';

/** Saved under `settings.flow`; mirrors `flow::Settings` in Rust. */
export type FlowSettings = {
  enabled: boolean;
  key: DictationKey;
  chord: Chord | null;
  /** Up to three, such as `en-US`; none means the Mac's own language. */
  languages: string[];
  mute: boolean;
  dictionary: string[];
  replacements: Replacement[];
  snippets: Snippet[];
  wakeWord: boolean;
  recordCalls: boolean;
};
export const defaults: FlowSettings = {
  enabled: false, key: 'rightOption', chord: null, languages: [], mute: true,
  dictionary: [], replacements: [], snippets: [], wakeWord: false, recordCalls: false,
};

/** What you pick from the pill; mirrors `flow::Choices`. */
export type Choices = { appStyles: Record<string, Style>; mode: Mode; translateTo: string };

export type ModelStatus = { phase: 'idle' | 'downloading' | 'waiting' | 'ready' | 'failed'; doneBytes: number; totalBytes: number; message: string };
export type FlowStatus = {
  enabled: boolean;
  watching: boolean;
  accessibility: boolean;
  engine: 'parakeet' | 'apple';
  models: ModelStatus;
  /** Seconds since 1970 when a call started recording. */
  recording: number | null;
  wake: boolean;
};
export type Dictation = { id: number; at: string; app: string; words: number; text: string; pasted: boolean };
export type Stats = { words: number; dictations: number; minutesSaved: number; wordsPerMinute: number; streak: number; apps: { app: string; words: number }[] };
export type NoteLine = { speaker: string | null; text: string };
export type Note = {
  id: string; title: string; createdAt: string; seconds: number;
  state: 'recording' | 'processing' | 'ready' | 'failed';
  lines: NoteLine[]; summary: string; keyPoints: string[]; todos: string[];
};
/** A stretch of a call heard while it records. */
export type LiveLine = { id: string; side: 'mic' | 'system'; start: number; end: number; text: string };

/** What `flow_call` accepts; mirrors `flow::Op`. */
export type FlowOp =
  | { op: 'ping' } | { op: 'status' } | { op: 'apply' } | { op: 'stats' }
  | { op: 'history'; limit?: number } | { op: 'forgetDictation'; id: number }
  | { op: 'notes' } | { op: 'deleteNote'; id: string } | { op: 'startNotes' } | { op: 'stopNotes' }
  | { op: 'choices' } | { op: 'choose'; mode: Mode; translateTo?: string } | { op: 'setStyle'; bundle: string; style: Style | null }
  | { op: 'downloadModels' } | { op: 'cancelDownload' } | { op: 'removeModels' } | { op: 'openLibrary' };

export const keyLabels: Record<Exclude<DictationKey, 'chord'>, string> = { rightOption: 'Right ⌥', rightCommand: 'Right ⌘', rightControl: 'Right ⌃', fn: 'fn' };

export const languageNames: Record<string, string> = {
  'en-US': 'English (US)', 'en-GB': 'English (UK)', 'es-ES': 'Spanish', 'fr-FR': 'French', 'de-DE': 'German', 'it-IT': 'Italian',
  'pt-BR': 'Portuguese', 'nl-NL': 'Dutch', 'pl-PL': 'Polish', 'sv-SE': 'Swedish', 'da-DK': 'Danish', 'fi-FI': 'Finnish',
  'ru-RU': 'Russian', 'uk-UA': 'Ukrainian', 'el-GR': 'Greek', 'cs-CZ': 'Czech', 'ro-RO': 'Romanian', 'hu-HU': 'Hungarian',
  'ja-JP': 'Japanese', 'zh-CN': 'Chinese', 'ko-KR': 'Korean', 'hi-IN': 'Hindi', 'ar-SA': 'Arabic', 'tr-TR': 'Turkish',
};

/** The chord a key press makes, or null for a bare key (it would type). */
export function chordFrom(event: { code: string; key: string; metaKey: boolean; altKey: boolean; ctrlKey: boolean; shiftKey: boolean }): Chord | null {
  const keyCode = macKeyCodes[event.code];
  const modifiers = ([['control', event.ctrlKey], ['option', event.altKey], ['shift', event.shiftKey], ['command', event.metaKey]] as const).filter(([, on]) => on).map(([name]) => name);
  if (keyCode === undefined || modifiers.length === 0) return null;
  const symbols: Record<string, string> = { control: '⌃', option: '⌥', shift: '⇧', command: '⌘' };
  const name = event.code.startsWith('Key') ? event.code.slice(3) : event.code.startsWith('Digit') ? event.code.slice(5) : event.code === 'Space' ? 'Space' : event.key.toUpperCase();
  return { keyCode, modifiers: [...modifiers], label: modifiers.map(modifier => symbols[modifier]).join('') + name };
}

/** macOS virtual key codes by DOM `code` (the key's position, as Carbon reads it). */
const macKeyCodes: Record<string, number> = {
  KeyA: 0, KeyS: 1, KeyD: 2, KeyF: 3, KeyH: 4, KeyG: 5, KeyZ: 6, KeyX: 7, KeyC: 8, KeyV: 9, KeyB: 11, KeyQ: 12, KeyW: 13, KeyE: 14, KeyR: 15,
  KeyY: 16, KeyT: 17, Digit1: 18, Digit2: 19, Digit3: 20, Digit4: 21, Digit6: 22, Digit5: 23, Digit9: 25, Digit7: 26, Digit8: 28, Digit0: 29,
  KeyO: 31, KeyU: 32, KeyI: 34, KeyP: 35, KeyL: 37, KeyJ: 38, KeyK: 40, KeyN: 45, KeyM: 46, Space: 49, Backquote: 50, Period: 47, Comma: 43, Slash: 44, Semicolon: 41,
};

export function minutes(seconds: number): string {
  const total = Math.max(0, Math.round(seconds));
  const hours = Math.floor(total / 3600);
  const rest = `${Math.floor(total / 60) % 60}:${String(total % 60).padStart(2, '0')}`;
  return hours ? `${hours}:${rest.padStart(5, '0')}` : rest;
}

/** What the browser preview answers for `flow_call`. */
export function preview(op: FlowOp): unknown {
  switch (op.op) {
    case 'ping': return true;
    case 'status': return { enabled: false, watching: false, accessibility: false, engine: 'apple', models: { phase: 'idle', doneBytes: 0, totalBytes: 654832571, message: '' }, recording: null, wake: false } satisfies FlowStatus;
    case 'stats': return { words: 1840, dictations: 63, minutesSaved: 31.5, wordsPerMinute: 148, streak: 4, apps: [{ app: 'Slack', words: 920 }, { app: 'Mail', words: 610 }, { app: 'Terminal', words: 310 }] } satisfies Stats;
    case 'history': return [
      { id: 2, at: new Date().toISOString(), app: 'Slack', words: 9, text: 'On my way, five minutes out. Start without me.', pasted: true },
      { id: 1, at: new Date(Date.now() - 3600_000).toISOString(), app: 'Terminal', words: 2, text: 'git status', pasted: true },
    ] satisfies Dictation[];
    case 'notes': return [{
      id: 'preview-note', title: 'Launch sync', createdAt: new Date().toISOString(), seconds: 1834, state: 'ready',
      lines: [{ speaker: 'Speaker 1', text: 'Let’s start with the launch plan.' }, { speaker: 'You', text: 'Can we fix those by Wednesday?' }],
      summary: 'The launch moves to Thursday once two onboarding fixes land.', keyPoints: ['Two onboarding problems found in review'], todos: ['Daniel fixes the sign-in screen today'],
    }] satisfies Note[];
    case 'choices': return { appStyles: {}, mode: 'dictate', translateTo: '' } satisfies Choices;
    case 'forgetDictation': case 'deleteNote': case 'apply': return null;
    default: throw new Error('Open the desktop app to use this.');
  }
}
