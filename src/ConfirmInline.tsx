import { useEffect, useRef, type ReactNode } from 'react';
import { LoaderCircle } from 'lucide-react';

type Props = { label: string; action: string; working: boolean; onCancel: () => void; onConfirm: () => void; children: ReactNode };

/** A second step before something that can't be undone: says what goes, then asks once more.
    Focus starts on Cancel, so a stray Return keeps the data; Esc cancels too. */
export default function ConfirmInline({ label, action, working, onCancel, onConfirm, children }: Props) {
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => { cancel.current?.focus(); }, []);
  return <div className="inline-approval erase-confirm" role="alertdialog" aria-label={label} onKeyDown={event => { if (event.key === 'Escape' && !working) { event.stopPropagation(); onCancel(); } }}>
    <p>{children}</p>
    <div><button type="button" ref={cancel} className="text-button" onClick={onCancel} disabled={working}>Cancel</button><button type="button" className="danger-button" disabled={working} onClick={onConfirm}>{working ? <LoaderCircle size={13} className="spin" /> : null}{action}</button></div>
  </div>;
}
