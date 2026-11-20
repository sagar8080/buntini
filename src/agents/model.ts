// Shared shapes for coding agents beyond requests and hooks (stream B). No bridge import, so the bridge can use them.
import { t } from '../i18n.ts';

/** Saved under `settings.agents`; mirrors `agents::Settings` in Rust. */
export type AgentsSettings = Record<string, never>;
export const defaults: AgentsSettings = {};

export interface RecapStat { label: string; value: string }
export interface RecapWeek { week: string; stats: RecapStat[] }

/** What `agents_call` accepts; mirrors `agents::Op`. */
export type AgentsOp =
  | { op: 'ping' }
  | { op: 'projects' }
  | { op: 'start'; agent: string; project: string; prompt: string }
  | { op: 'open'; pill: string }
  | { op: 'drop'; pill: string; paths: string[] }
  | { op: 'recap' }
  | { op: 'shareRecap' }
  | { op: 'planUsage'; on: boolean };

/** A sample week, labelled as Rust labels it. */
const previewWeek = (): RecapWeek => ({ week: '16–22 Nov', stats: [{ label: t('agents.recap.time'), value: '6 h 40 min' }, { label: t('agents.recap.sessions'), value: '14' }, { label: t('agents.recap.turns'), value: '61' }, { label: t('agents.recap.lines'), value: '+2,310 −480' }, { label: t('agents.recap.topAgent'), value: 'Claude Code' }, { label: t('agents.recap.busiestDay'), value: 'Tuesday' }] });

/** What the browser preview answers for `agents_call`. */
export function preview(op: AgentsOp): unknown {
  switch (op.op) {
    case 'ping': return true;
    case 'projects': return ['/Users/me/projects/buntini', '/Users/me/projects/shop'];
    case 'recap': return [previewWeek()];
    case 'start': case 'open': case 'drop': case 'planUsage': return null;
    case 'shareRecap': return '/Users/me/Downloads/Buntini week.png';
  }
}
