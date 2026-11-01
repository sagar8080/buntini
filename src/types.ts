export type Mode = 'auto' | 'files' | 'system' | 'web';
export type PetState = 'idle' | 'listening' | 'thinking' | 'working' | 'speaking' | 'approval';
export type CatCoat = 'ginger' | 'gray' | 'black' | 'white' | 'tuxedo' | 'calico' | 'siamese';
export type CatEyes = 'amber' | 'green' | 'blue' | 'copper' | 'odd';
export type CatAccessory = 'none' | 'bell' | 'bow' | 'bandana' | 'hat' | 'flowers';
export type PetMotion = 'calm' | 'playful' | 'sleepy';
/** accent is the resolved accessory colour (#RRGGBB). */
export interface PetLook { coat: CatCoat; eyes: CatEyes; accessory: CatAccessory; motion: PetMotion; accent: string }
export type ModelBackend = 'apple' | 'mlx' | 'cloud';
export type CloudProvider = 'anthropic' | 'openai' | 'gemini' | 'chatgpt';
export type KeyProvider = 'brave' | 'tavily' | 'ebay' | 'duffel' | CloudProvider;
export interface Settings {
  modelBackend: ModelBackend;
  assistantName: string;
  userName: string;
  personality: string;
  avatar: 'cat' | 'orb';
  accent: 'preset' | 'ember' | 'ocean' | 'violet' | 'sage' | 'custom';
  accentColor: string;
  themePreset: 'classic' | 'midnight' | 'paper' | 'sakura' | 'forest' | 'ocean';
  textSize: 'small' | 'medium' | 'large';
  density: 'compact' | 'comfortable';
  chatFont: 'sans' | 'rounded' | 'serif' | 'mono';
  corners: 'round' | 'soft' | 'square';
  panelSize: 'compact' | 'standard' | 'large';
  statusText: string;
  companionScale: number;
  catCoat: CatCoat;
  catEyes: CatEyes;
  catAccessory: CatAccessory;
  petMotion: PetMotion;
  petRoam: boolean;
  petAwareness: boolean;
  petNudges: boolean;
  voiceId: string;
  voiceRate: number;
  notificationsEnabled: boolean;
  morningBrief: boolean;
  headsUps: boolean;
  openAtLogin: boolean;
  monthlyBudgetUsd: number;
  costPerSearchUsd: number;
  vadModelPath: string;
  pythonPath: string;
  modelPath: string;
  whisperModel: string;
  searchProvider: 'brave' | 'tavily';
  monthlySearchLimit: number;
  dailySearchLimit: number;
  freeTierOnly: boolean;
  speakResponses: boolean;
  /** Answer out loud when the user asked by voice. */
  speakVoiceReplies: boolean;
  theme: 'system' | 'light' | 'dark';
  reducedMotion: boolean;
  ebayMarketplace: string;
  externalShares: string[];
  /** Apps Buntini may use (Settings → Apps). */
  apps: string[];
  cloudProvider: CloudProvider;
  cloudModel: string;
  cloudBudgetUsd: number;
  cloudInputPrice: number;
  cloudOutputPrice: number;
}
export interface Source { title: string; url: string; snippet: string; retrievedAt: string }
export interface ResearchContent { url: string; title: string; text: string; retrievedAt: string; truncated: boolean }
export interface PriceOffer { product: string; url: string; price: string; currency: string; availability?: string; seller?: string; isAggregate: boolean; highPrice?: string }
export interface CardField { label: string; value: string }
export interface ResultCard { kind: 'weather' | 'currency' | 'article' | 'place' | 'listing' | 'flight'; title: string; subtitle: string; fields: CardField[]; url?: string; attribution: string; retrievedAt: string }
export interface FileHit { name: string; path: string; size: number; modifiedAt: string; excerpt?: string }
export interface TaskRecord {
  id: string;
  prompt: string;
  answer: string;
  state: 'completed' | 'error' | 'cancelled' | 'running' | 'paused' | 'unknown' | 'waiting_action' | 'waiting_online';
  createdAt: string;
  sources: Source[];
  files: FileHit[];
  research?: ResearchContent[];
  prices?: PriceOffer[];
  cards?: ResultCard[];
  system?: Record<string, unknown>;
  backend?: string;
  touched?: string[];
  /** 'online' when the model asked to look something up; the app then asks for approval. */
  handoff?: string;
  action?: import('./ActionSheet').ActionProposal;
}
export interface Diagnostics {
  modelReady: boolean;
  appleIntelligence?: string;
  workerStatus: string;
  braveConfigured: boolean;
  tavilyConfigured: boolean;
  ebayConfigured?: boolean;
  duffelConfigured?: boolean;
  searchesUsed: number;
  searchesToday?: number;
  searchLimitMonth?: number;
  searchLimitDay?: number;
  estimatedSpendUsd?: number;
  remainingBudgetUsd?: number;
  cloudKeys?: string[];
  cloudSpendUsd?: number;
  chatgptAccount?: string | null;
}
export interface AuditEntry { id?: number; timestamp?: string; createdAt?: string; action: string; detail?: string; outcome?: string }
/** Something done on its own at a set time; the morning brief is the built-in one. */
export interface Routine { id: string; task: string; days: string; time: string; enabled: boolean; builtIn?: boolean; createdAt: string; nextRunAt: string; lastRunAt?: string; lastTaskId?: string; lastError?: string }
/** An aim that takes days, with check-ins until its last day or the end the user named. */
export interface Goal { id: string; aim: string; checkIn: string; style: 'run' | 'remind'; days: string; time: string; until: string; doneWhen: 'date' | 'reply' | 'you'; person?: string; state: 'active' | 'paused' | 'done' | 'ended'; createdAt: string; endedAt?: string; outcome?: string; lastTaskId?: string; note?: string }
export interface SavedSearch { id: string; query: string; intervalMinutes: number; enabled: boolean; createdAt: string; nextRunAt: string; lastResultTaskId?: string; lastError?: string }
export interface PricePoint { price: string; currency: string; observedAt: string; url: string }
export interface PriceWatch { id: string; url: string; label: string; currency: string; threshold: string; intervalMinutes: number; enabled: boolean; createdAt: string; nextRunAt: string; lastCheckedAt?: string; lastPrice?: string; lastError?: string; history: PricePoint[] }
export interface Voice { id: string; name: string; language: string }
export interface WebApproval { id: string; prompt: string; provider: string; expiresAt: string; approved: boolean; tool: string; arguments: Record<string, unknown>; service: string; summary: string; sends: CardField[]; host: string; usesKey: boolean; note?: string }
export interface ConversationSummary { id: string; title: string; createdAt: string; updatedAt: string; legacy: boolean; turnCount: number }
export interface ConversationPage { conversation: ConversationSummary; turns: TaskRecord[]; nextCursor: string | null }
export interface ConversationList { items: ConversationSummary[]; nextCursor: string | null }
export interface Bootstrap { settings: Settings; tasks: TaskRecord[]; conversation: ConversationPage; conversations: ConversationList; roots: string[]; diagnostics: Diagnostics; audit: AuditEntry[]; savedSearches: SavedSearch[]; priceWatches: PriceWatch[]; routines?: Routine[]; goals?: Goal[]; conversationStart?: string | null; onboarded?: boolean; memories?: Memory[]; briefDue?: boolean }
export interface AppStatus { id: string; name: string; can: string[]; note?: string | null; installed: boolean; allowed: boolean }
export interface AppList { apps: AppStatus[]; others: string[] }
export interface Memory { id: number; fact: string; createdAt: string }
export interface TaskEvent { type: 'state' | 'delta' | 'tool' | 'done' | 'error' | 'notice'; taskId: string; state?: string; text?: string; tool?: string; task?: TaskRecord }
export interface TaskOutcome { id: string; kind: string; description: string; tool?: string | null; acceptedTools: string[]; resources: string[]; dependencies: string[]; status: string; requiredArguments: Record<string, unknown>; requiredFields: string[]; completedBy: string[]; sourceRevision: number }
export interface FieldProvenance { value: unknown; origin: string; sourceStepId?: string | null; recordId?: string | null; fieldRef?: string | null; calculation?: unknown; source?: unknown; approved: boolean; verified: boolean; previous?: FieldProvenance | null }
export interface TaskContract { version: number; originalRequest: string; currentRequest: string; revision: number; intentStatus: string; outcomes: TaskOutcome[]; unresolvedFields: { outcomeId: string; field: string; reason: string; status: string }[]; constraints: unknown[]; provenance: Record<string, Record<string, FieldProvenance>>; revisions: unknown[] }
export interface TaskFieldUpdate { outcomeId: string; fields: Record<string, string> }
export interface TaskProgress { id: string; goal: string; state: string; updatedAt: string; error?: string; canResume?: boolean; contract?: TaskContract; steps: { id: string; tool: string; arguments: Record<string, unknown>; state: string; approvedAt?: string; finishedAt?: string; evidence?: unknown }[] }
/** What macOS reports for Open at login. */
export type LoginStatus = 'enabled' | 'requiresApproval' | 'notRegistered' | 'notFound' | 'unavailable';
/** A saved proposal waiting for the person; `description` never holds field values. */
/** A heads-up the window lists until it is opened, dismissed or no longer relevant. */
export interface HeadsUp { id: string; kind: 'meeting' | 'reminders' | 'reply'; title: string; detail: string; createdAt: string }
export interface PendingApprovalSummary { taskId: string; approvalId: string; tool: string; description: string; goal: string; createdAt: string }
export interface BackgroundStatus { executionMode: string; limitation: string; openAtLogin?: LoginStatus; timers: { id: string; seconds: number; dueAt: string; state: string; lateSeconds: number; error?: string }[]; monitors: { id: string; kind: string; label: string; enabled: boolean; nextRunAt: string; lastSuccessAt?: string; overdueSeconds: number; lastDeliveryError?: string; lastRun?: { state: string; lateSeconds: number; error?: string } }[] }
