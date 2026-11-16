import { command } from '../bridge';
import type { AgentsOp } from './model';

export const callAgents = <T>(op: AgentsOp) => command<T>('agents_call', { op });
