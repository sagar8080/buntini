// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { accentHex, appearance, idleStatus } from './themes.ts';
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
  assert.equal(accentHex({ ...base, accent: 'custom', accentColor: 'nope' }, false), '#c66b31', 'an invalid custom colour falls back to the theme');
  const custom = appearance({ ...base, accent: 'custom', accentColor: '#123abc' }, true);
  assert.equal(custom.variables['--accent'], '#123abc');
  assert.match(custom.variables['--accent-soft'], /color-mix/);
});

test('the idle status speaks in Buntini’s voice unless the user wrote their own', () => {
  assert.equal(idleStatus(base, false, 14), 'Purring nearby');
  assert.equal(idleStatus({ ...base, petMotion: 'calm' }, false, 14), 'Right here');
  assert.equal(idleStatus(base, true, 14), 'Napping');
  assert.equal(idleStatus(base, false, 1), 'Up late with you');
  assert.equal(idleStatus({ ...base, statusText: '  Coding mode  ' }, true, 1), 'Coding mode');
});
