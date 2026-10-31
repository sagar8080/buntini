import { useEffect, useRef, useState } from 'react';
import { AudioLines, ChevronRight, LoaderCircle, Moon, Laptop, Shuffle, Sparkles, Sun } from 'lucide-react';
import Pet from './Pet';
import Toggle from './Toggle';
import { command, desktop, errorMessage } from './bridge';
import { accessories, characters, coats, eyeColors, eyes, lookFrom } from './petOptions';
import { presets } from './themes.ts';
import type { BackgroundStatus, LoginStatus, Settings, Voice } from './types';

const pick = <T,>(items: { id: T }[]) => items[Math.floor(Math.random() * items.length)].id;
// A personality also sets how lively the cat is, so there's no separate motion choice.
const personalities: { label: string; text: string; motion: Settings['petMotion'] }[] = [
  { label: 'Companion', text: 'Warm, concise, practical, and honest.', motion: 'playful' },
  { label: 'Curious', text: 'Curious and playful. Use clear examples and gentle humor.', motion: 'playful' },
  { label: 'Focused', text: 'Direct and focused. Give short, actionable answers.', motion: 'calm' },
];
// Text size brings the panel along, so larger text never sits in a cramped bubble.
const sizes: [Settings['textSize'], Settings['panelSize'], string][] = [['small', 'compact', 'Small'], ['medium', 'standard', 'Medium'], ['large', 'large', 'Large']];
type Props = { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void; onPreviewVoice: (voiceId: string, rate: number, text: string) => Promise<void>; onRestartSetup: () => void };

