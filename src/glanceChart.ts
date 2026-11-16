import type { GlanceApp, GlanceMetric, GlanceSample, MacBattery, MacGlance, MemoryState } from './types';

export const RING_RADIUS = 32;
export const RING_LENGTH = 2 * Math.PI * RING_RADIUS;
export const HOUR = 3600;
export const DAY = 24 * HOUR;
/** Further apart than this, the Mac or Buntini wasn't running: minutes in an hour's chart,
 * ten-minute steps in a day's. */
export const longestStep = (span: number) => span === DAY ? 1260 : 150;

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

export type Series = 'cpu' | 'memory' | 'battery';
/** `kern.memorystatus_vm_pressure_level`: 1 normal, 2 warning, 4 critical. */
const pressureState = (level: number): MemoryState => level >= 4 ? 'struggling' : level >= 2 ? 'tight' : 'easy';
const valueOf = (sample: GlanceSample, series: Series) =>
  series === 'cpu' ? sample.cpu : series === 'battery' ? sample.battery : memoryFill[pressureState(sample.pressure)] * 100;
/** What a chart reads at one point. Memory is a state, so it reads as one. */
export function seriesValue(series: Series, value: number) {
  if (series !== 'memory') return `${Math.round(value)}%`;
  return value >= memoryFill.struggling * 100 ? memoryWord.struggling : value >= memoryFill.tight * 100 ? memoryWord.tight : memoryWord.easy;
}

export type TrendPoint = { at: number; value: number; x: number; y: number };
/** The samples of the `span` seconds before `now` as chart points, oldest first. */
export function trendPoints(samples: GlanceSample[], now: number, series: Series, width: number, height: number, span = HOUR, pad = 3): TrendPoint[] {
  const points: TrendPoint[] = [];
  for (const sample of samples) {
    const value = valueOf(sample, series);
    if (value === undefined || sample.at <= now - span || sample.at > now) continue;
    const level = Math.min(100, Math.max(0, value));
    points.push({ at: sample.at, value, x: (1 - (now - sample.at) / span) * width, y: height - pad - level / 100 * (height - pad * 2) });
  }
  return points;
}
/** Each unbroken stretch as a line and the area under it. A gap stays a gap, not a line across it. */
export function trendPaths(points: TrendPoint[], height: number, longest = longestStep(HOUR)) {
  const runs: TrendPoint[][] = [];
  points.forEach((point, index) => {
    if (index && point.at - points[index - 1].at <= longest) runs[runs.length - 1].push(point);
    else runs.push([point]);
  });
  const round = (value: number) => Math.round(value * 10) / 10;
  return runs.map(run => {
    // A lone point draws as a dot through its round line cap.
    const drawn = run.length === 1 ? [run[0], run[0]] : run;
    const line = drawn.map((point, index) => `${index ? 'L' : 'M'}${round(point.x)} ${round(point.y)}`).join(' ');
    return { line, area: `${line} L${round(drawn[drawn.length - 1].x)} ${height} L${round(drawn[0].x)} ${height} Z` };
  });
}
export function nearestPoint(points: TrendPoint[], x: number) {
  return points.reduce<TrendPoint | null>((best, point) => !best || Math.abs(point.x - x) < Math.abs(best.x - x) ? point : best, null);
}
export function describeTrend(points: TrendPoint[], name: string, series: Series = 'cpu') {
  const values = points.map(point => point.value);
  if (series === 'memory') return `${name}: at worst ${seriesValue('memory', Math.max(...values))}`;
  const average = values.reduce((sum, value) => sum + value, 0) / values.length;
  return `${name}: average ${Math.round(average)}%, lowest ${Math.round(Math.min(...values))}%, highest ${Math.round(Math.max(...values))}%`;
}

const measure = (app: GlanceApp, metric: GlanceMetric) => metric === 'cpu' ? app.cpuPercent : metric === 'memory' ? app.memoryBytes : app.energy ?? 0;
/** The apps using the most by one measure, biggest first. */
export function rankApps(apps: GlanceApp[], metric: GlanceMetric, most = 3) {
  return apps.filter(app => measure(app, metric) > 0).sort((a, b) => measure(b, metric) - measure(a, metric)).slice(0, most);
}
/** How long an app's bar is, and what it says. CPU and memory are shares of the whole Mac;
 * energy has no ceiling, so it's relative to whatever uses the most. */
export function appBar(app: GlanceApp, metric: GlanceMetric, memoryTotal: number, apps: GlanceApp[]) {
  if (metric === 'memory') return { fraction: memoryTotal ? Math.min(1, app.memoryBytes / memoryTotal) : 0, value: `${gigabytes(app.memoryBytes)} GB` };
  if (metric === 'energy') {
    const energy = (item: GlanceApp) => item.energy ?? 0;
    const top = Math.max(...apps.map(energy), 0);
    return { fraction: top ? energy(app) / top : 0, value: top ? `${Math.round(energy(app))}` : '' };
  }
  return { fraction: Math.min(1, app.cpuPercent / 100), value: `${Math.round(app.cpuPercent)}%` };
}

export type Section = 'overview' | 'storage' | 'memory' | 'battery';
/** Which detail a question opens on. */
export function sectionFor(focus: MacGlance['focus']): Section {
  return focus === 'slow' ? 'memory' : focus;
}
/** What a section's chart and app bars show when it's opened. */
export function sectionView(section: Section, glance: MacGlance): { series: Series; metric: GlanceMetric } {
  if (section === 'memory') return { series: 'memory', metric: 'memory' };
  if (section === 'battery') return { series: 'battery', metric: 'energy' };
  return { series: 'cpu', metric: glance.metric };
}
/** The ring's middle while one part of the disk is pointed at. */
export const partNote: Record<string, string> = { apps: 'GB apps', caches: 'GB caches', folders: 'GB folders', other: 'GB other', free: 'GB free' };
