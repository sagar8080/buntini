// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { applyAnswers, blocker, connectPane, connectRows, initialAnswers, message, next, previous, questions, reply, shortcutNeeds, steps } from './setupSteps.ts';
import type { Diagnostics, Settings } from './types';

const settings = {
  assistantName: 'Buntini', userName: '', catCoat: 'ginger', avatar: 'cat', modelBackend: 'apple', brainAuto: true, pythonPath: '', modelPath: '',
  cloudProvider: 'anthropic', cloudModel: '', cloudBudgetUsd: 5, cloudInputPrice: 4, cloudOutputPrice: 20,
  searchProvider: 'brave', freeTierOnly: true, dailySearchLimit: 30, petRoam: true, petAwareness: true, petNudges: true,
} as Settings;
const diagnostics = (appleIntelligence: string, cloudKeys: string[] = []) => ({ appleIntelligence, cloudKeys }) as Diagnostics;

test('steps run in order and stop at the ends', () => {
  assert.deepEqual(steps, ['hello', 'brain', 'roam', 'done'], 'no search-provider step: browser research needs no key');
  assert.equal(questions, 3);
  assert.equal(next('hello'), 'brain');
  assert.equal(next('done'), 'done');
  assert.equal(previous('hello'), 'hello');
});

test('a Mac without Apple Intelligence starts on another brain and cannot pick it', () => {
  assert.equal(initialAnswers(settings, diagnostics('available')).backend, 'apple');
  assert.equal(initialAnswers(settings, diagnostics('appleIntelligenceNotEnabled')).backend, 'apple', 'it can be turned on later');
  const language = initialAnswers(settings, diagnostics('languageNotSupported'));
  assert.equal(language.backend, 'apple', 'a Mac in Hindi can switch to English (India) and turn it on');
  assert.equal(blocker('brain', language, diagnostics('languageNotSupported')), '');
  assert.equal(initialAnswers(settings, diagnostics('modelStuck')).backend, 'apple', 'fixed in System Settings, it gets ready');
  const unsupported = initialAnswers(settings, diagnostics('unsupported'));
  assert.equal(unsupported.backend, 'cloud');
  assert.match(blocker('brain', { ...unsupported, backend: 'apple' }, diagnostics('unsupported')), /Sign in with ChatGPT or set up on this Mac/);
  assert.match(message('brain', unsupported, diagnostics('unsupported')), /isn’t ready on this Mac/);
  assert.equal(unsupported.cloudProvider, 'chatgpt');
});

test('each step says what is missing', () => {
  const answers = initialAnswers(settings, diagnostics('available'));
  assert.equal(blocker('hello', answers, diagnostics('available')), '');
  assert.match(blocker('hello', { ...answers, assistantName: ' ' }, diagnostics('available')), /name/);
  assert.match(blocker('brain', { ...answers, backend: 'mlx' }, diagnostics('available')), /Finish setting up on this Mac/);
  assert.equal(blocker('brain', { ...answers, backend: 'mlx', pythonPath: '/env/bin/python', modelPath: '/models/qwen' }, diagnostics('available')), '');
  assert.match(blocker('brain', { ...answers, backend: 'cloud' }, diagnostics('available')), /Sign in with ChatGPT/);
  const key = { ...answers, backend: 'cloud' as const, cloudProvider: 'anthropic' as const };
  assert.match(blocker('brain', key, diagnostics('available')), /API key in Settings/);
  assert.equal(blocker('brain', key, diagnostics('available', ['anthropic'])), '', 'a key saved earlier is enough');
});

