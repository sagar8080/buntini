import { useCallback, useEffect, useRef, useState, type CSSProperties } from 'react';
import { ArrowLeft, ArrowUp, AudioLines, Bell, Check, ChevronRight, CircleHelp, Clock3, Command, Folder, Globe2, History, Laptop, LoaderCircle, Mic, Minus, MoreHorizontal, Play, Plus, RefreshCw, Search, Settings2, ShieldCheck, Sparkles, Square, Sun, Trash2, X } from 'lucide-react';
import { currentMonitor, getCurrentWindow, LogicalSize, PhysicalPosition } from '@tauri-apps/api/window';
import { listen } from '@tauri-apps/api/event';
import Pet from './Pet';
import { ApprovalSheet, ResultCards } from './Online';
import { backendLabel } from './brain.ts';
import { lookFrom } from './petOptions';
import type { PetReaction } from './usePetLife';
import { usePetWorld } from './usePetWorld';
import type { NudgeAction } from './petWorld';
import PriceWatches from './PriceWatches';
import Conversation from './Conversation';
import Onboarding from './Onboarding';
import SettingsView from './SettingsView';
import YourData from './YourData';
import ActionSheet, { type ActionProposal } from './ActionSheet';
import { appearance, idleStatus, panelScale } from './themes.ts';
import { greeting, mergeTurns } from './thread';
import { relativeTime } from './format';
import { SpeechQueue } from './speech';
import { bootstrap, command, desktop, errorMessage, runTask, saveSettings } from './bridge';
import type { Bootstrap, KeyProvider, Mode, PetState, SavedSearch, Settings, TaskRecord, WebApproval } from './types';

