/**
 * Types mirrored from the Rust IPC surface (`src-tauri/src/commands.rs` and
 * `bytepet-core`). Keep in sync by hand — the surface is small and stable.
 */

export type PetState =
  | "idle"
  | "running-right"
  | "running-left"
  | "waving"
  | "jumping"
  | "failed"
  | "waiting"
  | "running"
  | "review"
  | "look-row-9"
  | "look-row-10";

export interface FrameSpec {
  width: number;
  height: number;
  columns: number;
  rows: number;
}

export interface Animation {
  state: PetState;
  row: number;
  sprites: number[];
  durationsMs: number[];
  loopAnim: boolean;
  fallback: PetState;
  totalMs: number;
}

export interface PetEntry {
  id: string;
  displayName: string;
  description: string | null;
  root: "app-data" | "codex" | "unipet" | "custom";
  dir: string;
  spritesheet: string;
  frame: FrameSpec;
  spriteVersionNumber: number | null;
  linked: boolean;
  animations: Partial<Record<PetState, Animation>>;
}

export interface LibraryRoot {
  kind: "app-data" | "codex" | "unipet" | "custom";
  path: string;
  writable: boolean;
}

export interface PersonaTraits {
  tone: string;
  verbosity: "short" | "normal" | "detailed" | string;
  language: string;
  emoji: boolean;
}

export interface SamplingConfig {
  temperature: number;
  maxTokens: number;
}

export interface ModelRef {
  provider: string;
  model: string | null;
}

export interface PersonaMemoryConfig {
  enabled: boolean;
  windowTurns: number;
  longTerm: boolean;
  summarizeAfterTurns: number;
}

export interface PersonaTtsConfig {
  enabled: boolean;
  voice: string | null;
  rate: number;
}

export interface ProactiveConfig {
  enabled: boolean;
  idleMinutes: number;
}

export interface Persona {
  id: string;
  name: string;
  description: string | null;
  avatarPet: string | null;
  systemPrompt: string;
  greeting: string | null;
  traits: PersonaTraits;
  sampling: SamplingConfig;
  model: ModelRef | null;
  memory: PersonaMemoryConfig;
  tts: PersonaTtsConfig;
  proactive: ProactiveConfig;
  builtin: boolean;
}

export type ProviderKind =
  | "anthropic"
  | "open-ai-chat"
  | "open-ai-responses"
  | "codex-cli"
  | "claude-cli";

export interface ProviderConfig {
  id: string;
  label: string;
  kind: ProviderKind;
  baseUrl: string | null;
  model: string;
  apiKeyRef: string | null;
  extraHeaders: Record<string, string>;
  enabled: boolean;
  options: Record<string, unknown>;
}

export interface PetWindowConfig {
  scale: number;
  opacity: number;
  alwaysOnTop: boolean;
  clickThrough: "auto" | "rect" | "passthrough";
  startPosition: { x: number; y: number; monitor: number | null } | null;
  autoWalk: {
    enabled: boolean;
    speedPxS: number;
    pauseSeconds: number;
    edgeMargin: number;
    userGraceSeconds: number;
  };
}

export interface AppConfig {
  schemaVersion: number;
  activePet: string | null;
  activePersona: string | null;
  defaultProvider: string | null;
  pet: PetWindowConfig;
  chat: {
    sendOnEnter: boolean;
    showReasoning: boolean;
    speakReplies: boolean;
    maxContextTurns: number;
  };
  agent: {
    enabled: boolean;
    port: number;
    autoInstallHooks: boolean;
    defaultTtlSeconds: number;
  };
  tts: { enabled: boolean; voice: string | null; rate: number; maxChars: number };
  ui: { language: string; theme: string; launchAtLogin: boolean };
  providers: Record<string, ProviderConfig>;
}

export interface BootstrapState {
  config: AppConfig;
  pets: PetEntry[];
  personas: Persona[];
  activePet: PetEntry | null;
  activePersona: Persona | null;
  libraryRoots: LibraryRoot[];
  agentUrl: string;
  dataDir: string;
  keyringAvailable: boolean;
}

export interface PetStateEvent {
  state: PetState;
  source: string;
  message: string | null;
  oneShot: boolean;
}

export interface ChatMessageView {
  id: number;
  role: "user" | "assistant" | "system";
  content: string;
  provider: string | null;
  model: string | null;
  tokens: number;
  createdAt: number;
}

export interface ConversationView {
  id: string;
  personaId: string;
  title: string;
  createdAt: number;
  updatedAt: number;
  messageCount: number;
}

export interface ValidationReport {
  ok: boolean;
  id: string;
  displayName: string;
  dir: string;
  spritesheet: string | null;
  frame: FrameSpec;
  imageSize: [number, number] | null;
  animations: { state: PetState; frames: number; loopAnim: boolean; fallback: PetState }[];
  errors: string[];
  warnings: string[];
}
