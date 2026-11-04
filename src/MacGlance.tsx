import { useState, type PointerEvent, type ReactNode } from 'react';
import { CircleCheck, Thermometer, TriangleAlert } from 'lucide-react';
import { appBar, batteryNote, batteryTone, describeTrend, gigabytes, memoryFill, memoryTone, memoryWord, nearestPoint, RING_LENGTH, RING_RADIUS, ringSegments, trendPaths, trendPoints, type Tone, type TrendPoint } from './glanceChart';
import type { MacGlance } from './types';

type Props = { glance: MacGlance; actions: boolean; onQuit: (app: string) => void; onPane: (pane: string) => void };

/** Mac at a glance: a verdict, then the numbers as graphics, then what to do about it. */
export default function MacGlanceCard({ glance, actions, onQuit, onPane }: Props) {
  const fine = glance.verdict.kind === 'fine';
  const now = Math.floor(new Date(glance.capturedAt).getTime() / 1000);
  return <div className="mac-glance">
    <div className="mac-verdict">{fine ? <CircleCheck size={17} className="tone-good" aria-hidden /> : <TriangleAlert size={17} className="tone-critical" aria-hidden />}<strong>{glance.verdict.title}</strong></div>
    {glance.focus === 'overview' && <Overview glance={glance} />}
    {glance.focus === 'storage' && <StorageHero glance={glance} />}
    {glance.focus === 'battery' && glance.battery && <BatteryHero glance={glance} />}
    {glance.focus === 'slow' && <MemoryHero glance={glance} />}
    {glance.hot && glance.verdict.kind !== 'hot' && <div className="mac-hot"><Thermometer size={13} aria-hidden /> Running hot</div>}
    {glance.focus === 'battery'
      ? <Trend glance={glance} now={now} series="battery" name="Battery, last hour" current={glance.battery?.percent} />
      : glance.focus !== 'storage' && <Trend glance={glance} now={now} series="cpu" name="Busy, last hour" current={glance.cpuPercent} />}
    {glance.focus !== 'storage' && <Apps glance={glance} />}
    {actions && glance.verdict.actions.length > 0 && <div className="mac-actions">{glance.verdict.actions.map(action => <button key={action.label} className="secondary-button" onClick={() => action.kind === 'quit' ? onQuit(action.target) : onPane(action.target)}>{action.label}</button>)}</div>}
  </div>;
}

function Ring({ label, children, big }: { label: string; children: ReactNode; big?: boolean }) {
  return <svg className={big ? 'mac-ring is-big' : 'mac-ring'} viewBox="0 0 80 80" role="img" aria-label={label}>
    <circle className="mac-ring-track" cx="40" cy="40" r={RING_RADIUS} />
    {children}
  </svg>;
}
function Arc({ fraction, tone }: { fraction: number; tone: Tone }) {
  const length = Math.max(0, Math.min(1, fraction)) * RING_LENGTH;
  return <circle className={`mac-arc tone-${tone}`} cx="40" cy="40" r={RING_RADIUS} strokeDasharray={`${length} ${RING_LENGTH}`} strokeLinecap="round" transform="rotate(-90 40 40)" />;
}
function RingText({ value, note, small }: { value: string; note?: string; small?: boolean }) {
  return <>
    <text className={small ? 'mac-ring-value is-small' : 'mac-ring-value'} x="40" y={note ? 39 : 45} textAnchor="middle">{value}</text>
    {note && <text className="mac-ring-note" x="40" y="51" textAnchor="middle">{note}</text>}
  </>;
}

function storageLabel(glance: MacGlance) {
  const { freeBytes, totalBytes, parts } = glance.storage;
  return `Storage: ${gigabytes(freeBytes)} GB free of ${gigabytes(totalBytes)} GB. ${parts.map(part => `${part.label} ${gigabytes(part.bytes)} GB`).join(', ')}`;
}
function StorageRing({ glance, big }: { glance: MacGlance; big?: boolean }) {
  const { freeBytes, totalBytes, parts } = glance.storage;
  return <Ring big={big} label={storageLabel(glance)}>
    <g transform="rotate(-90 40 40)">{ringSegments(parts, totalBytes).map(segment => <circle key={segment.key} className={`mac-arc part-${segment.key}`} cx="40" cy="40" r={RING_RADIUS} strokeDasharray={`${segment.length} ${RING_LENGTH}`} strokeDashoffset={segment.offset} />)}</g>
    <RingText value={gigabytes(freeBytes)} note="GB free" />
  </Ring>;
}
function MemoryRing({ glance, big }: { glance: MacGlance; big?: boolean }) {
  const { state } = glance.memory;
  return <Ring big={big} label={`Memory: ${memoryWord[state]}`}><Arc fraction={memoryFill[state]} tone={memoryTone[state]} /><RingText value={memoryWord[state]} small={state === 'struggling'} /></Ring>;
}
function BatteryRing({ glance, big }: { glance: MacGlance; big?: boolean }) {
  const battery = glance.battery!;
  const note = batteryNote(battery);
  return <Ring big={big} label={`Battery: ${battery.percent} percent${note ? `, ${note}` : ''}${battery.healthPercent ? `, ${battery.healthPercent} percent health` : ''}`}><Arc fraction={battery.percent / 100} tone={batteryTone(battery)} /><RingText value={`${battery.percent}%`} note={note.length > 9 ? note.replace(' left', '') : note} /></Ring>;
}

