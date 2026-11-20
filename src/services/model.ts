// Shared shapes for developer services (stream D). No bridge import, so the bridge can use them.
import { t } from '../i18n.ts';

/** Saved under `settings.services`; mirrors `services::Settings` in Rust. Keys live in the
    Keychain, never here. */
export type ServicesSettings = { gitlabHost: string; giteaHost: string; n8nHost: string };
export const defaults: ServicesSettings = { gitlabHost: '', giteaHost: '', n8nHost: '' };

export type ServiceId = 'github' | 'gitlab' | 'gitea' | 'vercel' | 'stripe' | 'n8n' | 'notion' | 'calcom' | 'resend';
export type Health = 'waiting' | 'ok' | 'rejected' | 'limited' | 'budget' | 'failing';

/** One service as `services_call` reports it: whether a key is saved, never the key. */
export type ServiceStatus = {
  id: ServiceId;
  name: string;
  configured: boolean;
  hidden: boolean;
  health: Health;
  title: string | null;
  message: string | null;
  checkedAt: number | null;
};

/** What `services_call` accepts; mirrors `services::Op`. */
export type ServicesOp =
  | { op: 'ping' }
  | { op: 'status' }
  | { op: 'unwatch' }
  | { op: 'saveKey'; service: ServiceId; key: string; host?: string }
  | { op: 'show'; service: ServiceId };

export type ServiceInfo = { id: ServiceId; name: string; readonly detail: string; readonly placeholder: string; host?: keyof ServicesSettings; hostPlaceholder?: string };

/** Every service, in the order Settings lists them. Details and worded placeholders are read in
    the language spoken when shown. */
export const services: ServiceInfo[] = [
  { id: 'github', name: 'GitHub', get detail() { return t('settings.services.detail.pullRequests'); }, placeholder: 'github_pat_…' },
  { id: 'gitlab', name: 'GitLab', get detail() { return t('settings.services.detail.mergeRequests'); }, placeholder: 'glpat-…', host: 'gitlabHost', hostPlaceholder: 'gitlab.com' },
  { id: 'gitea', name: 'Gitea', get detail() { return t('settings.services.detail.pullRequests'); }, get placeholder() { return t('settings.services.accessToken'); }, host: 'giteaHost', hostPlaceholder: 'git.example.com' },
  { id: 'vercel', name: 'Vercel', get detail() { return t('settings.services.detail.deployments'); }, get placeholder() { return t('settings.services.accessToken'); } },
  { id: 'stripe', name: 'Stripe', get detail() { return t('settings.services.detail.payments'); }, placeholder: 'rk_live_…' },
  { id: 'n8n', name: 'n8n', get detail() { return t('settings.services.detail.workflowRuns'); }, get placeholder() { return t('settings.apiKey'); }, host: 'n8nHost', hostPlaceholder: 'n8n.example.com' },
  { id: 'notion', name: 'Notion', get detail() { return t('settings.services.detail.pagesChanged'); }, placeholder: 'ntn_…' },
  { id: 'calcom', name: 'Cal.com', get detail() { return t('settings.services.detail.bookings'); }, placeholder: 'cal_live_…' },
  { id: 'resend', name: 'Resend', get detail() { return t('settings.services.detail.mail'); }, placeholder: 're_…' },
];

/** `https://Git.Example.com:8443/path` → `git.example.com:8443`. Null for anything that isn't an
    https address; '' for an empty field. */
export function normalizeHost(input: string): string | null {
  const text = input.trim();
  if (!text) return '';
  if (/^http:\/\//i.test(text)) return null;
  try {
    const url = new URL(/^https:\/\//i.test(text) ? text : `https://${text}`);
    if (url.username || url.password || !url.hostname || /\s/.test(text)) return null;
    return url.host.toLowerCase();
  } catch {
    return null;
  }
}

/** What Save sends for a service: the typed key, and the host when it changed. A key is bound to
    its host, so a connected service needs its key again for a new host. Null while there is
    nothing to save; an Error for an address that isn't https. */
export function saveRequest(info: ServiceInfo, key: string, typedHost: string, savedHost: string, connected: boolean): { key: string; host?: string } | Error | null {
  const trimmed = key.trim();
  const hostChanged = !!info.host && typedHost.trim() !== savedHost;
  if (!hostChanged) return trimmed ? { key: trimmed } : null;
  const host = normalizeHost(typedHost);
  if (host === null) return new Error(t('settings.services.httpsAddress'));
  if (host === savedHost) return trimmed ? { key: trimmed } : null;
  if (connected && !trimmed) return null;
  return { key: trimmed, host };
}

/** The line under a service's name: what it shows now, or why it doesn't. */
export function statusLine(info: ServiceInfo, status: ServiceStatus | undefined): string {
  if (!status?.configured) return info.detail;
  if (status.hidden) return t('settings.services.hidden');
  switch (status.health) {
    case 'ok': return status.title ?? info.detail;
    case 'rejected': return t('settings.services.keyRefused');
    case 'limited': return t('settings.services.limited', { service: info.name });
    case 'budget': return t('settings.services.budget');
    case 'failing': return t('settings.services.failing');
    default: return info.detail;
  }
}

/** What the browser preview answers for `services_call`. */
export function preview(op: ServicesOp): unknown {
  if (op.op === 'ping') return true;
  if (op.op === 'unwatch') return null;
  if (op.op === 'status') {
    return services.map((service): ServiceStatus => ({ id: service.id, name: service.name, configured: false, hidden: false, health: 'waiting', title: null, message: null, checkedAt: null }));
  }
  throw new Error(t('common.desktopOnly'));
}
