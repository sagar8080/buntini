// What a cloud brain may see, asked when one is picked. Settings → Privacy keeps the
// full list; these group it into the few choices people make at that moment.

export const shareGroups: { id: string; label: string; mine: string; shares: string[] }[] = [
  { id: 'personal', label: 'Email, calendar, notes and contacts', mine: 'my email, calendar, notes and contacts', shares: ['mail', 'calendar', 'notes', 'contacts'] },
  { id: 'files', label: 'Your files', mine: 'my files', shares: ['files'] },
  { id: 'doing', label: 'What you’re doing on the Mac', mine: 'what I’m doing on the Mac', shares: ['activity', 'selection', 'browser', 'music', 'system'] },
  { id: 'remembered', label: 'Earlier messages and memories', mine: 'our earlier messages and what you remember', shares: ['history', 'memory'] },
];

type Group = (typeof shareGroups)[number];

/** On only when every kind in the group is shared, so a tick never hides a gap. */
export const groupShared = (group: Group, shared: string[]) => group.shares.every(id => shared.includes(id));

export function setGroup(group: Group, shared: string[], on: boolean): string[] {
  return on ? [...new Set([...shared, ...group.shares])] : shared.filter(id => !group.shares.includes(id));
}

/** How the choice reads in the user's words during setup. */
export function sharedSentence(shared: string[]): string {
  const groups = shareGroups.filter(group => groupShared(group, shared)).map(group => group.mine);
  if (!groups.length) return 'It sees nothing personal.';
  return `It may see ${groups.length === 1 ? groups[0] : `${groups.slice(0, -1).join(', ')} and ${groups.at(-1)}`}.`;
}
