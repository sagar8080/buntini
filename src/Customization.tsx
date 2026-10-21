import { useEffect, useState } from 'react';
import { AudioLines, ChevronRight, LoaderCircle, Moon, Laptop, Shuffle, Sparkles, Sun } from 'lucide-react';
import Pet from './Pet';
import Toggle from './Toggle';
import { command, desktop, errorMessage } from './bridge';
import { accents, accessories, characters, coats, eyeColors, eyes, lookFrom, motions } from './petOptions';
import type { Settings, Voice } from './types';

const pick = <T,>(items: { id: T }[]) => items[Math.floor(Math.random() * items.length)].id;
const personalities = [
  { label: 'Companion', text: 'Warm, concise, practical, and honest.' },
  { label: 'Curious', text: 'Curious and playful. Use clear examples and gentle humor.' },
  { label: 'Focused', text: 'Direct and focused. Give short, actionable answers.' },
];
type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void; onPreviewVoice: (voiceId: string, rate: number, text: string) => Promise<void>; onRestartSetup: () => void };

/** The Buntini tab: how it looks, sounds and behaves. */
export default function Customization({ draft, update, onError, onPreviewVoice, onRestartSetup }: Props) {
  const [voices, setVoices] = useState<Voice[]>([]);
  const [notificationStatus, setNotificationStatus] = useState('unknown');
  const [previewing, setPreviewing] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    void command<Voice[]>('list_voices').then(value => { if (active) setVoices(value); }).catch(() => {});
    void command<string>('notification_permission_status').then(value => { if (active) setNotificationStatus(value); }).catch(() => {});
    return () => { active = false; };
  }, []);
  const look = lookFrom(draft);
  const preset = personalities.find(item => item.text === draft.personality);
  async function notifications(on: boolean) {
    update('notificationsEnabled', on);
    if (!on || notificationStatus === 'authorized' || !desktop) return;
    try {
      const allowed = await command<boolean>('request_notification_permission');
      setNotificationStatus(allowed ? 'authorized' : 'denied');
      if (!allowed) onError('Notifications are off for Buntini in System Settings → Notifications.');
    } catch (error) { onError(errorMessage(error)); }
  }
  return <>
    <section className="settings-section">
      <div className="profile-card"><Pet avatar={draft.avatar} look={look} state="idle" alive /><label className="profile-name"><span>Name</span><input value={draft.assistantName} onChange={event => update('assistantName', event.target.value)} maxLength={40} required aria-label="Assistant name" /></label></div>
      <div className="character-options" aria-label="Character">{characters.map(item => <button type="button" key={item.id} aria-pressed={draft.avatar === item.id} className={draft.avatar === item.id ? 'active' : ''} onClick={() => update('avatar', item.id)}><Pet avatar={item.id} look={look} state="idle" /><span>{item.label}</span></button>)}</div>
    </section>
    {draft.avatar === 'cat' && <section className="settings-section">
      <div className="settings-section-heading"><h3>Look</h3><button type="button" className="surprise-button" onClick={() => { update('catCoat', pick(coats)); update('catEyes', pick(eyes)); update('catAccessory', pick(accessories)); }}><Shuffle size={12} />Surprise me</button></div>
      <div className="pet-swatches" aria-label="Coat">{coats.map(item => <button type="button" key={item.id} aria-pressed={draft.catCoat === item.id} className={draft.catCoat === item.id ? 'active' : ''} onClick={() => update('catCoat', item.id)} title={item.label}><Pet look={{ ...look, coat: item.id, accessory: 'none' }} state="idle" /><span>{item.label}</span></button>)}</div>
      <div className="chip-row" aria-label="Eye color">{eyes.map(item => <button type="button" key={item.id} aria-pressed={draft.catEyes === item.id} className={draft.catEyes === item.id ? 'active' : ''} onClick={() => update('catEyes', item.id)}><i style={{ background: item.id === 'odd' ? `linear-gradient(90deg, ${eyeColors.blue} 50%, ${eyeColors.amber} 50%)` : eyeColors[item.id] }} />{item.label}</button>)}</div>
      <div className="pet-swatches" aria-label="Accessory">{accessories.map(item => <button type="button" key={item.id} aria-pressed={draft.catAccessory === item.id} className={draft.catAccessory === item.id ? 'active' : ''} onClick={() => update('catAccessory', item.id)} title={item.label}><Pet look={{ ...look, accessory: item.id }} state="idle" /><span>{item.label}</span></button>)}</div>
    </section>}
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Style</h3></div>
      <div className="chip-row" aria-label="Accent color">{accents.map(item => <button key={item.id} type="button" aria-pressed={draft.accent === item.id} className={draft.accent === item.id ? 'active' : ''} onClick={() => update('accent', item.id)}><i style={{ background: item.color }} />{item.label}</button>)}</div>
      <div className="chip-row" aria-label="Appearance">{[{ id: 'light', icon: Sun, label: 'Light' }, { id: 'dark', icon: Moon, label: 'Dark' }, { id: 'system', icon: Laptop, label: 'Auto' }].map(item => <button key={item.id} type="button" aria-pressed={draft.theme === item.id} className={draft.theme === item.id ? 'active' : ''} onClick={() => update('theme', item.id as Settings['theme'])}><item.icon size={13} />{item.label}</button>)}</div>
      <div className="chip-row" aria-label="Liveliness">{motions.map(item => <button type="button" key={item.id} aria-pressed={draft.petMotion === item.id} className={draft.petMotion === item.id ? 'active' : ''} onClick={() => update('petMotion', item.id)} title={item.detail}>{item.label}</button>)}</div>
      <div className="chip-row" aria-label="Size">{[{ value: .8, label: 'Small' }, { value: 1, label: 'Medium' }, { value: 1.2, label: 'Large' }].map(item => <button type="button" key={item.label} aria-pressed={draft.companionScale === item.value} className={draft.companionScale === item.value ? 'active' : ''} onClick={() => update('companionScale', item.value)}>{item.label}</button>)}</div>
      <Toggle label="Less motion" checked={draft.reducedMotion} onChange={value => update('reducedMotion', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Personality</h3></div>
      <div className="chip-row" aria-label="Personality">{personalities.map(item => <button type="button" key={item.label} aria-pressed={preset === item} className={preset === item ? 'active' : ''} onClick={() => update('personality', item.text)}>{item.label}</button>)}</div>
      <details className="disclosure" open={!preset}><summary>Write your own</summary><textarea aria-label="Personality" value={draft.personality} onChange={event => update('personality', event.target.value)} maxLength={1200} rows={3} /></details>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Voice</h3></div>
      <div className="voice-row"><select aria-label="Speaking voice" value={draft.voiceId} onChange={event => update('voiceId', event.target.value)}><option value="">System voice</option>{voices.map(voice => <option key={voice.id} value={voice.id}>{voice.name} · {voice.language}</option>)}</select>
        <button type="button" className="icon-button" aria-label="Try this voice" title="Try this voice" disabled={previewing || !desktop} onClick={async () => { setPreviewing(true); try { await onPreviewVoice(draft.voiceId, draft.voiceRate, `Hi, I'm ${draft.assistantName || 'your companion'}.`); } catch (error) { onError(errorMessage(error)); } finally { setPreviewing(false); } }}>{previewing ? <LoaderCircle size={14} className="spin" /> : <AudioLines size={14} />}</button></div>
      <label className="range-row"><span>Pace</span><input type="range" min="100" max="300" step="10" value={draft.voiceRate} onChange={event => update('voiceRate', Number(event.target.value))} aria-label="Speaking pace" /><span>{draft.voiceRate}</span></label>
      <Toggle label="Read replies aloud" checked={draft.speakResponses} onChange={value => update('speakResponses', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Out and about</h3></div>
      <Toggle label="Wander around the screen" checked={draft.petRoam} onChange={value => update('petRoam', value)} />
      <Toggle label="Offer a hand now and then" detail={draft.petAwareness ? undefined : 'Needs “Notice what I’m doing” in Privacy'} disabled={!draft.petAwareness} checked={draft.petNudges && draft.petAwareness} onChange={value => update('petNudges', value)} />
      <Toggle label="Desktop notifications" detail={notificationStatus === 'denied' ? 'Turned off in System Settings' : undefined} checked={draft.notificationsEnabled} onChange={value => void notifications(value)} />
    </section>
    <button type="button" className="audit-button" onClick={onRestartSetup}><Sparkles size={14} /><span>Run setup again</span><ChevronRight size={14} /></button>
  </>;
}
