import { useEffect, useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import { listen } from '@tauri-apps/api/event';
import { command, desktop, errorMessage } from './bridge';

export interface LocalPlan { repo: string; label: string; downloadBytes: number; problem?: string | null }
export interface LocalStatus { phase: 'idle' | 'downloading' | 'waiting' | 'installing' | 'testing' | 'ready' | 'failed'; doneBytes: number; totalBytes: number; message: string; python?: string | null; model?: string | null }

const gigabytes = (bytes: number) => `${(bytes / 1e9).toFixed(1)} GB`;
const percent = (status: LocalStatus) => status.totalBytes > 0 ? Math.min(100, Math.floor(status.doneBytes * 100 / status.totalBytes)) : 0;
const doing: Partial<Record<LocalStatus['phase'], string>> = { installing: 'Installing', testing: 'Trying it out' };

type Props = {
  onStart?: () => void; onStop?: () => void; onReady: (python: string, model: string) => void; onRemoved?: () => void; onError: (message: string) => void;
  /** Settings can free the disk; first-run setup has nothing to remove. */
  removable?: boolean;
};

/** The local model in one press: its size, real progress, and what to do if it stops. */
export default function LocalSetup({ onStart, onStop, onReady, onRemoved, onError, removable }: Props) {
  const [plan, setPlan] = useState<LocalPlan | null>(null);
  const [status, setStatus] = useState<LocalStatus | null>(null);
  useEffect(() => {
    let current = true;
    void command<LocalPlan>('local_setup_plan').then(found => { if (current) setPlan(found); }).catch(error => onError(errorMessage(error)));
    void command<LocalStatus>('local_setup_status').then(found => { if (current) setStatus(found); }).catch(() => {});
    const events = desktop ? listen<LocalStatus>('local-setup', event => setStatus(event.payload)) : null;
    return () => { current = false; void events?.then(unlisten => unlisten()); };
  }, []);
  const phase = status?.phase;
  const python = status?.python, model = status?.model;
  useEffect(() => { if (phase === 'ready' && python && model) onReady(python, model); }, [phase, python, model]);
  useEffect(() => { if (phase === 'downloading' || phase === 'waiting' || phase === 'installing' || phase === 'testing') onStart?.(); if (phase === 'failed') onStop?.(); }, [phase]);
  async function act(name: string) {
    try { await command(name); setStatus(await command<LocalStatus>('local_setup_status')); }
    catch (error) { onError(errorMessage(error)); }
  }
  async function cancel() { await act('local_setup_cancel'); onStop?.(); }
  async function remove() { await act('local_setup_remove'); onRemoved?.(); }
  if (!plan || !status) return <div className="local-setup"><LoaderCircle size={14} className="spin" /></div>;
  if (phase === 'ready') return <div className="local-setup-ready"><Check size={14} /><span>{plan.label} is ready on this Mac</span>{removable && <button type="button" className="text-button danger" onClick={() => void remove()}>Remove</button>}</div>;
  if (phase === 'failed') return <div className="local-setup">
    <p className="field-help" role="alert">{status.message}</p>
    <div className="row-actions"><button type="button" className="secondary-button" onClick={() => void act('local_setup_start')}>Try again</button><button type="button" className="text-button" onClick={() => void cancel()}>Cancel</button></div>
  </div>;
  if (phase !== 'idle') {
    const value = percent(status);
    return <div className="local-setup">
      <div className="local-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={value} aria-label="Setting up on this Mac"><i style={{ width: `${value}%` }} /></div>
      <div className="row-actions"><span className="field-help" role="status">{status.message || doing[phase!] || `${value}% of ${gigabytes(status.totalBytes)}`}</span><button type="button" className="text-button" onClick={() => void cancel()}>Cancel</button></div>
    </div>;
  }
  if (plan.problem) return <p className="field-help">{plan.problem}</p>;
  return <button type="button" className="secondary-button local-setup-start" onClick={() => { onStart?.(); void act('local_setup_start'); }}>Set up on this Mac · {gigabytes(plan.downloadBytes)}</button>;
}
