// Shared shapes for shortcuts and settings search (stream F). No bridge import, so the bridge can use them.

/** Saved under `settings.utilities`; mirrors `utilities::Settings` in Rust. Shortcuts, the
    focus timer and muting are kept apart, since they change outside the settings sheet. */
export type UtilitiesSettings = Record<string, never>;
export const defaults: UtilitiesSettings = {};

/** A key combination; mirrors `utilities::shortcuts::Shortcut`. */
export type Shortcut = { keyCode: number; modifiers: ('control' | 'option' | 'shift' | 'command')[]; label: string };
/** One action and its shortcut, as `shortcuts` answers. */
export type ShortcutRow = { id: string; label: string; shortcut: Shortcut | null; default: Shortcut | null; problem: string | null };

/** What `utilities_call` accepts; mirrors `utilities::Op`. */
export type UtilitiesOp =
  | { op: 'ping' } | { op: 'shortcuts' } | { op: 'resetShortcuts' }
  | { op: 'setShortcut'; id: string; shortcut: Shortcut | null }
  | { op: 'recording'; on: boolean }
  | { op: 'focus'; minutes?: number };

/** macOS virtual key codes and how each key reads, by DOM `code` (the key's position, as Carbon reads it). */
const keys: Record<string, [number, string]> = {
  KeyA: [0, 'A'], KeyS: [1, 'S'], KeyD: [2, 'D'], KeyF: [3, 'F'], KeyH: [4, 'H'], KeyG: [5, 'G'], KeyZ: [6, 'Z'], KeyX: [7, 'X'], KeyC: [8, 'C'], KeyV: [9, 'V'],
  KeyB: [11, 'B'], KeyQ: [12, 'Q'], KeyW: [13, 'W'], KeyE: [14, 'E'], KeyR: [15, 'R'], KeyY: [16, 'Y'], KeyT: [17, 'T'], KeyO: [31, 'O'], KeyU: [32, 'U'],
  KeyI: [34, 'I'], KeyP: [35, 'P'], KeyL: [37, 'L'], KeyJ: [38, 'J'], KeyK: [40, 'K'], KeyN: [45, 'N'], KeyM: [46, 'M'],
  Digit1: [18, '1'], Digit2: [19, '2'], Digit3: [20, '3'], Digit4: [21, '4'], Digit6: [22, '6'], Digit5: [23, '5'], Digit9: [25, '9'], Digit7: [26, '7'], Digit8: [28, '8'], Digit0: [29, '0'],
  Equal: [24, '='], Minus: [27, '-'], BracketRight: [30, ']'], BracketLeft: [33, '['], Quote: [39, '\''], Semicolon: [41, ';'], Backslash: [42, '\\'],
  Comma: [43, ','], Slash: [44, '/'], Period: [47, '.'], Backquote: [50, '`'],
  Enter: [36, '↩'], Tab: [48, '⇥'], Space: [49, 'Space'], Backspace: [51, '⌫'], Delete: [117, '⌦'], Home: [115, '↖'], End: [119, '↘'], PageUp: [116, '⇞'], PageDown: [121, '⇟'],
  ArrowLeft: [123, '←'], ArrowRight: [124, '→'], ArrowDown: [125, '↓'], ArrowUp: [126, '↑'],
  F1: [122, 'F1'], F2: [120, 'F2'], F3: [99, 'F3'], F4: [118, 'F4'], F5: [96, 'F5'], F6: [97, 'F6'], F7: [98, 'F7'], F8: [100, 'F8'], F9: [101, 'F9'], F10: [109, 'F10'],
  F11: [103, 'F11'], F12: [111, 'F12'], F13: [105, 'F13'], F14: [107, 'F14'], F15: [113, 'F15'], F16: [106, 'F16'], F17: [64, 'F17'], F18: [79, 'F18'], F19: [80, 'F19'], F20: [90, 'F20'],
};
const symbols = { control: '⌃', option: '⌥', shift: '⇧', command: '⌘' } as const;
type Press = { code: string; metaKey: boolean; altKey: boolean; ctrlKey: boolean; shiftKey: boolean };

/** The modifiers held, as macOS writes them: ⌃⌥⇧⌘. */
export function held(event: Omit<Press, 'code'>): string {
  return (['control', 'option', 'shift', 'command'] as const).filter(name => ({ control: event.ctrlKey, option: event.altKey, shift: event.shiftKey, command: event.metaKey })[name]).map(name => symbols[name]).join('');
}

/** The shortcut a key press makes; null for a modifier on its own or a key Buntini can't use.
    Whether it needs a modifier is for Buntini to say, so the reason shows. */
