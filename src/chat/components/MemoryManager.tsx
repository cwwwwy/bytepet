/** Memory manager: long-term facts + conversation history for one persona. */

import { useEffect, useState } from "preact/hooks";
import { ipc, type MemoryFact } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import { formatRelative } from "../../shared/format";
import type { BootstrapState, ConversationView } from "../../shared/types";
import { Badge, EmptyState, Field, Section } from "./ui";
import { confirmAction } from "./Confirm";
import { toast, toastResult } from "./Toast";
import { IconRefresh, IconTrash } from "./icons";

export interface MemoryManagerProps {
  boot: BootstrapState;
  onConversationsChanged: () => void;
}

function confidencePercent(confidence: number): number {
  const value = confidence <= 1 ? confidence * 100 : confidence;
  return Math.max(0, Math.min(100, Math.round(value)));
}

export function MemoryManager(props: MemoryManagerProps) {
  const [personaId, setPersonaId] = useState(
    props.boot.activePersona?.id ?? props.boot.personas[0]?.id ?? "",
  );
  const [facts, setFacts] = useState<MemoryFact[]>([]);
  const [conversations, setConversations] = useState<ConversationView[]>([]);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);

  const load = async (id: string) => {
    if (!id) {
      setFacts([]);
      setConversations([]);
      return;
    }
    setLoading(true);
    const [factsResult, conversationsResult] = await Promise.all([
      ipc.listFacts(id),
      ipc.listConversations(id),
    ]);
    setLoading(false);
    if (factsResult.ok) setFacts(factsResult.value);
    else toast.error(factsResult.error);
    if (conversationsResult.ok) setConversations(conversationsResult.value);
    else toast.error(conversationsResult.error);
  };

  useEffect(() => {
    void load(personaId);
  }, [personaId]);

  const personaName =
    props.boot.personas.find((persona) => persona.id === personaId)?.name ?? personaId;

  const removeFact = async (fact: MemoryFact) => {
    const result = await ipc.deleteFact(fact.id);
    if (toastResult(result, t("memory.factDeleted"))) {
      setFacts((current) => current.filter((item) => item.id !== fact.id));
    }
  };

  const clearMemory = async () => {
    if (!personaId) return;
    const confirmed = await confirmAction({
      body: t("memory.clearConfirm", { name: personaName }),
      danger: true,
    });
    if (!confirmed) return;
    setBusy(true);
    const result = await ipc.clearMemory(personaId);
    setBusy(false);
    if (toastResult(result, t("memory.cleared"))) setFacts([]);
  };

  const removeConversation = async (conversation: ConversationView) => {
    const confirmed = await confirmAction({
      body: t("memory.deleteConversationConfirm"),
      danger: true,
    });
    if (!confirmed) return;
    const result = await ipc.deleteConversation(conversation.id);
    if (toastResult(result, t("memory.deleted"))) {
      setConversations((current) => current.filter((item) => item.id !== conversation.id));
      props.onConversationsChanged();
    }
  };

  return (
    <div class="stack">
      <Section
        title={t("memory.title")}
        desc={t("memory.persona")}
        actions={
          <div class="row" style="gap:6px">
            <button type="button" class="icon-btn" aria-label={t("common.refresh")} onClick={() => void load(personaId)}>
              <IconRefresh size={14} />
            </button>
            <button
              type="button"
              class="btn btn-sm btn-danger"
              disabled={busy || !personaId}
              onClick={() => void clearMemory()}
            >
              <IconTrash size={13} />
              {t("memory.clear")}
            </button>
          </div>
        }
      >
        <Field label={t("memory.persona")}>
          <select
            class="select"
            value={personaId}
            onChange={(event) => setPersonaId((event.currentTarget as HTMLSelectElement).value)}
          >
            {props.boot.personas.map((persona) => (
              <option key={persona.id} value={persona.id}>
                {persona.name}
              </option>
            ))}
          </select>
        </Field>

        <div class="card-title" style="margin:2px 0 0">
          <span>{t("memory.facts")}</span>
          <Badge kind="accent">{facts.length}</Badge>
        </div>

        {loading ? (
          <div class="small muted">{t("common.loading")}</div>
        ) : facts.length === 0 ? (
          <EmptyState>{t("memory.noFacts")}</EmptyState>
        ) : (
          <div class="card scroll-x">
            <table class="table">
              <thead>
                <tr>
                  <th>{t("memory.key")}</th>
                  <th>{t("memory.value")}</th>
                  <th>{t("memory.confidence")}</th>
                  <th>{t("memory.updated")}</th>
                  <th aria-label={t("common.delete")} />
                </tr>
              </thead>
              <tbody>
                {facts.map((fact) => (
                  <tr key={fact.id}>
                    <td class="mono">{fact.key}</td>
                    <td style="overflow-wrap:anywhere">{fact.value}</td>
                    <td style="min-width:96px">
                      <div class="row" style="gap:6px">
                        <div class="meter grow">
                          <span style={`width:${confidencePercent(fact.confidence)}%`} />
                        </div>
                        <span class="small muted">{confidencePercent(fact.confidence)}%</span>
                      </div>
                    </td>
                    <td class="small muted">{formatRelative(fact.updatedAt)}</td>
                    <td style="width:1%">
                      <button
                        type="button"
                        class="icon-btn danger"
                        aria-label={t("common.delete")}
                        onClick={() => void removeFact(fact)}
                      >
                        <IconTrash size={13} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      <Section
        title={t("memory.conversations")}
        actions={<Badge kind="accent">{conversations.length}</Badge>}
      >
        {conversations.length === 0 ? (
          <EmptyState>{t("memory.noConversations")}</EmptyState>
        ) : (
          <div class="card scroll-x">
            <table class="table">
              <thead>
                <tr>
                  <th>{t("chat.newConversationTitle")}</th>
                  <th>{t("memory.updated")}</th>
                  <th>{t("chat.usage")}</th>
                  <th aria-label={t("common.delete")} />
                </tr>
              </thead>
              <tbody>
                {conversations.map((conversation) => (
                  <tr key={conversation.id}>
                    <td style="overflow-wrap:anywhere">
                      {conversation.title || t("chat.newConversationTitle")}
                    </td>
                    <td class="small muted">{formatRelative(conversation.updatedAt)}</td>
                    <td class="small muted">
                      {t("memory.messages", { n: conversation.messageCount })}
                    </td>
                    <td style="width:1%">
                      <button
                        type="button"
                        class="icon-btn danger"
                        aria-label={t("chat.deleteConversation")}
                        onClick={() => void removeConversation(conversation)}
                      >
                        <IconTrash size={13} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>
    </div>
  );
}
