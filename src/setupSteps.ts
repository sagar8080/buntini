// First-run setup, guided by Buntini in the chat bubble. Pure step logic: what Buntini
// says, what the user's answer reads like, what blocks a step, and the settings it saves.
import { appleStatus, cloudProviders } from './brain.ts';
import { sharedSentence } from './cloudShares.ts';
import { coats } from './petOptions.ts';
import type { AppConnection, AppStatus, CatCoat, CloudProvider, Diagnostics, ModelBackend, Settings } from './types';

// Online lookups need no setup (browser research works without a key), so setup asks
// only what Buntini can't decide for itself: names, a brain and whether it may roam.
export type Step = 'hello' | 'brain' | 'roam' | 'done';
export const steps: Step[] = ['hello', 'brain', 'roam', 'done'];
/** The steps that ask something, for "1 of 3". */
export const questions = steps.length - 1;

export interface Answers {
  userName: string; assistantName: string; coat: CatCoat;
  backend: ModelBackend; pythonPath: string; modelPath: string;
  /** The local model is being set up; setup needn't wait for it. */
  localStarted: boolean;
  cloudProvider: CloudProvider;
  /** What a cloud brain may see (Settings::externalShares). */
  shares: string[];
  roam: boolean; awareness: boolean; nudges: boolean;
}

/** The cloud cap setup uses when none was saved; it can be changed in Settings → Brain. */
export const defaultCloudBudget = 5;
// Apple Intelligence can be turned on later in these cases; in the others it never runs here.
const appleLater = ['available', 'appleIntelligenceNotEnabled', 'modelNotReady', 'modelStuck', 'languageNotSupported', 'checking', undefined];

export function initialAnswers(settings: Settings, diagnostics: Diagnostics): Answers {
  // A brain the user picked stays. Otherwise: Apple Intelligence, their ChatGPT plan, a local model.
  const auto = settings.brainAuto;
  const appleOut = !appleLater.includes(diagnostics.appleIntelligence);
  const usablePlan = diagnostics.chatgptPlan === 'usable';
  const elsewhere: ModelBackend = !usablePlan && (settings.modelPath || diagnostics.chatgptPlan === 'refused') ? 'mlx' : 'cloud';
  const backend: ModelBackend = !auto ? settings.modelBackend
    : appleStatus(diagnostics.appleIntelligence).ready ? 'apple'
    : appleOut || usablePlan ? elsewhere : 'apple';
  return {
    userName: settings.userName, assistantName: settings.assistantName, coat: settings.catCoat,
    backend, pythonPath: settings.pythonPath, modelPath: settings.modelPath, localStarted: false,
    cloudProvider: auto ? 'chatgpt' : settings.cloudProvider, shares: settings.externalShares ?? [],
    roam: settings.petRoam, awareness: settings.petAwareness, nudges: settings.petNudges,
  };
}

export const next = (step: Step): Step => steps[Math.min(steps.length - 1, steps.indexOf(step) + 1)];
export const previous = (step: Step): Step => steps[Math.max(0, steps.indexOf(step) - 1)];

/** Why the user can't continue from this step yet, or '' when they can. */
export function blocker(step: Step, answers: Answers, diagnostics: Diagnostics): string {
  if (step === 'hello') {
    const name = answers.assistantName.trim();
    if (!name || name.length > 40) return 'Give me a name of up to 40 characters.';
    if (answers.userName.trim().length > 40) return 'Your name can be up to 40 characters.';
  }
  if (step === 'brain') {
    if (answers.backend === 'apple' && !appleLater.includes(diagnostics.appleIntelligence)) return `${appleStatus(diagnostics.appleIntelligence).advice || 'Apple Intelligence can’t run here.'} Sign in with ChatGPT or set up on this Mac.`;
    if (answers.backend === 'mlx' && !answers.localStarted && (!answers.pythonPath.trim() || !answers.modelPath.trim())) return 'Finish setting up on this Mac, or sign in with ChatGPT.';
    if (answers.backend === 'cloud' && answers.cloudProvider === 'chatgpt') {
      if (!diagnostics.chatgptAccount) return 'Sign in with ChatGPT, or set up on this Mac.';
      if (diagnostics.chatgptPlan === 'refused') return 'This ChatGPT account can’t use its plan in other apps. Set up on this Mac instead.';
    } else if (answers.backend === 'cloud') {
      if (!diagnostics.cloudKeys?.includes(answers.cloudProvider)) return 'Add your API key in Settings → Brain, or sign in with ChatGPT.';
    }
  }
  return '';
}

/** What Buntini says at each step. */
export function message(step: Step, answers: Answers, diagnostics: Diagnostics): string {
  const you = answers.userName.trim();
  switch (step) {
    case 'hello': return 'Hi! I’m the little cat in the corner of your screen. What should I call you?';
    case 'brain': {
      const apple = appleStatus(diagnostics.appleIntelligence);
      const hello = `Nice to meet you${you ? `, ${you}` : ''}!`;
      if (apple.ready) return `${hello} I’ll think with Apple Intelligence.`;
      if (diagnostics.appleIntelligence === 'checking' || diagnostics.appleIntelligence === undefined) return `${hello} Let me check how I can think on this Mac.`;
      return `${hello} Apple Intelligence isn’t ready on this Mac (${apple.label.toLowerCase()}). How should I think?`;
    }
    case 'roam': return 'Last one: may I wander around your screen and notice which apps you’re using?';
    case 'done': return `All set${you ? `, ${you}` : ''}! Ask me anything.`;
  }
}

