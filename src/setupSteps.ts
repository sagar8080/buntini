// First-run setup, guided by Buntini in the chat bubble. Pure step logic: what Buntini
// says, what the user's answer reads like, what blocks a step, and the settings it saves.
import { appleStatus, cloudProviders } from './brain.ts';
import { coats } from './petOptions.ts';
import type { CatCoat, CloudProvider, Diagnostics, KeyProvider, ModelBackend, Settings } from './types';

export type Step = 'hello' | 'brain' | 'online' | 'roam' | 'done';
export const steps: Step[] = ['hello', 'brain', 'online', 'roam', 'done'];

export interface Answers {
  userName: string; assistantName: string; coat: CatCoat;
  backend: ModelBackend; pythonPath: string; modelPath: string;
  cloudProvider: CloudProvider; cloudModel: string; cloudKey: string; cloudBudget: number;
  searchProvider: 'brave' | 'tavily'; searchKey: string; freeTier: boolean; dailyLimit: number;
  roam: boolean; awareness: boolean; nudges: boolean;
}

const searchNames = { brave: 'Brave Search', tavily: 'Tavily' };
// Apple Intelligence can be turned on later in these cases; in the others it never runs here.
const appleLater = ['available', 'appleIntelligenceNotEnabled', 'modelNotReady', 'checking', undefined];

export function initialAnswers(settings: Settings, diagnostics: Diagnostics): Answers {
  const appleUsable = appleLater.includes(diagnostics.appleIntelligence);
  const backend: ModelBackend = settings.modelBackend === 'apple' && !appleUsable ? (settings.modelPath ? 'mlx' : 'cloud') : settings.modelBackend;
  return {
    userName: settings.userName, assistantName: settings.assistantName, coat: settings.catCoat,
    backend, pythonPath: settings.pythonPath, modelPath: settings.modelPath,
    cloudProvider: settings.cloudProvider, cloudModel: settings.cloudModel, cloudKey: '', cloudBudget: settings.cloudBudgetUsd,
    searchProvider: settings.searchProvider, searchKey: '', freeTier: settings.freeTierOnly, dailyLimit: settings.dailySearchLimit,
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
    if (answers.backend === 'apple' && !appleLater.includes(diagnostics.appleIntelligence)) return `${appleStatus(diagnostics.appleIntelligence).advice || 'Apple Intelligence can’t run here.'} Choose a local or cloud model instead.`;
    if (answers.backend === 'mlx' && (!answers.pythonPath.trim() || !answers.modelPath.trim())) return 'Add the Python interpreter and the model folder for your local model.';
    if (answers.backend === 'cloud' && answers.cloudProvider === 'chatgpt') {
      if (!diagnostics.chatgptAccount) return 'Sign in with ChatGPT, or choose another brain.';
    } else if (answers.backend === 'cloud') {
      if (!answers.cloudKey.trim() && !diagnostics.cloudKeys?.includes(answers.cloudProvider)) return 'Paste your API key, or choose another brain.';
      if (!(answers.cloudBudget > 0)) return 'Set a monthly spending cap above $0.';
    }
  }
  if (step === 'online' && !(Number.isInteger(answers.dailyLimit) && answers.dailyLimit >= 1 && answers.dailyLimit <= 2000)) return 'Choose a daily limit between 1 and 2,000.';
  return '';
}

/** What Buntini says at each step. */
export function message(step: Step, answers: Answers, diagnostics: Diagnostics): string {
  const you = answers.userName.trim();
  switch (step) {
    case 'hello': return 'Hi! I’m the little cat who lives in the corner of your screen. What should I call you? You can rename me and pick my coat too.';
    case 'brain': {
      const apple = appleStatus(diagnostics.appleIntelligence);
      const nudge = apple.ready ? 'Apple Intelligence is built into your Mac: private, quick and nothing to set up.' : `Apple Intelligence isn’t ready on this Mac (${apple.label.toLowerCase()}), so a local or cloud model might suit you better.`;
      return `Nice to meet you${you ? `, ${you}` : ''}! Which brain should I think with? ${nudge}`;
    }
    case 'online': return 'I search in the browser first and read relevant pages. You can add an optional Brave or Tavily key for fallback searches and saved searches, with a request limit. I ask you to approve online research before it starts.';
    case 'roam': return `Last one: may I wander around your screen and notice which apps you’re using? That stays on this Mac${answers.backend === 'cloud' ? ', and the cloud model only sees it if you allow it in Settings' : ''}. Hold the mic to talk to me any time.`;
    case 'done': return `All set${you ? `, ${you}` : ''}! Ask me anything. I’ll remember our chat until you start a new topic.`;
  }
}

/** The user's side of the conversation once a step is answered. */
export function reply(step: Step, answers: Answers): string {
  switch (step) {
    case 'hello': {
      const coat = coats.find(item => item.id === answers.coat)?.label.toLowerCase() || 'cat';
      return `${answers.userName.trim() ? `Call me ${answers.userName.trim()}.` : 'Hi!'} You’re ${answers.assistantName.trim()}, my ${coat}.`;
    }
    case 'brain': {
      if (answers.backend === 'apple') return 'Use Apple Intelligence.';
      if (answers.backend === 'mlx') return 'Use my local MLX model.';
      const provider = cloudProviders.find(item => item.id === answers.cloudProvider)!;
      if (provider.id === 'chatgpt') return 'Use ChatGPT with my own plan.';
      return `Use ${provider.name} (${answers.cloudModel.trim() || provider.model}), up to $${answers.cloudBudget} a month.`;
    }
    case 'online': return answers.searchKey.trim() ? `Use ${searchNames[answers.searchProvider]} as fallback, up to ${answers.dailyLimit} API requests a day.` : 'Use browser search without an API fallback for now.';
    case 'roam': return answers.roam ? `Roam around${answers.awareness ? ' and notice what I’m up to' : ''}.` : 'Stay put for now.';
    case 'done': return '';
  }
}

export function applyAnswers(settings: Settings, answers: Answers): Settings {
  const provider = cloudProviders.find(item => item.id === answers.cloudProvider)!;
  const sameProvider = settings.cloudProvider === answers.cloudProvider;
  return {
    ...settings,
    userName: answers.userName.trim(), assistantName: answers.assistantName.trim(), avatar: 'cat', catCoat: answers.coat,
    modelBackend: answers.backend, pythonPath: answers.pythonPath.trim(), modelPath: answers.modelPath.trim(),
    cloudProvider: answers.cloudProvider, cloudModel: answers.cloudModel.trim(), cloudBudgetUsd: answers.cloudBudget,
    // A new provider starts from its own suggested prices.
    cloudInputPrice: sameProvider ? settings.cloudInputPrice : provider.input, cloudOutputPrice: sameProvider ? settings.cloudOutputPrice : provider.output,
    searchProvider: answers.searchProvider, freeTierOnly: answers.freeTier, dailySearchLimit: answers.dailyLimit,
    petRoam: answers.roam, petAwareness: answers.awareness, petNudges: answers.awareness && answers.nudges,
  };
}

/** Keys typed during setup, saved to the Keychain before the settings. */
export function keysToSave(answers: Answers): [KeyProvider, string][] {
  const keys: [KeyProvider, string][] = [];
  if (answers.backend === 'cloud' && answers.cloudProvider !== 'chatgpt' && answers.cloudKey.trim()) keys.push([answers.cloudProvider, answers.cloudKey.trim()]);
  if (answers.searchKey.trim()) keys.push([answers.searchProvider, answers.searchKey.trim()]);
  return keys;
}
