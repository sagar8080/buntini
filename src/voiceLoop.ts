/** What the microphone session reports when it opens (see `start_recording`). */
export interface Listening {
  session: number;
  /** Words arrive while the user talks (Apple's live recognition), not only at the end. */
  live: boolean;
}

function words(text: string): string[] {
  return text.toLowerCase().replace(/[’]/g, "'").replace(/[^\p{L}\p{N}' ]+/gu, ' ').split(/\s+/).filter(Boolean);
}

// Said on its own, each of these ends a voice conversation instead of being sent.
const endings = [
  "that's all", "that is all", "that's it", "that is it", "that'll be all", "that will be all",
  'stop', 'stop listening', 'stop talking', 'be quiet', 'quiet', 'enough', 'never mind', 'nevermind', 'cancel',
  'goodbye', 'bye', 'bye bye', 'no thanks', 'no thank you', "i'm done", 'i am done', "we're done", 'all done',
  'nothing else', 'nothing', "that's everything", 'done',
];
const politeness = new Set(['ok', 'okay', 'thanks', 'thank', 'you', 'cheers', 'great', 'cool', 'alright', 'all', 'right', 'no', 'please', 'buntini', 'for', 'now']);

/** True for a short sign-off such as “that's all”, “thanks, that's it” or “stop”.
 * A real request that merely contains these words (“stop the timer”) is not one. */
export function isConversationEnd(text: string): boolean {
  const said = words(text);
  if (said.length === 0 || said.length > 7) return false;
  const phrase = said.join(' ');
  for (const ending of endings) {
    const at = phrase.indexOf(ending);
    if (at < 0) continue;
    const rest = (phrase.slice(0, at) + ' ' + phrase.slice(at + ending.length)).split(' ').filter(Boolean);
    if (rest.every(word => politeness.has(word))) return true;
  }
  // “Thanks” or “thank you” alone, after an answer, is a goodbye too.
  return said.every(word => politeness.has(word)) && said.some(word => word === 'thanks' || word === 'thank' || word === 'cheers');
}

const interruptions = new Set(['stop', 'wait', 'pause', 'quiet', 'enough', 'hold', 'hey', 'sorry']);

/** Whether words heard over Buntini's reply (its own echo already removed) mean the
 * user is talking: two words, or one that clearly interrupts. A lone stray word stays
 * quiet, so a cough or a misheard echo fragment does not cut the reply off. */
export function shouldBargeIn(text: string): boolean {
  const said = words(text);
  return said.length >= 2 || (said.length === 1 && interruptions.has(said[0]));
}

export type EndpointDecision = 'listening' | 'endpoint' | 'timeout' | 'limit';

export interface EndpointOptions {
  /** A pause this long after the user's last new word ends their turn. */
  pauseMs?: number;
  /** Nothing said for this long (after Buntini goes quiet) ends the session. */
  noSpeechMs?: number;
  /** A turn never runs longer than this, counted from when Buntini went quiet. */
  maxTurnMs?: number;
}

/** Decides when the user has finished talking, so nobody has to press stop.
 * With live words, the turn ends when no new word has arrived for `pauseMs`.
 * Without them (the recording fallback), it ends after `pauseMs` of quiet that
 * follows speech louder than the room. While Buntini itself is talking, nothing
 * counts: its voice is neither the user speaking nor silence. */
export class Endpointer {
  private readonly pauseMs: number;
  private readonly noSpeechMs: number;
  private readonly maxTurnMs: number;
  private live = false;
  private open = false;
  private speaking = false;
  private listenFrom = 0;
  private heard = '';
  private lastActivity = 0;
  private started = false;
  private floor = 0.01;
  private loudRun = 0;

  constructor(options: EndpointOptions = {}) {
    this.pauseMs = options.pauseMs ?? 1500;
    this.noSpeechMs = options.noSpeechMs ?? 8000;
    this.maxTurnMs = options.maxTurnMs ?? 30000;
  }

  /** The microphone is open; `live` says whether words will stream. */
  begin(live: boolean, now: number): void {
    this.live = live;
    this.open = true;
    this.listenFrom = now;
  }

  setSpeaking(speaking: boolean, now: number): void {
    if (this.speaking === speaking) return;
    this.speaking = speaking;
    // Waiting for the user starts once Buntini stops talking.
    if (!speaking) { this.listenFrom = Math.max(this.listenFrom, now); if (this.started) this.lastActivity = now; }
  }

  /** Live words so far (the whole turn, not a delta). */
  text(text: string, now: number): void {
    const heard = text.trim();
    if (heard === this.heard) return;
    this.heard = heard;
    if (heard) { this.started = true; this.lastActivity = now; }
  }

  /** Loudness (RMS, 0–1) about every 100 ms. */
  level(rms: number, now: number): void {
    if (this.speaking || !Number.isFinite(rms)) return;
    const threshold = Math.max(0.02, this.floor * 3);
    if (rms >= threshold) {
      this.loudRun += 1;
      // Steady noise slowly becomes the new floor instead of holding the turn open.
      this.floor = this.floor * 0.995 + rms * 0.005;
      if (!this.live && this.loudRun >= 3) { this.started = true; this.lastActivity = now; }
      else if (!this.live && this.started) this.lastActivity = now;
    } else {
      this.loudRun = 0;
      this.floor = this.floor * 0.95 + rms * 0.05;
    }
  }

  get heardSomething(): boolean { return this.started; }

  check(now: number): EndpointDecision {
    if (!this.open || this.speaking) return 'listening';
    if (now - this.listenFrom >= this.maxTurnMs) return this.started ? 'limit' : 'timeout';
    if (!this.started) return now - this.listenFrom >= this.noSpeechMs ? 'timeout' : 'listening';
    return now - this.lastActivity >= this.pauseMs ? 'endpoint' : 'listening';
  }
}
