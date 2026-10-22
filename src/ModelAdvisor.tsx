import { useEffect, useRef, useState } from 'react';
import { Check, Copy, Cpu, LoaderCircle } from 'lucide-react';
import { command } from './bridge';
import type { Settings } from './types';

export interface ModelChoice { repo: string; label: string; downloadGb: number; note: string }
export interface MacProfile {
  chip: string; memoryGb: number; appleSilicon: boolean;
  recommended: ModelChoice; alternative?: ModelChoice | null; selected: ModelChoice;
  steps: { title: string; command: string; done: boolean }[];
  python?: string | null; modelFolder?: string | null;
}

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void };

/** "Recommended for this Mac": the Qwen or Gemma model that suits this chip and memory,
 *  the commands to set it up, and the paths filled in once they're ready. */
export default function ModelAdvisor({ draft, update }: Props) {
  const [profile, setProfile] = useState<MacProfile | null>(null);
  const [choice, setChoice] = useState<string | null>(null);
  const [copied, setCopied] = useState(-1);
  const [failed, setFailed] = useState(false);
  const filled = useRef('');
  const ready = !!profile?.python && !!profile?.modelFolder;
  const inUse = ready && draft.pythonPath === profile!.python && draft.modelPath === profile!.modelFolder;

  useEffect(() => {
    let active = true;
    const check = () => command<MacProfile>('mac_profile', { model: choice }).then(next => { if (active) { setProfile(next); setFailed(false); } }).catch(() => { if (active) setFailed(true); });
    void check();
    // Keep checking while the user runs the commands; stop once everything is in place.
    const id = window.setInterval(() => { if (!inUse) void check(); }, 4000);
    return () => { active = false; window.clearInterval(id); };
  }, [choice, inUse]);

  // Fill in the paths the moment both parts are ready, unless the user typed their own.
  useEffect(() => {
    if (!profile?.python || !profile.modelFolder) return;
    const key = `${profile.python}|${profile.modelFolder}`;
    if (filled.current === key) return;
    const untouched = (value: string) => !value.trim() || value === filled.current.split('|')[0] || value === filled.current.split('|')[1];
    if (untouched(draft.pythonPath) && untouched(draft.modelPath)) {
      update('pythonPath', profile.python);
      update('modelPath', profile.modelFolder);
      filled.current = key;
    }
  }, [profile?.python, profile?.modelFolder]);

  if (failed && !profile) return null;
  if (!profile) return <div className="advisor"><LoaderCircle size={14} className="spin" /> Checking this Mac…</div>;
  const options = [profile.recommended, ...(profile.alternative ? [profile.alternative] : [])];
  return <div className="advisor">
    <div className="advisor-head"><Cpu size={16} /><span><strong>Recommended for this Mac</strong><small>{profile.chip} · {profile.memoryGb} GB memory</small></span></div>
    {!profile.appleSilicon && <p className="field-help">Local models need an Apple silicon Mac.</p>}
    <div className="advisor-options" role="radiogroup" aria-label="Model">
      {options.map((option, index) => <button type="button" key={option.repo} role="radio" aria-checked={profile.selected.repo === option.repo} className={profile.selected.repo === option.repo ? 'active' : ''} onClick={() => setChoice(index === 0 ? null : option.repo)}>
        <strong>{option.label}</strong><em>{index === 0 ? 'Best fit' : 'Also fits'} · {option.downloadGb.toFixed(1)} GB</em><small>{option.note}</small>
      </button>)}
    </div>
    {inUse ? <p className="advisor-done"><Check size={14} /> {profile.selected.label} is set up and in use.</p> : <>
      <ol className="advisor-steps">
        {profile.steps.map((step, index) => <li key={step.title} className={step.done ? 'done' : ''}>
          <span className="advisor-step-title">{step.done ? <Check size={13} /> : <b>{index + 1}</b>}{step.title}</span>
          {!step.done && <div className="advisor-command"><code>{step.command}</code><button type="button" aria-label={`Copy: ${step.title}`} title="Copy" onClick={() => { void navigator.clipboard?.writeText(step.command).then(() => { setCopied(index); window.setTimeout(() => setCopied(current => current === index ? -1 : current), 1500); }); }}>{copied === index ? <Check size={13} /> : <Copy size={13} />}</button></div>}
        </li>)}
      </ol>
      <p className="field-help">{ready ? 'Everything’s ready.' : 'Run these in Terminal. I’ll fill in the paths below as soon as the download finishes.'}</p>
      {ready && <button type="button" className="secondary-button" onClick={() => { update('pythonPath', profile.python!); update('modelPath', profile.modelFolder!); filled.current = `${profile.python}|${profile.modelFolder}`; }}>Use {profile.selected.label}</button>}
    </>}
  </div>;
}
