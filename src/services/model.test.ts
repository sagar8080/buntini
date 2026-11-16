// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { defaults, normalizeHost, preview, services, statusLine, type ServiceStatus } from './model.ts';

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