function Overview({ glance }: { glance: MacGlance }) {
  const { parts } = glance.storage;
  return <>
    <div className={`mac-rings ${glance.battery ? 'has-three' : ''}`}>
      <figure><StorageRing glance={glance} /><figcaption>Storage</figcaption></figure>
      <figure><MemoryRing glance={glance} /><figcaption>Memory</figcaption></figure>
      {glance.battery && <figure><BatteryRing glance={glance} /><figcaption>Battery</figcaption></figure>}
    </div>
    {parts.length > 1 && <div className="mac-legend">{parts.map(part => <span key={part.key}><i className={`part-${part.key}`} />{part.label}</span>)}</div>}
  </>;
}
function StorageHero({ glance }: { glance: MacGlance }) {
  const { freeBytes, parts } = glance.storage;
  return <div className="mac-hero"><StorageRing glance={glance} big /><dl className="mac-sizes">
    {[...parts].sort((a, b) => b.bytes - a.bytes).map(part => <div key={part.key}><dt><i className={`part-${part.key}`} />{part.label}</dt><dd>{gigabytes(part.bytes)} GB</dd></div>)}
    <div><dt><i className="part-free" />Free</dt><dd>{gigabytes(freeBytes)} GB</dd></div>
  </dl></div>;
}
function BatteryHero({ glance }: { glance: MacGlance }) {
  const battery = glance.battery!;
  return <div className="mac-hero"><BatteryRing glance={glance} big /><dl className="mac-sizes">
    {battery.healthPercent !== undefined && <div><dt>Health</dt><dd>{battery.healthPercent}%</dd></div>}
    {battery.cycles !== undefined && <div><dt>Cycles</dt><dd>{battery.cycles}</dd></div>}
  </dl></div>;
}
function MemoryHero({ glance }: { glance: MacGlance }) {
  return <div className="mac-hero"><MemoryRing glance={glance} big /><dl className="mac-sizes">
    <div><dt>Busy now</dt><dd>{Math.round(glance.cpuPercent)}%</dd></div>
    <div><dt>Swap</dt><dd>{gigabytes(glance.memory.swapBytes)} GB</dd></div>
    <div><dt>Memory</dt><dd>{gigabytes(glance.memory.totalBytes)} GB</dd></div>
  </dl></div>;
}

const CHART_WIDTH = 308, CHART_HEIGHT = 44;
function Trend({ glance, now, series, name, current }: { glance: MacGlance; now: number; series: 'cpu' | 'battery'; name: string; current?: number }) {
  const [hover, setHover] = useState<TrendPoint | null>(null);
  const points = trendPoints(glance.history, now, series, CHART_WIDTH, CHART_HEIGHT);
  if (points.length < 2) return null;
  const move = (event: PointerEvent<SVGSVGElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    if (box.width) setHover(nearestPoint(points, (event.clientX - box.left) / box.width * CHART_WIDTH));
  };
  const shown = hover ? hover.value : current;
  return <div className="mac-trend">
    <div className="mac-section-head"><span>{name}</span><span>{hover && <small>{new Date(hover.at * 1000).toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })} · </small>}{shown === undefined ? '' : `${Math.round(shown)}%`}</span></div>
    <svg viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`} preserveAspectRatio="none" role="img" aria-label={describeTrend(points, name)} onPointerMove={move} onPointerLeave={() => setHover(null)}>
      <line className="mac-trend-base" x1="0" x2={CHART_WIDTH} y1={CHART_HEIGHT - 1} y2={CHART_HEIGHT - 1} vectorEffect="non-scaling-stroke" />
      {trendPaths(points).map((path, index) => <path key={index} className="mac-trend-line" d={path} vectorEffect="non-scaling-stroke" />)}
      {hover && <line className="mac-trend-cross" x1={hover.x} x2={hover.x} y1="0" y2={CHART_HEIGHT} vectorEffect="non-scaling-stroke" />}
    </svg>
  </div>;
}

function Apps({ glance }: { glance: MacGlance }) {
  if (!glance.apps.length) return null;
  return <div className="mac-apps"><div className="mac-section-head"><span>Using the most</span></div>{glance.apps.map(app => {
    const bar = appBar(app, glance.metric, glance.memory.totalBytes, glance.apps);
    return <div className="mac-app" key={app.name} role="img" aria-label={`${app.name}: ${bar.value}`}><span title={app.name}>{app.name}</span><div className="mac-app-track"><i style={{ width: `${Math.max(bar.fraction * 100, 2)}%` }} /></div><small>{bar.value}</small></div>;
  })}</div>;
}