test('answers read naturally and become settings and keys', () => {
  const answers = { ...initialAnswers(settings, diagnostics('available')), userName: ' Sam ', assistantName: 'Miso', coat: 'calico' as const, backend: 'cloud' as const, cloudProvider: 'openai' as const, awareness: false };
  assert.equal(reply('hello', answers), 'Call me Sam. You’re Miso, my calico.');
  assert.equal(reply('brain', answers, settings), 'Use OpenAI, up to $5 a month. It sees nothing personal.');
  assert.equal(reply('brain', answers, { cloudBudgetUsd: 0 }), 'Use OpenAI, up to $5 a month. It sees nothing personal.', 'no saved cap: the default');
  assert.equal(reply('brain', { ...answers, cloudProvider: 'chatgpt', shares: ['files'] }), 'Use ChatGPT with my own plan. It may see my files.');
  assert.match(message('done', answers, diagnostics('available')), /^All set, Sam! Ask me anything\.$/);
  const saved = applyAnswers({ ...settings, cloudModel: 'claude-old', freeTierOnly: false, dailySearchLimit: 12 }, answers);
  assert.equal(saved.userName, 'Sam');
  assert.equal(saved.modelBackend, 'cloud');
  assert.equal(saved.cloudModel, '', 'a new provider starts on its suggested model');
  assert.deepEqual([saved.cloudInputPrice, saved.cloudOutputPrice], [0.25, 2], 'a new provider starts from its list prices');
  assert.equal(saved.cloudBudgetUsd, 5);
  assert.deepEqual([saved.freeTierOnly, saved.dailySearchLimit], [false, 12], 'search choices saved earlier are kept');
  assert.equal(applyAnswers({ ...settings, cloudBudgetUsd: 0 }, answers).cloudBudgetUsd, 5, 'a cap of $0 would block the brain just picked');
  assert.equal(applyAnswers({ ...settings, cloudModel: 'claude-old' }, { ...answers, cloudProvider: 'anthropic' }).cloudModel, 'claude-old', 'the same provider keeps its model');
  assert.equal(saved.petNudges, false, 'nudges need awareness');
  const chatgpt = { ...answers, cloudProvider: 'chatgpt' as const };
  assert.match(blocker('brain', chatgpt, diagnostics('available')), /Sign in with ChatGPT/);
  assert.equal(blocker('brain', chatgpt, { ...diagnostics('available'), chatgptAccount: 'sam@example.com' }), '', 'signed in is enough; no key or cap');
  assert.equal(reply('brain', chatgpt), 'Use ChatGPT with my own plan. It sees nothing personal.');
});

test('welcome shortcuts ask for the apps they read instead of dead-ending', () => {
  assert.deepEqual(shortcutNeeds('reply', [])?.apps, ['mail']);
  assert.match(shortcutNeeds('reply', [])!.allow, /Allow Mail/);
  assert.equal(shortcutNeeds('reply', ['mail']), null);
  assert.deepEqual(shortcutNeeds('day', ['music'])?.apps, ['calendar', 'reminders']);
  assert.equal(shortcutNeeds('day', ['reminders']), null, 'the brief works with any one of its apps');
  assert.equal(shortcutNeeds('day', ['mail']), null);
});

test('what a cloud brain may see is saved only when a cloud brain was chosen', () => {
  const start = { ...settings, externalShares: ['music'] } as Settings;
  const answers = initialAnswers(start, diagnostics('unsupported'));
  assert.deepEqual(answers.shares, ['music'], 'starts from the Privacy choices');
  assert.deepEqual(applyAnswers(start, { ...answers, backend: 'cloud', shares: ['mail', 'files'] }).externalShares, ['mail', 'files']);
  assert.deepEqual(applyAnswers(start, { ...answers, backend: 'apple', shares: ['mail'] }).externalShares, ['music']);
});

