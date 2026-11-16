import { useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import { command, errorMessage } from './bridge';
import { t } from './i18n.ts';

type Props = {
  account?: string | null;
  onChange: () => Promise<void>; onError: (message: string) => void; onNotice?: (message: string) => void;
};

/** Sign in with ChatGPT: your own account and plan, no API key. */
/** Buntini uses the first model the plan lists, so there is no model to pick. */
export default function ChatGPTAccount({ account, onChange, onError, onNotice }: Props) {
  const [signingIn, setSigningIn] = useState(false);
  async function signIn() {
    setSigningIn(true);
    try { await command<string>('chatgpt_sign_in'); onNotice?.(t('settings.chatgpt.signedIn')); }
    // Rust's own words for a cancelled sign-in, which needs no message.
    catch (error) { const message = errorMessage(error); if (message !== 'Sign-in was cancelled.') onError(message); }
    finally { setSigningIn(false); await onChange(); }
  }
  async function signOut() {
    try { const confirmed = await command<boolean>('chatgpt_sign_out'); onNotice?.(confirmed ? t('settings.chatgpt.signedOut') : t('settings.chatgpt.signedOutHere')); }
    catch (error) { onError(errorMessage(error)); }
    finally { await onChange(); }
  }
  if (signingIn) return <div className="chatgpt-account"><LoaderCircle size={14} className="spin" /><span>{t('settings.chatgpt.finishInBrowser')}</span><button type="button" className="text-button" onClick={() => void command('chatgpt_cancel_sign_in')}>{t('common.cancel')}</button></div>;
  if (!account) return <div className="chatgpt-signin">
    <button type="button" className="primary-button" onClick={() => void signIn()}>{t('settings.chatgpt.signIn')}</button>
  </div>;
  // The account is bold wherever the language puts it.
  const [before, after = ''] = t('settings.chatgpt.signedInAs').split('{account}');
  return <div className="chatgpt-account"><Check size={14} /><span>{before}<strong>{account}</strong>{after}</span><button type="button" className="text-button" onClick={() => void signOut()}>{t('settings.chatgpt.signOut')}</button></div>;
}
