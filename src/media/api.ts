import { command } from '../bridge';
import type { MediaOp } from './model';

export const callMedia = <T>(op: MediaOp) => command<T>('media_call', { op });
