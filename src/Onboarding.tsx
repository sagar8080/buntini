import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { Apple, ArrowLeft, ArrowRight, Check, Cloud, Cpu, KeyRound, LoaderCircle } from 'lucide-react';
import Pet from './Pet';
import { palettes } from './Cat';
import { appleStatus, cloudProviders } from './brain.ts';
import { coats, lookFrom } from './petOptions';
import { applyAnswers, blocker, initialAnswers, keysToSave, message, next, previous, reply, steps, type Answers, type Step } from './setupSteps';
import { errorMessage } from './bridge';
import ChatGPTAccount from './ChatGPTAccount';
import type { Bootstrap, CatCoat, CloudProvider, KeyProvider, Settings } from './types';

type Props = { data: Bootstrap; onFinish: (settings: Settings, keys: [KeyProvider, string][]) => Promise<void>; onSkip: () => Promise<void>; onRefresh: () => Promise<void> };

/** First-run setup as a conversation: Buntini asks, the user answers with inline controls. */
export default function Onboarding({ data, onFinish, onSkip, onRefresh }: Props) {
  const [step, setStep] = useState<Step>('hello');
  const [answers, setAnswers] = useState<Answers>(() => initialAnswers(data.settings, data.diagnostics));
  const [said, setSaid] = useState<{ step: Step; buntini: string; you: string }[]>([]);
  const [typing, setTyping] = useState(true);
  const [error, setError] = useState('');
  const [saving, setSaving] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  const set = <K extends keyof Answers>(key: K, value: Answers[K]) => { setAnswers(previous => ({ ...previous, [key]: value })); setError(''); };
  const look = lookFrom({ ...data.settings, catCoat: answers.coat });
  useEffect(() => {
    const still = data.settings.reducedMotion || window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    setTyping(!still);
    const id = window.setTimeout(() => setTyping(false), still ? 0 : 650);
    return () => window.clearTimeout(id);
  }, [step, data.settings.reducedMotion]);
  useEffect(() => { scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: 'smooth' }); }, [step, typing, said.length]);

  async function advance() {
    const problem = blocker(step, answers, data.diagnostics);
    if (problem) { setError(problem); return; }
    if (step !== 'done') {
      setSaid(previous => [...previous, { step, buntini: message(step, answers, data.diagnostics), you: reply(step, answers) }]);
      setStep(next(step));
      return;
    }
    setSaving(true); setError('');
    try { await onFinish(applyAnswers(data.settings, answers), keysToSave(answers)); }
    catch (failure) { setError(errorMessage(failure)); }
    finally { setSaving(false); }
  }
  function back() {
    setSaid(previous => previous.slice(0, -1));
    setStep(previous(step)); setError('');
  }
  const bubble = (text: string, key: string, children?: ReactNode) => <div className="bubble-row is-buntini" key={key}>
    <span className="bubble-avatar"><Pet state="idle" avatar="cat" look={look} /></span>
    <div className="bubble bubble-buntini"><div className="answer-text">{text}</div>{children}</div>
  </div>;
  const index = steps.indexOf(step);
  const skipOnline = step === 'online' && !answers.searchKey.trim();

  return <>
    <div className="conversation" ref={scroller}>
      <div className="thread setup-thread">
        <div className="setup-progress"><span>Getting to know you · {Math.min(index + 1, 4)} of 4</span><button type="button" className="text-button" onClick={() => void onSkip()} disabled={saving}>Skip setup</button></div>
        {said.map(item => <div className="turn" key={item.step}>{bubble(item.buntini, `${item.step}-b`)}<div className="bubble-row is-you"><div className="bubble bubble-you">{item.you}</div></div></div>)}
        {typing ? <div className="bubble-row is-buntini"><span className="bubble-avatar"><Pet state="thinking" avatar="cat" look={look} /></span><div className="bubble bubble-buntini"><div className="typing" role="status" aria-label="Typing"><i /><i /><i /></div></div></div>
          : bubble(message(step, answers, data.diagnostics), step, step !== 'done' && <form className="setup-card" onSubmit={event => { event.preventDefault(); void advance(); }}>
            {step === 'hello' && <Hello answers={answers} set={set} />}
            {step === 'brain' && <Brain answers={answers} set={set} data={data} onRefresh={onRefresh} onError={setError} />}
            {step === 'online' && <Online answers={answers} set={set} />}
            {step === 'roam' && <Roam answers={answers} set={set} />}
            <button type="submit" hidden aria-hidden="true" tabIndex={-1} />
          </form>)}
      </div>
    </div>
    <div className="setup-actions">
      {error && <p className="setup-error" role="alert">{error}</p>}
      <div>
        <button type="button" className="text-button" onClick={back} disabled={index === 0 || saving}><ArrowLeft size={12} /> Back</button>
        <button type="button" className="primary-button" onClick={() => void advance()} disabled={saving || typing}>
          {saving ? <LoaderCircle size={13} className="spin" /> : step === 'done' ? <Check size={13} /> : null}
          {saving ? 'Saving…' : step === 'done' ? (error ? 'Try again' : 'Start chatting') : skipOnline ? 'Skip for now' : 'Continue'}
          {!saving && step !== 'done' && <ArrowRight size={13} />}
        </button>
      </div>
    </div>
  </>;
}

