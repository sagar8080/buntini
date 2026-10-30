// Settings save on their own. The saver lives as long as the app, not the Settings page, so an
// edit made just before Esc still saves. It sends one save at a time, always the newest edit,
// and waits out a running task (which keeps the policy it started with) instead of dropping it.
import type { Settings } from './types';

export type SaveState = 'idle' | 'saving' | 'waiting' | 'saved' | 'error';

/** The backend refuses model and policy changes while a task runs (controller.rs save_settings).
    That is worth waiting for, not reporting as a failure. */
export const isBusyRefusal = (message: string) => message.startsWith('Finish or cancel the current task');

const messageOf = (error: unknown) => error instanceof Error ? error.message : typeof error === 'string' ? error : 'Something went wrong. Please try again.';

export class SettingsSaver {
  state: SaveState = 'idle';
  problem = '';
  private queued: Settings | null = null;
  private sending: Settings | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running: Promise<void> | null = null;
  private listeners = new Set<(state: SaveState, problem: string) => void>();
  private save: (settings: Settings) => Promise<unknown>;
  private delayMs: number;
  private retryMs: number;

  constructor(save: (settings: Settings) => Promise<unknown>, { delayMs = 600, retryMs = 1500 } = {}) {
    this.save = save; this.delayMs = delayMs; this.retryMs = retryMs;
  }

  /** Follows the save state; returns a function that stops following. */
  subscribe(listener: (state: SaveState, problem: string) => void) {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  /** The newest edit that is not saved yet, so reopening Settings shows it rather than an older copy. */
  pending(): Settings | null { return this.queued ?? this.sending; }

  /** Saves a moment after the user stops editing. */
  edit(settings: Settings) {
    this.queued = settings;
    this.cancelTimer();
    this.set(this.state === 'waiting' ? 'waiting' : 'saving');
    this.timer = setTimeout(() => { this.timer = null; void this.drain(); }, this.delayMs);
  }

  /** Saves any edit still waiting for the pause, right away: Settings is closing. */
  flush(): Promise<void> {
    if (!this.timer) return this.running ?? Promise.resolve();
    this.cancelTimer();
    return this.drain();
  }

  /** Saves now and resolves once it is saved, for one-tap choices such as Allow Mail. */
  async saveNow(settings: Settings): Promise<void> {
    this.queued = settings;
    this.cancelTimer();
    await this.drain();
    if (this.state === 'error') throw new Error(this.problem);
  }

  private cancelTimer() { if (this.timer) { clearTimeout(this.timer); this.timer = null; } }

  private set(state: SaveState, problem = '') {
    this.state = state; this.problem = problem;
    for (const listener of this.listeners) listener(state, problem);
  }

  private drain(): Promise<void> {
    this.running ??= this.loop().finally(() => { this.running = null; });
    return this.running;
  }

  private async loop() {
    while (this.queued) {
      const next = this.queued;
      this.queued = null; this.sending = next;
      if (this.state !== 'waiting') this.set('saving');
      try {
        await this.save(next);
        this.sending = null;
        if (!this.queued) this.set('saved');
      } catch (error) {
        const message = messageOf(error);
        this.sending = null;
        if (!isBusyRefusal(message)) { if (!this.queued) this.set('error', message); continue; }
        // A newer edit replaces this one; otherwise try the same edit again shortly.
        this.queued ??= next;
        this.set('waiting');
        await new Promise(resolve => setTimeout(resolve, this.retryMs));
      }
    }
  }
}
