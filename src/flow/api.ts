import { command, desktop } from '../bridge';
import type { SettingsSaver } from '../settingsSave';
import type { FlowOp, FlowSettings } from './model';

export const callFlow = <T>(op: FlowOp) => command<T>('flow_call', { op });

/** Once saved, dictation picks up a new key, wake word or language right away. Returns a
    function that stops following. */
export function applyFlowOnSave(saver: SettingsSaver, current: FlowSettings) {
  let last = JSON.stringify(current);
  return saver.subscribeSaved((_, saved) => {
    const flow = JSON.stringify(saved.flow);
    if (flow === last) return;
    last = flow;
    if (desktop) void callFlow({ op: 'apply' }).catch(() => {});
  });
}