/** The user's side of the conversation once a step is answered. */
export function reply(step: Step, answers: Answers, settings?: Pick<Settings, 'cloudBudgetUsd'>): string {
  switch (step) {
    case 'hello': {
      const coat = coats.find(item => item.id === answers.coat)?.label.toLowerCase() || 'cat';
      return `${answers.userName.trim() ? `Call me ${answers.userName.trim()}.` : 'Hi!'} You’re ${answers.assistantName.trim()}, my ${coat}.`;
    }
    case 'brain': {
      if (answers.backend === 'apple') return 'Use Apple Intelligence.';
      if (answers.backend === 'mlx') return 'Set up on this Mac.';
      const provider = cloudProviders.find(item => item.id === answers.cloudProvider)!;
      if (provider.id === 'chatgpt') return `Use ChatGPT with my own plan. ${sharedSentence(answers.shares)}`;
      return `Use ${provider.name}, up to $${cloudBudget(settings)} a month. ${sharedSentence(answers.shares)}`;
    }
    case 'roam': return answers.roam ? `Roam around${answers.awareness ? ' and notice what I’m up to' : ''}.` : 'Stay put for now.';
    case 'done': return '';
  }
}

/** The monthly cloud cap: the one saved earlier, else a small default. */
const cloudBudget = (settings?: Pick<Settings, 'cloudBudgetUsd'>) => settings && settings.cloudBudgetUsd > 0 ? settings.cloudBudgetUsd : defaultCloudBudget;

export function applyAnswers(settings: Settings, answers: Answers): Settings {
  const provider = cloudProviders.find(item => item.id === answers.cloudProvider)!;
  const sameProvider = settings.cloudProvider === answers.cloudProvider;
  return {
    ...settings,
    userName: answers.userName.trim(), assistantName: answers.assistantName.trim(), avatar: 'cat', catCoat: answers.coat,
    modelBackend: answers.backend, pythonPath: answers.pythonPath.trim(), modelPath: answers.modelPath.trim(),
    // A new provider starts on its suggested model and list prices; the cap carries over.
    cloudProvider: answers.cloudProvider, cloudModel: sameProvider ? settings.cloudModel : '', cloudBudgetUsd: cloudBudget(settings),
    cloudInputPrice: sameProvider ? settings.cloudInputPrice : provider.input, cloudOutputPrice: sameProvider ? settings.cloudOutputPrice : provider.output,
    // Only a cloud brain was asked; choosing another leaves the Privacy choices as they were.
    externalShares: answers.backend === 'cloud' ? answers.shares : settings.externalShares,
    petRoam: answers.roam, petAwareness: answers.awareness, petNudges: answers.awareness && answers.nudges,
  };
}

/** The welcome shortcuts that read an app. */
export type Shortcut = 'day' | 'reply';
const shortcutApps: Record<Shortcut, { reads: string[]; apps: string[]; ask: string; allow: string }> = {
  // The brief uses whichever of these is allowed, so any one of them is enough to start.
  day: { reads: ['calendar', 'reminders', 'mail'], apps: ['calendar', 'reminders'], ask: 'I’ll need Calendar and Reminders for that.', allow: 'Allow Calendar and Reminders' },
  reply: { reads: ['mail'], apps: ['mail'], ask: 'I’ll need Mail for that.', allow: 'Allow Mail' },
};

/** What a welcome shortcut still needs on a fresh install, so it can be allowed in place
    instead of dead-ending; null when it can run. */
export function shortcutNeeds(shortcut: Shortcut, allowed: string[]): { apps: string[]; ask: string; allow: string } | null {
  const { reads, apps, ask, allow } = shortcutApps[shortcut];
  return reads.some(app => allowed.includes(app)) ? null : { apps, ask, allow };
}

/** The apps the connect card offers. */
const CONNECTABLE = ['mail', 'calendar'];
export type ConnectRow = AppStatus & { state: AppConnection | 'off' };
/** The card's rows. An app allowed before needs no second look; `found` is what connecting
 * just found, and null is a try that failed, which can be tried again. */
export function connectRows(apps: AppStatus[], allowed: string[], found: Record<string, AppConnection | null>): ConnectRow[] {
  return CONNECTABLE.flatMap(id => {
    const app = apps.find(item => item.id === id);
    if (!app?.installed) return [];
    const known = found[id];
    return [{ ...app, state: known === null ? 'off' as const : known ?? (allowed.includes(id) ? 'ready' as const : 'off' as const) }];
  });
}
/** Where System Settings can fix a row, if anywhere. */
export function connectPane(row: ConnectRow) {
  if (row.state === 'noAccount') return 'accounts';
  return row.state === 'denied' ? (row.id === 'mail' ? 'automation' : 'calendars') : null;
}