type StepProps = { answers: Answers; set: <K extends keyof Answers>(key: K, value: Answers[K]) => void };

function swatch(id: CatCoat): CSSProperties {
  const coat = palettes[id];
  const fill = coat.patchA ? `linear-gradient(135deg, ${coat.base} 0 40%, ${coat.patchA} 40% 68%, ${coat.patchB} 68%)` : coat.point ? `radial-gradient(circle, ${coat.base} 45%, ${coat.point})` : coat.belly !== coat.base && id === 'tuxedo' ? `linear-gradient(${coat.base} 58%, ${coat.belly} 58%)` : coat.marks ? `repeating-linear-gradient(120deg, ${coat.base} 0 4px, ${coat.marks} 4px 6px)` : coat.base;
  return { background: fill, borderColor: coat.outline };
}

function Hello({ answers, set }: StepProps) {
  return <>
    <label>Your name<input value={answers.userName} onChange={event => set('userName', event.target.value)} placeholder="What should I call you?" maxLength={40} autoFocus /></label>
    <label>My name<input value={answers.assistantName} onChange={event => set('assistantName', event.target.value)} maxLength={40} required /></label>
    <div className="coat-picker" role="radiogroup" aria-label="Coat">{coats.map(coat => <button type="button" key={coat.id} role="radio" aria-checked={answers.coat === coat.id} className={answers.coat === coat.id ? 'active' : ''} onClick={() => set('coat', coat.id)}><i style={swatch(coat.id)} />{coat.label}</button>)}</div>
  </>;
}

