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
  private savedListeners = new Set<(sent: Settings, saved: Settings) => void>();
  /** Bumped by discard(): a save or retry from before it never lands afterwards. */
  private epoch = 0;
  private save: (settings: Settings) => Promise<Settings | void>;
  private delayMs: number;
  private retryMs: number;

  constructor(save: (settings: Settings) => Promise<Settings | void>, { delayMs = 600, retryMs = 1500 } = {}) {
    this.save = save; this.delayMs = delayMs; this.retryMs = retryMs;
  }

  /** Follows the save state; returns a function that stops following. */
  subscribe(listener: (state: SaveState, problem: string) => void) {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  }

  /** Follows each finished save with what was sent and what the backend kept, which can
      differ when macOS refuses a change (Open at login). */
  subscribeSaved(listener: (sent: Settings, saved: Settings) => void) {
    this.savedListeners.add(listener);
    return () => { this.savedListeners.delete(listener); };
  }

  /** Drops every edit not saved yet, and stops any retry: the settings they were based on
      are being erased or replaced (Delete everything, setup). */
  discard() {
    this.epoch += 1;
    this.cancelTimer();
    this.queued = null; this.sending = null; this.running = null;
    this.set('idle');
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
    if (!this.running) {
      const run: Promise<void> = this.loop().finally(() => { if (this.running === run) this.running = null; });
      this.running = run;
    }
    return this.running;
  }

  private async loop() {
    const epoch = this.epoch;
    while (this.queued && epoch === this.epoch) {
      const next = this.queued;
      this.queued = null; this.sending = next;
      if (this.state !== 'waiting') this.set('saving');
      try {
        const saved = await this.save(next);
        if (epoch !== this.epoch) return;
        this.sending = null;
        if (saved) {
          // A field the backend kept differently (macOS refused Open at login) carries into
          // the next edit, so that edit doesn't ask for the refused change again.
          if (this.queued) {
            const queued: Record<string, unknown> = { ...(this.queued as unknown as Record<string, unknown>) };
            const asked = next as unknown as Record<string, unknown>;
            for (const [key, value] of Object.entries(saved as unknown as Record<string, unknown>)) {
              if (typeof value !== 'object' && asked[key] !== value && queued[key] === asked[key]) queued[key] = value;
            }
            this.queued = queued as unknown as Settings;
          }
          for (const listener of this.savedListeners) listener(next, saved);
        }
        if (!this.queued) this.set('saved');
      } catch (error) {
        if (epoch !== this.epoch) return;
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
