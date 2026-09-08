/**
 * App shell: bootstrap + global event wiring, two-pane layout, view switching.
 *
 * The chat pane stays mounted while settings is open so a streaming turn is
 * never interrupted by navigation.
 */

import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { events, ipc } from "../shared/ipc";
import { clearMarkdownCache } from "../shared/markdown";
import { getLanguage, setLanguage, t } from "../shared/i18n";
import type {
  AppConfig,
  BootstrapState,
  ConversationView,
  Persona,
  PetStateEvent,
  ProviderConfig,
} from "../shared/types";
import { ChatPane } from "./components/ChatPane";
import { Sidebar } from "./components/Sidebar";
import { SettingsView } from "./components/SettingsView";
import { confirmAction, DialogHost } from "./components/Confirm";
import { ToastHost, toast } from "./components/Toast";
import { IconClose, IconMenu, IconPlus } from "./components/icons";
import { settingsTabLabelKey, type SettingsTab, type View } from "./view";

function upsertPersona(personas: Persona[], persona: Persona): Persona[] {
  const index = personas.findIndex((item) => item.id === persona.id);
  if (index === -1) return [...personas, persona];
  const next = [...personas];
  next[index] = persona;
  return next;
}

export function App() {
  const [boot, setBoot] = useState<BootstrapState | null>(null);
  const [bootError, setBootError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [conversations, setConversations] = useState<ConversationView[]>([]);
  const [activeConversationId, setActiveConversationId] = useState<string | null>(null);
  const [view, setView] = useState<View>({ kind: "chat" });
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [petState, setPetState] = useState<PetStateEvent | null>(null);
  const [speaking, setSpeaking] = useState(false);

  const bootRef = useRef(boot);
  const personaIdRef = useRef<string | null>(null);
  bootRef.current = boot;

  const language = boot?.config.ui.language ?? "zh-CN";
  if (getLanguage() !== language) setLanguage(language);

  const load = useCallback(async () => {
    setLoading(true);
    const result = await ipc.getBootstrapState();
    setLoading(false);
    if (result.ok) {
      setBoot(result.value);
      setBootError(null);
    } else {
      setBootError(result.error);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const applyBootstrap = useCallback((state: BootstrapState) => {
    setBoot(state);
    setBootError(null);
  }, []);

  const refreshConversations = useCallback(async (personaId?: string) => {
    const id = personaId ?? personaIdRef.current;
    if (!id) {
      setConversations([]);
      return;
    }
    const result = await ipc.listConversations(id);
    if (result.ok) setConversations(result.value);
  }, []);

  // Reload the conversation list whenever the active persona changes.
  useEffect(() => {
    const personaId = boot?.activePersona?.id ?? null;
    personaIdRef.current = personaId;
    setActiveConversationId(null);
    void refreshConversations(personaId ?? undefined);
  }, [boot?.activePersona?.id, refreshConversations]);

  // Theme + markdown cache (code-block labels are localised).
  const theme = boot?.config.ui.theme ?? "system";
  useEffect(() => {
    const root = document.documentElement;
    if (theme === "dark" || theme === "light") root.dataset.theme = theme;
    else delete root.dataset.theme;
  }, [theme]);

  useEffect(() => {
    clearMarkdownCache();
    document.documentElement.lang = language;
  }, [language]);

  // Global Rust -> frontend events.
  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn[] = [];
    void (async () => {
      const list = await Promise.all([
        events.onSettingsChanged((config) => {
          setBoot((current) => (current ? { ...current, config } : current));
        }),
        events.onPersonaChanged((persona) => {
          setBoot((current) =>
            current
              ? {
                  ...current,
                  personas: upsertPersona(current.personas, persona),
                  activePersona:
                    current.activePersona?.id === persona.id ? persona : current.activePersona,
                }
              : current,
          );
        }),
        events.onOpenSettings(() => {
          setView({ kind: "settings", tab: "general" });
          setSidebarOpen(false);
        }),
        events.onPetState((payload) => setPetState(payload)),
        events.onTtsState((payload) => setSpeaking(payload.speaking)),
        events.onPetLibraryChanged(() => {
          void load();
        }),
      ]);
      if (disposed) for (const off of list) off();
      else unlisten = list;
    })().catch(() => {
      /* event bridge unavailable (e.g. plain browser dev server) */
    });
    return () => {
      disposed = true;
      for (const off of unlisten) off();
    };
  }, [load]);

  // Closing the window hides it; the tray keeps the app alive.
  useEffect(() => {
    const window = getCurrentWindow();
    let unlisten: UnlistenFn | undefined;
    void window
      .onCloseRequested((event) => {
        event.preventDefault();
        void ipc.hideChat();
      })
      .then((off) => {
        unlisten = off;
      })
      .catch(() => {
        /* window permissions unavailable — the OS default applies */
      });
    return () => {
      unlisten?.();
    };
  }, []);

  const openConversation = useCallback((id: string) => {
    setActiveConversationId(id);
    setView({ kind: "chat" });
    setSidebarOpen(false);
  }, []);

  const createConversation = useCallback(async (): Promise<string | null> => {
    const personaId = bootRef.current?.activePersona?.id;
    if (!personaId) {
      toast.error(t("chat.emptyNoPersona"));
      return null;
    }
    const result = await ipc.createConversation(personaId);
    if (!result.ok) {
      toast.error(result.error);
      return null;
    }
    setConversations((current) => [result.value, ...current.filter((item) => item.id !== result.value.id)]);
    setActiveConversationId(result.value.id);
    setView({ kind: "chat" });
    setSidebarOpen(false);
    return result.value.id;
  }, []);

  const deleteConversation = useCallback(
    async (id: string) => {
      const title =
        conversations.find((item) => item.id === id)?.title ?? t("chat.newConversationTitle");
      const confirmed = await confirmAction({
        body: t("chat.deleteConfirm", { title }),
        danger: true,
      });
      if (!confirmed) return;
      const result = await ipc.deleteConversation(id);
      if (!result.ok) {
        toast.error(result.error);
        return;
      }
      setConversations((current) => current.filter((item) => item.id !== id));
      if (activeConversationId === id) setActiveConversationId(null);
      toast.success(t("memory.deleted"));
    },
    [activeConversationId, conversations],
  );

  const selectPersona = useCallback(
    async (id: string) => {
      if (!id || id === bootRef.current?.activePersona?.id) return;
      const result = await ipc.usePersona(id);
      if (!result.ok) {
        toast.error(result.error);
        return;
      }
      applyBootstrap(result.value);
      setView({ kind: "chat" });
      setSidebarOpen(false);
    },
    [applyBootstrap],
  );

  const openSettings = useCallback((tab: SettingsTab) => {
    setView({ kind: "settings", tab });
    setSidebarOpen(false);
  }, []);

  const onConfig = useCallback((config: AppConfig) => {
    setBoot((current) => (current ? { ...current, config } : current));
  }, []);

  const onPersonas = useCallback((personas: Persona[]) => {
    setBoot((current) => {
      if (!current) return current;
      const active = current.activePersona
        ? (personas.find((persona) => persona.id === current.activePersona?.id) ??
          current.activePersona)
        : null;
      return { ...current, personas, activePersona: active };
    });
  }, []);

  const onProviders = useCallback((providers: ProviderConfig[]) => {
    setBoot((current) => {
      if (!current) return current;
      return {
        ...current,
        config: {
          ...current.config,
          providers: Object.fromEntries(providers.map((provider) => [provider.id, provider])),
        },
      };
    });
  }, []);

  const onConversationTouched = useCallback(() => {
    void refreshConversations();
  }, [refreshConversations]);

  if (bootError && !boot) {
    return (
      <div class="panel stack">
        <h1>{t("app.bootFailed")}</h1>
        <div class="alert alert-danger">{bootError}</div>
        <div class="row">
          <button type="button" class="btn btn-primary" onClick={() => void load()}>
            {t("app.retryBoot")}
          </button>
        </div>
      </div>
    );
  }

  if (!boot) {
    return (
      <div class="panel">
        <span class="muted">{loading ? t("app.loading") : t("common.empty")}</span>
      </div>
    );
  }

  const activeConversation = conversations.find((item) => item.id === activeConversationId);
  const title =
    view.kind === "settings"
      ? t(settingsTabLabelKey(view.tab))
      : activeConversation?.title || boot.activePersona?.name || t("app.chat");
  const subtitle =
    view.kind === "settings" ? t("settings.title") : (boot.activePersona?.name ?? "");

  return (
    <div class="app">
      {sidebarOpen ? (
        <div class="sidebar-backdrop" role="presentation" onClick={() => setSidebarOpen(false)} />
      ) : null}

      <Sidebar
        boot={boot}
        conversations={conversations}
        activeConversationId={activeConversationId}
        view={view}
        petState={petState}
        speaking={speaking}
        open={sidebarOpen}
        onSelectConversation={openConversation}
        onNewConversation={() => void createConversation()}
        onDeleteConversation={(id) => void deleteConversation(id)}
        onSelectPersona={(id) => void selectPersona(id)}
        onOpenSettings={() => openSettings("general")}
        onOpenAgent={() => openSettings("agent")}
        onClose={() => setSidebarOpen(false)}
      />

      <div class="main">
        <header class="topbar">
          <button
            type="button"
            class="icon-btn hamburger"
            aria-label={sidebarOpen ? t("app.sidebarClose") : t("app.toggleSidebar")}
            onClick={() => setSidebarOpen((open) => !open)}
          >
            {sidebarOpen ? <IconClose size={16} /> : <IconMenu size={16} />}
          </button>

          <div class="topbar-title grow">
            <strong>{title}</strong>
            {subtitle ? <span>{subtitle}</span> : null}
          </div>

          {view.kind === "chat" ? (
            <button
              type="button"
              class="btn btn-sm"
              onClick={() => void createConversation()}
              title={t("app.newChat")}
            >
              <IconPlus size={14} />
              <span class="btn-label">{t("app.newChat")}</span>
            </button>
          ) : null}

          <button
            type="button"
            class="icon-btn"
            aria-label={t("app.hideWindow")}
            title={t("app.hideWindow")}
            onClick={() => void ipc.hideChat()}
          >
            <IconClose size={15} />
          </button>
        </header>

        <div class={view.kind === "chat" ? "view-slot" : "view-slot view-hidden"}>
          <ChatPane
            conversationId={activeConversationId}
            persona={boot.activePersona}
            config={boot.config}
            onRequestConversation={createConversation}
            onConversationTouched={onConversationTouched}
          />
        </div>

        {view.kind === "settings" ? (
          <SettingsView
            boot={boot}
            view={view}
            onTab={(tab) => setView({ kind: "settings", tab })}
            onBootstrap={applyBootstrap}
            onConfig={onConfig}
            onPersonas={onPersonas}
            onProviders={onProviders}
            onConversationsChanged={onConversationTouched}
          />
        ) : null}
      </div>

      <ToastHost />
      <DialogHost />
    </div>
  );
}
