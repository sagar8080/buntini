import { Apple, Check, Cpu, WifiOff } from 'lucide-react';
import SetupChecks from './SetupChecks';
import type { Diagnostics, ModelBackend, Settings } from './types';

/** Plain-language Apple Intelligence status from the helper's availability report. */
export function appleStatus(reason?: string): { ready: boolean; label: string; advice?: string } {
  switch (reason) {
    case 'available': return { ready: true, label: 'Ready on this Mac' };
    case 'appleIntelligenceNotEnabled': return { ready: false, label: 'Turned off', advice: 'Turn on Apple Intelligence in System Settings → Apple Intelligence & Siri.' };
    case 'modelNotReady': return { ready: false, label: 'Downloading', advice: 'macOS is still downloading the model. Try again in a few minutes.' };
    case 'deviceNotEligible': return { ready: false, label: 'Not supported', advice: 'This Mac doesn’t support Apple Intelligence.' };
    case 'unsupported': return { ready: false, label: 'Needs macOS 26', advice: 'Apple Intelligence needs macOS 26 on an Apple silicon Mac.' };
    case 'missing': return { ready: false, label: 'Missing', advice: 'The Apple Intelligence helper is missing from this installation.' };
    case 'checking': case undefined: return { ready: false, label: 'Checking…' };
    default: return { ready: false, label: 'Unavailable', advice: 'Apple Intelligence isn’t available right now.' };
  }
}

export const backendLabel: Record<ModelBackend, string> = { apple: 'Apple Intelligence', mlx: 'Local MLX model', cloud: 'Cloud model' };

/** Which model answers: Apple Intelligence, a local MLX model or (later) a cloud model. */
export default function BrainSettings({ draft, update, diagnostics, onError }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; diagnostics: Diagnostics; onError: (error: string) => void }) {
  const apple = appleStatus(diagnostics.appleIntelligence);
  const choose = (backend: ModelBackend) => update('modelBackend', backend);
  return <section className="settings-section"><div className="settings-section-heading"><Cpu size={15} /><h3>Brain</h3><span className={`pill ${diagnostics.modelReady ? 'good' : ''}`}>{diagnostics.modelReady ? 'Ready' : 'Needs setup'}</span></div>
    <p>Choose which model answers you. On-device models can use everything on this Mac, because nothing leaves it.</p>
    <div className="brain-options" role="radiogroup" aria-label="Model">
      <button type="button" role="radio" aria-checked={draft.modelBackend === 'apple'} className={draft.modelBackend === 'apple' ? 'active' : ''} onClick={() => choose('apple')}>
        <Apple size={16} /><span><strong>Apple Intelligence</strong><small>Built into macOS · private · fast · no setup</small></span>
        <em className={apple.ready ? 'ready' : ''}>{apple.ready && <Check size={10} />}{apple.label}</em>
      </button>
      <button type="button" role="radio" aria-checked={draft.modelBackend === 'mlx'} className={draft.modelBackend === 'mlx' ? 'active' : ''} onClick={() => choose('mlx')}>
        <Cpu size={16} /><span><strong>Local MLX model</strong><small>Your own downloaded model (for example Qwen) · bigger and slower</small></span>
      </button>
    </div>
    {draft.modelBackend === 'apple' && apple.advice && <div className="field-help">{apple.advice}</div>}
    {draft.modelBackend === 'mlx' && <>
      <label>Python interpreter<input value={draft.pythonPath} onChange={event => update('pythonPath', event.target.value)} placeholder="/path/to/venv/bin/python" spellCheck={false} /></label>
      <label>MLX model folder<input value={draft.modelPath} onChange={event => update('modelPath', event.target.value)} placeholder="/path/to/local/model" spellCheck={false} /></label>
      <div className="field-help"><WifiOff size={11} /> Local model files only. Buntini never downloads models.</div>
      <SetupChecks onError={onError} />
    </>}
    <div className="worker-status"><span className={`status-dot ${diagnostics.modelReady ? '' : 'amber'}`} />{diagnostics.workerStatus}</div>
  </section>;
}
