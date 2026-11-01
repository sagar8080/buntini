import { useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import { command, errorMessage } from './bridge';

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
    try { await command<string>('chatgpt_sign_in'); onNotice?.('Signed in with ChatGPT.'); }
    catch (error) { const message = errorMessage(error); if (message !== 'Sign-in was cancelled.') onError(message); }
    finally { setSigningIn(false); await onChange(); }
  }
  async function signOut() {
    try { const confirmed = await command<boolean>('chatgpt_sign_out'); onNotice?.(confirmed ? 'Signed out of ChatGPT.' : 'Signed out here. Also disconnect Buntini in ChatGPT → Settings.'); }
    catch (error) { onError(errorMessage(error)); }
    finally { await onChange(); }
  }
  if (signingIn) return <div className="chatgpt-account"><LoaderCircle size={14} className="spin" /><span>Finish in your browser…</span><button type="button" className="text-button" onClick={() => void command('chatgpt_cancel_sign_in')}>Cancel</button></div>;
  if (!account) return <div className="chatgpt-signin">
    <button type="button" className="primary-button" onClick={() => void signIn()}>Sign in with ChatGPT</button>
  </div>;
  return <div className="chatgpt-account"><Check size={14} /><span>Signed in as <strong>{account}</strong></span><button type="button" className="text-button" onClick={() => void signOut()}>Sign out</button></div>;
}
