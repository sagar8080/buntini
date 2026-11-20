import { useEffect, useState } from 'react';
import { BookOpen, Download, Plus, Trash2, X } from 'lucide-react';
import Toggle from '../Toggle';
import { command, desktop, errorMessage } from '../bridge';
import { listen } from '@tauri-apps/api/event';
import type { Settings } from '../types';
import { callFlow } from './api';
import { chordFrom, keyLabels, languageName, languageNames, type DictationKey, type FlowSettings as Flow, type FlowStatus, type ModelStatus } from './model';
import { number, t } from '../i18n.ts';
import './flow.css';

type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void };

const gigabytes = (bytes: number) => { const digits = bytes > 1e9 ? 1 : 2; return number(bytes / 1e9, { style: 'unit', unit: 'gigabyte', minimumFractionDigits: digits, maximumFractionDigits: digits }); };

/** Dictation anywhere, the speech model, the wake word, call notes and your words. */
export default function FlowSettings({ draft, update, onError }: Props) {
  const flow = draft.flow;
  const set = <K extends keyof Flow>(key: K, value: Flow[K]) => update('flow', { ...flow, [key]: value });
  const [status, setStatus] = useState<FlowStatus | null>(null);
  const [recording, setRecording] = useState(false);
  const [language, setLanguage] = useState('');

  useEffect(() => {
    let active = true;
    const load = () => callFlow<FlowStatus>({ op: 'status' }).then(next => { if (active) setStatus(next); }).catch(() => {});
    void load();
    if (!desktop) return () => { active = false; };
    const models = listen<ModelStatus>('flow-models', event => setStatus(current => current && { ...current, models: event.payload }));
    const timer = setInterval(() => void load(), 4000);
    return () => { active = false; clearInterval(timer); void models.then(stop => stop()); };
  }, [flow.enabled, flow.wakeWord]);

  useEffect(() => {
    if (!recording) return;
    const listener = (event: KeyboardEvent) => {
      event.preventDefault();
      if (event.key === 'Escape') { setRecording(false); return; }
      const chord = chordFrom(event);
      if (chord) { update('flow', { ...flow, key: 'chord', chord }); setRecording(false); }
    };
    window.addEventListener('keydown', listener, true);
    return () => window.removeEventListener('keydown', listener, true);
  }, [recording, flow, update]);

  const models = status?.models;
  const download = () => {
    setStatus(current => current && { ...current, models: { phase: 'downloading', doneBytes: 0, totalBytes: current.models.totalBytes, message: '' } });
    void callFlow({ op: 'downloadModels' }).catch(error => onError(errorMessage(error)));
  };
  const remove = () => void callFlow<null>({ op: 'removeModels' }).then(() => callFlow<FlowStatus>({ op: 'status' })).then(setStatus).catch(error => onError(errorMessage(error)));
  const percent = models && models.totalBytes ? Math.round(models.doneBytes * 100 / models.totalBytes) : 0;
  const choose = (key: DictationKey) => key === 'chord' ? setRecording(true) : update('flow', { ...flow, key });

  return <>
    <section className="settings-section flow-settings">
      <div className="settings-section-heading"><h3>{t('settings.flow.title')}</h3></div>
      <Toggle label={t('settings.flow.dictateAnywhere')} detail={flow.enabled && status && !status.accessibility ? t('settings.flow.needsAccessibility') : undefined} checked={flow.enabled} onChange={value => set('enabled', value)} />
      {flow.enabled && status && !status.accessibility && <div className="row-actions"><button type="button" className="secondary-button" onClick={() => void command('accessibility', { ask: true }).catch(error => onError(errorMessage(error)))}>{t('settings.flow.allowAccessibility')}</button></div>}
      <div className="setting-label">{t('settings.flow.hold')}</div>
      <div className="chip-row" role="radiogroup" aria-label={t('settings.flow.key')}>
        {(Object.keys(keyLabels) as (keyof typeof keyLabels)[]).map(key => <button type="button" key={key} role="radio" aria-checked={flow.key === key} className={flow.key === key ? 'active' : ''} onClick={() => choose(key)}>{keyLabels[key]}</button>)}
        <button type="button" role="radio" aria-checked={flow.key === 'chord'} className={flow.key === 'chord' || recording ? 'active' : ''} onClick={() => choose('chord')}>{recording ? t('settings.flow.pressKeys') : flow.chord?.label ?? t('settings.flow.yourOwn')}</button>
      </div>
      <div className="setting-label">{t('settings.flow.languages')}</div>
      <div className="flow-chips">
        {flow.languages.map(tag => <span className="flow-chip" key={tag}>{languageName(tag)}<button type="button" aria-label={t('common.removeItem', { item: languageName(tag) })} onClick={() => set('languages', flow.languages.filter(item => item !== tag))}><X size={11} /></button></span>)}
        {flow.languages.length < 3 && <select aria-label={t('settings.flow.addLanguage')} value={language} onChange={event => { const tag = event.target.value; setLanguage(''); if (tag && !flow.languages.includes(tag)) set('languages', [...flow.languages, tag]); }}>
          <option value="">{flow.languages.length ? t('common.add') : t('settings.flow.macLanguage')}</option>
          {Object.keys(languageNames).filter(tag => !flow.languages.includes(tag)).map(tag => <option key={tag} value={tag}>{languageName(tag)}</option>)}
        </select>}
      </div>
      <Toggle label={t('settings.flow.mute')} checked={flow.mute} onChange={value => set('mute', value)} />
      <Toggle label="Hey Buntini" checked={flow.wakeWord} onChange={value => set('wakeWord', value)} />
      <Toggle label={t('settings.flow.recordCalls')} checked={flow.recordCalls} onChange={value => set('recordCalls', value)} />
      <div className="setting-label">{t('settings.flow.speechModel')}</div>
      {models?.phase === 'ready'
        ? <div className="flow-model"><span>Parakeet</span><button type="button" className="icon-button" aria-label={t('settings.flow.removeModel')} title={t('common.remove')} onClick={remove}><Trash2 size={14} /></button></div>
        : models && (models.phase === 'downloading' || models.phase === 'waiting')
          ? <div className="flow-model"><div className="local-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={percent} aria-label={t('settings.flow.speechModel')}><i style={{ width: `${percent}%` }} /></div><span>{number(percent / 100, { style: 'percent' })}</span><button type="button" className="icon-button" aria-label={t('common.cancel')} title={t('common.cancel')} onClick={() => void callFlow({ op: 'cancelDownload' })}><X size={14} /></button></div>
          : <div className="flow-model"><button type="button" className="secondary-button" disabled={!desktop} onClick={download}><Download size={14} /> Parakeet · {gigabytes(models?.totalBytes ?? 654832571)}</button>{models?.phase === 'failed' && <span className="flow-problem">{models.message}</span>}</div>}
      <div className="row-actions"><button type="button" className="secondary-button" disabled={!desktop} onClick={() => void callFlow({ op: 'openLibrary' }).catch(error => onError(errorMessage(error)))}><BookOpen size={14} /> {t('library.title')}</button></div>
    </section>
    <section className="settings-section flow-settings">
      <div className="settings-section-heading"><h3>{t('settings.flow.yourWords')}</h3></div>
      <WordList label={t('settings.flow.dictionary')} items={flow.dictionary} placeholder="Buntini" onChange={items => set('dictionary', items)} />
      <PairList label={t('settings.flow.replacements')} items={flow.replacements.map(item => [item.from, item.to])} placeholders={[t('settings.flow.replacementExample'), 'sam@example.com']} onChange={items => set('replacements', items.map(([from, to]) => ({ from, to })))} />
      <PairList label={t('settings.flow.snippets')} long items={flow.snippets.map(item => [item.trigger, item.text])} placeholders={[t('settings.flow.snippetExample'), '1 Infinite Loop, Cupertino']} onChange={items => set('snippets', items.map(([trigger, text]) => ({ trigger, text })))} />
    </section>
  </>;
}

