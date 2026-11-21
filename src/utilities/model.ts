// Shared shapes for shortcuts and settings search (stream F). No bridge import, so the bridge can use them.
import { englishWords, t } from '../i18n.ts';

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

/** A tab's name as its button shows it. */
export function tabName(tab: SettingsTab): string {
  switch (tab) {
    case 'buntini': return 'Buntini';
    case 'brain': return t('settings.tabs.brain');
    case 'apps': return t('settings.tabs.apps');
    case 'connections': return t('settings.tabs.connections');
    case 'privacy': return t('settings.tabs.privacy');
  }
}

/** A setting to find: the catalog key of its label (empty for a name such as GitHub, which reads
    the same in every language), its label as Settings shows it now, and the words people use
    for it. */
export type SettingEntry = { tab: SettingsTab; key: string; readonly label: string; words: string[] };

const setting = (tab: SettingsTab, key: string, words: string[]): SettingEntry => ({ tab, key, words, get label() { return t(key); } });
const named = (tab: SettingsTab, name: string, words: string[]): SettingEntry => ({ tab, key: '', label: name, words });

/** Every setting, tab by tab. A label is found on screen by its text, so each uses the key the
    setting itself is shown with. */
export const settingsIndex: SettingEntry[] = [
  setting('buntini', 'settings.name.label', ['rename', 'call', 'assistant name', 'nickname']),
  setting('buntini', 'settings.character', ['cat', 'orb', 'avatar', 'pet', 'mascot']),
  setting('buntini', 'settings.look.title', ['coat', 'fur', 'eyes', 'eye colour', 'eye color', 'accessory', 'bell', 'bow', 'hat', 'outfit', 'wardrobe', 'surprise me']),
  setting('buntini', 'settings.theme.title', ['colours', 'colors', 'accent', 'palette', 'skin', 'style']),
  setting('buntini', 'settings.appearance.title', ['dark mode', 'light mode', 'auto', 'night']),
  setting('buntini', 'settings.textSize', ['font size', 'bigger', 'smaller', 'zoom', 'readability', 'panel size']),
  setting('buntini', 'settings.catSize', ['companion size', 'scale', 'bigger cat', 'smaller cat']),
  setting('buntini', 'settings.lessMotion', ['reduce motion', 'animation', 'still', 'calm', 'accessibility']),
  setting('buntini', 'settings.personality.title', ['tone', 'style', 'behaviour', 'behavior', 'instructions', 'prompt', 'how it talks']),
  setting('buntini', 'settings.voice.title', ['speaking voice', 'speech', 'text to speech', 'tts', 'siri voice']),
  setting('buntini', 'settings.voice.pace', ['speech rate', 'speed', 'talk faster', 'talk slower']),
  setting('buntini', 'settings.voice.answerAloud', ['speak replies', 'voice replies', 'talk back']),
  setting('buntini', 'settings.voice.readAloud', ['speak', 'read aloud', 'tts', 'voice']),
  setting('buntini', 'settings.outAbout.wander', ['roam', 'walk', 'move around', 'wander']),
  setting('buntini', 'settings.outAbout.offerHand', ['nudges', 'suggestions', 'tips', 'stretch break', 'help']),
  setting('buntini', 'settings.outAbout.morningBrief', ['daily summary', 'briefing', 'morning', 'my day']),
  setting('buntini', 'settings.outAbout.headsUps', ['heads up', 'upcoming', 'reminders', 'alerts']),
  setting('buntini', 'settings.outAbout.notifications', ['notifications', 'alerts', 'banners', 'notification centre', 'notification center']),
  setting('buntini', 'settings.outAbout.openAtLogin', ['startup', 'launch at login', 'start automatically', 'login items', 'boot']),
  setting('buntini', 'settings.language.title', ['app language', 'interface language', 'locale', 'localization', 'right to left']),
  setting('buntini', 'settings.runSetupAgain', ['onboarding', 'reset', 'start over', 'welcome']),
  setting('buntini', 'settings.flow.dictateAnywhere', ['dictation', 'voice typing', 'speech to text', 'microphone', 'mic']),
  setting('buntini', 'settings.flow.hold', ['dictation key', 'push to talk', 'right option', 'fn key', 'hotkey']),
  setting('buntini', 'settings.flow.languages', ['language', 'dictation language', 'english', 'translate']),
  setting('buntini', 'settings.flow.mute', ['mute', 'silence', 'music while dictating']),
  named('buntini', 'Hey Buntini', ['wake word', 'hey', 'voice activation', 'always listening', 'hands free']),
  setting('buntini', 'settings.flow.recordCalls', ['meeting notes', 'call notes', 'transcript', 'recording', 'zoom', 'meetings']),
  setting('buntini', 'settings.flow.speechModel', ['parakeet', 'download', 'offline speech', 'model']),
  setting('buntini', 'library.title', ['dictation history', 'notes', 'stats', 'transcripts']),
  setting('buntini', 'settings.flow.dictionary', ['words', 'spelling', 'names', 'vocabulary', 'your words']),
  setting('buntini', 'settings.flow.replacements', ['autocorrect', 'text replacement', 'expand', 'your words']),
  setting('buntini', 'settings.flow.snippets', ['templates', 'canned text', 'text expansion', 'your words']),
  setting('buntini', 'settings.shortcuts.title', ['keyboard shortcuts', 'hotkeys', 'hot keys', 'keys', 'key bindings', 'keybindings']),
  // The shortcut rows come from Rust, named with these keys.
  setting('buntini', 'utilities.shortcuts.companion', ['open buntini', 'companion', 'summon', 'toggle', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.chat', ['chat shortcut', 'ask', 'conversation', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.waiting', ['approval', 'request', 'agent waiting', 'permission', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.nextPill', ['next pill', 'next session', 'cycle', 'island', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.mute', ['sounds', 'mute', 'quiet', 'silence', 'chime', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.screenshot', ['screen capture', 'screen grab', 'print screen', 'clipboard', 'thumbnail', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.area', ['area capture', 'region', 'selection', 'snip', 'crop', 'mark up', 'annotate', 'shortcut']),
  setting('buntini', 'utilities.shortcuts.focus', ['pomodoro', 'timer', 'work session', 'break', 'concentrate', 'shortcut']),
  setting('brain', 'settings.brain.title', ['model', 'ai', 'llm', 'apple intelligence', 'local model', 'cloud model', 'qwen', 'gemma']),
  setting('brain', 'settings.brain.auto', ['automatic brain', 'auto model']),
  setting('brain', 'settings.brain.provider', ['anthropic', 'claude', 'openai', 'gpt', 'gemini', 'chatgpt', 'api key']),
  setting('brain', 'settings.brain.cap', ['budget', 'spending', 'cost', 'limit', 'money', 'price']),
  setting('brain', 'settings.brain.serverAddress', ['ollama', 'lm studio', 'own server', 'localhost', 'host']),
  setting('apps', 'settings.apps.title', ['permissions', 'mail', 'calendar', 'reminders', 'notes', 'music', 'spotify', 'safari', 'chrome', 'photos', 'messages', 'contacts', 'facetime', 'podcasts', 'vlc', 'iina', 'players']),
  setting('apps', 'settings.agents.title', ['claude code', 'codex', 'copilot', 'gemini cli', 'hooks', 'agents']),
  setting('apps', 'settings.apps.accessibility', ['permission', 'control', 'screen lock', 'system settings']),
  setting('connections', 'settings.connections.webSearch', ['brave', 'tavily', 'search api key', 'internet']),
  named('connections', 'eBay', ['shopping', 'listings', 'prices', 'marketplace']),
  setting('connections', 'settings.connections.flights', ['duffel', 'travel', 'airfare', 'planes']),
  named('connections', 'GitHub', ['pull requests', 'ci', 'developer services', 'git']),
  named('connections', 'Vercel', ['deployments', 'developer services']),
  named('connections', 'Stripe', ['payments', 'developer services']),
  named('connections', 'Notion', ['pages', 'developer services']),
  setting('connections', 'settings.tools.title', ['mcp', 'mcp server', 'tools', 'plugins', 'extensions']),
  setting('privacy', 'settings.privacy.notice', ['awareness', 'activity', 'tracking', 'app names']),
  setting('privacy', 'settings.privacy.folders', ['files', 'documents', 'file access', 'index', 'add folder']),
  setting('privacy', 'settings.privacy.memory', ['remember', 'forget', 'memories', 'facts']),
  setting('privacy', 'settings.privacy.cloudMaySee', ['sharing', 'data', 'privacy', 'what cloud sees', 'external']),
  setting('privacy', 'settings.macUpdates.title', ['software update', 'update', 'upgrade']),
  setting('privacy', 'settings.data.title', ['export', 'delete everything', 'erase', 'support info', 'backup']),
  setting('privacy', 'settings.privacy.activityLog', ['audit', 'history', 'log']),
];

// Letters and marks of any script stay, so labels in every language can be searched.
const plain = (text: string) => text.toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').replace(/[’']/g, '').replace(/[^\p{L}\p{M}\p{N}]+/gu, ' ').trim();
/** Scripts written without spaces, where a label is one long word. */
const unspaced = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Thai}]/u;