/** The Buntini tab: how it looks, sounds and behaves, with a few good choices for each. */
export default function Customization({ draft, update, onError, onPreviewVoice, onRestartSetup }: Props) {
  const [voices, setVoices] = useState<Voice[]>([]);
  const [notificationStatus, setNotificationStatus] = useState('unknown');
  const [loginStatus, setLoginStatus] = useState<LoginStatus | null>(desktop ? null : 'unavailable');
  const firstLoginCheck = useRef(true);
  const [previewing, setPreviewing] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    void command<Voice[]>('list_voices').then(value => { if (active) setVoices(value); }).catch(() => {});
    void command<string>('notification_permission_status').then(value => { if (active) setNotificationStatus(value); }).catch(() => {});
    return () => { active = false; };
  }, []);
  // Re-read what macOS says once the debounced save has applied the switch.
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    if (!firstLoginCheck.current) setLoginStatus(null);
    const id = window.setTimeout(() => { void command<BackgroundStatus>('background_status').then(value => { if (active && value.openAtLogin) setLoginStatus(value.openAtLogin); }).catch(() => {}); }, firstLoginCheck.current ? 0 : 1500);
    firstLoginCheck.current = false;
    return () => { active = false; window.clearTimeout(id); };
  }, [draft.openAtLogin]);
  const look = lookFrom(draft);
  // A labelled row of choices for one setting.
  const chips = <K extends keyof Settings>(label: string, key: K, options: [Settings[K], string][]) => <>
    <div className="setting-label">{label}</div>
    <div className="chip-row" role="radiogroup" aria-label={label}>{options.map(([value, text]) => <button type="button" key={String(value)} role="radio" aria-checked={draft[key] === value} className={draft[key] === value ? 'active' : ''} onClick={() => update(key, value)}>{text}</button>)}</div>
  </>;
  const preset = personalities.find(item => item.text === draft.personality);
  // A theme sets the whole look. Accents, fonts, corners and spacing chosen in earlier
  // versions keep showing until a theme is picked.
  const theme = (id: Settings['themePreset']) => { update('themePreset', id); update('accent', 'preset'); update('chatFont', 'sans'); update('corners', 'round'); update('density', 'comfortable'); };
  // Buntini words its own status line to suit the personality.
  const personality = (item: typeof personalities[number]) => { update('personality', item.text); update('petMotion', item.motion); update('statusText', ''); };
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
      <div className="theme-presets" role="radiogroup" aria-label="Theme">{presets.map(item => <button type="button" key={item.id} role="radio" aria-checked={draft.themePreset === item.id} className={draft.themePreset === item.id ? 'active' : ''} onClick={() => theme(item.id)}>
        <span className="theme-swatch"><i style={{ background: item.light.panel }} /><i style={{ background: item.dark.panel }} /><b style={{ background: item.dark.accent }} /></span>{item.label}
      </button>)}</div>
      <div className="chip-row" aria-label="Appearance">{[{ id: 'light', icon: Sun, label: 'Light' }, { id: 'dark', icon: Moon, label: 'Dark' }, { id: 'system', icon: Laptop, label: 'Auto' }].map(item => <button key={item.id} type="button" aria-pressed={draft.theme === item.id} className={draft.theme === item.id ? 'active' : ''} onClick={() => update('theme', item.id as Settings['theme'])}><item.icon size={13} />{item.label}</button>)}</div>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Size and motion</h3></div>
      <div className="setting-label">Text size</div>
      <div className="chip-row" role="radiogroup" aria-label="Text size">{sizes.map(([text, panel, label]) => <button type="button" key={text} role="radio" aria-checked={draft.textSize === text} className={draft.textSize === text ? 'active' : ''} onClick={() => { update('textSize', text); update('panelSize', panel); }}>{label}</button>)}</div>
      {chips('Cat size', 'companionScale', [[.8, 'Small'], [1, 'Medium'], [1.2, 'Large']])}
      <Toggle label="Less motion" checked={draft.reducedMotion} onChange={value => update('reducedMotion', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Personality</h3></div>
      <div className="chip-row" aria-label="Personality">{personalities.map(item => <button type="button" key={item.label} aria-pressed={preset === item} className={preset === item ? 'active' : ''} onClick={() => personality(item)}>{item.label}</button>)}</div>
      <details className="disclosure" open={!preset}><summary>Write your own</summary><textarea aria-label="Personality" value={draft.personality} onChange={event => update('personality', event.target.value)} maxLength={1200} rows={3} /></details>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Voice</h3></div>
      <div className="voice-row"><select aria-label="Speaking voice" value={draft.voiceId} onChange={event => update('voiceId', event.target.value)}><option value="">System voice</option>{voices.map(voice => <option key={voice.id} value={voice.id}>{voice.name.replace(/ \((Premium|Enhanced)\)$/, '')} · {voice.language}</option>)}{draft.voiceId && voices.length > 0 && !voices.some(voice => voice.id === draft.voiceId) && <option value={draft.voiceId}>{draft.voiceId}</option>}</select>
        <button type="button" className="icon-button" aria-label="Try this voice" title="Try this voice" disabled={previewing || !desktop} onClick={async () => { setPreviewing(true); try { await onPreviewVoice(draft.voiceId, draft.voiceRate, `Hi, I'm ${draft.assistantName || 'your companion'}.`); } catch (error) { onError(errorMessage(error)); } finally { setPreviewing(false); } }}>{previewing ? <LoaderCircle size={14} className="spin" /> : <AudioLines size={14} />}</button></div>
      <label className="range-row"><span>Pace</span><input type="range" min="100" max="300" step="10" value={draft.voiceRate} onChange={event => update('voiceRate', Number(event.target.value))} aria-label="Speaking pace" /><span>{draft.voiceRate}</span></label>
      <Toggle label="Answer out loud when I talk" checked={draft.speakVoiceReplies || draft.speakResponses} disabled={draft.speakResponses} onChange={value => update('speakVoiceReplies', value)} />
      <Toggle label="Read every reply aloud" checked={draft.speakResponses} onChange={value => update('speakResponses', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>Out and about</h3></div>
      <Toggle label="Wander around the screen" checked={draft.petRoam} onChange={value => update('petRoam', value)} />
      <Toggle label="Offer a hand now and then" detail={draft.petAwareness ? undefined : 'Needs “Notice what I’m doing” in Privacy'} disabled={!draft.petAwareness} checked={draft.petNudges && draft.petAwareness} onChange={value => update('petNudges', value)} />
      <Toggle label="Morning brief" detail="Today’s calendar and reminders when you first open me" checked={draft.morningBrief} onChange={value => update('morningBrief', value)} />
      <Toggle label="Heads-ups" detail="Before a meeting, when reminders are past due and when someone’s waiting on a reply" checked={draft.headsUps} onChange={value => { update('headsUps', value); if (value && !draft.notificationsEnabled) void notifications(true); }} />
      <Toggle label="Desktop notifications" detail={notificationStatus === 'denied' ? 'Turned off in System Settings' : notificationStatus === 'unavailable' ? 'Works in the installed app' : undefined} checked={draft.notificationsEnabled} onChange={value => void notifications(value)} />
      <Toggle label="Open at login" detail={loginStatus === 'unavailable' ? 'Works in the installed app' : loginStatus === 'requiresApproval' ? 'Allow me in System Settings → General → Login Items' : draft.openAtLogin && loginStatus && loginStatus !== 'enabled' ? 'Turned off in System Settings' : undefined} disabled={loginStatus === 'unavailable' && !draft.openAtLogin} checked={draft.openAtLogin} onChange={value => update('openAtLogin', value)} />
    </section>
    <button type="button" className="audit-button" onClick={onRestartSetup}><Sparkles size={14} /><span>Run setup again</span><ChevronRight size={14} /></button>
  </>;
}
