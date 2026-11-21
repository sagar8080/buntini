import { useEffect, useState } from 'react';
import Toggle from './Toggle';
import { command, errorMessage } from './bridge';
import { t } from './i18n.ts';

type Assistant = { name: string; allowed: boolean; added?: boolean | null };

/** Settings → Apps → Assistants: Buntini's tools in the other assistants on this Mac, and those
 * that asked to use them. Cursor and VS Code ask in the app before adding, so the list is read
 * again when Buntini comes back to the front. */
export default function AssistantsSection({ onError }: { onError: (error: string) => void }) {
  const [assistants, setAssistants] = useState<Assistant[]>([]);
  const [busy, setBusy] = useState('');
  useEffect(() => {
    const look = () => void command<Assistant[]>('assistants').then(setAssistants).catch(() => {});
    look();
    window.addEventListener('focus', look);
    return () => window.removeEventListener('focus', look);
  }, []);
  if (!assistants.length) return null;
  async function set(name: string, allowed: boolean) {
    setBusy(name);
    try { setAssistants(await command<Assistant[]>('set_assistant', { name, allowed })); }
    catch (error) { onError(errorMessage(error)); }
    finally { setBusy(''); }
  }
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>{t('settings.assistants.title')}</h3></div>
    {assistants.map(assistant => <div key={assistant.name} className="app-permission">
      <Toggle label={assistant.name} checked={assistant.allowed} disabled={busy === assistant.name} onChange={on => void set(assistant.name, on)} />
    </div>)}
  </section>;
}
