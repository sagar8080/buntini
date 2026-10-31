import type { CloudProvider, ModelBackend } from './types';

/** Plain-language Apple Intelligence status from the helper's availability report. */
export function appleStatus(reason?: string): { ready: boolean; label: string; advice?: string } {
  switch (reason) {
    case 'available': return { ready: true, label: 'Ready on this Mac' };
    case 'appleIntelligenceNotEnabled': return { ready: false, label: 'Turned off', advice: 'Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri.' };
    case 'modelNotReady': return { ready: false, label: 'Downloading' };
    case 'deviceNotEligible': return { ready: false, label: 'Not supported', advice: 'This Mac doesn’t support Apple Intelligence.' };
    case 'unsupported': return { ready: false, label: 'Needs macOS 26', advice: 'Apple Intelligence needs macOS 26 on an Apple silicon Mac.' };
    case 'missing': return { ready: false, label: 'Missing', advice: 'The Apple Intelligence helper is missing from this installation.' };
    case 'checking': case undefined: return { ready: false, label: 'Checking…' };
    default: return { ready: false, label: 'Unavailable', advice: 'Apple Intelligence isn’t available right now.' };
  }
}

// Mirrors src-tauri/src/cloud.rs PROVIDERS (suggested models and list prices, October 2026).
export const cloudProviders: { id: CloudProvider; name: string; model: string; input: number; output: number; keys: string }[] = [
  { id: 'anthropic', name: 'Anthropic', model: 'claude-opus-5-5', input: 4, output: 20, keys: 'console.anthropic.com' },
  { id: 'openai', name: 'OpenAI', model: 'gpt-5-mini', input: 0.25, output: 2, keys: 'platform.openai.com' },
  { id: 'gemini', name: 'Google Gemini', model: 'gemini-2.5-flash', input: 0.3, output: 2.5, keys: 'aistudio.google.com' },
  // Your own ChatGPT plan through Sign in with ChatGPT: no key, no per-token price.
  { id: 'chatgpt', name: 'ChatGPT', model: '', input: 0, output: 0, keys: 'chatgpt.com' },
];

export const backendLabel: Record<ModelBackend, string> = { apple: 'Apple Intelligence', mlx: 'Local MLX model', cloud: 'Cloud model' };

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
    if (!ready) return 'Not set up';
    if (status.startsWith('Not started')) return 'Ready';
    if (status.startsWith('Stopped')) return 'Paused';
    if (status.startsWith('Ready')) return 'Loaded';
  }
  return status;
}

export type SetupReport = { dependencies: Record<string, { installed: boolean; version: string | null }>; models: Record<string, { configured: boolean; files_present: boolean; ready_for_load: boolean; error: string | null }> };

/** The local setup check as a few plain lines; optional parts only when they're set up. */
export function setupLines(report: SetupReport): { label: string; ok: boolean; detail: string }[] {
  const mlx = report.dependencies.mlx_lm, model = report.models.qwen;
  const lines = [
    { label: 'MLX', ok: !!mlx?.installed, detail: mlx?.installed ? `Installed${mlx.version ? ` (${mlx.version})` : ''}` : 'Not installed in this environment. Run the Install MLX step.' },
    { label: 'Model', ok: !!model?.ready_for_load, detail: model?.ready_for_load ? 'Files are all there' : model?.configured ? model.error || 'Some files are missing. Download it again.' : 'Choose a model folder.' },
  ];
  const whisper = report.models.whisper;
  if (whisper?.configured) lines.push({ label: 'Whisper (voice)', ok: whisper.ready_for_load, detail: whisper.ready_for_load ? 'Files are all there' : whisper.error || 'Incomplete' });
  return lines;
}
