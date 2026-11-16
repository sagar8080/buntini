import { locale, number, t, tn } from './i18n.ts';

export function relativeTime(value: string) {
  const date = new Date(value);
  if (!Number.isFinite(date.getTime())) return value;
  const minutes = Math.floor((Date.now() - date.getTime()) / 60000);
  if (minutes < 1) return t('time.justNow');
  if (minutes < 60) return tn('time.minutesAgo', minutes);
  if (minutes < 1440) return tn('time.hoursAgo', Math.floor(minutes / 60));
  return date.toLocaleDateString(locale(), { month: 'short', day: 'numeric' });
}
const oneDecimal = { minimumFractionDigits: 1, maximumFractionDigits: 1, useGrouping: false };
export function fileSize(bytes: number) {
  if (bytes < 1024) return t('chat.fileSize.bytes', { size: number(bytes, { useGrouping: false }) });
  if (bytes < 1024 * 1024) return t('chat.fileSize.kilobytes', { size: number(bytes / 1024, oneDecimal) });
  return t('chat.fileSize.megabytes', { size: number(bytes / (1024 * 1024), oneDecimal) });
}
export function safeHostname(url: string) { try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return t('chat.source'); } }
