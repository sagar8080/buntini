import { useEffect, useState } from 'react';
import Toggle from './Toggle';
import { command, errorMessage } from './bridge';
import { t } from './i18n.ts';

type Assistant = { name: string; allowed: boolean };

/** Settings → Apps → Assistants: the other assistants that asked to use Buntini's tools. */
export default function AssistantsSection({ onError }: { onError: (error: string) => void }) {
  const [assistants, setAssistants] = useState<Assistant[]>([]);
  useEffect(() => { command<Assistant[]>('assistants').then(setAssistants).catch(() => {}); }, []);
  if (!assistants.length) return null;
  const set = (name: string, allowed: boolean) => command<Assistant[]>('set_assistant', { name, allowed }).then(setAssistants).catch(error => onError(errorMessage(error)));
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>{t('settings.assistants.title')}</h3></div>
    {assistants.map(assistant => <div key={assistant.name} className="app-permission">
      <Toggle label={assistant.name} checked={assistant.allowed} onChange={on => void set(assistant.name, on)} />
    </div>)}
  </section>;
}