type Page = 'home' | 'history' | 'settings' | 'audit';
const modes: { id: Mode; name: string; icon: typeof Sparkles; placeholder: string }[] = [
  { id: 'auto', name: 'Ask anything', icon: Sparkles, placeholder: 'What can I help you with?' },
  { id: 'files', name: 'My files', icon: Folder, placeholder: 'Find a file or a phrase…' },
  { id: 'system', name: 'My Mac', icon: Laptop, placeholder: 'How is my Mac doing?' },
  { id: 'web', name: 'Online', icon: Globe2, placeholder: 'Weather, rates, places, prices, flights or the web…' },
];
export default function App() {
  const [data, setData] = useState<Bootstrap | null>(null);
  const [page, setPage] = useState<Page>('home');
  const [expanded, setExpanded] = useState(false);
  const [mode, setMode] = useState<Mode>('auto');
  const [input, setInput] = useState('');
  const [turns, setTurns] = useState<TaskRecord[]>([]);
  const [live, setLive] = useState<TaskRecord | null>(null);
  const [boundary, setBoundary] = useState<string | null>(null);
  const [canLoadEarlier, setCanLoadEarlier] = useState(false);
  const [focusId, setFocusId] = useState('');
  const threadLoaded = useRef(false);
  const [busy, setBusy] = useState(false);
  const [petState, setPetState] = useState<PetState>('idle');
  const [petReaction, setPetReaction] = useState<PetReaction>();
  const react = (kind: PetReaction['kind']) => setPetReaction(previous => ({ kind, id: (previous?.id ?? 0) + 1 }));
  const [activity, setActivity] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  // Read out finished replies for VoiceOver without announcing every streamed word.
  const [announcement, setAnnouncement] = useState('');
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [voiceStopping, setVoiceStopping] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const [menu, setMenu] = useState(false);
  const [approval, setApproval] = useState<WebApproval | null>(null);
  const [approving, setApproving] = useState(false);
  const [proposal, setProposal] = useState<ActionProposal | null>(null);
  const [acting, setActing] = useState(false);
  const [planning, setPlanning] = useState(false);
  const activeId = useRef('');
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const conversationRef = useRef<HTMLDivElement>(null);
  const recordPromise = useRef<Promise<void> | null>(null);
  const recordHeld = useRef(false);
  const voiceVersion = useRef(0);
  const voiceStoppingRef = useRef(false);
  const taskBusy = useRef(false);
  const speechActive = useRef(false);
  const playbackOverride = useRef<{ voiceId: string; rate: number } | null>(null);
  const speechQueue = useRef<SpeechQueue | null>(null);
  if (!speechQueue.current) speechQueue.current = new SpeechQueue({
    play: text => playbackOverride.current ? command('preview_voice', { ...playbackOverride.current, text }) : command('speak', { text }),
    stop: () => command('stop_speaking'),
    onActive: active => { speechActive.current = active; setSpeaking(active); setPetState(active ? 'speaking' : taskBusy.current ? 'thinking' : 'idle'); },
    onError: error => setError(errorMessage(error)),
  });
  const resizeQueue = useRef<Promise<unknown>>(Promise.resolve());
  const petDragged = useRef(false);
  const mounted = useRef(true);
  const refresh = useCallback(async () => {
    const next = await bootstrap();
    if (mounted.current) setData(next);
    return next;
  }, []);
  useEffect(() => {
    mounted.current = true;
    refresh().catch(e => setError(errorMessage(e)));
    return () => { mounted.current = false; void speechQueue.current?.cancel().catch(() => {}); };
  }, [refresh]);
  // The thread loads once; later turns are added as they finish.
  useEffect(() => {
    if (!data || threadLoaded.current) return;
    threadLoaded.current = true;
    setTurns(mergeTurns([], data.tasks)); setBoundary(data.conversationStart ?? null); setCanLoadEarlier(data.tasks.length >= 100);
    // First launch: open the bubble so Buntini can introduce itself.
    if (data.onboarded === false) void expand(true);
  }, [data]);
  const onboarding = data?.onboarded === false;
  const chatting = turns.length > 0 || !!live || onboarding;
  // The look follows settings and, for "Auto", macOS's light or dark mode as it changes.
  const [systemDark, setSystemDark] = useState(() => window.matchMedia('(prefers-color-scheme: dark)').matches);
  useEffect(() => {
    const query = window.matchMedia('(prefers-color-scheme: dark)');
    const changed = (event: MediaQueryListEvent) => setSystemDark(event.matches);
    query.addEventListener('change', changed);
    return () => query.removeEventListener('change', changed);
  }, []);
  useEffect(() => {
    const root = document.documentElement;
    root.dataset.motion = data?.settings.reducedMotion ? 'reduced' : 'full';
    root.style.removeProperty('--accent'); root.style.removeProperty('--accent-soft');
    if (data) {
      const look = appearance(data.settings, systemDark);
      Object.entries(look.attributes).forEach(([key, value]) => { root.dataset[key] = value; });
      Object.entries(look.variables).forEach(([key, value]) => root.style.setProperty(key, value));
    }
    document.title = `${data?.settings.assistantName || 'Buntini'} — Desktop companion`;
    root.classList.toggle('is-desktop', desktop);
  }, [data?.settings, systemDark]);
  const expand = useCallback(async (open: boolean) => {
    setExpanded(open); setMenu(false);
    if (open) setTimeout(() => inputRef.current?.focus(), 100);
  }, []);
  const nudgeAction = useRef<(action: NudgeAction) => void>(() => {});
  const stillPreferred = typeof window !== 'undefined' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const world = usePetWorld({
    enabled: desktop && !!data,
    roam: !!data?.settings.petRoam && !data.settings.reducedMotion && !stillPreferred,
    aware: !!data?.settings.petAwareness,
    nudges: !!data?.settings.petNudges && !!data?.settings.petAwareness,
    motion: data?.settings.petMotion || 'playful',
    scale: data?.settings.companionScale || 1,
    expanded, petState, react,
    onAction: action => nudgeAction.current(action),
  });
  const nudging = !expanded && !!world.nudge;
  useEffect(() => {
    if (!desktop) return;
    const scale = data?.settings.companionScale || 1;
    // A nudge borrows room above the pet for its speech bubble.
    const size = panelScale[data?.settings.panelSize || 'standard'] ?? 1;
    const width = expanded ? Math.round(420 * size) : nudging ? Math.round(252 * scale) : Math.max(128, Math.round(128 * scale));
    const height = expanded ? Math.round((page === 'home' ? chatting ? 620 : approval ? 520 : 420 : 690) * size) + Math.round(120 * (scale - 1)) : nudging ? Math.round(250 * scale) : Math.max(136, Math.round(136 * scale));
    resizeQueue.current = resizeQueue.current.then(async () => {
      const window = getCurrentWindow();
      const [position, before, monitor] = await Promise.all([window.outerPosition(), window.outerSize(), currentMonitor()]);
      const availableHeight = monitor ? monitor.workArea.size.height / monitor.scaleFactor - 8 : height;
      await window.setSize(new LogicalSize(width, Math.min(height, availableHeight)));
      const after = await window.outerSize();
      let x = position.x + before.width - after.width;
      let y = position.y + before.height - after.height;
      if (monitor) {
        const area = monitor.workArea;
        x = Math.max(area.position.x, Math.min(x, area.position.x + area.size.width - after.width));
        y = Math.max(area.position.y, Math.min(y, area.position.y + area.size.height - after.height));
      }
      await window.setPosition(new PhysicalPosition(x, y));
    }).catch(e => setError(errorMessage(e)));
  }, [expanded, nudging, page, chatting, !!approval, data?.settings.companionScale, data?.settings.panelSize]);
  useEffect(() => {
    if (!desktop) return;
    const subscription = listen('companion-toggle', () => { void expand(true); });
    return () => { void subscription.then(unlisten => unlisten()); };
  }, [expand]);
  useEffect(() => {
    if (!desktop) return;
    const subscription = listen<{ searchId: string; task: TaskRecord }>('saved-search-result', event => {
      void refresh();
      setTurns(previous => mergeTurns(previous, [event.payload.task]));
      setNotice(event.payload.task.state === 'completed' ? `A saved search has an update: ${event.payload.task.prompt}` : 'A saved search needs attention. Check the task shelf.');
    });
    return () => { void subscription.then(unlisten => unlisten()); };
  }, [refresh]);
  useEffect(() => {
    if (!desktop) return;
    const budget = listen<string>('budget-warning', event => setNotice(event.payload));
    const subscription = listen<{ watch: { label: string }; belowThreshold: boolean; changed: boolean }>('price-watch-result', event => {
      void refresh();
      if (event.payload.belowThreshold) setNotice(`${event.payload.watch.label} reached your target price. Check the task shelf.`);
      else if (event.payload.changed) setNotice(`Price watch updated: ${event.payload.watch.label}`);
    });
    return () => { void subscription.then(unlisten => unlisten()); void budget.then(unlisten => unlisten()); };
  }, [refresh]);
  useEffect(() => {
    const listener = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        if (approval) { setApproval(null); setPetState('idle'); }
        else if (proposal && !acting) { void command('cancel_action', { id: proposal.id }).catch(() => {}); setProposal(null); setPetState('idle'); }
        else if (recordHeld.current || recording || transcribing) { void cancelVoice(); }
        else if (speaking) { void speechQueue.current?.cancel().catch(error => setError(errorMessage(error))); }
        else if (page !== 'home') setPage('home');
        else if (expanded && !busy && !recording) void expand(false);
      }
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault(); setPage('home'); void expand(true);
      }
    };
    window.addEventListener('keydown', listener);
    return () => window.removeEventListener('keydown', listener);
  }, [page, expanded, busy, recording, transcribing, speaking, expand, approval, proposal, acting]);
  useEffect(() => {
    const element = conversationRef.current;
    if (!element || page !== 'home') return;
    if (focusId) { document.getElementById(`turn-${focusId}`)?.scrollIntoView({ block: 'start' }); return; }
    element.scrollTo({ top: element.scrollHeight, behavior: live ? 'smooth' : 'instant' });
  }, [page, expanded, focusId, boundary, live?.answer, activity, turns.at(-1)?.id]);
  useEffect(() => {
    if (!notice) return;
    const id = window.setTimeout(() => setNotice(''), 3500);
    return () => window.clearTimeout(id);
  }, [notice]);
  useEffect(() => {
    const lostFocus = () => { if (recordHeld.current) void cancelVoice(); };
    const visibility = () => { if (document.hidden) lostFocus(); };
    const keyReleased = (event: KeyboardEvent) => { if (recordHeld.current && (event.key === ' ' || event.key === 'Enter')) void finishVoice(); };
    window.addEventListener('blur', lostFocus);
    window.addEventListener('keyup', keyReleased);
    document.addEventListener('visibilitychange', visibility);
    return () => { window.removeEventListener('blur', lostFocus); window.removeEventListener('keyup', keyReleased); document.removeEventListener('visibilitychange', visibility); };
  }, []);

  const speak = useCallback(async (text: string) => {
    playbackOverride.current = null;
    try { await speechQueue.current?.speakText(text); } catch (error) { setError(errorMessage(error)); }
  }, []);
  // The local model proposes one online tool; nothing is sent until the user approves the exact request.
  const planOnline = useCallback(async (prompt: string, webOnly: boolean) => {
    const planId = crypto.randomUUID();
    activeId.current = planId;
    setError(''); setPlanning(true); setPetState('thinking');
    try { const next = await command<WebApproval>('plan_online_request', { prompt, planId, webOnly }); setApproval(next); setPetState('approval'); }
    catch (error) { const message = errorMessage(error); if (message !== 'Task cancelled.') setError(message); setPetState('idle'); }
    finally { setPlanning(false); if (activeId.current === planId) activeId.current = ''; }
  }, []);
  const submit = useCallback(async (prompt: string = input, chosenMode: Mode = mode, approvalId?: string) => {
    if (!prompt.trim() || busy || planning || recording || transcribing || voiceStoppingRef.current) return;
    if (chosenMode === 'web' && !approvalId) {
      await planOnline(prompt.trim(), false);
      return;
    }
    playbackOverride.current = null;
    const speechToken = speechQueue.current!.begin();
    const useSpeech = !!data?.settings.speakResponses;
    let streamedText = '', answerSoFar = '', liveId = '', goOnline = false, askAction = false;
    taskBusy.current = true;
    setError(''); setInput(''); setPage('home'); setBusy(true); setPetState('thinking'); setActivity('Getting my thoughts together'); activeId.current = '';
    const provisional: TaskRecord = { id: '', prompt: prompt.trim(), answer: '', state: 'running', createdAt: new Date().toISOString(), sources: [], files: [] };
    setFocusId(''); setLive(provisional);
    try {
      const result = await runTask(prompt.trim(), chosenMode, event => {
        if (event.taskId) { activeId.current = event.taskId; liveId = event.taskId; }
        if (event.type === 'state') {
          if (!speechActive.current) setPetState(event.state === 'working' ? 'working' : 'thinking');
          setActivity(event.text || ({ thinking: 'Thinking locally', working: 'Taking a look', planning: 'Making a plan' }[event.state || ''] || 'Working on it'));
        } else if (event.type === 'delta') {
          answerSoFar += event.text || '';
          setLive(previous => previous && { ...previous, id: event.taskId, answer: previous.answer + (event.text || '') });
          if (useSpeech) { streamedText += event.text || ''; speechQueue.current?.append(event.text || '', speechToken); }
        }
        else if (event.type === 'tool') { if (!speechActive.current) setPetState('working'); setActivity(event.text || `Using ${event.tool?.replaceAll('_', ' ') || 'a local tool'}`); }
        else if (event.type === 'error') setError(event.text || 'The task could not finish.');
        else if (event.type === 'notice' && event.text) { setNotice(event.text); react('tilt'); }
      }, approvalId);
      setLive(null);
      if (result.handoff === 'online') { goOnline = true; await speechQueue.current?.cancel(); return; }
      if (result.handoff === 'action' && result.action) { setProposal(result.action); askAction = true; await speechQueue.current?.cancel(); return; }
      setTurns(previous => mergeTurns(previous, [result]));
      if (result.state === 'completed') setAnnouncement(`${data?.settings.assistantName || 'Buntini'}: ${result.answer.slice(0, 400)}`);
      if (result.state !== 'cancelled') react(result.state === 'completed' ? 'happy' : 'sad');
      if (result.state === 'completed' && useSpeech) {
        if (!streamedText && result.answer) speechQueue.current?.append(result.answer, speechToken);
        speechQueue.current?.finish(speechToken);
      } else await speechQueue.current?.cancel();
      await refresh();
    } catch (e) {
      const message = errorMessage(e);
      setError(message); react('sad');
      setLive(null); setTurns(thread => mergeTurns(thread, [{ ...provisional, id: liveId || `local-${provisional.createdAt}`, state: 'error', answer: answerSoFar || message }])); await speechQueue.current?.cancel().catch(() => {});
    } finally {
      taskBusy.current = false; setBusy(false); setActivity(''); activeId.current = ''; if (!speechActive.current) setPetState('idle');
      // The model wants to look this up online: show the approval sheet for the same message.
      if (goOnline) void planOnline(prompt.trim(), false);
      if (askAction) setPetState('approval');
    }
  }, [input, mode, busy, planning, planOnline, recording, transcribing, refresh, data?.settings.speakResponses]);
  nudgeAction.current = action => {
    if (action === 'checkMac') { void expand(true); void submit('How is my Mac doing?', 'system'); }
    else if (action === 'lookUp') { setMode('web'); void expand(true); }
    else if (action === 'findFile') { setMode('files'); void expand(true); }
  };
  async function cancel() {
    const taskId = activeId.current;
    const stopTask = taskId ? command('cancel_task', { taskId }).then(() => {
      if (activeId.current === taskId) setActivity('Stopping…');
    }).catch(error => setError(errorMessage(error))) : Promise.resolve();
    await Promise.all([stopTask, speechQueue.current?.cancel().catch(error => setError(errorMessage(error)))]);
  }
  async function startVoice() {
    if (busy || transcribing || recordHeld.current || voiceStoppingRef.current) return;
    if (!data?.settings.whisperModel) { setError('Choose a local Whisper model in Settings before using voice input.'); setPage('settings'); return; }
    playbackOverride.current = null;
    void speechQueue.current?.cancel().catch(error => setError(errorMessage(error)));
    const version = ++voiceVersion.current;
    recordHeld.current = true; setError('');
    setRecording(true); setPetState('listening');
    recordPromise.current = (async () => {
      try { await command('start_recording'); if (version !== voiceVersion.current) await command('cancel_recording'); }
      catch (e) { if (version === voiceVersion.current) { recordHeld.current = false; setRecording(false); setError(errorMessage(e)); setPetState('idle'); } throw e; }
    })();
    try { await recordPromise.current; } catch { /* Error is shown above. */ }
  }
  async function finishVoice() {
    if (!recordHeld.current) return;
    const version = voiceVersion.current;
    recordHeld.current = false;
    try { await recordPromise.current; } catch { return; }
    if (version !== voiceVersion.current) return;
    setRecording(false); setTranscribing(true); setPetState('thinking');
    try {
      const text = await command<string>('stop_recording');
      if (version !== voiceVersion.current) return;
      setInput(text); setPage('home'); inputRef.current?.focus();
      if (!text.trim()) setNotice('No speech detected. Try holding the mic a little longer.');
    } catch (e) { if (version === voiceVersion.current) setError(errorMessage(e)); }
    finally { if (version === voiceVersion.current) { setTranscribing(false); setPetState('idle'); recordPromise.current = null; } }
  }
  async function cancelVoice() {
    if (voiceStoppingRef.current) return;
    voiceStoppingRef.current = true; setVoiceStopping(true);
    const pendingStart = recordPromise.current;
    const version = ++voiceVersion.current;
    recordHeld.current = false; setRecording(false); setTranscribing(false); setPetState('idle');
    try { await command('cancel_recording'); } catch (e) { setError(errorMessage(e)); }
    finally { await pendingStart?.catch(() => {}); if (version === voiceVersion.current) { recordPromise.current = null; voiceStoppingRef.current = false; setVoiceStopping(false); } }
  }
  async function finishSetup(settings: Settings, keys: [KeyProvider, string][]) {
    for (const [provider, key] of keys) await command('set_api_key', { provider, key });
    // A scheduled task may briefly hold the settings; try once more before giving up.
    const saved = await saveSettings(settings).catch(() => new Promise(resolve => setTimeout(resolve, 800)).then(() => saveSettings(settings)));
    await command('finish_onboarding');
    setData(previous => previous && { ...previous, settings: saved, onboarded: true });
    await refresh(); react('happy');
  }
  const briefing = useRef(false);
  async function brief() {
    if (briefing.current) return;
    briefing.current = true;
    try { const turn = await command<TaskRecord>('morning_brief'); setTurns(previous => mergeTurns(previous, [turn])); react('happy'); if (data?.settings.speakResponses) void speak(turn.answer); void refresh(); }
    catch (e) { setError(errorMessage(e)); }
    finally { briefing.current = false; }
  }
  // Once a day, the first time the bubble opens.
  useEffect(() => { if (expanded && data?.briefDue && !busy) void brief(); }, [expanded, data?.briefDue]);
  async function newTopic() {
    try { setBoundary(await command<string>('new_topic')); setFocusId(''); setNotice('Fresh start. I won’t bring up what we talked about before.'); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function loadEarlier() {
    const oldest = turns[0];
    if (!oldest) return;
    try { const older = await command<TaskRecord[]>('conversation', { before: oldest.createdAt, limit: 30 }); setTurns(previous => mergeTurns(previous, older)); setCanLoadEarlier(older.length === 30); }
    catch (e) { setError(errorMessage(e)); }
  }
  async function addFolder() {
    try {
      const path = await command<string | null>('pick_folder');
      if (path) { await command('add_root', { path }); await refresh(); setNotice('Folder access granted. You can revoke it any time.'); }
    } catch (e) { setError(errorMessage(e)); }
  }
  async function safeAction(action: () => Promise<unknown>, message?: string) {
    setError('');
    try { await action(); await refresh(); if (message) setNotice(message); } catch (e) { setError(errorMessage(e)); }
  }
  const currentMode = modes.find(item => item.id === mode)!;
  const modelReady = data?.diagnostics.modelReady;
  const pending = busy || planning || recording || transcribing || voiceStopping;
  const assistantName = data?.settings.assistantName || 'Buntini';
  const companionScale = data?.settings.companionScale || 1;

  return <main className={`desktop-stage ${expanded ? 'is-expanded' : 'is-collapsed'}${world.quiet ? ' is-quiet' : ''}`}>
    <div className="companion" style={{ '--companion-scale': companionScale, '--dock-height': `${120 * companionScale}px`, ...(!desktop ? { height: expanded ? (page === 'home' ? chatting ? 620 : approval ? 520 : 420 : 690) * (panelScale[data?.settings.panelSize || 'standard'] ?? 1) + 120 * (companionScale - 1) : Math.max(136, 136 * companionScale) } : {}) } as CSSProperties}>
      {expanded && <section className="panel" aria-label="Buntini companion">
        <header className="panel-header">
          <button className="brand" onClick={() => setPage('home')} aria-label={`${assistantName} home`}><span className="brand-mark">{data ? <Pet state={busy ? 'thinking' : 'idle'} avatar={data.settings.avatar} look={lookFrom(data.settings)} /> : <Sparkles size={15} strokeWidth={1.8} />}</span><span>{assistantName}<span className="brand-dot">.</span></span></button>
          <span className="header-status"><span className={`status-dot ${busy ? 'pulsing' : ''}`} />{recording ? 'Listening' : busy ? 'On it' : speaking ? 'Speaking' : data ? idleStatus(data.settings, world.asleep, new Date().getHours()) : 'Right here'}</span>
          <div className="header-actions">
            <button className={`icon-button ${page === 'history' ? 'selected' : ''}`} title="Task history" aria-label="Task history" onClick={() => setPage(page === 'history' ? 'home' : 'history')}><History size={16} /></button>
            <button className={`icon-button ${page === 'settings' ? 'selected' : ''}`} title="Settings" aria-label="Settings" onClick={() => setPage(page === 'settings' ? 'home' : 'settings')}><Settings2 size={16} /></button>
            <button className="icon-button" title="Collapse companion" aria-label="Collapse companion" onClick={() => void expand(false)}><Minus size={17} /></button>
          </div>
        </header>

        {page === 'home' && onboarding && data && <Onboarding data={data} onFinish={finishSetup} onSkip={() => safeAction(() => command('finish_onboarding'))} onRefresh={async () => { await refresh(); }} />}
        {page === 'home' && !onboarding && <>
          <div className="conversation" ref={conversationRef}>
            {!chatting ? <div className="overlay-welcome"><span>{greeting(new Date(), data?.settings.userName)} What’s on your mind?</span><div><button onClick={() => { setMode('files'); inputRef.current?.focus(); }}><Folder size={12} /> Find a file</button><button onClick={() => void submit('How is my Mac doing?', 'system')}><Laptop size={12} /> Check my Mac</button><button onClick={() => { setMode('web'); inputRef.current?.focus(); }}><Globe2 size={12} /> Look it up</button><button onClick={() => void brief()}><Sun size={12} /> My day</button></div></div> : <>
              <div className="thread-bar"><span>{greeting(new Date(), data?.settings.userName)}</span><button className="text-button" onClick={() => void newTopic()} disabled={busy || !!boundary && turns.every(turn => turn.createdAt <= boundary) && !live} title="Start fresh: earlier messages stay here but aren’t sent to the model"><Plus size={11} /> New topic</button></div>
              <Conversation turns={turns} live={live} boundary={boundary} busy={busy} activity={activity} settings={data?.settings} focusId={focusId} speaking={speaking} scroller={conversationRef} canLoadEarlier={canLoadEarlier} onLoadEarlier={loadEarlier}
                onOpen={url => { if (desktop) void safeAction(() => command('open_source', { url })); else window.open(url, '_blank', 'noreferrer'); }}
                onSpeak={text => void speak(text)} onStopSpeaking={() => void speechQueue.current?.cancel().catch(error => setError(errorMessage(error)))} />
            </>}
          </div>
          <div className="composer-area">
            {planning && <div className="planning-status" role="status"><LoaderCircle size={12} className="spin" /><span>Choosing the right source…</span><button type="button" className="text-button" onClick={() => void cancel()}>Stop</button></div>}
            {mode !== 'auto' && <div className="mode-pill"><currentMode.icon size={13} /><span>{currentMode.name}</span><button type="button" aria-label="Back to asking anything" onClick={() => setMode('auto')} disabled={busy}><X size={12} /></button></div>}
            <form className={`composer ${recording ? 'is-recording' : ''}`} onSubmit={event => { event.preventDefault(); void submit(); }}>
              <textarea ref={inputRef} rows={2} value={input} onChange={event => setInput(event.target.value)} placeholder={recording ? 'I’m listening… release to finish' : transcribing ? 'Turning your voice into words…' : currentMode.placeholder} aria-label={`Message ${assistantName}`} disabled={pending} maxLength={8000} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); if (!busy) void submit(); } }} />
              <div className="composer-bottom"><span>{recording ? <><span className="record-dot" /> Release to transcribe</> : transcribing ? <><LoaderCircle size={11} className="spin" /> Transcribing on your Mac</> : <><ShieldCheck size={11} />{mode === 'web' ? 'Asks before anything is sent online' : data?.settings.modelBackend === 'cloud' ? 'Cloud model · sees only what you share' : 'Local by default'}</>}</span><div>
                <button type="button" className={`mic-button ${recording ? 'active' : ''}`} title="Hold to talk" aria-label="Hold to talk; release to transcribe" disabled={busy || transcribing || voiceStopping} onPointerDown={event => { event.preventDefault(); event.currentTarget.setPointerCapture(event.pointerId); void startVoice(); }} onPointerUp={() => void finishVoice()} onPointerCancel={() => void cancelVoice()} onLostPointerCapture={() => { if (recordHeld.current) void cancelVoice(); }} onKeyDown={event => { if ((event.key === ' ' || event.key === 'Enter') && !event.repeat) { event.preventDefault(); void startVoice(); } }} onKeyUp={event => { if (event.key === ' ' || event.key === 'Enter') { event.preventDefault(); void finishVoice(); } }}>{recording ? <AudioLines size={17} /> : <Mic size={17} />}</button>
                {recording || transcribing ? <button type="button" className="send-button stop-button" aria-label="Cancel voice input" onClick={() => void cancelVoice()}><X size={15} /></button> : busy ? <button type="button" className="send-button stop-button" aria-label="Stop task" onClick={() => void cancel()} disabled={!activeId.current}><Square size={13} fill="currentColor" /></button> : <button type="submit" className="send-button" aria-label="Send message" disabled={!input.trim() || pending}><ArrowUp size={17} /></button>}
              </div></div>
            </form>
            <div className="composer-hint"><span>{modelReady ? <><span className="status-dot" /> {backendLabel[data?.settings.modelBackend || 'apple']}</> : <button onClick={() => setPage('settings')}><span className="status-dot amber" /> Choose a brain <ChevronRight size={10} /></button>}</span><span><kbd>↵</kbd> to send <span className="hint-separator">·</span> <kbd>⇧ ↵</kbd> new line</span></div>
          </div>
        </>}

        {page === 'history' && <div className="subpage"><div className="subpage-title"><h2>History</h2></div>
          <SavedSearches searches={data?.savedSearches || []} onAction={async (name, args) => { const result = await command(name, args); await refresh(); return result; }} onResult={result => { setTurns(previous => mergeTurns(previous, [result])); setFocusId(result.id); setPage('home'); }} onError={setError} />
          <PriceWatches watches={data?.priceWatches || []} onAction={async (name, args) => { const result = await command(name, args); await refresh(); return result; }} onError={setError} />
          <div className="section-label history-section-label">PAST CONVERSATIONS</div>
          {data?.tasks.length ? <><div className="history-list">{data.tasks.map(item => <button key={item.id} className="history-item" onClick={() => { setTurns(previous => mergeTurns(previous, [item])); setFocusId(item.id); setError(''); setPage('home'); }}><span className="history-icon">{item.sources.length ? <Globe2 size={16} /> : <Clock3 size={16} />}</span><span><strong>{item.prompt || 'Morning brief'}</strong><small>{relativeTime(item.createdAt)} · {item.state}</small></span><ChevronRight size={14} /></button>)}</div><button className="text-button danger clear-history" onClick={() => void safeAction(async () => { await command('clear_history'); setTurns([]); setLive(null); setCanLoadEarlier(false); }, 'Task history cleared.')}><Trash2 size={13} /> Clear task history</button></> : <div className="empty-state"><History size={28} strokeWidth={1.3} /><h3>A fresh start.</h3><p>Your conversations will find a home here.</p></div>}
          <div className="history-privacy"><ShieldCheck size={13} /> Stored in SQLite. Never synced to a cloud.</div>
        </div>}

        {page === 'settings' && data && <SettingsView data={data} onPreviewVoice={async (voiceId, rate, text) => { const override = { voiceId, rate }; playbackOverride.current = override; try { await speechQueue.current!.speakText(text); } finally { if (playbackOverride.current === override) playbackOverride.current = null; } }} onSave={async settings => {
          const saved = await saveSettings(settings); setData(previous => previous && { ...previous, settings: saved }); await refresh();
        }} onError={setError} onAddFolder={addFolder} onRemoveFolder={path => safeAction(() => command('remove_root', { path }))} onKey={async (provider, key) => { await command('set_api_key', { provider, key }); await refresh(); setNotice(key ? 'API key stored in macOS Keychain.' : 'API key removed.'); }} onAudit={() => setPage('audit')} onRestartSetup={() => void safeAction(async () => { await command('restart_onboarding'); setPage('home'); })} onRefresh={async () => { await refresh(); }} onNotice={setNotice} onPolicy={() => { const url = 'https://github.com/sagar8080/nexon/blob/main/PRIVACY.md'; if (desktop) void safeAction(() => command('open_source', { url })); else window.open(url, '_blank', 'noreferrer'); }} onForget={async id => { await command('forget_memory', { id }); await refresh(); }} privacyExtras={<YourData onNotice={setNotice} onError={setError} onErased={async () => { setTurns([]); setLive(null); setBoundary(null); setCanLoadEarlier(false); setPage('home'); await refresh(); }} />} />}

        {page === 'audit' && <div className="subpage"><button className="back-link" onClick={() => setPage('settings')}><ArrowLeft size={13} /> Settings</button><div className="subpage-title"><h2>Activity log</h2><p>Tools used and permissions given, kept on this Mac.</p></div>{data?.audit.length ? <div className="audit-list">{data.audit.map((entry, index) => <article key={entry.id || index}><div><ShieldCheck size={13} /><strong>{entry.action.replaceAll('_', ' ')}</strong><small>{relativeTime(entry.timestamp || entry.createdAt || '')}</small></div><p>{entry.detail || entry.outcome}</p></article>)}</div> : <div className="empty-state"><ShieldCheck size={28} strokeWidth={1.3} /><h3>Quiet so far.</h3><p>Activity will appear here as we work.</p></div>}</div>}

        {approval && <ApprovalSheet approval={approval} approving={approving} onCancel={() => { setApproval(null); setPetState('idle'); }} onWebInstead={() => { const prompt = approval.prompt; setApproval(null); void planOnline(prompt, true); }} onAllow={async () => { setApproving(true); try { const allowed = await command<WebApproval>('approve_web_approval', { id: approval.id }); setApproval(null); await submit(allowed.prompt, 'web', allowed.id); } catch (error) { setError(errorMessage(error)); setApproval(null); setPetState('idle'); } finally { setApproving(false); } }} />}
        {proposal && <ActionSheet proposal={proposal} running={acting} onCancel={() => { void command('cancel_action', { id: proposal.id }).catch(() => {}); setProposal(null); setPetState('idle'); }} onAllow={async args => {
          setActing(true); setError('');
          try { const done = await command<TaskRecord>('run_action', { id: proposal.id, args }); setTurns(previous => mergeTurns(previous, [done])); setProposal(null); react('happy'); if (data?.settings.speakResponses) void speak(done.answer); }
          catch (failure) { setError(errorMessage(failure)); setProposal(null); react('sad'); }
          finally { setActing(false); setPetState('idle'); }
        }} />}
        <div className="sr-only" aria-live="polite">{announcement}</div>
        {(error || notice) && <div className={`toast ${error ? 'toast-error' : ''}`} role={error ? 'alert' : 'status'}><span>{error || notice}</span><button aria-label="Dismiss message" onClick={() => { setError(''); setNotice(''); }}><X size={13} /></button></div>}
        <footer className="panel-footer"><span><span className="footer-dot" />{desktop ? 'Made for your little corner of the world' : 'Browser preview · desktop tools need the app'}</span><button title="About Buntini" aria-label="About Buntini" onClick={() => setNotice('Buntini 0.1 · Local-first desktop companion. Shortcut: ⌘ ⇧ Space. No unrestricted shell tools.')}><CircleHelp size={12} /></button></footer>
      </section>}

      <div className="pet-dock">
        {nudging && world.nudge && <div className="nudge-bubble" role="status" aria-live="polite"><p>{world.nudge.text}</p><div>{world.nudge.actions.map(item => <button key={item.label} onClick={() => world.act(item.action)}>{item.label}</button>)}</div></div>}
        {expanded && !desktop && <div className="pet-caption"><span>{petState === 'listening' ? 'All ears.' : petState === 'thinking' ? 'A little thinking…' : petState === 'working' ? 'On it, one moment.' : petState === 'speaking' ? 'Something to say.' : 'Here when you need me.'}</span><svg width="22" height="16" viewBox="0 0 22 16"><path d="M2 2c2 8 8 9 17 10m-5-5 5 5-6 2" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" fill="none" /></svg></div>}
        <button className="pet-button" aria-label={expanded ? `Drag ${assistantName}; click to collapse` : `Open ${assistantName}`}  title={expanded ? 'Drag to move · click to tuck away' : 'Click to open Buntini'} onClick={() => { if (petDragged.current) { petDragged.current = false; return; } void expand(!expanded); }} onPointerDown={event => {
          if (!desktop || event.button !== 0) return;
          petDragged.current = false;
          const startX = event.clientX, startY = event.clientY;
          const button = event.currentTarget;
          const move = (moveEvent: PointerEvent) => {
            if (Math.abs(moveEvent.clientX - startX) + Math.abs(moveEvent.clientY - startY) > 5) {
              petDragged.current = true;
              button.removeEventListener('pointermove', move); world.beforeDrag(); void getCurrentWindow().startDragging();
            }
          };
          button.addEventListener('pointermove', move);
          button.addEventListener('pointerup', () => button.removeEventListener('pointermove', move), { once: true });
        }}><Pet state={petState} avatar={data?.settings.avatar || 'cat'} look={data ? lookFrom(data.settings) : undefined} alive reaction={petReaction} moving={world.moving} facing={world.facing} landing={world.landing} asleep={world.asleep} speed={world.speed} /></button>
        {expanded && <div className="dock-tools"><span className="shortcut"><Command size={10} /><span>⇧</span><span>space</span></span><div className="dock-menu-wrap"><button className="dock-menu-button" aria-label="Companion menu" onClick={() => setMenu(!menu)}><MoreHorizontal size={19} /></button>{menu && <div className="dock-menu"><button onClick={() => void expand(false)}><Minus size={13} /> Tuck away</button><button onClick={() => { setMenu(false); setPage('settings'); }}><Settings2 size={13} /> Settings</button><button onClick={() => { setMenu(false); if (desktop) void getCurrentWindow().hide(); else void expand(false); }}><X size={13} /> Hide companion</button></div>}</div></div>}
        {!expanded && <span className="collapsed-label">buntini<span>·</span>{busy ? 'working' : 'here for you'}</span>}
      </div>
    </div>
  </main>;
}


