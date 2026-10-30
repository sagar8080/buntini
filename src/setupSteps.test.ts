// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { applyAnswers, blocker, initialAnswers, keysToSave, message, next, previous, reply, shortcutNeeds, steps } from './setupSteps.ts';
import type { Diagnostics, Settings } from './types';

const settings = {
  assistantName: 'Buntini', userName: '', catCoat: 'ginger', avatar: 'cat', modelBackend: 'apple', pythonPath: '', modelPath: '',
  cloudProvider: 'anthropic', cloudModel: '', cloudBudgetUsd: 5, cloudInputPrice: 4, cloudOutputPrice: 20,
  searchProvider: 'brave', freeTierOnly: true, dailySearchLimit: 30, petRoam: true, petAwareness: true, petNudges: true,
} as Settings;
const diagnostics = (appleIntelligence: string, cloudKeys: string[] = []) => ({ appleIntelligence, cloudKeys }) as Diagnostics;

test('steps run in order and stop at the ends', () => {
  assert.deepEqual(steps, ['hello', 'brain', 'online', 'roam', 'done']);
  assert.equal(next('hello'), 'brain');
  assert.equal(next('done'), 'done');
  assert.equal(previous('hello'), 'hello');
});

test('a Mac without Apple Intelligence starts on another brain and cannot pick it', () => {
  assert.equal(initialAnswers(settings, diagnostics('available')).backend, 'apple');
  assert.equal(initialAnswers(settings, diagnostics('appleIntelligenceNotEnabled')).backend, 'apple', 'it can be turned on later');
  const unsupported = initialAnswers(settings, diagnostics('unsupported'));
  assert.equal(unsupported.backend, 'cloud');
  assert.match(blocker('brain', { ...unsupported, backend: 'apple' }, diagnostics('unsupported')), /Choose a local or cloud model/);
  assert.match(message('brain', unsupported, diagnostics('unsupported')), /isn’t ready on this Mac/);
});

test('each step says what is missing', () => {
  const answers = initialAnswers(settings, diagnostics('available'));
  assert.equal(blocker('hello', answers, diagnostics('available')), '');
  assert.match(blocker('hello', { ...answers, assistantName: ' ' }, diagnostics('available')), /name/);
  assert.match(blocker('brain', { ...answers, backend: 'mlx' }, diagnostics('available')), /Python/);
  const cloud = { ...answers, backend: 'cloud' as const };
  assert.match(blocker('brain', cloud, diagnostics('available')), /API key/);
  assert.equal(blocker('brain', cloud, diagnostics('available', ['anthropic'])), '', 'a key saved earlier is enough');
  assert.match(blocker('brain', { ...cloud, cloudKey: 'k', cloudBudget: 0 }, diagnostics('available')), /cap/);
  assert.match(blocker('online', { ...answers, dailyLimit: 0 }, diagnostics('available')), /daily limit/);
});

test('answers read naturally and become settings and keys', () => {
  const answers = { ...initialAnswers(settings, diagnostics('available')), userName: ' Sam ', assistantName: 'Miso', coat: 'calico' as const, backend: 'cloud' as const, cloudProvider: 'openai' as const, cloudKey: ' sk-1 ', cloudBudget: 3, searchKey: 'bsk', dailyLimit: 20, awareness: false };
  assert.equal(reply('hello', answers), 'Call me Sam. You’re Miso, my calico.');
  assert.equal(reply('brain', answers), 'Use OpenAI (gpt-5-mini), up to $3 a month.');
  assert.equal(reply('online', answers), 'Use Brave Search as fallback, up to 20 API requests a day.');
  assert.equal(reply('online', { ...answers, searchKey: '' }), 'Use browser search without an API fallback for now.');
  assert.match(message('done', answers, diagnostics('available')), /All set, Sam!/);
  const saved = applyAnswers(settings, answers);
  assert.equal(saved.userName, 'Sam');
  assert.equal(saved.modelBackend, 'cloud');
  assert.deepEqual([saved.cloudInputPrice, saved.cloudOutputPrice], [0.25, 2], 'a new provider starts from its suggested prices');
  assert.equal(saved.petNudges, false, 'nudges need awareness');
  assert.deepEqual(keysToSave(answers), [['openai', 'sk-1'], ['brave', 'bsk']]);
  assert.deepEqual(keysToSave({ ...answers, backend: 'apple', searchKey: '' }), [], 'a cloud key is only saved for the cloud brain');
  const chatgpt = { ...answers, cloudProvider: 'chatgpt' as const, cloudKey: '' };
  assert.match(blocker('brain', chatgpt, diagnostics('available')), /Sign in with ChatGPT/);
  assert.equal(blocker('brain', chatgpt, { ...diagnostics('available'), chatgptAccount: 'sam@example.com' }), '', 'signed in is enough; no key or cap');
  assert.equal(reply('brain', chatgpt), 'Use ChatGPT with my own plan.');
});

test('welcome shortcuts ask for the apps they read instead of dead-ending', () => {
  assert.deepEqual(shortcutNeeds('reply', [])?.apps, ['mail']);
  assert.match(shortcutNeeds('reply', [])!.allow, /Allow Mail/);
  assert.equal(shortcutNeeds('reply', ['mail']), null);
  assert.deepEqual(shortcutNeeds('day', ['music'])?.apps, ['calendar', 'reminders']);
  assert.equal(shortcutNeeds('day', ['reminders']), null, 'the brief works with any one of its apps');
  assert.equal(shortcutNeeds('day', ['mail']), null);
});
