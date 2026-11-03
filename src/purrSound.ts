/* The purr you hear: low, soft and made on the spot, with no recording. Brown noise
   through a low-pass filter, pulsed 24 times a second like a cat's larynx and swelling
   with each breath, at a loudness that follows the purr level. It sits far below speech:
   at full purr it peaks around -30 dBFS. The page asks the app first whether it may be
   heard at all (pet_sound_allowed). */

/** Gain at a full purr: −30 dBFS for noise normalised to a peak of 1. */
export const PURR_GAIN = .0316;
const PULSE_HZ = 24;
const BREATH_HZ = 1 / 2.4;

/** How loud the purr should be now, or 0 when it mustn't be heard. */
export function purrGain(level: number, allowed: boolean): number {
  return allowed ? Math.min(1, Math.max(0, level)) * PURR_GAIN : 0;
}

type Context = Pick<AudioContext, 'currentTime' | 'destination' | 'sampleRate' | 'state' | 'createBuffer' | 'createBufferSource' | 'createBiquadFilter' | 'createGain' | 'createOscillator' | 'resume' | 'suspend'>;

export class PurrSound {
  private context: Context | null = null;
  private volume: GainNode | null = null;
  private quietSince = 0;
  private unlocked = false;
  private readonly make: () => Context | null;
  constructor(make: () => Context | null) { this.make = make; }

  /** A click or key press in Buntini: browsers start audio only after one. */
  unlock() { this.unlocked = true; }

  /** Follows the purr. Nothing is built until a purr may be heard, and the audio pauses
      after two seconds of silence. */
  set(level: number, allowed: boolean, now: number) {
    const gain = purrGain(level, allowed);
    if (!gain && !this.context) return;
    if (!this.context) {
      if (!this.unlocked) return;
      this.context = this.make();
      if (!this.context) return;
      this.volume = this.build(this.context);
    }
    const context = this.context;
    this.volume?.gain.setTargetAtTime(gain, context.currentTime, .25);
    if (gain) {
      this.quietSince = 0;
      if (context.state === 'suspended') void context.resume().catch(() => {});
    } else {
      if (!this.quietSince) this.quietSince = now;
      if (now - this.quietSince > 2000 && context.state === 'running') void context.suspend().catch(() => {});
    }
  }

  /** Whether it has nothing left to do: never started, paused, or past its two quiet
      seconds (when `set` has paused it). */
  resting(now: number): boolean {
    return !this.context || this.context.state !== 'running' || (!!this.quietSince && now - this.quietSince > 2000);
  }

  private build(context: Context): GainNode {
    // Two seconds of brown noise, looped.
    const buffer = context.createBuffer(1, Math.round(context.sampleRate * 2), context.sampleRate);
    const samples = buffer.getChannelData(0);
    let value = 0, peak = 0;
    for (let index = 0; index < samples.length; index++) {
      value = (value + .02 * (Math.random() * 2 - 1)) / 1.02;
      samples[index] = value;
      peak = Math.max(peak, Math.abs(value));
    }
    for (let index = 0; index < samples.length; index++) samples[index] /= peak || 1;
    const noise = context.createBufferSource();
    noise.buffer = buffer;
    noise.loop = true;
    const low = context.createBiquadFilter();
    low.type = 'lowpass';
    low.frequency.value = 180;
    // Each pulse and each breath swing the loudness around its middle.
    const swing = (hz: number, depth: number) => {
      const gain = context.createGain();
      gain.gain.value = 1 - depth;
      const wave = context.createOscillator();
      wave.frequency.value = hz;
      const amount = context.createGain();
      amount.gain.value = depth;
      wave.connect(amount).connect(gain.gain);
      wave.start();
      return gain;
    };
    const volume = context.createGain();
    volume.gain.value = 0;
    noise.connect(low).connect(swing(PULSE_HZ, .5)).connect(swing(BREATH_HZ, .3)).connect(volume).connect(context.destination);
    noise.start();
    return volume;
  }
}