export function shortcutFrom(event: Press): Shortcut | null {
  const key = keys[event.code];
  if (!key) return null;
  const modifiers = (['control', 'option', 'shift', 'command'] as const).filter(name => ({ control: event.ctrlKey, option: event.altKey, shift: event.shiftKey, command: event.metaKey })[name]);
  return { keyCode: key[0], modifiers: [...modifiers], label: held(event) + key[1] };
}

/** "⇧⌘Space" as keycaps: ["⇧", "⌘", "Space"]. */
export function keycaps(label: string): string[] {
  const caps: string[] = [];
  let rest = label;
  while (rest && '⌃⌥⇧⌘'.includes(rest[0])) { caps.push(rest[0]); rest = rest.slice(1); }
  if (rest) caps.push(rest);
  return caps;
}

export type SettingsTab = 'buntini' | 'brain' | 'apps' | 'connections' | 'privacy';
export const tabNames: Record<SettingsTab, string> = { buntini: 'Buntini', brain: 'Brain', apps: 'Apps', connections: 'Connections', privacy: 'Privacy' };

/** A setting to find: its label as Settings shows it, and the words people use for it. */
export type SettingEntry = { tab: SettingsTab; label: string; words: string[] };

/** Every setting, tab by tab. A label is found on screen by its text. */
export const settingsIndex: SettingEntry[] = [
  { tab: 'buntini', label: 'Name', words: ['rename', 'call', 'assistant name', 'nickname'] },
  { tab: 'buntini', label: 'Character', words: ['cat', 'orb', 'avatar', 'pet', 'mascot'] },
  { tab: 'buntini', label: 'Look', words: ['coat', 'fur', 'eyes', 'eye colour', 'eye color', 'accessory', 'bell', 'bow', 'hat', 'outfit', 'wardrobe', 'surprise me'] },
  { tab: 'buntini', label: 'Theme', words: ['colours', 'colors', 'accent', 'palette', 'skin', 'style'] },
  { tab: 'buntini', label: 'Appearance', words: ['dark mode', 'light mode', 'auto', 'night'] },
  { tab: 'buntini', label: 'Text size', words: ['font size', 'bigger', 'smaller', 'zoom', 'readability', 'panel size'] },
  { tab: 'buntini', label: 'Cat size', words: ['companion size', 'scale', 'bigger cat', 'smaller cat'] },
  { tab: 'buntini', label: 'Less motion', words: ['reduce motion', 'animation', 'still', 'calm', 'accessibility'] },
  { tab: 'buntini', label: 'Personality', words: ['tone', 'style', 'behaviour', 'behavior', 'instructions', 'prompt', 'how it talks'] },
  { tab: 'buntini', label: 'Voice', words: ['speaking voice', 'speech', 'text to speech', 'tts', 'siri voice'] },
  { tab: 'buntini', label: 'Pace', words: ['speech rate', 'speed', 'talk faster', 'talk slower'] },
  { tab: 'buntini', label: 'Answer out loud when I talk', words: ['speak replies', 'voice replies', 'talk back'] },
  { tab: 'buntini', label: 'Read every reply aloud', words: ['speak', 'read aloud', 'tts', 'voice'] },
  { tab: 'buntini', label: 'Wander around the screen', words: ['roam', 'walk', 'move around', 'wander'] },
  { tab: 'buntini', label: 'Offer a hand', words: ['nudges', 'suggestions', 'tips', 'stretch break', 'help'] },
  { tab: 'buntini', label: 'Morning brief', words: ['daily summary', 'briefing', 'morning', 'my day'] },
  { tab: 'buntini', label: 'Heads-ups', words: ['heads up', 'upcoming', 'reminders', 'alerts'] },
  { tab: 'buntini', label: 'Desktop notifications', words: ['notifications', 'alerts', 'banners', 'notification centre', 'notification center'] },
  { tab: 'buntini', label: 'Open at login', words: ['startup', 'launch at login', 'start automatically', 'login items', 'boot'] },
  { tab: 'buntini', label: 'Run setup again', words: ['onboarding', 'reset', 'start over', 'welcome'] },
  { tab: 'buntini', label: 'Dictate anywhere', words: ['dictation', 'voice typing', 'speech to text', 'microphone', 'mic'] },
  { tab: 'buntini', label: 'Hold', words: ['dictation key', 'push to talk', 'right option', 'fn key', 'hotkey'] },
  { tab: 'buntini', label: 'Languages', words: ['language', 'dictation language', 'english', 'translate'] },
  { tab: 'buntini', label: 'Mute the Mac while I speak', words: ['mute', 'silence', 'music while dictating'] },
  { tab: 'buntini', label: 'Hey Buntini', words: ['wake word', 'hey', 'voice activation', 'always listening', 'hands free'] },
  { tab: 'buntini', label: 'Record calls', words: ['meeting notes', 'call notes', 'transcript', 'recording', 'zoom', 'meetings'] },
  { tab: 'buntini', label: 'Speech model', words: ['parakeet', 'download', 'offline speech', 'model'] },
  { tab: 'buntini', label: 'Library', words: ['dictation history', 'notes', 'stats', 'transcripts'] },
  { tab: 'buntini', label: 'Dictionary', words: ['words', 'spelling', 'names', 'vocabulary', 'your words'] },
  { tab: 'buntini', label: 'Replacements', words: ['autocorrect', 'text replacement', 'expand', 'your words'] },
  { tab: 'buntini', label: 'Snippets', words: ['templates', 'canned text', 'text expansion', 'your words'] },
  { tab: 'buntini', label: 'Shortcuts', words: ['keyboard shortcuts', 'hotkeys', 'hot keys', 'keys', 'key bindings', 'keybindings'] },
  { tab: 'buntini', label: 'Show or hide Buntini', words: ['open buntini', 'companion', 'summon', 'toggle', 'shortcut'] },
  { tab: 'buntini', label: 'Open the chat', words: ['chat shortcut', 'ask', 'conversation', 'shortcut'] },
  { tab: 'buntini', label: 'Go to what’s waiting', words: ['approval', 'request', 'agent waiting', 'permission', 'shortcut'] },
  { tab: 'buntini', label: 'Next in the island', words: ['next pill', 'next session', 'cycle', 'island', 'shortcut'] },
  { tab: 'buntini', label: 'Mute the island', words: ['sounds', 'mute', 'quiet', 'silence', 'chime', 'shortcut'] },
  { tab: 'buntini', label: 'Screenshot', words: ['screen capture', 'screen grab', 'print screen', 'clipboard', 'thumbnail', 'shortcut'] },
  { tab: 'buntini', label: 'Capture an area', words: ['area capture', 'region', 'selection', 'snip', 'crop', 'mark up', 'annotate', 'shortcut'] },
  { tab: 'buntini', label: 'Focus timer', words: ['pomodoro', 'timer', 'work session', 'break', 'concentrate', 'shortcut'] },
  { tab: 'brain', label: 'Brain', words: ['model', 'ai', 'llm', 'apple intelligence', 'local model', 'cloud model', 'qwen', 'gemma'] },
  { tab: 'brain', label: 'Let Buntini choose', words: ['automatic brain', 'auto model'] },
  { tab: 'brain', label: 'Cloud provider', words: ['anthropic', 'claude', 'openai', 'gpt', 'gemini', 'chatgpt', 'api key'] },
  { tab: 'brain', label: 'Monthly cap', words: ['budget', 'spending', 'cost', 'limit', 'money', 'price'] },
  { tab: 'brain', label: 'Server address', words: ['ollama', 'lm studio', 'own server', 'localhost', 'host'] },
  { tab: 'apps', label: 'Apps Buntini can use', words: ['permissions', 'mail', 'calendar', 'reminders', 'notes', 'music', 'spotify', 'safari', 'chrome', 'photos', 'messages', 'contacts', 'facetime'] },
  { tab: 'apps', label: 'Coding agents', words: ['claude code', 'codex', 'copilot', 'gemini cli', 'hooks', 'agents'] },
  { tab: 'apps', label: 'Accessibility', words: ['permission', 'control', 'screen lock', 'system settings'] },
  { tab: 'connections', label: 'Web search', words: ['brave', 'tavily', 'search api key', 'internet'] },
  { tab: 'connections', label: 'eBay', words: ['shopping', 'listings', 'prices', 'marketplace'] },
  { tab: 'connections', label: 'Flights', words: ['duffel', 'travel', 'airfare', 'planes'] },
  { tab: 'connections', label: 'GitHub', words: ['pull requests', 'ci', 'developer services', 'git'] },
  { tab: 'connections', label: 'Vercel', words: ['deployments', 'developer services'] },
  { tab: 'connections', label: 'Stripe', words: ['payments', 'developer services'] },
  { tab: 'connections', label: 'Notion', words: ['pages', 'developer services'] },
  { tab: 'connections', label: 'Tools you added', words: ['mcp', 'mcp server', 'tools', 'plugins', 'extensions'] },
  { tab: 'privacy', label: 'Notice what I’m doing', words: ['awareness', 'activity', 'tracking', 'app names'] },
  { tab: 'privacy', label: 'Folders', words: ['files', 'documents', 'file access', 'index', 'add folder'] },
  { tab: 'privacy', label: 'Memory', words: ['remember', 'forget', 'memories', 'facts'] },
  { tab: 'privacy', label: 'Cloud models may see', words: ['sharing', 'data', 'privacy', 'what cloud sees', 'external'] },
  { tab: 'privacy', label: 'macOS updates', words: ['software update', 'update', 'upgrade'] },
  { tab: 'privacy', label: 'Your data', words: ['export', 'delete everything', 'erase', 'support info', 'backup'] },
  { tab: 'privacy', label: 'Activity log', words: ['audit', 'history', 'log'] },
];

