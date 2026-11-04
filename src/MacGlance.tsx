import { useEffect, useState, type PointerEvent, type ReactNode } from 'react';
import { CircleCheck, Thermometer, TriangleAlert } from 'lucide-react';
import { command } from './bridge';
import { appBar, batteryNote, batteryTone, DAY, describeTrend, gigabytes, HOUR, longestStep, memoryFill, memoryTone, memoryWord, nearestPoint, partNote, rankApps, RING_LENGTH, RING_RADIUS, ringSegments, sectionFor, sectionView, seriesValue, trendPaths, trendPoints, type Section, type Series, type Tone, type TrendPoint } from './glanceChart';
import type { GlanceMetric, MacGlance } from './types';

type Props = { glance: MacGlance; latest: boolean; busy: boolean; onQuit: (app: string) => void; onPane: (pane: string) => void };

const LIVE_MS = 5000, LIVE_ENERGY_MS = 10000;

/** The latest card keeps up with the Mac while someone is looking at it. Only then: it stops
 * when Buntini loses focus, and what it reads is shown and never saved. */
function useLiveGlance(saved: MacGlance, latest: boolean, busy: boolean, energy: boolean) {
  const [live, setLive] = useState<MacGlance | null>(null);
  useEffect(() => {
    if (!latest || busy) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const update = async () => {
      if (document.visibilityState === 'visible' && document.hasFocus()) {
        try {
          const next = await command<MacGlance | null>('mac_glance_now', { focus: saved.focus, energy });
          if (!stopped && next) setLive(next);
        } catch { /* The card it already shows still stands. */ }
      }
      if (!stopped) timer = setTimeout(() => void update(), energy ? LIVE_ENERGY_MS : LIVE_MS);
    };
    // Energy takes a moment to read, so asking for it shouldn't wait for the next update too.
    timer = setTimeout(() => void update(), energy ? 0 : LIVE_MS);
    return () => { stopped = true; clearTimeout(timer); };
  }, [latest, busy, saved.focus, energy]);
  return latest && live ? live : saved;
}

/** Mac at a glance: a verdict, then the numbers as graphics, then what to do about it. Tapping
 * and pointing change only this card: nothing is sent and no message is added. */
export default function MacGlanceCard({ glance: saved, latest, busy, onQuit, onPane }: Props) {
  const [section, setSection] = useState<Section>(sectionFor(saved.focus));
  const [series, setSeries] = useState<Series>(saved.focus === 'battery' ? 'battery' : 'cpu');
  const [metric, setMetric] = useState<GlanceMetric>(saved.metric);
  const glance = useLiveGlance(saved, latest, busy, metric === 'energy');
  const canAct = latest && !busy;
  const fine = glance.verdict.kind === 'fine';
  const canEnergy = !!glance.battery && (latest || glance.apps.some(app => app.energy !== undefined));
  const open = (next: Section) => {
    const target = next === section ? 'overview' : next;
    setSection(target);
    const view = sectionView(target, saved);
    setSeries(view.series);
    setMetric(view.metric === 'energy' && !canEnergy ? 'cpu' : view.metric);
  };
  return <div className="mac-glance">
    <div className="mac-verdict">{fine ? <CircleCheck size={17} className="tone-good" aria-hidden /> : <TriangleAlert size={17} className="tone-critical" aria-hidden />}<strong>{glance.verdict.title}</strong></div>
    {section === 'overview' ? <Overview glance={glance} onOpen={open} /> : <SectionChips glance={glance} section={section} onOpen={open} />}
    {section === 'storage' && <StorageHero glance={glance} />}
    {section === 'battery' && glance.battery && <BatteryHero glance={glance} />}
    {section === 'memory' && <MemoryHero glance={glance} />}
    {glance.hot && glance.verdict.kind !== 'hot' && <div className="mac-hot"><Thermometer size={13} aria-hidden /> Running hot</div>}
    {section !== 'storage' && <Trend glance={glance} series={series} onSeries={setSeries} />}
    {section !== 'storage' && <Apps glance={glance} metric={metric} canEnergy={canEnergy} canQuit={canAct} onMetric={setMetric} onQuit={onQuit} />}
    {canAct && glance.verdict.actions.length > 0 && <div className="mac-actions">{glance.verdict.actions.map(action => <button key={action.label} className="secondary-button" onClick={() => action.kind === 'quit' ? onQuit(action.target) : onPane(action.target)}>{action.label}</button>)}</div>}
  </div>;
}

