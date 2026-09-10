/**
 * The single IPC boundary of the frontend.
 *
 * Every Tauri command is wrapped here and returns a discriminated
 * [`IpcResult`] — callers never see a rejected promise, so `try/catch` never has
 * to be repeated in components. Rust error strings (`Result<_, String>`) are
 * normalised into `error`.
 *
 * Event subscriptions are thin typed wrappers over `listen`, one per channel in
 * the Rust `events` module.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  BootstrapState,
  ChatMessageView,
  ConversationView,
  Persona,
  PetEntry,
  PetStateEvent,
  ProviderConfig,
  ValidationReport,
} from "./types";

export type IpcResult<T> = { ok: true; value: T } | { ok: false; error: string };

export function ok<T>(value: T): IpcResult<T> {
  return { ok: true, value };
}

export function fail<T = never>(error: unknown): IpcResult<T> {
  return { ok: false, error: describeError(error) };
}

export function isOk<T>(result: IpcResult<T>): result is { ok: true; value: T } {
  return result.ok;
}

export function unwrapOr<T>(result: IpcResult<T>, fallback: T): T {
  return result.ok ? result.value : fallback;
}

/** Turn whatever Tauri rejected with into a human-readable string. */
export function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object") {
    const record = error as Record<string, unknown>;
    if (typeof record.message === "string") return record.message;
    if (typeof record.error === "string") return record.error;
    try {
      return JSON.stringify(error);
    } catch {
      return String(error);
    }
  }
  return String(error);
}

export async function invokeCommand<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<IpcResult<T>> {
  try {
    const value = await invoke<T>(command, args);
    return ok(value);
  } catch (error) {
    return fail<T>(error);
  }
}

// --- local shapes not (yet) mirrored in shared/types.ts --------------------

export interface MemoryFact {
  id: number;
  key: string;
  value: string;
  confidence: number;
  updatedAt: number;
}

export interface ProviderTestResult {
  ok: boolean;
  message: string;
  latencyMs?: number;
}

export interface HookStatus {
  kind: "codex" | "claude-code";
  configPath: string;
  exists: boolean;
  installed: boolean;
  detail: string;
}

export interface HooksReport {
  ok?: boolean;
  messages?: string[];
  detail?: string;
  changed?: boolean;
}

export interface ChatDeltaEvent {
  conversationId: string;
  kind: "text" | "reasoning" | "status";
  text: string;
}

export interface ChatDoneEvent {
  conversationId: string;
  messageId: number;
  content: string;
  inputTokens: number;
  outputTokens: number;
  finishReason: string | null;
}

export interface ChatErrorEvent {
  conversationId: string;
  message: string;
}

export interface TtsStateEvent {
  speaking: boolean;
}

// --- commands --------------------------------------------------------------

