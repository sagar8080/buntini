import { useState } from 'react';
import { Check, LoaderCircle, ShieldCheck, WifiOff } from 'lucide-react';
import { command, errorMessage } from './bridge';

type WorkerDiagnostics = { dependencies: Record<string, { installed: boolean; version: string | null }>; models: Record<string, { configured: boolean; files_present: boolean; ready_for_load: boolean; error: string | null }>; inference_verified: boolean };
export default function SetupChecks({ onError }: { onError: (error: string) => void }) {
  const [checking, setChecking] = useState(false);
  const [diagnostics, setDiagnostics] = useState<WorkerDiagnostics | null>(null);
  const [confirmUpdates, setConfirmUpdates] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [updates, setUpdates] = useState<{ state: string; checkedAt: string; summary: string; updates: string[] } | null>(null);
  return <div className="setup-checks">
    <button type="button" className="secondary-button" disabled={checking} onClick={async () => { setChecking(true); try { setDiagnostics(await command<WorkerDiagnostics>('worker_diagnostics')); } catch (error) { onError(errorMessage(error)); } finally { setChecking(false); } }}>{checking ? <LoaderCircle size={12} className="spin" /> : <WifiOff size={12} />} Check saved local setup</button>
    {diagnostics && <div className="setup-results">{Object.entries(diagnostics.dependencies).map(([name, entry]) => <div key={name}><span>{name.replaceAll('_', ' ')}</span><span className={entry.installed ? 'setup-good' : ''}>{entry.installed ? <><Check size={10} /> {entry.version}</> : 'Missing'}</span></div>)}{Object.entries(diagnostics.models).map(([name, entry]) => <div key={name}><span>{name === 'vad' ? 'Silero VAD' : `${name} model`}</span><span title={entry.error || ''} className={entry.ready_for_load ? 'setup-good' : ''}>{entry.ready_for_load ? 'Files present' : entry.configured ? entry.error || 'Incomplete' : name === 'vad' ? 'Optional' : 'Not configured'}</span></div>)}<p>Checks local files and packages. A successful model load is verified when you use it.</p></div>}
    <button type="button" className="text-button update-check-button" disabled={updating} onClick={() => setConfirmUpdates(!confirmUpdates)}><ShieldCheck size={12} /> Check macOS updates</button>
    {confirmUpdates && <div className="inline-approval"><p>Ask Apple’s Software Update service for available updates? This checks availability and does not install anything.</p><div><button type="button" className="text-button" disabled={updating} onClick={() => setConfirmUpdates(false)}>Cancel</button><button type="button" className="secondary-button" disabled={updating} onClick={async () => { setUpdating(true); try { setUpdates(await command('check_os_updates', { approved: true })); setConfirmUpdates(false); } catch (error) { onError(errorMessage(error)); } finally { setUpdating(false); } }}>{updating ? <LoaderCircle size={11} className="spin" /> : null}{updating ? 'Checking…' : 'Allow check'}</button></div></div>}
    {updates && <div className="update-results"><strong>{updates.summary}</strong><small>Checked {new Date(updates.checkedAt).toLocaleString()}</small>{updates.updates.map((update, index) => <p key={index}>{update}</p>)}</div>}
  </div>;
}
