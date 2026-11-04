import type { GlanceApp, GlanceMetric, GlanceSample, MacBattery, MemoryState } from './types';

export const RING_RADIUS = 32;
export const RING_LENGTH = 2 * Math.PI * RING_RADIUS;
const HOUR = 3600;
/** Points are a minute apart; further apart than this, the Mac or Buntini wasn't running. */
const LONGEST_STEP = 150;

/** Where each part of a ring sits, with a small gap between parts. */
export function ringSegments<T extends { key: string; bytes: number }>(parts: T[], total: number, gap = 2) {
  let start = 0;
  const shown = [];
  for (const part of parts) {
    const whole = total > 0 ? part.bytes / total * RING_LENGTH : 0;
    if (whole > 0) shown.push({ key: part.key, length: Math.max(whole - gap, 1), offset: -start });
    start += whole;
  }
  return shown;
}

export type Tone = 'good' | 'warn' | 'critical';
/** Memory has no meaningful percentage, so the ring shows the state: easy is a third full. */
export const memoryFill: Record<MemoryState, number> = { easy: .3, tight: .65, struggling: .95 };
export const memoryWord: Record<MemoryState, string> = { easy: 'Easy', tight: 'Tight', struggling: 'Struggling' };
export const memoryTone: Record<MemoryState, Tone> = { easy: 'good', tight: 'warn', struggling: 'critical' };
export function batteryTone(battery: MacBattery): Tone {
  if (battery.charging) return 'good';
  return battery.percent < 15 ? 'critical' : battery.percent < 30 ? 'warn' : 'good';
}

/** Decimal, as macOS counts storage. */
export function gigabytes(bytes: number) {
  const value = bytes / 1e9;
  return value >= 10 ? value.toFixed(0) : value.toFixed(1);
}
export function timeLeft(minutes: number) {
  const hours = Math.floor(minutes / 60), rest = minutes % 60;
  if (!hours) return `${rest} min`;
  return rest ? `${hours} h ${rest} min` : `${hours} h`;
}
export function batteryNote(battery: MacBattery) {
  if (battery.charging) return 'charging';
  return battery.minutesLeft ? `${timeLeft(battery.minutesLeft)} left` : '';
}

const valueOf = (sample: GlanceSample, series: 'cpu' | 'battery') => series === 'cpu' ? sample.cpu : sample.battery;
export type TrendPoint = { at: number; value: number; x: number; y: number };
/** The samples of the last hour before `now` as chart points, oldest first. */
export function trendPoints(samples: GlanceSample[], now: number, series: 'cpu' | 'battery', width: number, height: number, pad = 3): TrendPoint[] {
  const points: TrendPoint[] = [];
  for (const sample of samples) {
    const value = valueOf(sample, series);
    if (value === undefined || sample.at <= now - HOUR || sample.at > now) continue;
    const level = Math.min(100, Math.max(0, value));
    points.push({ at: sample.at, value, x: (1 - (now - sample.at) / HOUR) * width, y: height - pad - level / 100 * (height - pad * 2) });
  }
  return points;
}
/** Each unbroken stretch as an SVG path. A gap stays a gap, not a line across it. */
export function trendPaths(points: TrendPoint[]) {
  const runs: TrendPoint[][] = [];
  points.forEach((point, index) => {
    if (index && point.at - points[index - 1].at <= LONGEST_STEP) runs[runs.length - 1].push(point);
    else runs.push([point]);
  });
  const round = (value: number) => Math.round(value * 10) / 10;
  return runs.map(run => {
    // A lone point draws as a dot through its round line cap.
    const drawn = run.length === 1 ? [run[0], run[0]] : run;
    return drawn.map((point, index) => `${index ? 'L' : 'M'}${round(point.x)} ${round(point.y)}`).join(' ');
  });
}
export function nearestPoint(points: TrendPoint[], x: number) {
  return points.reduce<TrendPoint | null>((best, point) => !best || Math.abs(point.x - x) < Math.abs(best.x - x) ? point : best, null);
}
export function describeTrend(points: TrendPoint[], name: string) {
  const values = points.map(point => point.value);
  const average = values.reduce((sum, value) => sum + value, 0) / values.length;
  return `${name}: average ${Math.round(average)}%, lowest ${Math.round(Math.min(...values))}%, highest ${Math.round(Math.max(...values))}%`;
}

/** How long an app's bar is, and what it says. CPU and memory are shares of the whole Mac;
 * energy has no ceiling, so it's relative to whatever uses the most. */
export function appBar(app: GlanceApp, metric: GlanceMetric, memoryTotal: number, apps: GlanceApp[]) {
  if (metric === 'memory') return { fraction: memoryTotal ? Math.min(1, app.memoryBytes / memoryTotal) : 0, value: `${gigabytes(app.memoryBytes)} GB` };
  if (metric === 'energy') {
    const energy = (item: GlanceApp) => item.energy ?? 0;
    const top = Math.max(...apps.map(energy), 0);
    return { fraction: top ? energy(app) / top : 0, value: top ? `${Math.round(energy(app) / apps.reduce((sum, item) => sum + energy(item), 0) * 100)}%` : '' };
  }
  return { fraction: Math.min(1, app.cpuPercent / 100), value: `${Math.round(app.cpuPercent)}%` };
}
