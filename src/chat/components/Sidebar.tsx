/** Left rail: persona switcher, conversation list, settings/agent entries. */

import { t } from "../../shared/i18n";
import { formatRelative } from "../../shared/format";
import type { BootstrapState, ConversationView, PetStateEvent } from "../../shared/types";
import type { View } from "../view";
import { IconChat, IconClose, IconPlus, IconSettings, IconSparkles, IconTrash } from "./icons";

export interface SidebarProps {
  boot: BootstrapState;
  conversations: ConversationView[];
  activeConversationId: string | null;
  view: View;
  petState: PetStateEvent | null;
  speaking: boolean;
  open: boolean;
  onSelectConversation: (id: string) => void;
  onNewConversation: () => void;
  onDeleteConversation: (id: string) => void;
  onSelectPersona: (id: string) => void;
  onOpenSettings: () => void;
  onOpenAgent: () => void;
  onClose: () => void;
}

export function Sidebar(props: SidebarProps) {
  const { boot, conversations, activeConversationId, view } = props;
  const activePersona = boot.activePersona;

  return (
    <aside class={props.open ? "sidebar open" : "sidebar"} aria-label={t("app.name")}>
      <div class="sidebar-head">
        <div class="brand grow">
          <span class="brand-mark" aria-hidden="true" />
          <span class="brand-name">{t("app.name")}</span>
        </div>
        <button
          type="button"
          class="icon-btn"
          aria-label={t("app.newChat")}
          title={t("app.newChat")}
          onClick={props.onNewConversation}
        >
          <IconPlus size={15} />
        </button>
        <button
          type="button"
          class="icon-btn sidebar-close"
          aria-label={t("app.sidebarClose")}
          onClick={props.onClose}
        >
          <IconClose size={15} />
        </button>
      </div>

      <div class="sidebar-block">
        <span class="sidebar-label">{t("app.persona")}</span>
        <select
          class="select"
          aria-label={t("app.persona")}
          value={activePersona?.id ?? ""}
          onChange={(event) => props.onSelectPersona((event.currentTarget as HTMLSelectElement).value)}
        >
          {boot.personas.length === 0 ? <option value="">{t("persona.noPersonas")}</option> : null}
          {boot.personas.map((persona) => (
            <option key={persona.id} value={persona.id}>
              {persona.name}
              {persona.builtin ? ` · ${t("persona.builtin")}` : ""}
            </option>
          ))}
        </select>
      </div>

      <div class="sidebar-block" style="padding-bottom:2px">
        <span class="sidebar-label">{t("app.conversations")}</span>
      </div>

      <div class="conv-list">
        {conversations.length === 0 ? (
          <div class="small muted" style="padding:10px 8px">
            {t("app.noConversations")}
          </div>
        ) : null}
        {conversations.map((conversation) => (
          <div
            key={conversation.id}
            class={conversation.id === activeConversationId ? "conv-item active" : "conv-item"}
            role="button"
            tabIndex={0}
            aria-current={conversation.id === activeConversationId}
            onClick={() => props.onSelectConversation(conversation.id)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                props.onSelectConversation(conversation.id);
              }
            }}
          >
            <IconChat size={14} class="muted" />
            <span class="conv-main">
              <span class="conv-title">{conversation.title || t("chat.newConversationTitle")}</span>
              <span class="conv-sub">
                <span>{formatRelative(conversation.updatedAt)}</span>
                <span>·</span>
                <span>{t("chat.messageCount", { n: conversation.messageCount })}</span>
              </span>
            </span>
            <button
              type="button"
              class="icon-btn danger conv-del"
              aria-label={t("chat.deleteConversation")}
              title={t("chat.deleteConversation")}
              onClick={(event) => {
                event.stopPropagation();
                props.onDeleteConversation(conversation.id);
              }}
            >
              <IconTrash size={13} />
            </button>
          </div>
        ))}
      </div>

      <div class="sidebar-foot">
        <div class="pet-status">
          <span
            class={
              props.speaking
                ? "pet-dot speaking"
                : !props.petState || props.petState.state === "idle"
                  ? "pet-dot"
                  : "pet-dot busy"
            }
            aria-hidden="true"
          />
          <span class="truncate">
            {props.speaking
              ? t("app.speaking")
              : `${t("app.petState")}: ${props.petState?.state ?? "idle"}`}
          </span>
        </div>
        <button
          type="button"
          class={view.kind === "settings" && view.tab !== "agent" ? "foot-btn active" : "foot-btn"}
          onClick={props.onOpenSettings}
        >
          <IconSettings size={15} />
          {t("app.settings")}
        </button>
        <button
          type="button"
          class={view.kind === "settings" && view.tab === "agent" ? "foot-btn active" : "foot-btn"}
          onClick={props.onOpenAgent}
        >
          <IconSparkles size={15} />
          {t("app.agent")}
        </button>
      </div>
    </aside>
  );
}
