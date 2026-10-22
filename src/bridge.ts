import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { Bootstrap, Mode, Settings, TaskEvent, TaskRecord } from './types';

export const desktop = isTauri();
const defaults: Settings = {
  assistantName: 'Buntini', userName: '', personality: 'Warm, concise, practical, and honest.', avatar: 'cat', accent: 'preset', accentColor: '', themePreset: 'classic', textSize: 'medium', density: 'comfortable', chatFont: 'sans', corners: 'round', panelSize: 'standard', statusText: '', companionScale: 1,
  catCoat: 'ginger', catEyes: 'green', catAccessory: 'bell', petMotion: 'playful', petRoam: true, petAwareness: true, petNudges: true,
  voiceId: '', voiceRate: 180, notificationsEnabled: false, morningBrief: false, monthlyBudgetUsd: 5, costPerSearchUsd: .005, vadModelPath: '',
  modelBackend: 'apple', pythonPath: '', modelPath: '', whisperModel: '', searchProvider: 'brave',
  monthlySearchLimit: 1000, dailySearchLimit: 30, freeTierOnly: true, speakResponses: false, speakVoiceReplies: true, theme: 'system', reducedMotion: false, ebayMarketplace: 'EBAY_US', externalShares: [], apps: [],
  cloudProvider: 'anthropic', cloudModel: '', cloudBudgetUsd: 5, cloudInputPrice: 4, cloudOutputPrice: 20,
};
export async function bootstrap(): Promise<Bootstrap> {
  if (desktop) return invoke('bootstrap');
  let settings = defaults;
  try { settings = { ...defaults, ...JSON.parse(localStorage.getItem('buntini-preview-settings') || '{}') }; } catch { /* Invalid preview settings reset to defaults. */ }
  return {
    settings, tasks: previewTasks(), onboarded: previewOnboarded(), roots: [], audit: [], savedSearches: [], priceWatches: [],
    diagnostics: { modelReady: false, appleIntelligence: 'unsupported', workerStatus: 'Desktop app required', braveConfigured: false, tavilyConfigured: false, searchesUsed: 0, searchesToday: 0, searchLimitMonth: 1000, searchLimitDay: 30 },
  };
}
function previewOnboarded() {
  try { return localStorage.getItem('buntini-preview-onboarded') === '1' || previewTasks().length > 0; } catch { return true; }
}
// Preview-only stand-in for an 8 GB M2, so the model helper can be seen without the app.
function previewProfile(model?: string | null) {
  const qwen = { repo: 'Qwen3.5-4B-MLX-4bit', label: 'Qwen3.5 4B', downloadGb: 3.1, note: 'Good everyday answers with room for your apps.' };
  const gemma = { repo: 'gemma-4-e2b-it-4bit', label: 'Gemma 4 E2B', downloadGb: 3.6, note: 'Google’s small on-device model; quick on any Mac.' };
  const selected = model === gemma.repo ? gemma : qwen;
  return {
    chip: 'Apple M2', memoryGb: 8, appleSilicon: true, recommended: qwen, alternative: gemma, selected, python: null, modelFolder: null,
    steps: [
      { title: 'Create a Python environment', command: 'python3 -m venv ~/buntini-mlx', done: true },
      { title: 'Install MLX', command: '~/buntini-mlx/bin/pip install "mlx-lm>=0.30,<0.33"', done: false },
      { title: `Download ${selected.label} (${selected.downloadGb.toFixed(1)} GB)`, command: `~/buntini-mlx/bin/hf download mlx-community/${selected.repo} --local-dir ~/buntini-models/${selected.repo}`, done: false },
    ],
  };
}
// Preview-only stand-in for the apps on a Mac.
function previewApps() {
  const can: Record<string, string[]> = { mail: ['Read and search your recent email', 'Draft replies for you to send'], calendar: ['See what’s on any day', 'Add events you approve'], reminders: ['See your open reminders', 'Add reminders you approve'], notes: ['Find your notes', 'Write notes you approve'], music: ['See what’s playing', 'Play, pause and skip', 'Play songs and playlists from your library'], spotify: ['See what’s playing', 'Play, pause and skip', 'Open a search in Spotify'], shortcuts: ['Run your shortcuts after you approve'], safari: ['List and find your open tabs', 'Switch to a tab', 'Read the page you’re on'], chrome: ['List and find your open tabs', 'Switch to a tab'], brave: ['List and find your open tabs', 'Switch to a tab'], edge: ['List and find your open tabs', 'Switch to a tab'], arc: ['List and find your open tabs', 'Switch to a tab'] };
  const names: Record<string, string> = { mail: 'Mail', calendar: 'Calendar', reminders: 'Reminders', notes: 'Notes', music: 'Music', spotify: 'Spotify', shortcuts: 'Shortcuts', safari: 'Safari', chrome: 'Google Chrome', brave: 'Brave Browser', edge: 'Microsoft Edge', arc: 'Arc' };
  return { apps: Object.keys(names).map(id => ({ id, name: names[id], can: can[id], note: id === 'safari' ? 'Safari doesn’t say which windows are private, so Buntini can see those too. Close private windows before asking.' : null, installed: !['spotify', 'edge', 'arc'].includes(id), allowed: false })), others: ['Figma', 'Slack', 'Visual Studio Code', 'Xcode'] };
}
// Preview-only sample thread, seeded by hand through localStorage for UI checks.
function previewTasks(): TaskRecord[] {
  try { return JSON.parse(localStorage.getItem('buntini-preview-tasks') || '[]'); } catch { return []; }
}
export async function saveSettings(settings: Settings): Promise<Settings> {
  if (desktop) return invoke('save_settings', { settings });
  localStorage.setItem('buntini-preview-settings', JSON.stringify(settings));
  return settings;
}
export async function command<T = void>(name: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) {
    // The browser preview keeps the conversation controls usable without the desktop app.
    if (name === 'new_topic') return new Date().toISOString() as T;
    if (name === 'conversation') return [] as T;
    if (name === 'finish_onboarding') { localStorage.setItem('buntini-preview-onboarded', '1'); return undefined as T; }
    if (name === 'restart_onboarding') { localStorage.removeItem('buntini-preview-onboarded'); return undefined as T; }
    // Keys can't be kept without the desktop app's Keychain; setup still finishes.
    if (name === 'set_api_key') return undefined as T;
    if (name === 'mac_profile') return previewProfile(args?.model as string | null) as T;
    if (name === 'app_list') return previewApps() as T;
    // Voice controls can be looked at without a microphone; nothing is recorded.
    if (name === 'start_recording' || name === 'cancel_recording') return undefined as T;
    if (name === 'stop_recording') return 'What’s on my calendar today?' as T;
    if (name === 'check_app') return 'macOS will ask you once, the first time Buntini uses it.' as T;
    throw new Error('Open the desktop app to use local tools. Run npm run desktop in the project.');
  }
  return invoke<T>(name, args);
}
export async function runTask(prompt: string, mode: Mode, onEvent: (event: TaskEvent) => void, approvalId?: string): Promise<TaskRecord> {
  if (!desktop) throw new Error('This is the browser preview. Open Buntini with npm run desktop to run local tools and chat with Qwen.');
  const channel = new Channel<TaskEvent>();
  channel.onmessage = onEvent;
  return invoke('run_task', { prompt, mode, onEvent: channel, approvalId: approvalId || null });
}
export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : typeof error === 'string' ? error : 'Something went wrong. Please try again.';
}
