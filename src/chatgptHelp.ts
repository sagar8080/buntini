import type { Settings, TaskRecord } from './types';

export type ChatGptPlan = 'none' | 'usable' | 'refused' | undefined;
export const CHATGPT_URL = 'https://chatgpt.com/';

/** The question in "Ask ChatGPT …", without the part addressed to Buntini. */
export function askChatGptQuestion(prompt: string): string | null {
  const match = /^\s*(?:please\s+|can you\s+)?ask\s+chat\s?gpt\s*(?:[:,–—-]\s*|\s+to\s+|\s+)([\s\S]+)$/i.exec(prompt);
  return match?.[1].trim() || null;
}

/** The exact question sent to ChatGPT for this reply, if one was. */
export function askedChatGpt(task: TaskRecord): string | undefined {
  return task.researchReport?.attempts.find(attempt => attempt.route === 'chatgpt')?.detail;
}

const wantsImage = (prompt: string) => /\b(draw|generate|create|make|paint|design)\b[\s\S]*\b(image|picture|drawing|illustration|logo|painting|poster)\b/i.test(prompt);
const couldNotHelp = (answer: string) => answer.length < 240 && /\b(i can(?:['’]t|not)|i['’]m (?:not able|unable)|i am (?:not able|unable)|i don['’]t have access)\b/i.test(answer);

/** What ChatGPT can offer on this reply. Only ever a button; nothing goes anywhere by itself. */
export function chatGptHelp(task: TaskRecord, plan: ChatGptPlan, settings?: Pick<Settings, 'cloudProvider'>): 'ask' | 'open' | null {
  if (task.state !== 'completed' && task.state !== 'error') return null;
  if (wantsImage(task.prompt)) return 'open';
  if (askedChatGpt(task) !== undefined) return task.state === 'error' ? 'open' : null;
  if (task.backend === 'cloud') return task.state === 'error' && settings?.cloudProvider === 'chatgpt' ? 'open' : null;
  if (task.backend !== 'apple' && task.backend !== 'mlx') return null;
  if (task.state === 'completed' && !couldNotHelp(task.answer)) return null;
  return plan === 'usable' ? 'ask' : 'open';
}
