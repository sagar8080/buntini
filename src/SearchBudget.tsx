import type { Diagnostics, Settings } from './types';

// Mirrors src-tauri/src/budget.rs PRESETS (published allowances, October 2026).
const presets: Record<string, { free: number; cost: number; note: string }> = {
  brave: { free: 1000, cost: 0.005, note: 'Brave includes $5 of credit a month (about 1,000 searches); a card is required.' },
  tavily: { free: 1000, cost: 0.008, note: 'Tavily includes 1,000 free credits a month; a basic search uses one.' },
};

function Meter({ label, used, limit }: { label: string; used: number; limit: number }) {
  const ratio = limit > 0 ? Math.min(1, used / limit) : 1;
  return <div className="usage-meter">
    <div><span>{label}</span><strong>{used} of {limit}</strong></div>
    <div className={`usage-track ${ratio >= .8 ? 'is-high' : ''}`} role="meter" aria-valuemin={0} aria-valuemax={limit} aria-valuenow={used} aria-label={`${label}: ${used} of ${limit}`}>
      <i style={{ width: `${ratio * 100}%` }} /><b style={{ left: '80%' }} />
    </div>
  </div>;
}

/** Guardrails for online lookups: free-tier mode, a daily cap and live usage. */
export default function SearchBudget({ draft, update, diagnostics }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics }) {
  const preset = presets[draft.searchProvider];
  return <div className="search-budget">
    <Meter label="Today" used={diagnostics.searchesToday ?? 0} limit={diagnostics.searchLimitDay ?? draft.dailySearchLimit} />
    <Meter label="This month" used={diagnostics.searchesUsed} limit={diagnostics.searchLimitMonth ?? draft.monthlySearchLimit} />
    <label className="notification-choice"><input type="checkbox" checked={draft.freeTierOnly} onChange={event => update('freeTierOnly', event.target.checked)} />Stay within the free allowance{preset ? ` (${preset.free.toLocaleString()} a month)` : ''}</label>
    {preset && <div className="field-help">{preset.note} Allowances change, so check your provider’s dashboard.</div>}
    <label className="inline-field"><span>Daily lookup limit<small>Keeps one busy afternoon from using the whole month</small></span><input aria-label="Daily lookup limit" type="number" min="1" max="2000" value={draft.dailySearchLimit} onChange={event => update('dailySearchLimit', Number(event.target.value))} required /></label>
    {!draft.freeTierOnly && <>
      <label className="inline-field"><span>Monthly search cap<small>{diagnostics.searchesUsed} searches used this month</small></span><input aria-label="Monthly search cap" type="number" min="0" max="10000" value={draft.monthlySearchLimit} onChange={event => update('monthlySearchLimit', Number(event.target.value))} required /></label>
      <label className="inline-field"><span>Monthly estimated budget<small>${(diagnostics.estimatedSpendUsd || 0).toFixed(2)} estimated this month</small></span><input aria-label="Monthly estimated budget in USD" type="number" min="0" max="1000" step="0.01" value={draft.monthlyBudgetUsd} onChange={event => update('monthlyBudgetUsd', Number(event.target.value))} required /></label>
      <label className="inline-field"><span>Estimated cost per search<small>{preset ? `${draft.searchProvider === 'brave' ? 'Brave' : 'Tavily'} lists $${preset.cost} after the free allowance` : 'Set this from your provider’s plan'}</small></span><input aria-label="Estimated cost per search in USD" type="number" min="0" max="10" step="0.001" value={draft.costPerSearchUsd} onChange={event => update('costPerSearchUsd', Number(event.target.value))} required /></label>
    </>}
    <div className="field-help">Every lookup is counted before it is sent, and Buntini tells you once when a month reaches 80%. Your provider’s invoice is authoritative.</div>
  </div>;
}
