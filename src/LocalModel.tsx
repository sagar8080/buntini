import { useState } from 'react';
import { Check, CircleAlert, FolderOpen, LoaderCircle } from 'lucide-react';
import ModelAdvisor from './ModelAdvisor';
import { command, errorMessage } from './bridge';
import { folderName, setupLines, type SetupReport } from './brain.ts';
import type { Settings } from './types';

// `testable` is off in first-run setup: the check reads saved settings, and setup saves at the end.
type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void; testable?: boolean };

/** The local model, set up by choosing folders in Finder rather than typing paths. */
export default function LocalModel({ draft, update, onError, testable = true }: Props) {
  const [testing, setTesting] = useState(false);
  const [lines, setLines] = useState<ReturnType<typeof setupLines> | null>(null);
  const chosen = !!draft.pythonPath.trim() && !!draft.modelPath.trim();
  async function choose(kind: 'environment' | 'model') {
    try {
      const path = await command<string | null>('choose_local_folder', { kind });
      if (path) { update(kind === 'environment' ? 'pythonPath' : 'modelPath', path); setLines(null); }
    } catch (error) { onError(errorMessage(error)); }
  }
  async function test() {
    setTesting(true);
    try { setLines(setupLines(await command<SetupReport>('worker_diagnostics'))); }
    catch (error) { onError(errorMessage(error)); }
    finally { setTesting(false); }
  }
  const row = (label: string, hint: string, value: string, kind: 'environment' | 'model') => <div className="local-choice">
    <FolderOpen size={16} />
    <span><strong>{label}</strong><small title={value || undefined}>{value ? folderName(value) : hint}</small></span>
    <button type="button" className="secondary-button" onClick={() => void choose(kind)}>{value ? 'Change…' : 'Choose…'}</button>
  </div>;
  return <>
    {!chosen && <ModelAdvisor draft={draft} update={update} />}
    <div className="local-choices">
      {row('Python environment', 'The folder where MLX is installed', draft.pythonPath, 'environment')}
      {row('Model', 'The Qwen or Gemma folder you downloaded', draft.modelPath, 'model')}
    </div>
    {chosen && testable && <div className="row-actions"><button type="button" className="secondary-button" disabled={testing} onClick={() => void test()}>{testing ? <LoaderCircle size={14} className="spin" /> : <Check size={14} />}{testing ? 'Testing…' : 'Test setup'}</button></div>}
    {lines && <ul className="setup-report">{lines.map(line => <li key={line.label} className={line.ok ? 'ok' : 'problem'}>{line.ok ? <Check size={14} /> : <CircleAlert size={14} />}<span><strong>{line.label}</strong> {line.detail}</span></li>)}</ul>}
    <details className="disclosure"><summary>Enter paths manually</summary>
      <label className="field">Python<input value={draft.pythonPath} onChange={event => update('pythonPath', event.target.value)} placeholder="/path/to/venv/bin/python" spellCheck={false} /></label>
      <label className="field">Model folder<input value={draft.modelPath} onChange={event => update('modelPath', event.target.value)} placeholder="/path/to/model" spellCheck={false} /></label>
    </details>
  </>;
}