function SavedSearches({ searches, onAction, onResult, onError }: { searches: SavedSearch[]; onAction: (name: string, args: Record<string, unknown>) => Promise<unknown>; onResult: (result: TaskRecord) => void; onError: (message: string) => void }) {
  const [adding, setAdding] = useState(false);
  const [query, setQuery] = useState('');
  const [intervalMinutes, setIntervalMinutes] = useState(60);
  const [working, setWorking] = useState('');
  async function act(id: string, action: () => Promise<unknown>) { setWorking(id); try { await action(); } catch (e) { onError(errorMessage(e)); } finally { setWorking(''); } }
  return <div className="saved-searches"><div className="section-label"><Bell size={11} /> KEEP AN EYE ON IT <button className="text-button" onClick={() => setAdding(!adding)}><Plus size={12} /> New search</button></div>
    {adding && <form className="saved-search-form" onSubmit={event => { event.preventDefault(); void act('new', async () => { await onAction('save_search', { query: query.trim(), intervalMinutes }); setQuery(''); setAdding(false); }); }}><input aria-label="Saved search query" placeholder="What should I keep an eye on?" value={query} onChange={event => setQuery(event.target.value)} required maxLength={500} /><div><select aria-label="Check frequency" value={intervalMinutes} onChange={event => setIntervalMinutes(Number(event.target.value))}><option value={15}>Every 15 minutes</option><option value={60}>Every hour</option><option value={360}>Every 6 hours</option><option value={1440}>Every day</option></select><button className="primary-button" disabled={!query.trim() || working === 'new'}>{working === 'new' ? <LoaderCircle size={11} className="spin" /> : <Check size={11} />} Save</button></div><p>Sends this query to your search provider while Buntini is running. Your monthly cap applies.</p></form>}
    {searches.length ? searches.map(search => <article className={`saved-search ${search.enabled ? '' : 'paused'}`} key={search.id}><div className="saved-search-heading"><Search size={14} /><strong>{search.query}</strong><button className="icon-button" aria-label={`Delete saved search: ${search.query}`} disabled={!!working} onClick={() => void act(search.id, () => onAction('delete_saved_search', { id: search.id }))}><X size={12} /></button></div><div className="saved-search-info"><span>{search.enabled ? <><span className="status-dot" />{search.intervalMinutes < 60 ? `${search.intervalMinutes} min` : search.intervalMinutes < 1440 ? `${search.intervalMinutes / 60} hr` : 'Daily'} checks</> : 'Paused'}</span><button className="text-button" disabled={!!working} onClick={() => void act(search.id, () => onAction('update_saved_search', { id: search.id, enabled: !search.enabled, intervalMinutes: search.intervalMinutes }))}>{search.enabled ? <Square size={9} /> : <Play size={10} />}{search.enabled ? 'Pause' : 'Resume'}</button><button className="text-button" disabled={!!working} onClick={() => void act(search.id, async () => { const result = await onAction('run_saved_search', { id: search.id }); onResult(result as TaskRecord); })}>{working === search.id ? <LoaderCircle size={10} className="spin" /> : <RefreshCw size={10} />} Check now</button></div>{search.lastError && <p className="saved-search-error">{search.lastError}</p>}</article>) : !adding && <p className="saved-search-empty">Save a web query and I’ll check back for you.</p>}
  </div>;
}
