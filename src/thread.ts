// Pure helpers for the conversation thread: ordering, the "New topic" divider and the greeting.
import type { TaskRecord } from './types';

export type ThreadItem = { kind: 'turn'; turn: TaskRecord } | { kind: 'topic'; at: string };

/** Adds turns (in any order) to a chronological thread, replacing earlier copies by id. */
export function mergeTurns(thread: TaskRecord[], incoming: TaskRecord[]): TaskRecord[] {
  const byId = new Map(thread.map(turn => [turn.id, turn]));
  for (const turn of incoming) if (turn.id) byId.set(turn.id, turn);
  return [...byId.values()].sort((a, b) => a.createdAt.localeCompare(b.createdAt));
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
  const part = hour < 5 ? 'Up late' : hour < 12 ? 'Good morning' : hour < 17 ? 'Good afternoon' : hour < 22 ? 'Good evening' : 'Up late';
  const who = name?.trim();
  return who ? `${part}, ${who}.` : `${part}.`;
}