/** How well one word of the query matches some words. */
function fit(token: string, words: string[], label: boolean): number {
  let best = 0;
  for (const word of words) {
    if (word === token) best = Math.max(best, label ? 10 : 8);
    else if (word.startsWith(token)) best = Math.max(best, label ? 7 : 5);
    else if ((token.length >= 4 || (token.length >= 2 && unspaced.test(token))) && word.includes(token)) best = Math.max(best, label ? 3 : 2);
  }
  return best;
}

/** The settings that match `query` best, by label first and then common words. Every word of
    the query must match. A label matches as shown and in English, so English words still find
    a setting in any language. */
export function searchSettings(query: string, limit = 6): SettingEntry[] {
  const wanted = plain(query);
  if (!wanted) return [];
  const tokens = wanted.split(' ');
  const scored = settingsIndex.map((entry, index) => {
    const labels = [...new Set([entry.label, entry.key ? englishWords[entry.key] ?? '' : ''].map(plain).filter(Boolean))];
    const labelWords = labels.flatMap(label => label.split(' '));
    const phrases = entry.words.map(plain);
    const otherWords = phrases.flatMap(phrase => phrase.split(' '));
    let score = 0;
    for (const token of tokens) {
      const best = Math.max(fit(token, labelWords, true), fit(token, otherWords, false));
      if (!best) return { entry, index, score: 0 };
      score += best;
    }
    if (labels.includes(wanted)) score += 30;
    else if (labels.some(label => label.startsWith(wanted))) score += 15;
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
// Named as Rust names them, in the language spoken when the preview answers.
const defaultShortcuts = (): ShortcutRow[] => [
  row('companion', t('utilities.shortcuts.companion'), 49, ['shift', 'command'], '⇧⌘Space'),
  row('chat', t('utilities.shortcuts.chat'), 49, ['control', 'option'], '⌃⌥Space'),
  row('waiting', t('utilities.shortcuts.waiting'), 0, ['control', 'option'], '⌃⌥A'),
  row('nextPill', t('utilities.shortcuts.nextPill'), 30, ['control', 'option'], '⌃⌥]'),
  row('mute', t('utilities.shortcuts.mute'), 46, ['control', 'option'], '⌃⌥M'),
  row('screenshot', t('utilities.shortcuts.screenshot'), 19, ['shift', 'command'], '⇧⌘2'),
  row('area', t('utilities.shortcuts.area'), 1, ['control', 'option'], '⌃⌥S'),
  row('focus', t('utilities.shortcuts.focus'), 3, ['control', 'option'], '⌃⌥F'),
];
/** The preview's changed shortcuts, by id. */
let previewChosen: Record<string, Shortcut | null> = {};
const previewShortcuts = () => defaultShortcuts().map(row => row.id in previewChosen ? { ...row, shortcut: previewChosen[row.id] } : row);

/** What the browser preview answers for `utilities_call`: shortcuts kept in memory, with the
    simplest of Buntini's checks. */
export function preview(op: UtilitiesOp): unknown {
  switch (op.op) {
    case 'ping': return true;
    case 'shortcuts': return previewShortcuts();
    case 'resetShortcuts': previewChosen = {}; return previewShortcuts();
    case 'recording': case 'focus': return null;
    case 'setShortcut': {
      const chosen = op.shortcut;
      if (chosen) {
        const needs = chosen.modifiers.some(name => name !== 'shift') || (chosen.label.startsWith('F') && chosen.label.length > 1);
        if (!needs) throw new Error(t('utilities.shortcuts.addModifier'));
        const other = previewShortcuts().find(row => row.id !== op.id && row.shortcut?.keyCode === chosen.keyCode && [...row.shortcut.modifiers].sort().join() === [...chosen.modifiers].sort().join());
        if (other) throw new Error(t('utilities.shortcuts.usedBy', { action: other.label }));
      }
      previewChosen = { ...previewChosen, [op.id]: chosen };
      return previewShortcuts();
    }
    default: throw new Error(t('common.desktopOnly'));
  }
}
