import type { CloudProvider, ModelBackend } from './types';

/** Plain-language Apple Intelligence status from the helper's availability report. */
export function appleStatus(reason?: string): { ready: boolean; label: string; advice?: string } {
  switch (reason) {
    case 'available': return { ready: true, label: 'Ready on this Mac' };
    case 'appleIntelligenceNotEnabled': return { ready: false, label: 'Turned off', advice: 'Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri.' };
    case 'modelNotReady': return { ready: false, label: 'Downloading', advice: 'macOS is still downloading the model. Try again in a few minutes.' };
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
];

export const backendLabel: Record<ModelBackend, string> = { apple: 'Apple Intelligence', mlx: 'Local MLX model', cloud: 'Cloud model' };
