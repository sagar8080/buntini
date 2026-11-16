import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react';
import { ArrowLeft, ArrowRight, Check, Cloud, Cpu, LoaderCircle } from 'lucide-react';
import LocalModel from './LocalModel';
import LocalSetup from './LocalSetup';
import IntelligenceMark from './IntelligenceMark';
import Pet from './Pet';
import { palettes } from './Cat';
import { appleStatus, backendLabel, cloudProviders } from './brain.ts';
import { coats, lookFrom } from './petOptions';
import { applyAnswers, blocker, initialAnswers, message, next, previous, questions, reply, steps, type Answers, type Step } from './setupSteps';
import { errorMessage } from './bridge';
import { stillMotion } from './themes.ts';
import { number, t } from './i18n.ts';
import ChatGPTAccount from './ChatGPTAccount';
import { CloudShares, OpenAppleSettings } from './BrainSettings';
import type { Bootstrap, CatCoat, Settings } from './types';

type Props = { data: Bootstrap; onFinish: (settings: Settings) => Promise<void>; onSkip: () => Promise<void>; onRefresh: () => Promise<void>; toast?: ReactNode };

/** First-run setup as a conversation: Buntini asks, the user answers with inline controls. */
export default function Onboarding({ data, onFinish, onSkip, onRefresh, toast }: Props) {
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
    const still = stillMotion(data.settings.reducedMotion);
    setTyping(!still);
    const id = window.setTimeout(() => setTyping(false), still ? 0 : 650);
    return () => window.clearTimeout(id);
  }, [step, data.settings.reducedMotion]);
  useEffect(() => { scroller.current?.scrollTo({ top: scroller.current.scrollHeight, behavior: stillMotion(data.settings.reducedMotion) ? 'instant' : 'smooth' }); }, [step, typing, said.length]);

  async function advance() {
    const problem = blocker(step, answers, data.diagnostics);
    if (problem) { setError(problem); return; }
    if (step !== 'done') {
      setSaid(previous => [...previous, { step, buntini: message(step, answers, data.diagnostics), you: reply(step, answers, data.settings) }]);
      setStep(next(step));
      return;
    }
    setSaving(true); setError('');
    try { await onFinish(applyAnswers(data.settings, answers)); }
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

  return <>
    <div className="conversation" ref={scroller}>
      <div className="thread setup-thread">
        <div className="setup-progress"><span>{t('onboarding.progress', { step: number(Math.min(index + 1, questions)), total: number(questions) })}</span><button type="button" className="text-button" onClick={() => void onSkip()} disabled={saving}>{t('onboarding.skip')}</button></div>
        {said.map(item => <div className="turn" key={item.step}>{bubble(item.buntini, `${item.step}-b`)}<div className="bubble-row is-you"><div className="bubble bubble-you">{item.you}</div></div></div>)}
        {typing ? <div className="bubble-row is-buntini"><span className="bubble-avatar"><Pet state="thinking" avatar="cat" look={look} /></span><div className="bubble bubble-buntini"><div className="typing" role="status" aria-label={t('onboarding.typing')}><i /><i /><i /></div></div></div>
          : bubble(message(step, answers, data.diagnostics), step, step !== 'done' && <form className="setup-card" onSubmit={event => { event.preventDefault(); void advance(); }}>
            {step === 'hello' && <Hello answers={answers} set={set} />}
            {step === 'brain' && <Brain answers={answers} set={set} data={data} onRefresh={onRefresh} onError={setError} />}
            {step === 'roam' && <Roam answers={answers} set={set} />}
            <button type="submit" hidden aria-hidden="true" tabIndex={-1} />
          </form>)}
      </div>
    </div>
    <div className="setup-actions">
      {toast}
      {error && <p className="setup-error" role="alert">{error}</p>}
      <div>
        <button type="button" className="text-button" onClick={back} disabled={index === 0 || saving}><ArrowLeft size={12} /> {t('common.back')}</button>
        <button type="button" className="primary-button" onClick={() => void advance()} disabled={saving || typing}>
          {saving ? <LoaderCircle size={13} className="spin" /> : step === 'done' ? <Check size={13} /> : null}
          {saving ? t('common.saving') : step === 'done' ? (error ? t('common.tryAgain') : t('onboarding.startChatting')) : t('common.continue')}
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

const coatNames: Record<CatCoat, string> = {
  ginger: 'surface.coat.ginger', gray: 'surface.coat.gray', black: 'surface.coat.black', white: 'surface.coat.white',
  tuxedo: 'surface.coat.tuxedo', calico: 'surface.coat.calico', siamese: 'surface.coat.siamese',
};

function Hello({ answers, set }: StepProps) {
  return <>
    <label>{t('onboarding.yourName')}<input value={answers.userName} onChange={event => set('userName', event.target.value)} placeholder={t('onboarding.yourNamePlaceholder')} maxLength={40} autoFocus /></label>
    <label>{t('onboarding.myName')}<input value={answers.assistantName} onChange={event => set('assistantName', event.target.value)} maxLength={40} required /></label>
    <div className="coat-picker" role="radiogroup" aria-label={t('onboarding.coat')}>{coats.map(coat => <button type="button" key={coat.id} role="radio" aria-checked={answers.coat === coat.id} className={answers.coat === coat.id ? 'active' : ''} onClick={() => set('coat', coat.id)}><i style={swatch(coat.id)} />{t(coatNames[coat.id])}</button>)}</div>
  </>;
}

function Brain({ answers, set, data, onRefresh, onError }: StepProps & { data: Bootstrap; onRefresh: () => Promise<void>; onError: (message: string) => void }) {
  const { appleIntelligence: reason, chatgptAccount: account, chatgptPlan: plan } = data.diagnostics;
  const apple = appleStatus(reason);
  const auto = data.settings.brainAuto;
  const checking = reason === 'checking' || reason === undefined;
  const useChatGPT = () => { set('backend', 'cloud'); set('cloudProvider', 'chatgpt'); };
  // What the Mac and the sign-in now say moves the choice along, as the rule would.
  useEffect(() => {
    if (!auto) return;
    if (apple.ready) { if (answers.backend !== 'apple') set('backend', 'apple'); return; }
    if (plan === 'refused' && answers.backend !== 'mlx') set('backend', 'mlx');
    if (plan === 'usable' && answers.backend === 'apple') useChatGPT();
  }, [auto, apple.ready, plan]);
  const row = (icon: ReactNode, title: string, detail: string, ready: boolean) => <div className="brain-status">{icon}<span><strong>{title}</strong><small>{detail}</small></span>{ready && <Check size={14} />}</div>;
  if (!auto) return row(answers.backend === 'apple' ? <IntelligenceMark size={16} /> : answers.backend === 'mlx' ? <Cpu size={16} /> : <Cloud size={16} />, backendLabel[answers.backend], data.diagnostics.modelReady ? t('onboarding.brain.ready') : t('onboarding.brain.changeInSettings'), data.diagnostics.modelReady);
  if (apple.ready || checking) return row(<IntelligenceMark size={16} />, 'Apple Intelligence', apple.label, apple.ready);
  // The same local setup as Settings → Brain, reading and writing the answers instead.
  const local = { ...data.settings, pythonPath: answers.pythonPath, modelPath: answers.modelPath };
  const updateLocal = <K extends keyof Settings>(key: K, value: Settings[K]) => { if (key === 'pythonPath' || key === 'modelPath') set(key, value as string); };
  return <>
    {apple.advice && <div className="field-help">{apple.advice}</div>}
    <OpenAppleSettings reason={reason} onError={onError} />
    {plan === 'refused' && <div className="field-help">{t('onboarding.brain.planRefused')}</div>}
    {answers.backend === 'mlx'
      ? <>
        <LocalModel draft={local} update={updateLocal} onError={onError} testable={false} onStart={() => set('localStarted', true)} onStop={() => set('localStarted', false)} />
        {plan !== 'refused' && <button type="button" className="text-button" onClick={useChatGPT}>{t('onboarding.brain.signInInstead')}</button>}
      </>
      : <>
        <ChatGPTAccount account={account} onChange={onRefresh} onError={onError} />
        {account && plan === 'usable' && <CloudShares name="ChatGPT" shared={answers.shares} onChange={value => set('shares', value)} setup />}
        {!account && <LocalSetup onStart={() => { set('backend', 'mlx'); set('localStarted', true); }} onReady={(python, model) => { set('backend', 'mlx'); set('pythonPath', python); set('modelPath', model); }} onError={onError} />}
      </>}
  </>;
}

function Roam({ answers, set }: StepProps) {
  const choice = (key: 'roam' | 'awareness' | 'nudges', label: string, detail?: string, disabled = false) => <label className="notification-choice cloud-share"><input type="checkbox" checked={answers[key] && !disabled} disabled={disabled} onChange={event => set(key, event.target.checked)} /><span>{label}{detail && <small>{detail}</small>}</span></label>;
  return <>
    {choice('roam', t('onboarding.roam.wander'))}
    {choice('awareness', t('onboarding.roam.notice'), t('onboarding.roam.noticeDetail'))}
    {choice('nudges', t('onboarding.roam.offer'), undefined, !answers.awareness)}
  </>;
}
