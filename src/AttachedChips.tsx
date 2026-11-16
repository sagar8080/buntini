import { AppWindow, FileText, Image, X } from 'lucide-react';
import { chipLabel, isPicture } from './attachments';
import { t } from './i18n.ts';
import type { Attached } from './types';

/** What goes with the next messages, each removable. */
export default function AttachedChips({ items, disabled, onRemove }: { items: Attached[]; disabled: boolean; onRemove: (item: Attached) => void }) {
  return <div className="attached-chips" role="list" aria-label={t('chat.attached.label')}>
    {items.map(item => {
      const label = chipLabel(item);
      const icon = item.kind === 'window' ? <AppWindow size={12} /> : isPicture(item.path) ? <Image size={12} /> : <FileText size={12} />;
      const key = item.kind === 'window' ? `window-${item.id}` : item.path;
      return <span className="attached-chip" role="listitem" key={key} title={item.kind === 'window' ? `${item.app} · ${label}` : item.path}>
        {icon}<span>{label}</span>
        <button type="button" aria-label={t('chat.attached.remove', { name: label })} disabled={disabled} onClick={() => onRemove(item)}><X size={11} /></button>
      </span>;
    })}
  </div>;
}