const plain = (text: string) => text.toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').replace(/[’']/g, '').replace(/[^a-z0-9]+/g, ' ').trim();

/** How well one word of the query matches some words. */
function fit(token: string, words: string[], label: boolean): number {
  let best = 0;
  for (const word of words) {
    if (word === token) best = Math.max(best, label ? 10 : 8);
    else if (word.startsWith(token)) best = Math.max(best, label ? 7 : 5);
    else if (token.length >= 4 && word.includes(token)) best = Math.max(best, label ? 3 : 2);
  }
  return best;
}

/** The settings that match `query` best, by label first and then common words. Every word of
    the query must match. */
export function searchSettings(query: string, limit = 6): SettingEntry[] {
  const wanted = plain(query);
  if (!wanted) return [];
  const tokens = wanted.split(' ');
  const scored = settingsIndex.map((entry, index) => {
    const label = plain(entry.label);
    const labelWords = label.split(' ');
    const phrases = entry.words.map(plain);
    const otherWords = phrases.flatMap(phrase => phrase.split(' '));
    let score = 0;
    for (const token of tokens) {
      const best = Math.max(fit(token, labelWords, true), fit(token, otherWords, false));
      if (!best) return { entry, index, score: 0 };
      score += best;
    }
    if (label === wanted) score += 30;
    else if (label.startsWith(wanted)) score += 15;
    if (phrases.includes(wanted)) score += 12;
    else if (tokens.length > 1 && phrases.some(phrase => phrase.startsWith(wanted))) score += 6;
    return { entry, index, score };
  });
  return scored.filter(item => item.score > 0).sort((a, b) => b.score - a.score || a.index - b.index).slice(0, limit).map(item => item.entry);
}

