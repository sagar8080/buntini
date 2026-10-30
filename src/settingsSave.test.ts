// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { SettingsSaver, isBusyRefusal, type SaveState } from './settingsSave.ts';
import type { Settings } from './types';

const busy = 'Finish or cancel the current task before changing model or online policy settings.';
const settings = (apps: string[]) => ({ apps }) as Settings;
const pause = (ms: number) => new Promise(resolve => setTimeout(resolve, ms));

test('closing Settings straight after a toggle still saves it', async () => {
  const saved: string[][] = [];
  const saver = new SettingsSaver(async next => { saved.push(next.apps); }, { delayMs: 10_000 });
  saver.edit(settings(['mail']));
  assert.deepEqual(saver.pending()?.apps, ['mail'], 'reopening shows the edit, not the older copy');
  await saver.flush();
  assert.deepEqual(saved, [['mail']], 'Esc does not wait for the pause in editing');
  assert.equal(saver.pending(), null);
  assert.equal(saver.state, 'saved');
});

test('a save refused during a task waits and goes through when the task ends', async () => {
  let taskRunning = true;
  const states: SaveState[] = [];
  const saved: string[][] = [];
  const saver = new SettingsSaver(async next => { if (taskRunning) throw busy; saved.push(next.apps); }, { delayMs: 0, retryMs: 5 });
  saver.subscribe(state => states.push(state));
  saver.edit(settings(['mail']));
  const done = saver.flush();
  await pause(20);
  assert.equal(saver.state, 'waiting');
  assert.deepEqual(saver.pending()?.apps, ['mail'], 'the edit is kept while it waits');
  saver.edit(settings(['mail', 'notes']));
  taskRunning = false;
  await done;
  await saver.flush();
  assert.deepEqual(saved, [['mail', 'notes']], 'only the newest edit is sent');
  assert.ok(!states.includes('error'), 'waiting is never shown as a failure');
  assert.equal(saver.state, 'saved');
});

test('a real failure is reported and the edit is not kept as if saved', async () => {
  const saver = new SettingsSaver(async () => { throw new Error('Choose a daily limit between 1 and 2,000.'); }, { delayMs: 0 });
  await assert.rejects(saver.saveNow(settings(['mail'])), /daily limit/);
  assert.equal(saver.state, 'error');
  assert.equal(saver.pending(), null, 'reopening Settings shows the last saved state');
});

test('saves run one at a time, newest last', async () => {
  const order: string[] = [];
  let inFlight = 0;
  const saver = new SettingsSaver(async next => { inFlight++; assert.equal(inFlight, 1); await pause(5); order.push(next.apps.join()); inFlight--; }, { delayMs: 0 });
  const first = saver.saveNow(settings(['a']));
  saver.edit(settings(['b']));
  await first;
  await saver.flush();
  await pause(10);
  assert.deepEqual(order, ['a', 'b']);
  assert.ok(isBusyRefusal(busy) && !isBusyRefusal('Choose a daily limit'));
});
