// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { askChatGptQuestion, askedChatGpt, chatGptHelp } from './chatgptHelp.ts';
import type { TaskRecord } from './types';

const task = (fields: Partial<TaskRecord>): TaskRecord => ({ id: 't', prompt: 'Who won last night?', answer: 'Ana did.', state: 'completed', createdAt: '', sources: [], files: [], backend: 'apple', ...fields });
const asked = { researchReport: { attempts: [{ route: 'chatgpt', status: 'answered', detail: 'Who won last night?' }] } };

test('takes the question out of what was said', () => {
    assert.equal(askChatGptQuestion('Ask ChatGPT who won last night'), 'who won last night');
    assert.equal(askChatGptQuestion('ask chatgpt: who won'), 'who won');
    assert.equal(askChatGptQuestion('please ask chat gpt to write a poem'), 'write a poem');
    assert.equal(askChatGptQuestion('Can you ask ChatGPT, is it raining?'), 'is it raining?');
    assert.equal(askChatGptQuestion('ask chatgpt\nline one\nline two'), 'line one\nline two');
});
test('leaves everything else to the brain', () => {
    assert.equal(askChatGptQuestion('Ask ChatGPT'), null);
    assert.equal(askChatGptQuestion('ask chatgpt   '), null);
    assert.equal(askChatGptQuestion('What did I ask ChatGPT yesterday?'), null);
    assert.equal(askChatGptQuestion('ask chatgptish things'), null);
});

test('offers nothing on a reply that did the job', () => {
    assert.equal(chatGptHelp(task({}), 'usable'), null);
    assert.equal(chatGptHelp(task({ state: 'running' }), 'usable'), null);
    assert.equal(chatGptHelp(task({ state: 'cancelled' }), 'usable'), null);
    assert.equal(chatGptHelp(task({ answer: `I can't believe it. ${'She won by a mile. '.repeat(20)}` }), 'usable'), null);
});
test('offers to ask when Apple Intelligence or the local model could not answer', () => {
    assert.equal(chatGptHelp(task({ state: 'error', answer: 'Apple Intelligence declined to answer this request.' }), 'usable'), 'ask');
    assert.equal(chatGptHelp(task({ answer: 'I can’t look that up.' }), 'usable'), 'ask');
    assert.equal(chatGptHelp(task({ state: 'error', backend: 'mlx' }), 'usable'), 'ask');
});
test('hands off without a usable plan', () => {
    for (const plan of ['none', 'refused', undefined] as const) assert.equal(chatGptHelp(task({ state: 'error' }), plan), 'open');
});
test('hands off what plan use can’t do', () => {
    assert.equal(chatGptHelp(task({ prompt: 'Draw me a picture of a cat' }), 'usable'), 'open');
    assert.equal(chatGptHelp(task({ prompt: 'Make a logo for my shop', backend: 'cloud' }), 'usable'), 'open');
});
test('hands off when ChatGPT itself could not answer, and offers nothing once it has', () => {
    assert.equal(chatGptHelp(task({ ...asked }), 'usable'), null);
    assert.equal(chatGptHelp(task({ ...asked, answer: 'I can’t say.' }), 'usable'), null);
    assert.equal(chatGptHelp(task({ ...asked, state: 'error' }), 'usable'), 'open');
    assert.equal(chatGptHelp(task({ backend: 'cloud', state: 'error' }), 'none', { cloudProvider: 'chatgpt' }), 'open');
    assert.equal(chatGptHelp(task({ backend: 'cloud', state: 'error' }), 'none', { cloudProvider: 'anthropic' }), null);
    assert.equal(chatGptHelp(task({ backend: 'cloud', answer: 'I can’t say.' }), 'usable', { cloudProvider: 'chatgpt' }), null);
});
test('reads the question that was sent from the saved reply', () => {
    assert.equal(askedChatGpt(task({ ...asked })), 'Who won last night?');
    assert.equal(askedChatGpt(task({})), undefined);
    assert.equal(askedChatGpt(task({ researchReport: { attempts: [{ route: 'browser', status: 'read', detail: 'x' }] } })), undefined);
});
