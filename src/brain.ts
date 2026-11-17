import { t } from './i18n.ts';
import type { CloudProvider, ModelBackend } from './types';

/** Plain-language Apple Intelligence status from the helper's availability report. */
export function appleStatus(reason?: string): { ready: boolean; label: string; advice?: string } {
  switch (reason) {
    case 'available': return { ready: true, label: t('brain.apple.ready') };
    case 'appleIntelligenceNotEnabled': return { ready: false, label: t('brain.apple.off'), advice: t('brain.apple.offAdvice') };
    case 'modelNotReady': return { ready: false, label: t('brain.apple.downloading') };
    case 'modelStuck': return { ready: false, label: t('brain.apple.stuck'), advice: t('brain.apple.stuckAdvice') };
    case 'languageNotSupported': return { ready: false, label: t('brain.apple.language'), advice: t('brain.apple.languageAdvice') };
    case 'deviceNotEligible': return { ready: false, label: t('brain.apple.notSupported'), advice: t('brain.apple.notSupportedAdvice') };
    case 'unsupported': return { ready: false, label: t('brain.apple.needsMacos'), advice: t('brain.apple.needsMacosAdvice') };
    case 'missing': return { ready: false, label: t('brain.apple.missing'), advice: t('brain.apple.missingAdvice') };
    case 'checking': case undefined: return { ready: false, label: t('brain.apple.checking') };
    default: return { ready: false, label: t('brain.apple.unavailable'), advice: t('brain.apple.unavailableAdvice') };
  }
}

/** The System Settings pane that can fix Apple Intelligence here, if any. */
export function appleSettingsPane(reason?: string): 'siri' | 'language' | null {
  if (reason === 'languageNotSupported') return 'language';
  return reason === 'appleIntelligenceNotEnabled' || reason === 'modelStuck' ? 'siri' : null;
}

/** Whether to offer ChatGPT in Apple Intelligence's place: once Buntini knows it can't
    answer here yet. */
export function offersChatGPT(reason?: string): boolean {
  return reason !== undefined && reason !== 'checking' && !appleStatus(reason).ready;
}

/** How a cloud brain is reached: an API key, the ChatGPT sign-in, a server, or a
    command-line tool signed in on this Mac. */
export type ProviderKind = 'key' | 'plan' | 'server' | 'tool';

// Mirrors src-tauri/src/cloud.rs PROVIDERS and cli.rs TOOLS (suggested models and list prices).
export const cloudProviders: { id: CloudProvider; name: string; model: string; input: number; output: number; keys: string; kind: ProviderKind; signIn?: string }[] = [
  { id: 'anthropic', name: 'Anthropic', model: 'claude-opus-5-5', input: 4, output: 20, keys: 'console.anthropic.com', kind: 'key' },
  { id: 'openai', name: 'OpenAI', model: 'gpt-5-mini', input: 0.25, output: 2, keys: 'platform.openai.com', kind: 'key' },
  { id: 'gemini', name: 'Google Gemini', model: 'gemini-2.5-flash', input: 0.3, output: 2.5, keys: 'aistudio.google.com', kind: 'key' },
  { id: 'deepseek', name: 'DeepSeek', model: 'deepseek-flash', input: 0.3, output: 1.2, keys: 'platform.deepseek.com', kind: 'key' },
  { id: 'qwen', name: 'Qwen', model: 'qwen-plus', input: 0.4, output: 1.2, keys: 'modelstudio.console.alibabacloud.com', kind: 'key' },
  { id: 'moonshot', name: 'Kimi', model: 'kimi-k3.1-pro', input: 2, output: 10, keys: 'platform.kimi.ai', kind: 'key' },
  // Your own ChatGPT plan through Sign in with ChatGPT: no key, no per-token price.
  { id: 'chatgpt', name: 'ChatGPT', model: '', input: 0, output: 0, keys: 'chatgpt.com', kind: 'plan' },
  // Command-line tools on the person's own plan, shown only when installed here.
  { id: 'claude-code', name: 'Claude Code', model: '', input: 0, output: 0, keys: '', kind: 'tool', signIn: 'claude' },
  { id: 'codex', name: 'Codex', model: '', input: 0, output: 0, keys: '', kind: 'tool', signIn: 'codex login' },
  { id: 'gemini-cli', name: 'Gemini CLI', model: '', input: 0, output: 0, keys: '', kind: 'tool', signIn: 'gemini' },
  { id: 'copilot', name: 'Copilot', model: '', input: 0, output: 0, keys: '', kind: 'tool', signIn: 'copilot login' },
  // LM Studio's server on this Mac, ready-made.
  { id: 'lmstudio', name: 'LM Studio', model: '', input: 0, output: 0, keys: '', kind: 'server' },
  // The user's own server at an address they give: Ollama and the like.
  { id: 'custom', get name() { return t('brain.provider.yourServer'); }, model: '', input: 0, output: 0, keys: '', kind: 'server' },
];