function Ring({ label, children, big, mini }: { label?: string; children: ReactNode; big?: boolean; mini?: boolean }) {
  return <svg className={`mac-ring ${big ? 'is-big' : ''} ${mini ? 'is-mini' : ''}`} viewBox="0 0 80 80" role={label ? 'img' : undefined} aria-label={label} aria-hidden={label ? undefined : true}>
    <circle className="mac-ring-track" cx="40" cy="40" r={RING_RADIUS} />
    {children}
  </svg>;
}
function Arc({ fraction, tone }: { fraction: number; tone: Tone }) {
  const length = Math.max(0, Math.min(1, fraction)) * RING_LENGTH;
  return <circle className={`mac-arc tone-${tone}`} cx="40" cy="40" r={RING_RADIUS} style={{ strokeDasharray: `${length} ${RING_LENGTH}` }} strokeLinecap="round" transform="rotate(-90 40 40)" />;
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
function StorageArcs({ glance, pointed, onPoint }: { glance: MacGlance; pointed?: string; onPoint?: (key: string) => void }) {
  const { totalBytes, parts } = glance.storage;
  return <g transform="rotate(-90 40 40)">{ringSegments(parts, totalBytes).map(segment => <circle key={segment.key} className={`mac-arc part-${segment.key} ${pointed && pointed !== segment.key ? 'is-dim' : ''}`} cx="40" cy="40" r={RING_RADIUS} style={{ strokeDasharray: `${segment.length} ${RING_LENGTH}`, strokeDashoffset: segment.offset }} onPointerEnter={onPoint && (() => onPoint(segment.key))} />)}</g>;
}
function StorageRing({ glance }: { glance: MacGlance }) {
  return <Ring><StorageArcs glance={glance} /><RingText value={gigabytes(glance.storage.freeBytes)} note="GB free" /></Ring>;
}
function memoryArc(glance: MacGlance) {
  const { state } = glance.memory;
  return <Arc fraction={memoryFill[state]} tone={memoryTone[state]} />;
}
function MemoryRing({ glance, big }: { glance: MacGlance; big?: boolean }) {
  const { state } = glance.memory;
  return <Ring big={big} label={big ? `Memory: ${memoryWord[state]}` : undefined}>{memoryArc(glance)}<RingText value={memoryWord[state]} small={state === 'struggling'} /></Ring>;
}
function batteryLabel(glance: MacGlance) {
  const battery = glance.battery!, note = batteryNote(battery);
  return `Battery: ${battery.percent} percent${note ? `, ${note}` : ''}${battery.healthPercent ? `, ${battery.healthPercent} percent health` : ''}`;
}
function batteryArc(glance: MacGlance) {
  return <Arc fraction={glance.battery!.percent / 100} tone={batteryTone(glance.battery!)} />;
}
function BatteryRing({ glance, big }: { glance: MacGlance; big?: boolean }) {
  const battery = glance.battery!, note = batteryNote(battery);
  return <Ring big={big} label={big ? batteryLabel(glance) : undefined}>{batteryArc(glance)}<RingText value={`${battery.percent}%`} note={note.length > 9 ? note.replace(' left', '') : note} /></Ring>;
}

function Overview({ glance, onOpen }: { glance: MacGlance; onOpen: (section: Section) => void }) {
  const { parts } = glance.storage;
  return <>
    <div className={`mac-rings ${glance.battery ? 'has-three' : ''}`}>
      <button type="button" aria-label={storageLabel(glance)} onClick={() => onOpen('storage')}><StorageRing glance={glance} /><span>Storage</span></button>
      <button type="button" aria-label={`Memory: ${memoryWord[glance.memory.state]}`} onClick={() => onOpen('memory')}><MemoryRing glance={glance} /><span>Memory</span></button>
      {glance.battery && <button type="button" aria-label={batteryLabel(glance)} onClick={() => onOpen('battery')}><BatteryRing glance={glance} /><span>Battery</span></button>}
    </div>
    {parts.length > 1 && <div className="mac-legend">{parts.map(part => <span key={part.key}><i className={`part-${part.key}`} />{part.label}</span>)}</div>}
  </>;
}
/** Small rings to move between details. The open one, tapped, goes back to all three. */
function SectionChips({ glance, section, onOpen }: { glance: MacGlance; section: Section; onOpen: (section: Section) => void }) {
  const chip = (id: Section, label: string, arc: ReactNode) => <button type="button" aria-pressed={section === id} onClick={() => onOpen(id)}><Ring mini>{arc}</Ring>{label}</button>;
  return <div className="mac-chips">
    {chip('storage', 'Storage', <StorageArcs glance={glance} />)}
    {chip('memory', 'Memory', memoryArc(glance))}
    {glance.battery && chip('battery', 'Battery', batteryArc(glance))}
  </div>;
}

/** Pointing at a part, on the ring or in the list, says its size in the ring's middle. A tap
 * keeps it there, for when the pointer moves on. */
function StorageHero({ glance }: { glance: MacGlance }) {
  const { freeBytes, parts } = glance.storage;
  const [hovered, setHovered] = useState('');
  const [kept, setKept] = useState('');
  const pointed = hovered || kept;
  const rows = [...[...parts].sort((a, b) => b.bytes - a.bytes), { key: 'free', label: 'Free', bytes: freeBytes }];
  const shown = rows.find(row => row.key === pointed);
  return <div className="mac-hero" onPointerLeave={() => setHovered('')}>
    <Ring big label={storageLabel(glance)}>
      <StorageArcs glance={glance} pointed={pointed} onPoint={setHovered} />
      <RingText value={gigabytes(shown ? shown.bytes : freeBytes)} note={partNote[shown?.key ?? 'free']} />
    </Ring>
    <div className="mac-sizes">{rows.map(row => <button type="button" key={row.key} className={pointed && pointed !== row.key ? 'is-dim' : ''} aria-pressed={kept === row.key} onPointerEnter={() => setHovered(row.key)} onFocus={() => setHovered(row.key)} onBlur={() => setHovered('')} onClick={() => setKept(kept === row.key ? '' : row.key)}><span><i className={`part-${row.key}`} />{row.label}</span><b>{gigabytes(row.bytes)} GB</b></button>)}</div>
  </div>;
}
function Facts({ rows }: { rows: [string, string][] }) {
  return <dl className="mac-sizes">{rows.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>;
}
function BatteryHero({ glance }: { glance: MacGlance }) {
  const battery = glance.battery!;
  const rows: [string, string][] = [];
  if (battery.healthPercent !== undefined) rows.push(['Health', `${battery.healthPercent}%`]);
  if (battery.cycles !== undefined) rows.push(['Cycles', String(battery.cycles)]);
  return <div className="mac-hero"><BatteryRing glance={glance} big /><Facts rows={rows} /></div>;
}
function MemoryHero({ glance }: { glance: MacGlance }) {
  return <div className="mac-hero"><MemoryRing glance={glance} big /><Facts rows={[['Busy now', `${Math.round(glance.cpuPercent)}%`], ['Swap', `${gigabytes(glance.memory.swapBytes)} GB`], ['Memory', `${gigabytes(glance.memory.totalBytes)} GB`]]} /></div>;
}

function Tabs<T extends string>({ label, options, value, onChange }: { label: string; options: [T, string][]; value: T; onChange: (value: T) => void }) {
  return <div className="mac-tabs" role="group" aria-label={label}>{options.map(([id, text]) => <button type="button" key={id} aria-pressed={value === id} onClick={() => onChange(id)}>{text}</button>)}</div>;
}

const CHART_WIDTH = 308, CHART_HEIGHT = 52;
const seriesName: Record<Series, string> = { cpu: 'Busy', memory: 'Memory', battery: 'Battery' };
/** A day's chart starts and ends at the same clock time, so its times name the day too. */
const clock = (at: number, withDay = false) => new Date(at * 1000).toLocaleString(undefined, { ...(withDay ? { weekday: 'short' as const } : {}), hour: 'numeric', minute: '2-digit' });

function Trend({ glance, series, onSeries }: { glance: MacGlance; series: Series; onSeries: (series: Series) => void }) {
  const [hover, setHover] = useState<TrendPoint | null>(null);
  const [wanted, setWanted] = useState<'hour' | 'day'>('hour');
  const now = Math.floor(new Date(glance.capturedAt).getTime() / 1000);
  const day = glance.day ?? [];
  if (glance.history.length < 2 && day.length < 2) return null;
  // A new install has no hour yet, and a card saved before the day chart has no day.
  const range = wanted === 'day' ? (day.length >= 2 ? 'day' : 'hour') : glance.history.length >= 2 ? 'hour' : 'day';
  const span = range === 'day' ? DAY : HOUR;
  const points = trendPoints(range === 'day' ? day : glance.history, now, series, CHART_WIDTH, CHART_HEIGHT, span);
  const current = series === 'cpu' ? glance.cpuPercent : series === 'battery' ? glance.battery?.percent : memoryFill[glance.memory.state] * 100;
  const shown = hover ? hover.value : current;
  const move = (event: PointerEvent<SVGSVGElement>) => {
    const box = event.currentTarget.getBoundingClientRect();
    if (box.width) setHover(nearestPoint(points, (event.clientX - box.left) / box.width * CHART_WIDTH));
  };
  const options: [Series, string][] = [['cpu', seriesName.cpu], ['memory', seriesName.memory]];
  if (glance.battery) options.push(['battery', seriesName.battery]);
  const label = `${seriesName[series]}, last ${range === 'day' ? '24 hours' : 'hour'}`;
  return <div className="mac-trend">
    <div className="mac-section-head">
      <Tabs label="What the chart shows" options={options} value={series} onChange={next => { setHover(null); onSeries(next); }} />
      {day.length >= 2 && glance.history.length >= 2 && <Tabs label="How far back" options={[['hour', '1 h'], ['day', '24 h']]} value={range} onChange={next => { setHover(null); setWanted(next); }} />}
    </div>
    <div className="mac-readout"><span>{hover ? clock(hover.at, range === 'day') : 'Now'}</span><b>{shown === undefined ? '' : seriesValue(series, shown)}</b></div>
    <svg viewBox={`0 0 ${CHART_WIDTH} ${CHART_HEIGHT}`} role="img" aria-label={points.length ? describeTrend(points, label, series) : label} onPointerMove={move} onPointerLeave={() => setHover(null)}>
      <line className="mac-trend-base" x1="0" x2={CHART_WIDTH} y1={CHART_HEIGHT - .5} y2={CHART_HEIGHT - .5} />
      {trendPaths(points, CHART_HEIGHT, longestStep(span)).map((path, index) => <g key={index}><path className="mac-trend-area" d={path.area} /><path className="mac-trend-line" d={path.line} /></g>)}
      {hover && <><line className="mac-trend-cross" x1={hover.x} x2={hover.x} y1="0" y2={CHART_HEIGHT} /><circle className="mac-trend-dot" cx={hover.x} cy={hover.y} r="3.5" /></>}
    </svg>
    <div className="mac-axis"><span>{clock(now - span, range === 'day')}</span><span>{clock(now, range === 'day')}</span></div>
  </div>;
}

function Apps({ glance, metric, canEnergy, canQuit, onMetric, onQuit }: { glance: MacGlance; metric: GlanceMetric; canEnergy: boolean; canQuit: boolean; onMetric: (metric: GlanceMetric) => void; onQuit: (app: string) => void }) {
  const [opened, setOpened] = useState('');
  if (!glance.apps.length) return null;
  const options: [GlanceMetric, string][] = [['cpu', 'CPU'], ['memory', 'Memory']];
  if (canEnergy) options.push(['energy', 'Energy']);
  const ranked = rankApps(glance.apps, metric);
  return <div className="mac-apps">
    <div className="mac-section-head"><span>Using the most</span><Tabs label="Rank apps by" options={options} value={metric} onChange={onMetric} /></div>
    {ranked.map(app => {
      const bar = appBar(app, metric, glance.memory.totalBytes, ranked);
      const open = opened === app.name;
      return <div className="mac-app-group" key={app.name}>
        <button type="button" className="mac-app" aria-expanded={open} aria-label={`${app.name}: ${bar.value}`} onClick={() => setOpened(open ? '' : app.name)}><span title={app.name}>{app.name}</span><div className="mac-app-track"><i style={{ width: `${Math.max(bar.fraction * 100, 2)}%` }} /></div><small>{bar.value}</small></button>
        {open && <div className="mac-app-more">
          <span>CPU <b>{Math.round(app.cpuPercent)}%</b></span>
          <span>Memory <b>{gigabytes(app.memoryBytes)} GB</b></span>
          {app.energy !== undefined && <span>Energy <b>{Math.round(app.energy)}</b></span>}
          {canQuit && app.quittable && <button type="button" className="secondary-button" onClick={() => onQuit(app.name)}>Quit</button>}
        </div>}
      </div>;
    })}
  </div>;
}
