// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import { readFileSync } from 'node:fs';
import { accentChoices, accentHex, appearance, contrast, idleStatus, onAccent, presets, readableAccent } from './themes.ts';
import type { Settings } from './types';

const base = { theme: 'system', themePreset: 'classic', accent: 'preset', accentColor: '', textSize: 'medium', density: 'comfortable', chatFont: 'sans', corners: 'round', statusText: '', petMotion: 'playful' } as Settings;

test('themes follow light and dark, and the theme accent comes from the stylesheet', () => {
  const light = appearance(base, false), dark = appearance(base, true);
  assert.equal(light.attributes.mode, 'light');
  assert.equal(dark.attributes.mode, 'dark');
  assert.equal(light.variables['--accent'], undefined);
  assert.equal(appearance({ ...base, theme: 'dark' }, false).attributes.mode, 'dark', 'an explicit choice beats the system');
  assert.equal(appearance({ ...base, textSize: 'large' }, false).variables['--text-scale'], '1.12');
});

test('accents can follow the theme, use a fixed colour or a custom one', () => {
  assert.equal(accentHex({ ...base, themePreset: 'sakura' }, true), '#f08aa7');
  assert.equal(accentHex({ ...base, accent: 'ocean' }, false), '#71bacb');
  assert.equal(accentHex({ ...base, accent: 'custom', accentColor: '#123ABC' }, false), '#123ABC');
  assert.equal(accentHex({ ...base, accent: 'custom', accentColor: 'nope' }, false), '#ab4c19', 'an invalid custom colour falls back to the theme');
  const custom = appearance({ ...base, accent: 'custom', accentColor: '#123abc' }, false);
  assert.equal(custom.variables['--accent'], '#123abc', 'a readable colour is kept as picked');
  assert.match(custom.variables['--accent-soft'], /color-mix/);
  assert.notEqual(appearance({ ...base, accent: 'custom', accentColor: '#123abc' }, true).variables['--accent'], '#123abc', 'too dark for dark mode, so it is lightened');
});

test('the idle status speaks in Buntini’s voice unless the user wrote their own', () => {
  assert.equal(idleStatus(base, false, 14), 'Purring nearby');
  assert.equal(idleStatus({ ...base, petMotion: 'calm' }, false, 14), 'Right here');
  assert.equal(idleStatus(base, true, 14), 'Napping');
  assert.equal(idleStatus(base, false, 1), 'Up late with you');
  assert.equal(idleStatus({ ...base, statusText: '  Coding mode  ' }, true, 1), 'Coding mode');
});

// Reads each theme's colours from the stylesheet, as the app shows them.
const css = readFileSync(new URL('./styles.css', import.meta.url), 'utf8') as string;
function themeColours(id: string, dark: boolean) {
  const block = (selector: string) => { const at = css.indexOf(`${selector} {`); assert.ok(at >= 0, selector); return css.slice(at, css.indexOf('}', at)); };
  const read = (text: string) => Object.fromEntries([...text.matchAll(/--([a-z-]+):\s*(#[0-9a-f]{6})/gi)].map(match => [match[1], match[2].toLowerCase()]));
  const colours = { ...read(block(':root')), ...(dark ? read(block(':root[data-mode=dark]')) : {}) };
  if (id !== 'classic') Object.assign(colours, read(block(dark ? `:root[data-preset=${id}][data-mode=dark]` : `:root[data-preset=${id}]:not([data-mode=dark])`)));
  return colours;
}

test('every theme reads at 4.5:1 in light and dark mode', () => {
  for (const preset of presets) for (const dark of [false, true]) {
    const colours = themeColours(preset.id, dark), name = `${preset.id} ${dark ? 'dark' : 'light'}`;
    assert.equal(colours.accent, (dark ? preset.dark : preset.light).accent, `${name}: themes.ts and styles.css agree`);
    assert.equal(colours.panel, (dark ? preset.dark : preset.light).panel, `${name}: panels agree`);
    for (const back of ['panel', 'surface-soft']) {
      assert.ok(contrast(colours.text, colours[back]) >= 4.5, `${name}: text on ${back}`);
      assert.ok(contrast(colours.muted, colours[back]) >= 4.5, `${name}: secondary text on ${back}`);
    }
    for (const back of ['panel', 'accent-soft']) assert.ok(contrast(colours.accent, colours[back]) >= 4.5, `${name}: accent on ${back}`);
    assert.ok(contrast(onAccent(colours.accent), colours.accent) >= 4.5, `${name}: text on the accent`);
    assert.equal(appearance({ ...base, themePreset: preset.id }, dark).variables['--on-accent'], onAccent(colours.accent));
  }
});

test('any accent the user picks is made readable, and text on it follows its luminance', () => {
  const picks = [...accentChoices.flatMap(choice => choice.color ? [choice.color] : []), '#ffff00', '#777777', '#000000', '#ffffff', '#123abc'];
  for (const preset of presets) for (const dark of [false, true]) for (const color of picks) {
    const panel = (dark ? preset.dark : preset.light).panel;
    const accent = readableAccent(color, panel, dark);
    assert.ok(contrast(accent, panel) >= 4.5, `${color} on ${preset.id}`);
    assert.ok(contrast(onAccent(accent), accent) >= 4.5, `text on ${color} (${accent})`);
  }
  assert.equal(onAccent('#ffff00'), '#15171c', 'a light accent gets dark text');
  assert.equal(onAccent('#123abc'), '#ffffff', 'a dark accent gets white text');
  const ember = appearance({ ...base, accent: 'ember' }, false).variables;
  assert.notEqual(ember['--accent'], '#e9985d', 'a pale accent is darkened in light mode');
  assert.equal(appearance({ ...base, accent: 'ember' }, true).variables['--accent'], '#e9985d', 'and left alone where it already reads');
});
