import { command } from '../bridge';
import type { IslandOp } from './model';

export const callIsland = <T>(op: IslandOp) => command<T>('island_call', { op });
