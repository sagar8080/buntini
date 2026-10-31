import { useState } from 'react';
import { ClipboardCopy, Download, LoaderCircle, Trash2 } from 'lucide-react';
import { command, errorMessage } from './bridge';
import ConfirmInline from './ConfirmInline';

type Props = { onNotice: (text: string) => void; onError: (text: string) => void; onErase: () => Promise<void>; onErased: () => Promise<void> };

/** Export, a support summary to paste into a bug report, and Delete everything. */
export default function YourData({ onNotice, onError, onErase, onErased }: Props) {
  const [working, setWorking] = useState('');
  const [confirming, setConfirming] = useState(false);
  async function act(name: string, action: () => Promise<void>) {
    setWorking(name);
    try { await action(); } catch (error) { onError(errorMessage(error)); } finally { setWorking(''); }
  }
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>Your data</h3></div>
    <div className="row-actions">
      <button type="button" className="secondary-button" disabled={!!working} onClick={() => void act('export', async () => { const path = await command<string | null>('export_data'); if (path) onNotice('Exported.'); })}>{working === 'export' ? <LoaderCircle size={14} className="spin" /> : <Download size={14} />} Export</button>
      <button type="button" className="secondary-button" disabled={!!working} onClick={() => void act('support', async () => { await navigator.clipboard.writeText(await command<string>('support_info')); onNotice('Copied.'); })}>{working === 'support' ? <LoaderCircle size={14} className="spin" /> : <ClipboardCopy size={14} />} Copy support info</button>
    </div>
    {!confirming ? <button type="button" className="text-button danger erase-button" onClick={() => setConfirming(true)}><Trash2 size={13} /> Delete everything…</button>
      : <ConfirmInline label="Delete everything" action="Delete everything" working={working === 'erase'} onCancel={() => setConfirming(false)} onConfirm={() => void act('erase', async () => { await onErase(); setConfirming(false); await onErased(); })}>Erases everything here for good.</ConfirmInline>}
  </section>;
}
