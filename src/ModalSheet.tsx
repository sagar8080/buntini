import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';

const focusable = 'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), a[href], [tabindex]:not([tabindex="-1"])';

/** An approval sheet that is really modal: the rest of the panel and the pet can't be reached
    while it is open, Tab stays inside it, and focus goes back where it was when it closes.
    Messages and the screen reader's live region stay reachable, so an error can still be read. */
export default function ModalSheet({ className, labelledBy, children }: { className?: string; labelledBy: string; children: ReactNode }) {
  const sheet = useRef<HTMLElement>(null);
  // Read during render, before autoFocus moves focus into the sheet.
  const [before] = useState(() => document.activeElement as HTMLElement | null);
  useEffect(() => {
    const element = sheet.current;
    const backdrop = element?.parentElement;
    if (!element || !backdrop) return;
    const outside = [...(backdrop.parentElement?.children ?? []), document.querySelector('.pet-dock')]
      .filter((node): node is HTMLElement => node instanceof HTMLElement && node !== backdrop && !node.matches('.toast, [aria-live]') && !node.inert);
    for (const node of outside) node.inert = true;
    if (!element.contains(document.activeElement)) (element.querySelector<HTMLElement>(focusable) ?? element).focus();
    return () => {
      for (const node of outside) node.inert = false;
      // Back to where the user was, or to the composer if that is gone or still disabled.
      requestAnimationFrame(() => {
        if (before?.isConnected && before !== document.body && !before.closest('[inert]') && !(before as HTMLButtonElement).disabled) before.focus();
        else document.querySelector<HTMLTextAreaElement>('.composer textarea:not(:disabled)')?.focus();
      });
    };
  }, []);
  const trap = (event: KeyboardEvent) => {
    if (event.key !== 'Tab' || !sheet.current) return;
    const items = [...sheet.current.querySelectorAll<HTMLElement>(focusable)];
    if (!items.length) { event.preventDefault(); return; }
    const first = items[0], last = items[items.length - 1];
    if (event.shiftKey && (document.activeElement === first || document.activeElement === sheet.current)) { event.preventDefault(); last.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first.focus(); }
  };
  return <div className="approval-backdrop"><section ref={sheet} className={`approval-sheet${className ? ` ${className}` : ''}`} role="alertdialog" aria-modal="true" aria-labelledby={labelledBy} tabIndex={-1} onKeyDown={trap}>{children}</section></div>;
}
