// Pure helpers for the conversation thread: ordering, the "New topic" divider and the greeting.
import { t } from './i18n.ts';
import type { ConversationPage, TaskRecord } from './types';

export type ThreadItem = { kind: 'turn'; turn: TaskRecord } | { kind: 'topic'; at: string };

/** Adds turns (in any order) to a chronological thread, replacing earlier copies by id. */
export function mergeTurns(thread: TaskRecord[], incoming: TaskRecord[]): TaskRecord[] {
  const byId = new Map(thread.map(turn => [turn.id, turn]));
  for (const turn of incoming) if (turn.id) byId.set(turn.id, turn);
  return [...byId.values()].sort((a, b) => a.createdAt.localeCompare(b.createdAt) || a.id.localeCompare(b.id));
}

/** Switching threads replaces the visible turns. Refreshing one retains loaded older pages. */
export function mergeConversationTurns(currentId: string, turns: TaskRecord[], page: ConversationPage): TaskRecord[] {
  return mergeTurns(currentId === page.conversation.id ? turns : [], page.turns);
}

/** The thread with a divider where the latest topic began; turns before it are not sent to the model. */
export function threadItems(turns: TaskRecord[], boundary?: string | null): ThreadItem[] {
  const items: ThreadItem[] = [];
  let placed = !boundary || !turns.length || turns[0].createdAt > boundary;
  for (const turn of turns) {
    if (!placed && turn.createdAt > boundary!) { items.push({ kind: 'topic', at: boundary! }); placed = true; }
    items.push({ kind: 'turn', turn });
  }
  if (!placed) items.push({ kind: 'topic', at: boundary! });
  return items;
}

export function greeting(now: Date, name?: string): string {
  const hour = now.getHours();
  const part = hour < 5 ? 'late' : hour < 12 ? 'morning' : hour < 17 ? 'afternoon' : hour < 22 ? 'evening' : 'late';
  const who = name?.trim();
  if (who) return t(({ late: 'chat.greeting.lateNamed', morning: 'chat.greeting.morningNamed', afternoon: 'chat.greeting.afternoonNamed', evening: 'chat.greeting.eveningNamed' } as const)[part], { name: who });
  return t(({ late: 'chat.greeting.late', morning: 'chat.greeting.morning', afternoon: 'chat.greeting.afternoon', evening: 'chat.greeting.evening' } as const)[part]);
}

/** The line under the greeting, in Buntini's voice for the time of day. */
export function invitation(now: Date): string {
  const hour = now.getHours();
  if (hour < 5 || hour >= 22) return t('chat.invitation.late');
  if (hour < 12) return t('chat.invitation.morning');
  if (hour < 17) return t('chat.invitation.afternoon');
  return t('chat.invitation.evening');
}
