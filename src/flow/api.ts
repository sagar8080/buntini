import { command } from '../bridge';
import type { FlowOp } from './model';

export const callFlow = <T>(op: FlowOp) => command<T>('flow_call', { op });
