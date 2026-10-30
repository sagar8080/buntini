import Toggle from './Toggle';
import type { Diagnostics, Settings } from './types';

// Mirrors src-tauri/src/budget.rs PRESETS (published allowances and prices, October 2026).
const presets: Record<string, { name: string; free: number; cost: number }> = {
  brave: { name: 'Brave Search', free: 1000, cost: 0.005 },
  tavily: { name: 'Tavily', free: 1000, cost: 0.008 },
};

/** This month's lookups and the one choice that matters: stay free, or spend up to a budget.
    Prices and daily pacing come from Buntini's own table. */
export default function SearchBudget({ draft, update, diagnostics }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics }) {
  const preset = presets[draft.searchProvider] ?? presets.brave;
  const used = diagnostics.searchesUsed;
  // The same limit budget.rs search_limits works out, so the meter follows edits at once.
  const limit = draft.freeTierOnly ? preset.free : Math.min(10_000, Math.floor(draft.monthlyBudgetUsd / preset.cost + 1e-9));
  const ratio = limit > 0 ? Math.min(1, used / limit) : 1;
  return <div className="search-budget">
    <div className="usage-meter">
      <div><span>This month</span><strong>{used} of {limit}</strong></div>
      <div className={`usage-track ${ratio >= .8 ? 'is-high' : ''}`} role="meter" aria-valuemin={0} aria-valuemax={limit} aria-valuenow={used} aria-label={`Lookups this month: ${used} of ${limit}`}>
        <i style={{ width: `${ratio * 100}%` }} /><b style={{ left: '80%' }} />
      </div>
    </div>
    <Toggle label="Stay within the free allowance" detail={`About ${preset.free.toLocaleString()} searches a month`} checked={draft.freeTierOnly} onChange={value => update('freeTierOnly', value)} />
    {!draft.freeTierOnly && <label className="inline-field"><span>Monthly budget ($)<small>{preset.name} charges about ${preset.cost} a search</small></span><input aria-label="Monthly search budget in USD" type="number" min="0" max="1000" step="1" value={draft.monthlyBudgetUsd} onChange={event => update('monthlyBudgetUsd', Number(event.target.value))} /></label>}
    <p className="field-help">Every lookup is counted before it’s sent. You’ll hear once at 80%.</p>
  </div>;
}