function WordList({ label, items, placeholder, onChange }: { label: string; items: string[]; placeholder: string; onChange: (items: string[]) => void }) {
  const [word, setWord] = useState('');
  const add = () => { const value = word.trim(); if (value && !items.includes(value)) onChange([...items, value]); setWord(''); };
  return <>
    <div className="setting-label">{label}</div>
    <div className="flow-chips">
      {items.map(item => <span className="flow-chip" key={item}>{item}<button type="button" aria-label={t('common.removeItem', { item })} onClick={() => onChange(items.filter(other => other !== item))}><X size={11} /></button></span>)}
      <form className="flow-add" onSubmit={event => { event.preventDefault(); add(); }}><input aria-label={t('settings.flow.addTo', { list: label })} value={word} placeholder={placeholder} maxLength={64} onChange={event => setWord(event.target.value)} /><button type="submit" className="icon-button" aria-label={t('common.add')} disabled={!word.trim()}><Plus size={13} /></button></form>
    </div>
  </>;
}

function PairList({ label, items, placeholders, long, onChange }: { label: string; items: [string, string][]; placeholders: [string, string]; long?: boolean; onChange: (items: [string, string][]) => void }) {
  const [pair, setPair] = useState<[string, string]>(['', '']);
  const ready = pair[0].trim() && pair[1].trim();
  return <>
    <div className="setting-label">{label}</div>
    {items.map(([from, to], index) => <div className="flow-pair" key={`${from}-${index}`}><span>{from}</span><span className="flow-arrow">→</span><span className="flow-pair-to" title={to}>{to}</span><button type="button" className="icon-button" aria-label={t('common.removeItem', { item: from })} onClick={() => onChange(items.filter((_, other) => other !== index))}><X size={12} /></button></div>)}
    <form className="flow-pair flow-pair-new" onSubmit={event => { event.preventDefault(); if (ready) { onChange([...items, [pair[0].trim(), pair[1]]]); setPair(['', '']); } }}>
      <input aria-label={t('settings.flow.whatYouSay', { list: label })} value={pair[0]} placeholder={placeholders[0]} maxLength={64} onChange={event => setPair([event.target.value, pair[1]])} />
      <span className="flow-arrow">→</span>
      {long
        ? <textarea aria-label={t('settings.flow.whatsTyped', { list: label })} value={pair[1]} placeholder={placeholders[1]} maxLength={4000} rows={2} onChange={event => setPair([pair[0], event.target.value])} />
        : <input aria-label={t('settings.flow.whatsTyped', { list: label })} value={pair[1]} placeholder={placeholders[1]} maxLength={1000} onChange={event => setPair([pair[0], event.target.value])} />}
      <button type="submit" className="icon-button" aria-label={t('common.add')} disabled={!ready}><Plus size={13} /></button>
    </form>
  </>;
}
