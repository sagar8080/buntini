// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { folderName, friendlyStatus, setupLines } from './brain.ts';

test('chosen folders read as names, not paths', () => {
  assert.equal(folderName('/Users/sam/buntini-mlx/bin/python'), 'buntini-mlx');
  assert.equal(folderName('/Users/sam/buntini-models/Qwen3.5-4B-MLX-4bit/'), 'Qwen3.5-4B-MLX-4bit');
  assert.equal(folderName(''), '');
});

test('worker status reads plainly', () => {
  assert.equal(friendlyStatus('Not started · local tools are ready', true, 'mlx'), 'Ready. The model loads the first time you chat.');
  assert.equal(friendlyStatus('Not started · local tools are ready', false, 'mlx'), 'Not set up yet.');
  assert.equal(friendlyStatus('Ready · Apple Intelligence on this Mac', true, 'apple'), 'Ready · Apple Intelligence on this Mac');
});

test('the setup check becomes a few plain lines', () => {
  const lines = setupLines({
    dependencies: { mlx_lm: { installed: true, version: '0.32.0' }, mlx_whisper: { installed: false, version: null } },
    models: { qwen: { configured: true, files_present: true, ready_for_load: true, error: null }, whisper: { configured: false, files_present: false, ready_for_load: false, error: null } },
  });
  assert.deepEqual(lines.map(line => [line.label, line.ok, line.detail]), [['MLX', true, 'Installed (0.32.0)'], ['Model', true, 'Files are all there']]);
  const missing = setupLines({ dependencies: {}, models: {} });
  assert.match(missing[0].detail, /Install MLX/);
  assert.equal(missing[1].detail, 'Choose a model folder.');
});