export const ipc = {
  // bootstrap / pets ------------------------------------------------------
  getBootstrapState: () => invokeCommand<BootstrapState>("get_bootstrap_state"),
  listPets: () => invokeCommand<PetEntry[]>("list_pets"),
  validatePet: (path: string) => invokeCommand<ValidationReport>("validate_pet", { path }),
  importPet: (path: string, overwrite = false) =>
    invokeCommand<BootstrapState>("import_pet", { path, overwrite }),
  exportPet: (id: string, out: string) => invokeCommand<void>("export_pet", { id, out }),
  removePet: (id: string) => invokeCommand<BootstrapState>("remove_pet", { id }),
  usePet: (id: string) => invokeCommand<BootstrapState>("use_pet", { id }),

  // personas --------------------------------------------------------------
  listPersonas: () => invokeCommand<Persona[]>("list_personas"),
  personaTemplates: () => invokeCommand<Record<string, Persona>>("persona_templates"),
  savePersona: (persona: Persona) => invokeCommand<Persona[]>("save_persona", { persona }),
  deletePersona: (id: string) => invokeCommand<Persona[]>("delete_persona", { id }),
  duplicatePersona: (id: string, newId: string, newName: string) =>
    invokeCommand<Persona[]>("duplicate_persona", { id, newId, newName }),
  importPersona: (path: string, overwrite = false) =>
    invokeCommand<Persona[]>("import_persona", { path, overwrite }),
  exportPersona: (id: string, out: string) => invokeCommand<void>("export_persona", { id, out }),
  usePersona: (id: string) => invokeCommand<BootstrapState>("use_persona", { id }),

  // providers -------------------------------------------------------------
  listProviders: () => invokeCommand<ProviderConfig[]>("list_providers"),
  saveProvider: (provider: ProviderConfig) =>
    invokeCommand<ProviderConfig[]>("save_provider", { provider }),
  deleteProvider: (id: string) => invokeCommand<ProviderConfig[]>("delete_provider", { id }),
  testProvider: (id: string) => invokeCommand<ProviderTestResult>("test_provider", { id }),
  setApiKey: (providerId: string, key: string) =>
    invokeCommand<void>("set_api_key", { providerId, key }),
  // `id` is sent alongside `providerId` so the command works whether the handler
  // declares `IdArgs { id }` or `{ providerId }`.
  hasApiKey: (providerId: string) =>
    invokeCommand<boolean>("has_api_key", { providerId }),

  // conversations ---------------------------------------------------------
  listConversations: (personaId?: string) =>
    invokeCommand<ConversationView[]>("list_conversations", personaId ? { personaId } : undefined),
  createConversation: (personaId: string) =>
    invokeCommand<ConversationView>("create_conversation", { personaId }),
  getMessages: (conversationId: string, limit?: number) =>
    invokeCommand<ChatMessageView[]>("get_messages", { conversationId, limit }),
  deleteConversation: (id: string) => invokeCommand<void>("delete_conversation", { id }),
  sendMessage: (conversationId: string, text: string) =>
    invokeCommand<void>("send_message", { conversationId, text }),
  // `id` is sent alongside `conversationId` so the command works whether the
  // handler declares `IdArgs { id }` or `{ conversationId }`.
  cancelMessage: (conversationId: string) =>
    invokeCommand<void>("cancel_message", { conversationId }),

  // memory ----------------------------------------------------------------
  listFacts: (personaId: string) => invokeCommand<MemoryFact[]>("list_facts", { personaId }),
  deleteFact: (id: number) => invokeCommand<void>("delete_fact", { id }),
  clearMemory: (personaId: string) => invokeCommand<void>("clear_memory", { personaId }),

  // settings --------------------------------------------------------------
  getSettings: () => invokeCommand<AppConfig>("get_settings"),
  saveSettings: (config: AppConfig) => invokeCommand<AppConfig>("save_settings", { config }),

  // windows / pet / tts ---------------------------------------------------
  completeOnboarding: () => invokeCommand<AppConfig>("complete_onboarding"),
  openChat: () => invokeCommand<void>("open_chat"),
  hideChat: () => invokeCommand<void>("hide_chat"),
  setPetState: (state: string, message?: string, ttlMs?: number) =>
    invokeCommand<void>("set_pet_state", { state, message, ttlMs }),
  speak: (text: string) => invokeCommand<void>("speak", { text }),
  stopSpeaking: () => invokeCommand<void>("stop_speaking"),

  // agent / diagnostics ---------------------------------------------------
  hooksStatus: () => invokeCommand<HookStatus[]>("hooks_status"),
  installHooks: (agent: string) => invokeCommand<HooksReport>("install_hooks", { agent }),
  uninstallHooks: (agent: string) => invokeCommand<HooksReport>("uninstall_hooks", { agent }),
  openDataDir: () => invokeCommand<void>("open_data_dir"),
  exportLogs: () => invokeCommand<{ path: string }>("export_logs"),
} as const;

// --- events ----------------------------------------------------------------

function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (message) => handler(message.payload));
}

export const events = {
  onChatDelta: (handler: (payload: ChatDeltaEvent) => void) => on<ChatDeltaEvent>("chat://delta", handler),
  onChatDone: (handler: (payload: ChatDoneEvent) => void) => on<ChatDoneEvent>("chat://done", handler),
  onChatError: (handler: (payload: ChatErrorEvent) => void) =>
    on<ChatErrorEvent>("chat://error", handler),
  onPetState: (handler: (payload: PetStateEvent) => void) => on<PetStateEvent>("pet://state", handler),
  onPetLibraryChanged: (handler: (payload: string) => void) =>
    on<string>("pet://library-changed", handler),
  onPersonaChanged: (handler: (payload: Persona) => void) => on<Persona>("persona://changed", handler),
  onSettingsChanged: (handler: (payload: AppConfig) => void) =>
    on<AppConfig>("settings://changed", handler),
  onOpenSettings: (handler: () => void) => on<undefined>("ui://open-settings", handler),
  onTtsState: (handler: (payload: TtsStateEvent) => void) =>
    on<TtsStateEvent>("tts://state", handler),
};

/** Subscribe to several channels at once; the returned function unlistens all. */
export async function subscribeAll(
  subscriptions: Promise<UnlistenFn>[],
): Promise<() => void> {
  const unlisten = await Promise.all(subscriptions);
  return () => {
    for (const off of unlisten) off();
  };
}
