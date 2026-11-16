// Shared shapes for developer services (stream D). No bridge import, so the bridge can use them.

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
  | { op: 'saveKey'; service: ServiceId; key: string }
  | { op: 'show'; service: ServiceId };

export type ServiceInfo = { id: ServiceId; name: string; detail: string; placeholder: string; host?: keyof ServicesSettings; hostPlaceholder?: string };

/** Every service, in the order Settings lists them. */
export const services: ServiceInfo[] = [
  { id: 'github', name: 'GitHub', detail: 'Pull requests · reviews · CI', placeholder: 'github_pat_…' },
  { id: 'gitlab', name: 'GitLab', detail: 'Merge requests · reviews · pipelines', placeholder: 'glpat-…', host: 'gitlabHost', hostPlaceholder: 'gitlab.com' },
  { id: 'gitea', name: 'Gitea', detail: 'Pull requests · reviews · CI', placeholder: 'Access token', host: 'giteaHost', hostPlaceholder: 'git.example.com' },
  { id: 'vercel', name: 'Vercel', detail: 'Deployments', placeholder: 'Access token' },
  { id: 'stripe', name: 'Stripe', detail: 'Payments today · read only', placeholder: 'rk_live_…' },
  { id: 'n8n', name: 'n8n', detail: 'Workflow runs', placeholder: 'API key', host: 'n8nHost', hostPlaceholder: 'n8n.example.com' },
  { id: 'notion', name: 'Notion', detail: 'Pages changed', placeholder: 'ntn_…' },
  { id: 'calcom', name: 'Cal.com', detail: 'Upcoming bookings', placeholder: 'cal_live_…' },
  { id: 'resend', name: 'Resend', detail: 'Sent and bounced mail', placeholder: 're_…' },
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

/** The line under a service's name: what it shows now, or why it doesn't. */
export function statusLine(info: ServiceInfo, status: ServiceStatus | undefined): string {
  if (!status?.configured) return info.detail;
  if (status.hidden) return 'Hidden from the island';
  switch (status.health) {
    case 'ok': return status.title ?? info.detail;
    case 'rejected': return 'Key refused';
    case 'limited': return `${info.name} asked to wait`;
    case 'budget': return 'Today’s allowance used';
    case 'failing': return 'Can’t reach it right now';
    default: return info.detail;
  }
}

/** What the browser preview answers for `services_call`. */
export function preview(op: ServicesOp): unknown {
  if (op.op === 'ping') return true;
  if (op.op === 'status') {
    return services.map((service): ServiceStatus => ({ id: service.id, name: service.name, configured: false, hidden: false, health: 'waiting', title: null, message: null, checkedAt: null }));
  }
  throw new Error('Open the desktop app to use this.');
}
