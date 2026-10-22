import { useState } from 'react';
import { LoaderCircle, ShieldCheck } from 'lucide-react';
import { command, errorMessage } from './bridge';

type Result = { state: string; checkedAt: string; summary: string; updates: string[] };

/** Asks Apple's Software Update service what's available, only after the user agrees. Never installs. */
export default function MacUpdates({ onError }: { onError: (error: string) => void }) {
  const [confirming, setConfirming] = useState(false);
  const [checking, setChecking] = useState(false);
  const [result, setResult] = useState<Result | null>(null);
  async function check() {
    setChecking(true);
    try { setResult(await command<Result>('check_os_updates', { approved: true })); setConfirming(false); }
    catch (error) { onError(errorMessage(error)); }
    finally { setChecking(false); }
  }
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>macOS updates</h3></div>
    {!confirming && <button type="button" className="secondary-button" onClick={() => setConfirming(true)}><ShieldCheck size={14} /> Check for updates</button>}
    {confirming && <div className="inline-approval"><p>Ask Apple’s Software Update service what’s available? Nothing gets installed.</p><div><button type="button" className="text-button" disabled={checking} onClick={() => setConfirming(false)}>Cancel</button><button type="button" className="secondary-button" disabled={checking} onClick={() => void check()}>{checking ? <LoaderCircle size={13} className="spin" /> : null}{checking ? 'Checking…' : 'Check now'}</button></div></div>}
    {result && <div className="update-results"><strong>{result.summary}</strong><small>Checked {new Date(result.checkedAt).toLocaleString()}</small>{result.updates.map((update, index) => <p key={index}>{update}</p>)}</div>}
  </section>;
}
