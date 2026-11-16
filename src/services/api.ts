import { command } from '../bridge';
import type { ServicesOp } from './model';

export const callServices = <T>(op: ServicesOp) => command<T>('services_call', { op });
