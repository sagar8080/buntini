import { command } from '../bridge';
import type { UtilitiesOp } from './model';

export const callUtilities = <T>(op: UtilitiesOp) => command<T>('utilities_call', { op });
