// First-run setup, guided by Buntini in the chat bubble. Pure step logic: what Buntini
// says, what the user's answer reads like, what blocks a step, and the settings it saves.
import { appleStatus, cloudProviders } from './brain.ts';
import { sharedSentence } from './cloudShares.ts';
import { language, number, t } from './i18n.ts';
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
    if (!name || name.length > 40) return t('onboarding.blocker.assistantName');
    if (answers.userName.trim().length > 40) return t('onboarding.blocker.userName');
  }
  if (step === 'brain') {
    if (answers.backend === 'apple' && !appleLater.includes(diagnostics.appleIntelligence)) return t('onboarding.blocker.appleUnavailable', { reason: appleStatus(diagnostics.appleIntelligence).advice || t('onboarding.blocker.appleCantRun') });
    if (answers.backend === 'mlx' && !answers.localStarted && (!answers.pythonPath.trim() || !answers.modelPath.trim())) return t('onboarding.blocker.finishLocal');
    if (answers.backend === 'cloud' && answers.cloudProvider === 'chatgpt') {
      if (!diagnostics.chatgptAccount) return t('onboarding.blocker.signIn');
      if (diagnostics.chatgptPlan === 'refused') return t('onboarding.blocker.planRefused');
    } else if (answers.backend === 'cloud') {
      if (!diagnostics.cloudKeys?.includes(answers.cloudProvider)) return t('onboarding.blocker.apiKey');
    }
  }
  return '';
}

/** What Buntini says at each step. */
export function message(step: Step, answers: Answers, diagnostics: Diagnostics): string {
  const you = answers.userName.trim();
  switch (step) {
    case 'hello': return t('onboarding.message.hello');
    case 'brain': {
      const apple = appleStatus(diagnostics.appleIntelligence);
      if (apple.ready) return you ? t('onboarding.message.brainAppleNamed', { name: you }) : t('onboarding.message.brainApple');
      if (diagnostics.appleIntelligence === 'checking' || diagnostics.appleIntelligence === undefined) return you ? t('onboarding.message.brainCheckingNamed', { name: you }) : t('onboarding.message.brainChecking');
      // Lower case reads right inside an English sentence; other languages keep the label as written.
      const status = language() === 'en' ? apple.label.toLowerCase() : apple.label;
      return you ? t('onboarding.message.brainNotReadyNamed', { name: you, status }) : t('onboarding.message.brainNotReady', { status });
    }
    case 'roam': return t('onboarding.message.roam');
    case 'done': return you ? t('onboarding.message.doneNamed', { name: you }) : t('onboarding.message.done');
  }
}

/** The user's side of the conversation once a step is answered. */
export function reply(step: Step, answers: Answers, settings?: Pick<Settings, 'cloudBudgetUsd'>): string {
  switch (step) {
    case 'hello': {
      const coat = t(myCoat[answers.coat] ?? 'onboarding.myCoat.cat'), name = answers.assistantName.trim(), user = answers.userName.trim();
      return user ? t('onboarding.reply.helloNamed', { user, name, coat }) : t('onboarding.reply.hello', { name, coat });
    }
    case 'brain': {
      if (answers.backend === 'apple') return t('onboarding.reply.apple');
      if (answers.backend === 'mlx') return t('onboarding.reply.local');
      const provider = cloudProviders.find(item => item.id === answers.cloudProvider)!;
      if (provider.id === 'chatgpt') return t('onboarding.reply.chatgpt', { shared: sharedSentence(answers.shares) });
      return t('onboarding.reply.cloud', { provider: provider.name, budget: number(cloudBudget(settings)), shared: sharedSentence(answers.shares) });
    }
    case 'roam': return answers.roam ? t(answers.awareness ? 'onboarding.reply.roamNotice' : 'onboarding.reply.roam') : t('onboarding.reply.stay');
    case 'done': return '';
  }
}

/** The coat as it reads in "You’re Miso, my calico." */
const myCoat: Partial<Record<CatCoat, string>> = {
  ginger: 'onboarding.myCoat.ginger', gray: 'onboarding.myCoat.gray', black: 'onboarding.myCoat.black', white: 'onboarding.myCoat.white',
  tuxedo: 'onboarding.myCoat.tuxedo', calico: 'onboarding.myCoat.calico', siamese: 'onboarding.myCoat.siamese',
};

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
  day: { reads: ['calendar', 'reminders', 'mail'], apps: ['calendar', 'reminders'], ask: 'setup.shortcut.dayAsk', allow: 'setup.shortcut.dayAllow' },
  reply: { reads: ['mail'], apps: ['mail'], ask: 'setup.shortcut.replyAsk', allow: 'setup.shortcut.replyAllow' },
};

/** What a welcome shortcut still needs on a fresh install, so it can be allowed in place
    instead of dead-ending; null when it can run. */
export function shortcutNeeds(shortcut: Shortcut, allowed: string[]): { apps: string[]; ask: string; allow: string } | null {
  const { reads, apps, ask, allow } = shortcutApps[shortcut];
  return reads.some(app => allowed.includes(app)) ? null : { apps, ask: t(ask), allow: t(allow) };
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
