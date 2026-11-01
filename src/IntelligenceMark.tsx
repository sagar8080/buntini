import { useId } from 'react';

/** The mark for Buntini's built-in on-device brain (Apple Intelligence): a multicolour
    ring with a spark, sized like the lucide icons beside it. It is deliberately not
    Apple's own Apple Intelligence glyph, whose use in other apps' interfaces Apple
    restricts; the option's text names the feature. */
export default function IntelligenceMark({ size = 18 }: { size?: number }) {
  const id = useId().replaceAll(':', '');
  return <svg width={size} height={size} viewBox="0 0 24 24" fill="none" aria-hidden="true" className="intelligence-mark">
    <defs>
      <linearGradient id={`${id}-ring`} x1="3" y1="3" x2="21" y2="21" gradientUnits="userSpaceOnUse">
        <stop stopColor="#FFB547" /><stop offset=".3" stopColor="#FF5F8A" /><stop offset=".62" stopColor="#A66CFF" /><stop offset="1" stopColor="#3FC4FF" />
      </linearGradient>
    </defs>
    <circle cx="12" cy="12" r="8.6" stroke={`url(#${id}-ring)`} strokeWidth="2.4" />
    <path d="M12 7.6C12.5 10.3 13.7 11.5 16.4 12C13.7 12.5 12.5 13.7 12 16.4C11.5 13.7 10.3 12.5 7.6 12C10.3 11.5 11.5 10.3 12 7.6Z" fill="currentColor" />
  </svg>;
}