const row = (id: string, label: string, keyCode: number, modifiers: Shortcut['modifiers'], keys: string): ShortcutRow => {
  const shortcut = { keyCode, modifiers, label: keys };
  return { id, label, shortcut, default: shortcut, problem: null };
};
const defaultShortcuts: ShortcutRow[] = [
  row('companion', 'Show or hide Buntini', 49, ['shift', 'command'], '⇧⌘Space'),
  row('chat', 'Open the chat', 49, ['control', 'option'], '⌃⌥Space'),
  row('waiting', 'Go to what’s waiting', 0, ['control', 'option'], '⌃⌥A'),
  row('nextPill', 'Next in the island', 30, ['control', 'option'], '⌃⌥]'),
  row('mute', 'Mute the island', 46, ['control', 'option'], '⌃⌥M'),
  row('screenshot', 'Screenshot', 19, ['shift', 'command'], '⇧⌘2'),
  row('area', 'Capture an area', 1, ['shift', 'command'], '⇧⌘S'),
  row('focus', 'Focus timer', 3, ['control', 'option'], '⌃⌥F'),
];
let previewShortcuts = defaultShortcuts.map(row => ({ ...row }));

/** What the browser preview answers for `utilities_call`: shortcuts kept in memory, with the
    simplest of Buntini's checks. */
export function preview(op: UtilitiesOp): unknown {
  switch (op.op) {
    case 'ping': return true;
    case 'shortcuts': return previewShortcuts;
    case 'resetShortcuts': previewShortcuts = defaultShortcuts.map(row => ({ ...row })); return previewShortcuts;
    case 'recording': case 'focus': return null;
    case 'setShortcut': {
      const chosen = op.shortcut;
      if (chosen) {
        const needs = chosen.modifiers.some(name => name !== 'shift') || (chosen.label.startsWith('F') && chosen.label.length > 1);
        if (!needs) throw new Error('Add ⌘, ⌃ or ⌥');
        const other = previewShortcuts.find(row => row.id !== op.id && row.shortcut?.keyCode === chosen.keyCode && [...row.shortcut.modifiers].sort().join() === [...chosen.modifiers].sort().join());
        if (other) throw new Error(`${other.label} uses it`);
      }
      previewShortcuts = previewShortcuts.map(row => row.id === op.id ? { ...row, shortcut: chosen } : row);
      return previewShortcuts;
    }
    default: throw new Error('Open the desktop app to use this.');
  }
}
