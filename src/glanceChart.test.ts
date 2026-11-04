// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
import { appBar, batteryNote, batteryTone, describeTrend, gigabytes, nearestPoint, RING_LENGTH, ringSegments, timeLeft, trendPaths, trendPoints } from './glanceChart.ts';
import type { GlanceApp, GlanceSample } from './types';

const NOW = 1_800_000_000;
const sample = (minutesAgo: number, cpu = 50, battery?: number): GlanceSample => ({ at: NOW - minutesAgo * 60, cpu, pressure: 1, battery });

test('ring parts follow each other and stop at what is used', () => {
  const parts = [{ key: 'apps', bytes: 25 }, { key: 'none', bytes: 0 }, { key: 'other', bytes: 25 }];
  const segments = ringSegments(parts, 100);
  assert.deepEqual(segments.map(segment => segment.key), ['apps', 'other'], 'an empty part draws nothing');
  assert.equal(segments[0].offset, -0);
  assert.equal(segments[1].offset, -RING_LENGTH / 4);
  assert.equal(segments[1].length, RING_LENGTH / 4 - 2, 'a gap between parts');
  assert.deepEqual(ringSegments(parts, 0), [], 'no disk reading draws nothing');
});

test('a gap in the history stays a gap', () => {
  const points = trendPoints([sample(30), sample(29), sample(28), sample(10), sample(9)], NOW, 'cpu', 300, 40);
  const paths = trendPaths(points);
  assert.equal(paths.length, 2, 'Buntini was off for 18 minutes');
  assert.equal(paths[0].match(/L/g)?.length, 2);
  assert.equal(paths[1].match(/L/g)?.length, 1);
});

test('a lone point still shows', () => {
  const paths = trendPaths(trendPoints([sample(30), sample(5), sample(4)], NOW, 'cpu', 300, 40));
  assert.equal(paths.length, 2);
  assert.match(paths[0], /^M\S+ \S+ L\S+ \S+$/);
});

test('a chart holds only the last hour, in its own box', () => {
  const points = trendPoints([sample(90), sample(60), sample(30, 0), sample(0, 100), sample(-5)], NOW, 'cpu', 300, 40);
  assert.equal(points.length, 2);
  assert.equal(points[0].x, 150);
  assert.equal(points[1].x, 300);
  assert.equal(points[0].y, 37, 'none at the bottom');
  assert.equal(points[1].y, 3, 'all at the top');
  assert.equal(trendPoints([sample(1, 400)], NOW, 'cpu', 300, 40)[0].y, 3, 'stays inside');
});

test('a battery chart leaves out minutes without a battery reading', () => {
  const points = trendPoints([sample(5, 10), sample(4, 10, 60), sample(3, 10, 59)], NOW, 'battery', 300, 40);
  assert.deepEqual(points.map(point => point.value), [60, 59]);
});

test('hovering picks the closest minute and the chart says itself in words', () => {
  const points = trendPoints([sample(40, 10), sample(20, 30), sample(0, 80)], NOW, 'cpu', 300, 40);
  assert.equal(nearestPoint(points, 195)?.value, 30);
  assert.equal(nearestPoint([], 10), null);
  assert.equal(describeTrend(points, 'Busy'), 'Busy: average 40%, lowest 10%, highest 80%');
});

const app = (fields: Partial<GlanceApp>): GlanceApp => ({ name: 'A', cpuPercent: 0, memoryBytes: 0, quittable: true, ...fields });

test('app bars are shares of the Mac, and energy is relative', () => {
  assert.deepEqual(appBar(app({ cpuPercent: 25.4 }), 'cpu', 0, []), { fraction: .254, value: '25%' });
  assert.equal(appBar(app({ cpuPercent: 400 }), 'cpu', 0, []).fraction, 1);
  assert.deepEqual(appBar(app({ memoryBytes: 4e9 }), 'memory', 16e9, []), { fraction: .25, value: '4.0 GB' });
  const heavy = app({ energy: 75 }), light = app({ energy: 25 });
  assert.deepEqual(appBar(heavy, 'energy', 0, [heavy, light]), { fraction: 1, value: '75%' });
  assert.deepEqual(appBar(light, 'energy', 0, [heavy, light]), { fraction: 1 / 3, value: '25%' });
  assert.deepEqual(appBar(app({}), 'energy', 0, [app({})]), { fraction: 0, value: '' });
});

test('numbers read as people say them', () => {
  assert.equal(gigabytes(175e9), '175');
  assert.equal(gigabytes(4.26e9), '4.3');
  assert.equal(timeLeft(45), '45 min');
  assert.equal(timeLeft(120), '2 h');
  assert.equal(timeLeft(114), '1 h 54 min');
  assert.equal(batteryNote({ percent: 50, charging: true, minutesLeft: 30 }), 'charging');
  assert.equal(batteryNote({ percent: 50, charging: false, minutesLeft: 180 }), '3 h left');
  assert.equal(batteryNote({ percent: 50, charging: false }), '');
  assert.equal(batteryTone({ percent: 9, charging: false }), 'critical');
  assert.equal(batteryTone({ percent: 9, charging: true }), 'good');
  assert.equal(batteryTone({ percent: 20, charging: false }), 'warn');
});
