import { useState } from 'react';
import { ClipboardCopy, Download, LoaderCircle, Trash2 } from 'lucide-react';
import { command, errorMessage } from './bridge';

type Props = { onNotice: (text: string) => void; onError: (text: string) => void; onErased: () => Promise<void> };

/** Export, a support summary to paste into a bug report, and Delete everything. */
export default function YourData({ onNotice, onError, onErased }: Props) {
  const [working, setWorking] = useState('');
  const [confirming, setConfirming] = useState(false);
  async function act(name: string, action: () => Promise<void>) {
    setWorking(name);
    try { await action(); } catch (error) { onError(errorMessage(error)); } finally { setWorking(''); }
  }
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>Your data</h3></div>
    <div className="row-actions">
      <button type="button" className="secondary-button" disabled={!!working} onClick={() => void act('export', async () => { const path = await command<string | null>('export_data'); if (path) onNotice('Exported. Your keys were not included.'); })}>{working === 'export' ? <LoaderCircle size={14} className="spin" /> : <Download size={14} />} Export</button>
      <button type="button" className="secondary-button" disabled={!!working} onClick={() => void act('support', async () => { await navigator.clipboard.writeText(await command<string>('support_info')); onNotice('Support info copied. It has no messages, files or keys.'); })}>{working === 'support' ? <LoaderCircle size={14} className="spin" /> : <ClipboardCopy size={14} />} Copy support info</button>
    </div>
    {!confirming ? <button type="button" className="text-button danger erase-button" onClick={() => setConfirming(true)}><Trash2 size={13} /> Delete everything…</button>
      : <div className="inline-approval erase-confirm" role="alertdialog" aria-label="Delete everything">
        <p>This erases your conversation, settings, folders, lists, saved keys and recordings on this Mac. It can’t be undone.</p>
        <div><button type="button" className="text-button" onClick={() => setConfirming(false)} disabled={!!working}>Cancel</button><button type="button" className="danger-button" disabled={!!working} onClick={() => void act('erase', async () => { await command('delete_all_data', { confirm: 'delete everything' }); setConfirming(false); await onErased(); })}>{working === 'erase' ? <LoaderCircle size={13} className="spin" /> : null}Delete everything</button></div>
      </div>}
  </section>;
}
