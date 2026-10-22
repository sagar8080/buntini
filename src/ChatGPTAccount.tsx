import { useEffect, useState } from 'react';
import { Check, LoaderCircle } from 'lucide-react';
import { command, errorMessage } from './bridge';

type Model = { slug: string; displayName: string };
type Props = {
  account?: string | null; model: string; onModel?: (slug: string) => void;
  onChange: () => Promise<void>; onError: (message: string) => void; onNotice?: (message: string) => void;
};

/** Sign in with ChatGPT: your own account and plan, no API key. */
export default function ChatGPTAccount({ account, model, onModel, onChange, onError, onNotice }: Props) {
  const [signingIn, setSigningIn] = useState(false);
  const [models, setModels] = useState<Model[]>([]);
  useEffect(() => {
    if (!account || !onModel) return;
    let active = true;
    void command<Model[]>('chatgpt_models').then(list => { if (active) setModels(list); }).catch(() => {});
    return () => { active = false; };
  }, [account]);
  async function signIn() {
    setSigningIn(true);
    try { await command<string>('chatgpt_sign_in'); onNotice?.('Signed in with ChatGPT.'); }
    catch (error) { const message = errorMessage(error); if (message !== 'Sign-in was cancelled.') onError(message); }
    finally { setSigningIn(false); await onChange(); }
  }
  async function signOut() {
    try { const confirmed = await command<boolean>('chatgpt_sign_out'); onNotice?.(confirmed ? 'Signed out of ChatGPT.' : 'Signed out on this Mac. To be sure, disconnect Buntini in ChatGPT → Settings.'); }
    catch (error) { onError(errorMessage(error)); }
    finally { await onChange(); }
  }
  if (signingIn) return <div className="chatgpt-account"><LoaderCircle size={14} className="spin" /><span>Finish signing in in your browser…</span><button type="button" className="text-button" onClick={() => void command('chatgpt_cancel_sign_in')}>Cancel</button></div>;
  if (!account) return <div className="chatgpt-signin">
    <button type="button" className="primary-button" onClick={() => void signIn()}>Sign in with ChatGPT</button>
    <p className="field-help">Uses your own ChatGPT plan. Your browser opens to sign in and allow Buntini.</p>
  </div>;
  return <>
    <div className="chatgpt-account"><Check size={14} /><span>Signed in as <strong>{account}</strong></span><button type="button" className="text-button" onClick={() => void signOut()}>Sign out</button></div>
    {onModel && models.length > 0 && <label className="inline-field"><span>Model</span><select aria-label="ChatGPT model" value={model || models[0].slug} onChange={event => onModel(event.target.value)}>{models.map(item => <option key={item.slug} value={item.slug}>{item.displayName}</option>)}</select></label>}
    <p className="field-help">Uses your ChatGPT plan, so there’s no per-message cost. OpenAI sets the limits.</p>
  </>;
}
