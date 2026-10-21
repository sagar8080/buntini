export type Mode = 'auto' | 'files' | 'system' | 'web';
export type PetState = 'idle' | 'listening' | 'thinking' | 'working' | 'speaking' | 'approval';
export type CatCoat = 'ginger' | 'gray' | 'black' | 'white' | 'tuxedo' | 'calico' | 'siamese';
export type CatEyes = 'amber' | 'green' | 'blue' | 'copper' | 'odd';
export type CatAccessory = 'none' | 'bell' | 'bow' | 'bandana' | 'hat' | 'flowers';
export type PetMotion = 'calm' | 'playful' | 'sleepy';
export interface PetLook { coat: CatCoat; eyes: CatEyes; accessory: CatAccessory; motion: PetMotion; accent: Settings['accent'] }
export interface Settings {
  assistantName: string;
  personality: string;
  avatar: 'kyuubi' | 'fox' | 'cat' | 'orb';
  accent: 'ember' | 'ocean' | 'violet' | 'sage';
  companionScale: number;
  catCoat: CatCoat;
  catEyes: CatEyes;
  catAccessory: CatAccessory;
  petMotion: PetMotion;
  voiceId: string;
  voiceRate: number;
  notificationsEnabled: boolean;
  monthlyBudgetUsd: number;
  costPerSearchUsd: number;
  vadModelPath: string;
  pythonPath: string;
  modelPath: string;
  whisperModel: string;
  searchProvider: 'brave' | 'tavily';
  monthlySearchLimit: number;
  speakResponses: boolean;
  theme: 'system' | 'light' | 'dark';
  reducedMotion: boolean;
  ebayMarketplace: string;
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
  state: 'completed' | 'error' | 'cancelled' | 'running';
  createdAt: string;
  sources: Source[];
  files: FileHit[];
  research?: ResearchContent[];
  prices?: PriceOffer[];
  cards?: ResultCard[];
  system?: Record<string, unknown>;
}
export interface Diagnostics {
  modelReady: boolean;
  workerStatus: string;
  braveConfigured: boolean;
  tavilyConfigured: boolean;
  ebayConfigured?: boolean;
  duffelConfigured?: boolean;
  searchesUsed: number;
  estimatedSpendUsd?: number;
  remainingBudgetUsd?: number;
}
export interface AuditEntry { id?: number; timestamp?: string; createdAt?: string; action: string; detail?: string; outcome?: string }
export interface SavedSearch { id: string; query: string; intervalMinutes: number; enabled: boolean; createdAt: string; nextRunAt: string; lastResultTaskId?: string; lastError?: string }
export interface PricePoint { price: string; currency: string; observedAt: string; url: string }
export interface PriceWatch { id: string; url: string; label: string; currency: string; threshold: string; intervalMinutes: number; enabled: boolean; createdAt: string; nextRunAt: string; lastCheckedAt?: string; lastPrice?: string; lastError?: string; history: PricePoint[] }
export interface Voice { id: string; name: string; language: string }
export interface WebApproval { id: string; prompt: string; provider: string; expiresAt: string; approved: boolean; tool: string; arguments: Record<string, unknown>; service: string; summary: string; sends: CardField[]; host: string; usesKey: boolean; note?: string }
export interface Bootstrap { settings: Settings; tasks: TaskRecord[]; roots: string[]; diagnostics: Diagnostics; audit: AuditEntry[]; savedSearches: SavedSearch[]; priceWatches: PriceWatch[] }
export interface TaskEvent { type: 'state' | 'delta' | 'tool' | 'done' | 'error'; taskId: string; state?: string; text?: string; tool?: string; task?: TaskRecord }