test('setup follows the rule: Apple Intelligence, then the ChatGPT plan, then this Mac', () => {
  const plan = (appleIntelligence: string, chatgptPlan: 'none' | 'usable' | 'refused') => ({ appleIntelligence, chatgptPlan, cloudKeys: [] }) as unknown as Diagnostics;
  const start = (reason: string, chatgptPlan: 'none' | 'usable' | 'refused', from = settings) => initialAnswers(from, plan(reason, chatgptPlan));
  assert.equal(start('available', 'usable').backend, 'apple', 'ready: nothing else is considered');
  assert.equal(start('checking', 'none').backend, 'apple');
  assert.deepEqual([start('modelStuck', 'usable').backend, start('modelStuck', 'usable').cloudProvider], ['cloud', 'chatgpt'], 'not ready, and a plan that works');
  assert.equal(start('modelNotReady', 'none').backend, 'apple', 'it can still get ready, so nothing is forced yet');
  assert.equal(start('deviceNotEligible', 'none').backend, 'cloud', 'sign-in comes first');
  assert.equal(start('deviceNotEligible', 'refused').backend, 'mlx', 'the plan said no');
  assert.equal(start('deviceNotEligible', 'none', { ...settings, modelPath: '/models/qwen' }).backend, 'mlx', 'a local model is already here');
  assert.equal(start('deviceNotEligible', 'usable', { ...settings, modelPath: '/models/qwen' }).backend, 'cloud', 'the plan comes before the local model');

  const mine = { ...settings, brainAuto: false, modelBackend: 'cloud', cloudProvider: 'openai' } as Settings;
  const kept = start('available', 'usable', mine);
  assert.deepEqual([kept.backend, kept.cloudProvider], ['cloud', 'openai'], 'a brain the user picked stays');
  assert.equal(applyAnswers(mine, kept).brainAuto, false);
  assert.equal(applyAnswers(settings, start('available', 'none')).brainAuto, true, 'setup leaves it to Buntini');

  assert.match(blocker('brain', { ...start('deviceNotEligible', 'refused'), backend: 'cloud' }, { ...plan('deviceNotEligible', 'refused'), chatgptAccount: 'sam@example.com' }), /can’t use its plan/);
  assert.equal(blocker('brain', start('modelStuck', 'none'), plan('modelStuck', 'none')), '', 'Continue still works while it may yet get ready');
  assert.match(message('brain', start('available', 'none'), plan('available', 'none')), /I’ll think with Apple Intelligence/);
  assert.match(message('brain', start('modelStuck', 'none'), plan('modelStuck', 'none')), /How should I think\?/);
  assert.equal(reply('brain', { ...start('available', 'none'), backend: 'mlx' }), 'Set up on this Mac.');
  const local = { ...start('deviceNotEligible', 'none'), backend: 'mlx' as const };
  assert.match(blocker('brain', local, plan('deviceNotEligible', 'none')), /Finish setting up/);
  assert.equal(blocker('brain', { ...local, localStarted: true }, plan('deviceNotEligible', 'none')), '', 'setup needn’t wait for a download');
});

test('the connect card shows where Mail and Calendar stand, and what can fix them', () => {
  const app = (id: string, installed = true) => ({ id, name: id, can: [], installed, allowed: false });
  const apps = [app('mail'), app('calendar'), app('notes')];
  const states = (allowed: string[], found = {}, list = apps) => connectRows(list, allowed, found).map(row => [row.id, row.state]);
  assert.deepEqual(states([]), [['mail', 'off'], ['calendar', 'off']], 'only these two, whatever else exists');
  assert.deepEqual(states(['calendar', 'notes']), [['mail', 'off'], ['calendar', 'ready']]);
  assert.deepEqual(states(['mail'], { mail: 'noAccount' }), [['mail', 'noAccount'], ['calendar', 'off']], 'allowed is not the same as working');
  assert.deepEqual(states(['mail'], { mail: null }), [['mail', 'off'], ['calendar', 'off']], 'a failed try can be tried again');
  assert.deepEqual(states([], {}, [app('mail', false), app('calendar')]), [['calendar', 'off']]);
  const pane = (id: string, found: 'noAccount' | 'denied' | 'ready') => connectPane(connectRows(apps, [id], { [id]: found }).find(row => row.id === id)!);
  assert.equal(pane('mail', 'noAccount'), 'accounts');
  assert.equal(pane('mail', 'denied'), 'automation');
  assert.equal(pane('calendar', 'denied'), 'calendars');
  assert.equal(pane('mail', 'ready'), null);
});
