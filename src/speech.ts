export interface SpeechQueueDependencies {
  /** Resolves when this chunk finishes, including after stop interrupts it. */
  play(text: string): Promise<void>;
  /** Stops active audio independently of the play promise. */
  stop(): Promise<void>;
  onActive(active: boolean): void;
  onError?(error: unknown): void;
  maxChunkLength?: number;
}

interface Completion {
  resolve(): void;
  reject(error: unknown): void;
}

/** Serial sentence playback with generation tokens that invalidate late deltas. */
export class SpeechQueue {
  private readonly dependencies: SpeechQueueDependencies;
  private readonly limit: number;
  private generation = 0;
  private buffer = '';
  private pending: string[] = [];
  private closed = false;
  private pumping = false;
  private inFlight = false;
  private stopRequested = false;
  private active = false;
  private stopBarrier: Promise<void> = Promise.resolve();
  private completions = new Map<number, Completion>();

  constructor(dependencies: SpeechQueueDependencies) {
    const limit = dependencies.maxChunkLength ?? 800;
    if (!Number.isInteger(limit) || limit < 2 || limit > 800) {
      throw new RangeError('Speech chunks must contain between 2 and 800 characters.');
    }
    this.dependencies = dependencies;
    this.limit = limit;
  }

  /** Starts a fresh stream and immediately invalidates any older speech. */
  begin(): number {
    this.invalidate();
    this.closed = false;
    return this.generation;
  }

  /** Buffers unfinished tails; only complete sentences enter the audio queue. */
  append(text: string, token: number): void {
    if (token !== this.generation || this.closed || !text) return;
    this.buffer += text;
    const endings = /[.!?]+(?:["'”’)\]]*)(?=\s|$)/g;
    let consumed = 0;
    for (let match = endings.exec(this.buffer); match; match = endings.exec(this.buffer)) {
      const end = match.index + match[0].length;
      // A token ending in "12." may be followed by "5". Wait for another
      // character or finish() before treating a terminal period as a sentence.
      if (end === this.buffer.length && match[0].startsWith('.')) break;
      const prefix = this.buffer.slice(0, match.index);
      if (match[0].startsWith('.') && /\b(?:mr|mrs|ms|dr|prof|sr|jr|vs|etc|e\.g|i\.e)$/i.test(prefix)) continue;
      this.enqueue(this.buffer.slice(consumed, end));
      consumed = end;
    }
    this.buffer = this.buffer.slice(consumed);
    this.kick();
  }

  /** Flushes the final unfinished tail once; late deltas for this token are ignored. */
  finish(token: number): void {
    if (token !== this.generation || this.closed) return;
    this.enqueue(this.buffer);
    this.buffer = '';
    this.closed = true;
    this.kick();
    this.completeIfIdle();
  }

  /** Invalidates pending text synchronously, then waits for active audio to stop. */
  cancel(): Promise<void> {
    const stopped = this.invalidate();
    this.closed = true;
    return stopped;
  }

  /** Convenience for an explicit read-aloud action; interruption resolves normally. */
  speakText(text: string): Promise<void> {
    const token = this.begin();
    const completion = new Promise<void>((resolve, reject) => {
      this.completions.set(token, { resolve, reject });
    });
    this.append(text, token);
    this.finish(token);
    return completion;
  }

  private invalidate(): Promise<void> {
    this.completions.get(this.generation)?.resolve();
    this.completions.delete(this.generation);
    this.generation += 1;
    this.pending = [];
    this.buffer = '';
    this.setActive(false);
    if (this.inFlight && !this.stopRequested) {
      this.stopRequested = true;
      let stopped: Promise<void>;
      try {
        stopped = Promise.resolve(this.dependencies.stop());
      } catch (error) {
        stopped = Promise.reject(error);
      }
      // Keep a handled barrier for future playback even if stop fails. The
      // cancel caller still receives the original rejection for its UI error.
      this.stopBarrier = stopped.catch(error => {
        this.dependencies.onError?.(error);
      });
      return stopped;
    }
    return this.stopBarrier;
  }

  private enqueue(value: string): void {
    let text = value.trim().replace(/\s+/g, ' ');
    while (text.length > this.limit) {
      const space = text.lastIndexOf(' ', this.limit);
      let split = space > this.limit / 2 ? space : this.limit;
      // Never cut an emoji or other supplementary Unicode character in half.
      const before = text.charCodeAt(split - 1);
      const after = text.charCodeAt(split);
      if (before >= 0xD800 && before <= 0xDBFF && after >= 0xDC00 && after <= 0xDFFF) split -= 1;
      this.pending.push(text.slice(0, split).trim());
      text = text.slice(split).trimStart();
    }
    if (text) this.pending.push(text);
  }

  private kick(): void {
    if (this.pumping || this.pending.length === 0) return;
    this.pumping = true;
    void this.pump();
  }

  private async pump(): Promise<void> {
    try {
      while (this.pending.length > 0) {
        const token = this.generation;
        await this.stopBarrier;
        if (token !== this.generation) continue;
        const text = this.pending.shift();
        if (!text) continue;
        this.inFlight = true;
        this.stopRequested = false;
        this.setActive(true);
        try {
          await this.dependencies.play(text);
        } catch (error) {
          if (token === this.generation) {
            this.pending = [];
            this.buffer = '';
            this.closed = true;
            this.completions.get(token)?.reject(error);
            this.completions.delete(token);
            this.dependencies.onError?.(error);
          }
        } finally {
          this.inFlight = false;
        }
      }
    } finally {
      this.pumping = false;
      this.setActive(false);
      this.completeIfIdle();
      // A new session may have queued text while an old play promise settled.
      if (this.pending.length > 0) this.kick();
    }
  }

  private completeIfIdle(): void {
    if (this.closed && !this.inFlight && this.pending.length === 0) {
      this.completions.get(this.generation)?.resolve();
      this.completions.delete(this.generation);
    }
  }

  private setActive(active: boolean): void {
    if (this.active === active) return;
    this.active = active;
    this.dependencies.onActive(active);
  }
}