function Brain({ answers, set, data, onRefresh, onError }: StepProps & { data: Bootstrap; onRefresh: () => Promise<void>; onError: (message: string) => void }) {
  const apple = appleStatus(data.diagnostics.appleIntelligence);
  const provider = cloudProviders.find(item => item.id === answers.cloudProvider)!;
  const saved = !!data.diagnostics.cloudKeys?.includes(provider.id);
  const option = (id: Answers['backend'], icon: ReactNode, title: string, detail: string, badge?: ReactNode) => <button type="button" role="radio" aria-checked={answers.backend === id} className={answers.backend === id ? 'active' : ''} onClick={() => set('backend', id)}>{icon}<span><strong>{title}</strong><small>{detail}</small></span>{badge}</button>;
  return <>
    <div className="brain-options" role="radiogroup" aria-label="Brain">
      {option('apple', <Apple size={16} />, 'Apple Intelligence', 'Built in · private · fast', <em className={apple.ready ? 'ready' : ''}>{apple.ready && <Check size={10} />}{apple.label}</em>)}
      {option('mlx', <Cpu size={16} />, 'Local model', 'Your own MLX model folder · bigger, slower, private')}
      {option('cloud', <Cloud size={16} />, 'Cloud model', 'Anthropic, OpenAI or Gemini · your key, your cap')}
    </div>
    {answers.backend === 'apple' && apple.advice && <div className="field-help">{apple.advice}</div>}
    {answers.backend === 'mlx' && <>
      <label>Python interpreter<input value={answers.pythonPath} onChange={event => set('pythonPath', event.target.value)} placeholder="/path/to/venv/bin/python" spellCheck={false} /></label>
      <label>Model folder<input value={answers.modelPath} onChange={event => set('modelPath', event.target.value)} placeholder="/path/to/local/model" spellCheck={false} /></label>
    </>}
    {answers.backend === 'cloud' && provider.id === 'chatgpt' && <>
      <label>Provider<select value={provider.id} onChange={event => { set('cloudProvider', event.target.value as CloudProvider); set('cloudModel', ''); set('cloudKey', ''); }}>{cloudProviders.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
      <ChatGPTAccount account={data.diagnostics.chatgptAccount} model="" onChange={onRefresh} onError={onError} />
    </>}
    {answers.backend === 'cloud' && provider.id !== 'chatgpt' && <>
      <label>Provider<select value={provider.id} onChange={event => { set('cloudProvider', event.target.value as CloudProvider); set('cloudModel', ''); set('cloudKey', ''); }}>{cloudProviders.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>
      <label><span className="setup-label">API key <span className="label-status">{saved ? 'Saved in Keychain' : `From ${provider.keys}`}</span></span><div className="key-input"><KeyRound size={13} /><input type="password" autoComplete="off" value={answers.cloudKey} onChange={event => set('cloudKey', event.target.value)} placeholder={saved ? '••••••••••••••••' : `Paste your ${provider.name} key`} aria-label={`${provider.name} API key`} /></div></label>
      <label className="inline-field"><span>Monthly spending cap<small>I stop using {provider.name} when it’s reached</small></span><input aria-label="Monthly cloud spending cap in USD" type="number" min="0" max="10000" step="0.5" value={answers.cloudBudget} onChange={event => set('cloudBudget', Number(event.target.value))} /></label>
      <div className="field-help">I’ll use {provider.model}; you can change the model, prices and what it may see in Settings.</div>
    </>}
  </>;
}

function Online({ answers, set }: StepProps) {
  return <>
    <label>Fallback search provider<select value={answers.searchProvider} onChange={event => { set('searchProvider', event.target.value as Answers['searchProvider']); set('searchKey', ''); }}><option value="brave">Brave Search</option><option value="tavily">Tavily</option></select></label>
    <label><span className="setup-label">API key <span className="label-status">Optional</span></span><div className="key-input"><KeyRound size={13} /><input type="password" autoComplete="off" value={answers.searchKey} onChange={event => set('searchKey', event.target.value)} placeholder="Paste your key, or skip" aria-label="Search API key" /></div></label>
    <div className="field-help">Get one at {answers.searchProvider === 'brave' ? 'brave.com/search/api' : 'app.tavily.com'}. It stays in your Mac’s Keychain.</div>
    <label className="notification-choice"><input type="checkbox" checked={answers.freeTier} onChange={event => set('freeTier', event.target.checked)} />Stay within the free allowance</label>
    <label className="inline-field"><span>Daily lookup limit<small>Keeps one busy afternoon from using the month</small></span><input aria-label="Daily lookup limit" type="number" min="1" max="2000" value={answers.dailyLimit} onChange={event => set('dailyLimit', Number(event.target.value))} /></label>
  </>;
}

function Roam({ answers, set }: StepProps) {
  const choice = (key: 'roam' | 'awareness' | 'nudges', label: string, detail: string, disabled = false) => <label className="notification-choice cloud-share"><input type="checkbox" checked={answers[key] && !disabled} disabled={disabled} onChange={event => set(key, event.target.checked)} /><span>{label}<small>{detail}</small></span></label>;
  return <>
    {choice('roam', 'Wander around', 'Walk along the bottom of the screen and nap on window tops.')}
    {choice('awareness', 'Notice what you’re doing', 'App names and active times, on this Mac only. Never window contents.')}
    {choice('nudges', 'Offer a hand', 'Now and then, suggest something helpful.', !answers.awareness)}
  </>;
}
