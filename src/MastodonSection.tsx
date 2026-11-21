import { useEffect, useState } from 'react';
import { command, errorMessage } from './bridge';
import { t } from './i18n.ts';

type Account = { server: string; handle: string };

/** Settings → Apps → Mastodon: signed in with the person's own account, in the browser. */
export default function MastodonSection({ onError }: { onError: (error: string) => void }) {
  const [account, setAccount] = useState<Account | null>(null);
  const [server, setServer] = useState('');
  const [busy, setBusy] = useState(false);
  useEffect(() => { command<Account | null>('mastodon_account').then(setAccount).catch(() => {}); }, []);
  async function run(work: () => Promise<void>) {
    setBusy(true);
    try { await work(); } catch (error) { onError(errorMessage(error)); } finally { setBusy(false); }
  }
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>Mastodon</h3></div>
    {account
      ? <div className="app-permission">
          <p className="field-help">{t('settings.chatgpt.signedInAs', { account: account.handle })}</p>
          <button type="button" className="secondary-button" disabled={busy} onClick={() => void run(async () => { await command('mastodon_sign_out'); setAccount(null); })}>{t('settings.chatgpt.signOut')}</button>
        </div>
      : <form className="saved-search-form" onSubmit={event => { event.preventDefault(); void run(async () => setAccount(await command<Account>('mastodon_sign_in', { server }))); }}>
          <input aria-label="Mastodon" placeholder="mastodon.social" value={server} maxLength={253} onChange={event => setServer(event.target.value)} />
          <button type="submit" className="secondary-button" disabled={busy || !server.trim()}>{t('settings.mastodon.signIn')}</button>
        </form>}
  </section>;
}
