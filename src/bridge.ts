import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { Bootstrap, Mode, Settings, TaskEvent, TaskRecord } from './types';

export const desktop = isTauri();
const defaults: Settings = {
  assistantName: 'Buntini', personality: 'Warm, concise, practical, and honest.', avatar: 'cat', accent: 'ember', companionScale: 1,
  catCoat: 'ginger', catEyes: 'green', catAccessory: 'bell', petMotion: 'playful', petRoam: true, petAwareness: true, petNudges: true,
  voiceId: '', voiceRate: 180, notificationsEnabled: false, monthlyBudgetUsd: 5, costPerSearchUsd: .005, vadModelPath: '',
  modelBackend: 'apple', pythonPath: '', modelPath: '', whisperModel: '', searchProvider: 'brave',
  monthlySearchLimit: 1000, dailySearchLimit: 30, freeTierOnly: true, speakResponses: false, theme: 'system', reducedMotion: false, ebayMarketplace: 'EBAY_US', externalShares: [],
};
export async function bootstrap(): Promise<Bootstrap> {
  if (desktop) return invoke('bootstrap');
  let settings = defaults;
  try { settings = { ...defaults, ...JSON.parse(localStorage.getItem('buntini-preview-settings') || '{}') }; } catch { /* Invalid preview settings reset to defaults. */ }
  return {
    settings, tasks: previewTasks(), roots: [], audit: [], savedSearches: [], priceWatches: [],
    diagnostics: { modelReady: false, appleIntelligence: 'unsupported', workerStatus: 'Desktop app required', braveConfigured: false, tavilyConfigured: false, searchesUsed: 0, searchesToday: 0, searchLimitMonth: 1000, searchLimitDay: 30 },
  };
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