/** The providers to offer: every one, except command-line tools that aren't installed
    (one already chosen stays, so it can be seen and changed). */
export function offeredProviders(installed: string[] | undefined, chosen: CloudProvider) {
  return cloudProviders.filter(item => item.kind !== 'tool' || item.id === chosen || (installed ?? []).includes(item.id));
}

/** LM Studio's usual address on this Mac, as the literal IPv4 loopback: "localhost" can
    mean ::1 first, where another program could be listening. */
export const lmStudioAddress = '127.0.0.1:1234';

// Read in the language spoken when shown.
export const backendLabel: Readonly<Record<ModelBackend, string>> = { apple: 'Apple Intelligence', get mlx() { return t('brain.backend.mlx'); }, get cloud() { return t('brain.backend.cloud'); } };

/** A readable name for a chosen folder: the environment for a Python path, else the folder. */
export function folderName(path: string): string {
  const parts = path.trim().replace(/\/+$/, '').split('/').filter(Boolean);
  if (!parts.length) return '';
  const python = parts.length >= 3 && parts[parts.length - 2] === 'bin' && parts[parts.length - 1].startsWith('python');
  return python ? parts[parts.length - 3] : parts[parts.length - 1];
}

/** The model worker's status in plain words. */
export function friendlyStatus(status: string, ready: boolean, backend: ModelBackend): string {
  if (backend === 'mlx') {
    if (!ready) return t('brain.status.notSetUp');
    if (status.startsWith('Not started')) return t('brain.status.ready');
    if (status.startsWith('Stopped')) return t('brain.status.paused');
    if (status.startsWith('Ready')) return t('brain.status.loaded');
  }
  return status;
}

export type SetupReport = { dependencies: Record<string, { installed: boolean; version: string | null }>; models: Record<string, { configured: boolean; files_present: boolean; ready_for_load: boolean; error: string | null }> };

/** The local setup check as a few plain lines; optional parts only when they're set up. */
export function setupLines(report: SetupReport): { label: string; ok: boolean; detail: string }[] {
  const mlx = report.dependencies.mlx_lm, model = report.models.qwen;
  const lines = [
    { label: 'MLX', ok: !!mlx?.installed, detail: mlx?.installed ? (mlx.version ? t('brain.setup.installedVersion', { version: mlx.version }) : t('brain.setup.installed')) : t('brain.setup.mlxMissing') },
    { label: t('settings.brain.model'), ok: !!model?.ready_for_load, detail: model?.ready_for_load ? t('brain.setup.filesThere') : model?.configured ? model.error || t('brain.setup.filesMissing') : t('brain.setup.chooseFolder') },
  ];
  const whisper = report.models.whisper;
  if (whisper?.configured) lines.push({ label: t('brain.setup.whisper'), ok: whisper.ready_for_load, detail: whisper.ready_for_load ? t('brain.setup.filesThere') : whisper.error || t('brain.setup.incomplete') });
  return lines;
}
