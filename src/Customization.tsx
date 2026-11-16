import { useEffect, useRef, useState } from 'react';
import { AudioLines, ChevronRight, LoaderCircle, Moon, Laptop, Shuffle, Sparkles, Sun } from 'lucide-react';
import Pet from './Pet';
import Toggle from './Toggle';
import { command, desktop, errorMessage } from './bridge';
import { accessories, characters, coats, eyeColors, eyes, lookFrom } from './petOptions';
import { presets } from './themes.ts';
import { languages, number, t } from './i18n.ts';
import { follow } from './locales';
import type { BackgroundStatus, LoginStatus, Settings, Voice } from './types';

const pick = <T,>(items: { id: T }[]) => items[Math.floor(Math.random() * items.length)].id;
// A personality also sets how lively the cat is, so there's no separate motion choice. Its
// `text` is the model's instruction and is matched against saved settings, so it stays English.
const personalities: { labelKey: string; text: string; motion: Settings['petMotion'] }[] = [
  { labelKey: 'settings.personality.companion', text: 'Warm, concise, practical, and honest.', motion: 'playful' },
  { labelKey: 'settings.personality.curious', text: 'Curious and playful. Use clear examples and gentle humor.', motion: 'playful' },
  { labelKey: 'settings.personality.focused', text: 'Direct and focused. Give short, actionable answers.', motion: 'calm' },
];
// Text size brings the panel along, so larger text never sits in a cramped bubble.
const sizes: [Settings['textSize'], Settings['panelSize'], string][] = [['small', 'compact', 'settings.size.small'], ['medium', 'standard', 'settings.size.medium'], ['large', 'large', 'settings.size.large']];
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
      if (!allowed) onError(t('settings.outAbout.notificationsOff'));
    } catch (error) { onError(errorMessage(error)); }
  }
  return <>
    <section className="settings-section">
      <div className="profile-card"><Pet avatar={draft.avatar} look={look} state="idle" alive /><label className="profile-name"><span>{t('settings.name.label')}</span><input value={draft.assistantName} onChange={event => update('assistantName', event.target.value)} maxLength={40} required aria-label={t('settings.name.aria')} /></label></div>
      <div className="character-options" aria-label={t('settings.character')}>{characters.map(item => <button type="button" key={item.id} aria-pressed={draft.avatar === item.id} className={draft.avatar === item.id ? 'active' : ''} onClick={() => update('avatar', item.id)}><Pet avatar={item.id} look={look} state="idle" /><span>{item.label}</span></button>)}</div>
    </section>
    {draft.avatar === 'cat' && <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.look.title')}</h3><button type="button" className="surprise-button" onClick={() => { update('catCoat', pick(coats)); update('catEyes', pick(eyes)); update('catAccessory', pick(accessories)); }}><Shuffle size={12} />{t('settings.look.surprise')}</button></div>
      <div className="pet-swatches" aria-label={t('surface.wardrobe.coat')}>{coats.map(item => <button type="button" key={item.id} aria-pressed={draft.catCoat === item.id} className={draft.catCoat === item.id ? 'active' : ''} onClick={() => update('catCoat', item.id)} title={item.label}><Pet look={{ ...look, coat: item.id, accessory: 'none' }} state="idle" /><span>{item.label}</span></button>)}</div>
      <div className="chip-row" aria-label={t('settings.look.eyeColor')}>{eyes.map(item => <button type="button" key={item.id} aria-pressed={draft.catEyes === item.id} className={draft.catEyes === item.id ? 'active' : ''} onClick={() => update('catEyes', item.id)}><i style={{ background: item.id === 'odd' ? `linear-gradient(90deg, ${eyeColors.blue} 50%, ${eyeColors.amber} 50%)` : eyeColors[item.id] }} />{item.label}</button>)}</div>
      <div className="pet-swatches" aria-label={t('surface.wardrobe.accessory')}>{accessories.map(item => <button type="button" key={item.id} aria-pressed={draft.catAccessory === item.id} className={draft.catAccessory === item.id ? 'active' : ''} onClick={() => update('catAccessory', item.id)} title={item.label}><Pet look={{ ...look, accessory: item.id }} state="idle" /><span>{item.label}</span></button>)}</div>
    </section>}
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.theme.title')}</h3></div>
      <div className="theme-presets" role="radiogroup" aria-label={t('settings.theme.title')}>{presets.map(item => <button type="button" key={item.id} role="radio" aria-checked={draft.themePreset === item.id} className={draft.themePreset === item.id ? 'active' : ''} onClick={() => theme(item.id)}>
        <span className="theme-swatch"><i style={{ background: item.light.panel }} /><i style={{ background: item.dark.panel }} /><b style={{ background: item.dark.accent }} /></span>{item.label}
      </button>)}</div>
      <div className="chip-row" aria-label={t('settings.appearance.title')}>{[{ id: 'light', icon: Sun, label: t('settings.appearance.light') }, { id: 'dark', icon: Moon, label: t('settings.appearance.dark') }, { id: 'system', icon: Laptop, label: t('settings.appearance.auto') }].map(item => <button key={item.id} type="button" aria-pressed={draft.theme === item.id} className={draft.theme === item.id ? 'active' : ''} onClick={() => update('theme', item.id as Settings['theme'])}><item.icon size={13} />{item.label}</button>)}</div>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.sizeMotion')}</h3></div>
      <div className="setting-label">{t('settings.textSize')}</div>
      <div className="chip-row" role="radiogroup" aria-label={t('settings.textSize')}>{sizes.map(([text, panel, labelKey]) => <button type="button" key={text} role="radio" aria-checked={draft.textSize === text} className={draft.textSize === text ? 'active' : ''} onClick={() => { update('textSize', text); update('panelSize', panel); }}>{t(labelKey)}</button>)}</div>
      {chips(t('settings.catSize'), 'companionScale', [[.8, t('settings.size.small')], [1, t('settings.size.medium')], [1.2, t('settings.size.large')]])}
      <Toggle label={t('settings.lessMotion')} checked={draft.reducedMotion} onChange={value => update('reducedMotion', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.personality.title')}</h3></div>
      <div className="chip-row" aria-label={t('settings.personality.title')}>{personalities.map(item => <button type="button" key={item.labelKey} aria-pressed={preset === item} className={preset === item ? 'active' : ''} onClick={() => personality(item)}>{t(item.labelKey)}</button>)}</div>
      <details className="disclosure" open={!preset}><summary>{t('settings.personality.custom')}</summary><textarea aria-label={t('settings.personality.title')} value={draft.personality} onChange={event => update('personality', event.target.value)} maxLength={1200} rows={3} /></details>
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.voice.title')}</h3></div>
      <div className="voice-row"><select aria-label={t('settings.voice.speaking')} value={draft.voiceId} onChange={event => update('voiceId', event.target.value)}><option value="">{t('settings.voice.system')}</option>{voices.map(voice => <option key={voice.id} value={voice.id}>{voice.name} · {voice.language}</option>)}{draft.voiceId && voices.length > 0 && !voices.some(voice => voice.id === draft.voiceId) && <option value={draft.voiceId}>{draft.voiceId}</option>}</select>
        <button type="button" className="icon-button" aria-label={t('settings.voice.try')} title={t('settings.voice.try')} disabled={previewing || !desktop} onClick={async () => { setPreviewing(true); try { await onPreviewVoice(draft.voiceId, draft.voiceRate, draft.assistantName ? t('settings.voice.sample', { name: draft.assistantName }) : t('settings.voice.sampleUnnamed')); } catch (error) { onError(errorMessage(error)); } finally { setPreviewing(false); } }}>{previewing ? <LoaderCircle size={14} className="spin" /> : <AudioLines size={14} />}</button></div>
      <label className="range-row"><span>{t('settings.voice.pace')}</span><input type="range" min="100" max="300" step="10" value={draft.voiceRate} onChange={event => update('voiceRate', Number(event.target.value))} aria-label={t('settings.voice.paceAria')} /><span>{number(draft.voiceRate)}</span></label>
      <Toggle label={t('settings.voice.answerAloud')} checked={draft.speakVoiceReplies || draft.speakResponses} disabled={draft.speakResponses} onChange={value => update('speakVoiceReplies', value)} />
      <Toggle label={t('settings.voice.readAloud')} checked={draft.speakResponses} onChange={value => update('speakResponses', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.outAbout.title')}</h3></div>
      <Toggle label={t('settings.outAbout.wander')} checked={draft.petRoam} onChange={value => update('petRoam', value)} />
      <Toggle label={t('settings.outAbout.offerHand')} detail={draft.petAwareness ? undefined : t('settings.outAbout.needsIn', { setting: t('settings.privacy.notice'), tab: t('settings.tabs.privacy') })} disabled={!draft.petAwareness} checked={draft.petNudges && draft.petAwareness} onChange={value => update('petNudges', value)} />
      <Toggle label={t('settings.outAbout.morningBrief')} checked={draft.morningBrief} onChange={value => update('morningBrief', value)} />
      <Toggle label={t('settings.outAbout.headsUps')} checked={draft.headsUps} onChange={value => { update('headsUps', value); if (value && !draft.notificationsEnabled) void notifications(true); }} />
      <Toggle label={t('settings.outAbout.notifications')} detail={notificationStatus === 'denied' ? t('settings.outAbout.offInSystemSettings') : notificationStatus === 'unavailable' ? t('settings.outAbout.installedOnly') : undefined} checked={draft.notificationsEnabled} onChange={value => void notifications(value)} />
      <Toggle label={t('settings.outAbout.openAtLogin')} detail={loginStatus === 'unavailable' ? t('settings.outAbout.installedOnly') : loginStatus === 'requiresApproval' ? t('settings.outAbout.allowLoginItems') : draft.openAtLogin && loginStatus && loginStatus !== 'enabled' ? t('settings.outAbout.offInSystemSettings') : undefined} disabled={loginStatus === 'unavailable' && !draft.openAtLogin} checked={draft.openAtLogin} onChange={value => update('openAtLogin', value)} />
    </section>
    <section className="settings-section">
      <div className="settings-section-heading"><h3>{t('settings.language.title')}</h3></div>
      <select aria-label={t('settings.language.title')} value={draft.language} onChange={event => { update('language', event.target.value); void follow(event.target.value); }}>
        <option value="">{t('settings.language.system')}</option>
        {languages.map(item => <option key={item.code} value={item.code} lang={item.code}>{item.name}</option>)}
      </select>
    </section>
    <button type="button" className="audit-button" onClick={onRestartSetup}><Sparkles size={14} /><span>{t('settings.runSetupAgain')}</span><ChevronRight size={14} /></button>
  </>;
}
