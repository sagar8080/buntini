import { useEffect, useState } from 'react';
import { AudioLines, Bell, Cat, Check, LoaderCircle, Palette, ShieldCheck, Shuffle, Sparkles } from 'lucide-react';
import Pet from './Pet';
import { command, desktop, errorMessage } from './bridge';
import { accents, accessories, characters, coats, eyeColors, eyes, lookFrom, motions } from './petOptions';
import type { Settings, Voice } from './types';

const pick = <T,>(items: { id: T }[]) => items[Math.floor(Math.random() * items.length)].id;


export default function Customization({ draft, update, onError, onPreviewVoice }: { draft: Settings; update: <K extends keyof Settings>(key: K, value: Settings[K]) => void; onError: (error: string) => void; onPreviewVoice: (voiceId: string, rate: number, text: string) => Promise<void> }) {
  const [voices, setVoices] = useState<Voice[]>([]);
  const [notificationStatus, setNotificationStatus] = useState('unknown');
  const [requesting, setRequesting] = useState(false);
  const [previewing, setPreviewing] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    void command<Voice[]>('list_voices').then(value => { if (active) setVoices(value); }).catch(() => {});
    void command<string>('notification_permission_status').then(value => { if (active) setNotificationStatus(value); }).catch(() => {});
    return () => { active = false; };
  }, []);
  const look = lookFrom(draft);
  return <>
    <section className="settings-section customization-section">
      <div className="settings-section-heading"><Sparkles size={15} /><h3>Meet your assistant</h3><span className="pill">Local profile</span></div>
      <p>A familiar face, a name, and a way of talking that feels like you.</p>
      <div className="custom-profile-preview"><Pet avatar={draft.avatar} look={look} state="idle" alive /><div><strong>{draft.assistantName || 'Your companion'}</strong><span>{draft.avatar === 'cat' ? `${coats.find(item => item.id === draft.catCoat)?.label} cat` : characters.find(item => item.id === draft.avatar)?.label} · on your Mac</span></div></div>
      <label>Assistant name<input value={draft.assistantName} onChange={event => update('assistantName', event.target.value)} maxLength={40} required placeholder="Buntini" /></label>
      <div className="character-options" aria-label="Companion character">{characters.map(item => <button type="button" key={item.id} aria-pressed={draft.avatar === item.id} className={draft.avatar === item.id ? 'active' : ''} onClick={() => update('avatar', item.id)}><Pet avatar={item.id} look={look} state="idle" /><span>{item.label}</span>{draft.avatar === item.id && <Check size={10} />}</button>)}</div>
      <label>Personality<textarea value={draft.personality} onChange={event => update('personality', event.target.value)} maxLength={1200} rows={3} placeholder="Warm, concise, practical, and honest." /></label>
      <div className="personality-presets">{['Warm, concise, practical, and honest.', 'Curious and playful. Use clear examples and gentle humor.', 'Direct and focused. Give short, actionable answers.'].map((personality, index) => <button type="button" key={personality} aria-pressed={draft.personality === personality} onClick={() => update('personality', personality)}>{['Companion', 'Curious', 'Focused'][index]}</button>)}</div>
      <div className="field-help"><ShieldCheck size={11} /> Personality changes how I respond. Your permissions still apply.</div>
    </section>
    {draft.avatar === 'cat' && <section className="settings-section"><div className="settings-section-heading"><Cat size={15} /><h3>Dress up your cat</h3><button type="button" className="surprise-button" onClick={() => { update('catCoat', pick(coats)); update('catEyes', pick(eyes)); update('catAccessory', pick(accessories)); }}><Shuffle size={10} />Surprise me</button></div>
      <div className="cat-studio-label">Coat<small>{coats.find(item => item.id === draft.catCoat)?.label}</small></div>
      <div className="pet-swatches" aria-label="Coat">{coats.map(item => <button type="button" key={item.id} aria-pressed={draft.catCoat === item.id} className={draft.catCoat === item.id ? 'active' : ''} onClick={() => update('catCoat', item.id)}><Pet look={{ ...look, coat: item.id, accessory: 'none' }} state="idle" /><span>{item.label}</span>{draft.catCoat === item.id && <Check size={10} />}</button>)}</div>
      <div className="cat-studio-label">Eyes<small>{eyes.find(item => item.id === draft.catEyes)?.label}</small></div>
      <div className="eye-options" aria-label="Eye color">{eyes.map(item => <button type="button" key={item.id} aria-pressed={draft.catEyes === item.id} className={draft.catEyes === item.id ? 'active' : ''} onClick={() => update('catEyes', item.id)}><i style={{ background: item.id === 'odd' ? `linear-gradient(90deg, ${eyeColors.blue} 50%, ${eyeColors.amber} 50%)` : eyeColors[item.id] }} />{item.label}</button>)}</div>
      <div className="cat-studio-label">Accessory<small>Tinted with your accent</small></div>
      <div className="pet-swatches" aria-label="Accessory">{accessories.map(item => <button type="button" key={item.id} aria-pressed={draft.catAccessory === item.id} className={draft.catAccessory === item.id ? 'active' : ''} onClick={() => update('catAccessory', item.id)}><Pet look={{ ...look, accessory: item.id }} state="idle" /><span>{item.label}</span>{draft.catAccessory === item.id && <Check size={10} />}</button>)}</div>
    </section>}
    <section className="settings-section"><div className="settings-section-heading"><Palette size={15} /><h3>Make it yours</h3></div>
      <div className="accent-options" aria-label="Accent color">{accents.map(item => <button key={item.id} type="button" aria-pressed={draft.accent === item.id} className={draft.accent === item.id ? 'active' : ''} onClick={() => update('accent', item.id)}><i style={{ background: item.color }} />{item.label}</button>)}</div>
      <div className="cat-studio-label">How lively<small>Applies to every character</small></div>
      <div className="motion-options" aria-label="Motion personality">{motions.map(item => <button type="button" key={item.id} aria-pressed={draft.petMotion === item.id} onClick={() => update('petMotion', item.id)}>{item.label}</button>)}</div>
      <div className="field-help">{draft.reducedMotion ? 'Animations are off because “A little less motion” is on.' : motions.find(item => item.id === draft.petMotion)?.detail}</div>
      <div className="cat-studio-label">Out and about<small>Stays on this Mac</small></div>
      <label className="notification-choice"><input type="checkbox" checked={draft.petRoam} onChange={event => update('petRoam', event.target.checked)} />Walk around the screen and sit on the window I’m using</label>
      <label className="notification-choice"><input type="checkbox" checked={draft.petAwareness} onChange={event => update('petAwareness', event.target.checked)} />Notice what I’m doing: time away, the app in front (name only) and Mac health</label>
      <label className="notification-choice"><input type="checkbox" checked={draft.petNudges} disabled={!draft.petAwareness} onChange={event => update('petNudges', event.target.checked)} />Helpful nudges: breaks, focus timers, battery and app-aware offers</label>
      <div className="field-help">Never window titles or contents. It stays on this Mac: Buntini’s on-device model uses it to understand what you’re doing, and nothing is saved or sent online. Buntini naps when you’re away and hides during meetings and full screen.</div>
      <label className="inline-field"><span>Companion size<small>The bubble stays easy to read</small></span><select value={draft.companionScale} onChange={event => update('companionScale', Number(event.target.value))}><option value={.8}>Small</option><option value={1}>Default</option><option value={1.2}>Large</option></select></label>
    </section>
    <section className="settings-section"><div className="settings-section-heading"><AudioLines size={15} /><h3>A voice to match</h3></div><p>Choose from voices already installed on your Mac.</p>
      <label>Speaking voice<select value={draft.voiceId} onChange={event => update('voiceId', event.target.value)}><option value="">System default</option>{voices.map(voice => <option key={voice.id} value={voice.id}>{voice.name} · {voice.language}</option>)}</select></label>
      <label className="voice-speed">Speaking pace <span>{draft.voiceRate} words/min</span><input type="range" min="100" max="300" step="10" value={draft.voiceRate} onChange={event => update('voiceRate', Number(event.target.value))} /></label>
      <button type="button" className="secondary-button voice-preview" disabled={previewing || !desktop} onClick={async () => { setPreviewing(true); try { await onPreviewVoice(draft.voiceId, draft.voiceRate, `Hi, I'm ${draft.assistantName || 'your companion'}. A little help, right here.`); } catch (error) { onError(errorMessage(error)); } finally { setPreviewing(false); } }}>{previewing ? <LoaderCircle size={13} className="spin" /> : <AudioLines size={13} />}{previewing ? 'Speaking… press Esc to stop' : 'Try this voice'}</button>
    </section>
    <section className="settings-section"><div className="settings-section-heading"><Bell size={15} /><h3>Only when it matters</h3><span className="pill">{notificationStatus === 'authorized' ? 'Allowed' : notificationStatus === 'denied' ? 'Disabled in macOS' : 'Optional'}</span></div><p>Desktop alerts for changed searches, price thresholds, and failures. Unchanged checks stay quiet.</p>
      <label className="notification-choice"><input type="checkbox" checked={draft.notificationsEnabled} onChange={event => update('notificationsEnabled', event.target.checked)} />Enable desktop notifications</label>
      {draft.notificationsEnabled && notificationStatus !== 'authorized' && <button type="button" className="secondary-button" disabled={requesting || !desktop} onClick={async () => { setRequesting(true); try { const allowed = await command<boolean>('request_notification_permission'); setNotificationStatus(allowed ? 'authorized' : 'denied'); if (!allowed) onError('Notifications are disabled. You can enable Buntini in macOS System Settings → Notifications.'); } catch (error) { onError(errorMessage(error)); } finally { setRequesting(false); } }}>{requesting ? 'Requesting…' : 'Allow notifications in macOS'}</button>}
    </section>
  </>;
}
