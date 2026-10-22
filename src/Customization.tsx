import { useEffect, useState } from 'react';
import { AudioLines, ChevronRight, LoaderCircle, Moon, Laptop, Shuffle, Sparkles, Sun } from 'lucide-react';
import Pet from './Pet';
import Toggle from './Toggle';
import { command, desktop, errorMessage } from './bridge';
import { accessories, characters, coats, eyeColors, eyes, lookFrom, motions } from './petOptions';
import { accentChoices, accentHex, idleStatus, isHex, presets } from './themes.ts';
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
  // A labelled row of choices for one setting.
  const chips = <K extends keyof Settings>(label: string, key: K, options: [Settings[K], string][]) => <>
    <div className="setting-label">{label}</div>
    <div className="chip-row" role="radiogroup" aria-label={label}>{options.map(([value, text]) => <button type="button" key={String(value)} role="radio" aria-checked={draft[key] === value} className={draft[key] === value ? 'active' : ''} onClick={() => update(key, value)}>{text}</button>)}</div>
  </>;
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
      <div className="settings-section-heading"><h3>Theme</h3></div>
      <div className="theme-presets" role="radiogroup" aria-label="Theme">{presets.map(item => <button type="button" key={item.id} role="radio" aria-checked={draft.themePreset === item.id} className={draft.themePreset === item.id ? 'active' : ''} onClick={() => update('themePreset', item.id)}>
        <span className="theme-swatch"><i style={{ background: item.light.panel }} /><i style={{ background: item.dark.panel }} /><b style={{ background: item.dark.accent }} /></span>{item.label}
      </button>)}</div>
      <div className="chip-row" aria-label="Appearance">{[{ id: 'light', icon: Sun, label: 'Light' }, { id: 'dark', icon: Moon, label: 'Dark' }, { id: 'system', icon: Laptop, label: 'Auto' }].map(item => <button key={item.id} type="button" aria-pressed={draft.theme === item.id} className={draft.theme === item.id ? 'active' : ''} onClick={() => update('theme', item.id as Settings['theme'])}><item.icon size={13} />{item.label}</button>)}</div>
      <div className="chip-row" aria-label="Accent color">{accentChoices.map(item => <button key={item.id} type="button" aria-pressed={draft.accent === item.id} className={draft.accent === item.id ? 'active' : ''} onClick={() => { update('accent', item.id); if (item.id === 'custom' && !isHex(draft.accentColor)) update('accentColor', accentHex(draft, true)); }}>
        <i style={{ background: item.id === 'custom' ? (isHex(draft.accentColor) ? draft.accentColor : 'conic-gradient(#e9985d, #bc92df, #71bacb, #9db98b, #e9985d)') : accentHex({ ...draft, accent: item.id }, true) }} />{item.label}
      </button>)}</div>
      {draft.accent === 'custom' && <label className="inline-field"><span>Custom colour</span><input type="color" aria-label="Custom accent colour" value={isHex(draft.accentColor) ? draft.accentColor : '#e9985d'} onChange={event => update('accentColor', event.target.value)} /></label>}
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Chat</h3></div>
      {chips('Text size', 'textSize', [['small', 'Small'], ['medium', 'Medium'], ['large', 'Large']])}
      {chips('Spacing', 'density', [['compact', 'Compact'], ['comfortable', 'Roomy']])}
      {chips('Font', 'chatFont', [['sans', 'Default'], ['rounded', 'Rounded'], ['serif', 'Serif'], ['mono', 'Mono']])}
      {chips('Corners', 'corners', [['round', 'Round'], ['soft', 'Soft'], ['square', 'Square']])}
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Panel</h3></div>
      {chips('Bubble size', 'panelSize', [['compact', 'Compact'], ['standard', 'Standard'], ['large', 'Large']])}
      <div className="setting-label">Cat size</div>
      <div className="chip-row" aria-label="Cat size">{[{ value: .8, label: 'Small' }, { value: 1, label: 'Medium' }, { value: 1.2, label: 'Large' }].map(item => <button type="button" key={item.label} aria-pressed={draft.companionScale === item.value} className={draft.companionScale === item.value ? 'active' : ''} onClick={() => update('companionScale', item.value)}>{item.label}</button>)}</div>
      <label className="field">Status line<input value={draft.statusText} onChange={event => update('statusText', event.target.value)} maxLength={40} placeholder={idleStatus({ ...draft, statusText: '' }, false, new Date().getHours())} aria-label="Status line" /></label>
      <p className="field-help">Shown next to my name when I’m not busy. Leave it empty and I’ll pick one.</p>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Motion</h3></div>
      <div className="chip-row" aria-label="Liveliness">{motions.map(item => <button type="button" key={item.id} aria-pressed={draft.petMotion === item.id} className={draft.petMotion === item.id ? 'active' : ''} onClick={() => update('petMotion', item.id)} title={item.detail}>{item.label}</button>)}</div>
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
      <Toggle label="Morning brief" detail="Today’s calendar and reminders when you first open me" checked={draft.morningBrief} onChange={value => update('morningBrief', value)} />
      <Toggle label="Desktop notifications" detail={notificationStatus === 'denied' ? 'Turned off in System Settings' : undefined} checked={draft.notificationsEnabled} onChange={value => void notifications(value)} />
    </section>
    <button type="button" className="audit-button" onClick={onRestartSetup}><Sparkles size={14} /><span>Run setup again</span><ChevronRight size={14} /></button>
  </>;
}
