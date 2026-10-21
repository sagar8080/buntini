import Toggle from './Toggle';
import type { Diagnostics, Settings } from './types';

// Mirrors src-tauri/src/budget.rs PRESETS (published allowances, October 2026).
const presets: Record<string, { free: number }> = { brave: { free: 1000 }, tavily: { free: 1000 } };

function Meter({ label, used, limit }: { label: string; used: number; limit: number }) {
  const ratio = limit > 0 ? Math.min(1, used / limit) : 1;
  return <div className="usage-meter">
    <div><span>{label}</span><strong>{used} of {limit}</strong></div>
    <div className={`usage-track ${ratio >= .8 ? 'is-high' : ''}`} role="meter" aria-valuemin={0} aria-valuemax={limit} aria-valuenow={used} aria-label={`${label}: ${used} of ${limit}`}>
      <i style={{ width: `${ratio * 100}%` }} /><b style={{ left: '80%' }} />
    </div>
  </div>;
}

/** Lookup usage at a glance; the limits fold away. */
export default function SearchBudget({ draft, update, diagnostics }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics }) {
  const preset = presets[draft.searchProvider];
  return <div className="search-budget">
    <Meter label="Lookups today" used={diagnostics.searchesToday ?? 0} limit={diagnostics.searchLimitDay ?? draft.dailySearchLimit} />
    <Meter label="This month" used={diagnostics.searchesUsed} limit={diagnostics.searchLimitMonth ?? draft.monthlySearchLimit} />
    <details className="disclosure"><summary>Limits</summary>
      <Toggle label="Stay within the free allowance" detail={preset ? `About ${preset.free.toLocaleString()} searches a month` : undefined} checked={draft.freeTierOnly} onChange={value => update('freeTierOnly', value)} />
      <label className="inline-field"><span>Daily limit</span><input aria-label="Daily lookup limit" type="number" min="1" max="2000" value={draft.dailySearchLimit} onChange={event => update('dailySearchLimit', Number(event.target.value))} /></label>
      {!draft.freeTierOnly && <>
        <label className="inline-field"><span>Monthly limit</span><input aria-label="Monthly search cap" type="number" min="0" max="10000" value={draft.monthlySearchLimit} onChange={event => update('monthlySearchLimit', Number(event.target.value))} /></label>
        <label className="inline-field"><span>Monthly budget ($)</span><input aria-label="Monthly estimated budget in USD" type="number" min="0" max="1000" step="0.01" value={draft.monthlyBudgetUsd} onChange={event => update('monthlyBudgetUsd', Number(event.target.value))} /></label>
        <label className="inline-field"><span>Cost per search ($)</span><input aria-label="Estimated cost per search in USD" type="number" min="0" max="10" step="0.001" value={draft.costPerSearchUsd} onChange={event => update('costPerSearchUsd', Number(event.target.value))} /></label>
      </>}
      <p className="field-help">Every lookup is counted before it’s sent. You’ll hear once at 80%.</p>
    </details>
  </div>;
}
