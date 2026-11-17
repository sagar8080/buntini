// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { defaults, normalizeHost, preview, saveRequest, services, statusLine, type ServiceStatus } from './model.ts';

test('hosts become https host names, as Rust checks them', () => {
  assert.equal(normalizeHost(' https://Git.Example.com:8443/some/page '), 'git.example.com:8443');
  assert.equal(normalizeHost('n8n.example.com'), 'n8n.example.com');
  assert.equal(normalizeHost('https://gitlab.example.com:443'), 'gitlab.example.com');
  assert.equal(normalizeHost(''), '');
  assert.equal(normalizeHost('http://git.example.com'), null);
  assert.equal(normalizeHost('user:pass@git.example.com'), null);
  assert.equal(normalizeHost('git example.com'), null);
});

test('every service is listed once, and only the self-hosted ones ask for a host', () => {
  assert.deepEqual(services.map(service => service.id), ['github', 'gitlab', 'gitea', 'vercel', 'stripe', 'n8n', 'notion', 'calcom', 'resend']);
  assert.deepEqual(services.filter(service => service.host).map(service => service.host), ['gitlabHost', 'giteaHost', 'n8nHost']);
  assert.deepEqual(Object.keys(defaults).sort(), ['giteaHost', 'gitlabHost', 'n8nHost']);
});

test('the line under a service says what it shows, or why not', () => {
  const github = services[0];
  const status = (patch: Partial<ServiceStatus>): ServiceStatus => ({ id: 'github', name: 'GitHub', configured: true, hidden: false, health: 'ok', title: '2 reviews requested', message: null, checkedAt: 1, ...patch });
  assert.equal(statusLine(github, undefined), github.detail);
  assert.equal(statusLine(github, status({ configured: false })), github.detail);
  assert.equal(statusLine(github, status({})), '2 reviews requested');
  assert.equal(statusLine(github, status({ health: 'rejected' })), 'Key refused');
  assert.equal(statusLine(github, status({ health: 'limited' })), 'GitHub asked to wait');
  assert.equal(statusLine(github, status({ hidden: true })), 'Hidden from the island');
});

test('the browser preview lists every service as not connected and never pretends to save', () => {
  const listed = preview({ op: 'status' }) as ServiceStatus[];
  assert.equal(listed.length, services.length);
  assert.ok(listed.every(status => !status.configured));
  assert.throws(() => preview({ op: 'saveKey', service: 'github', key: 'x' }));
});

test('Save sends the host with the key, and a new host needs the key again', () => {
  const [github, gitlab] = services;
  // Fixed hosts: just the key.
  assert.deepEqual(saveRequest(github, ' github_pat_x ', '', '', false), { key: 'github_pat_x' });
  assert.equal(saveRequest(github, '', '', '', true), null);
  // A first self-hosted GitLab: the host goes in the same request as the key.
  assert.deepEqual(saveRequest(gitlab, 'glpat-x', 'https://Git.Corp.example/', '', false), { key: 'glpat-x', host: 'git.corp.example' });
  // Connected, with a new host: nothing to save until the key is entered again.
  assert.equal(saveRequest(gitlab, '', 'git.other.example', 'git.corp.example', true), null);
  assert.deepEqual(saveRequest(gitlab, 'glpat-y', 'git.other.example', 'git.corp.example', true), { key: 'glpat-y', host: 'git.other.example' });
  // Not connected: the host alone may be saved.
  assert.deepEqual(saveRequest(gitlab, '', 'git.other.example', '', false), { key: '', host: 'git.other.example' });
  // The same host typed differently is no change.
  assert.deepEqual(saveRequest(gitlab, 'glpat-x', 'GIT.corp.example', 'git.corp.example', true), { key: 'glpat-x' });
  assert.ok(saveRequest(gitlab, 'glpat-x', 'http://git.corp.example', '', false) instanceof Error);
});

test('the browser preview lets Settings stop watching', () => {
  assert.equal(preview({ op: 'unwatch' }), null);
});
