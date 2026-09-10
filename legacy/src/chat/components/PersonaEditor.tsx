/**
 * Persona manager: list + full editor. Every `Persona` field is editable and a
 * read-only panel shows the system prompt the runtime will assemble.
 */

import { useEffect, useMemo, useState } from "preact/hooks";
import { ipc } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import { uid } from "../../shared/format";
import type { BootstrapState, Persona, ProviderKind } from "../../shared/types";
import { Badge, EmptyState, Field, NumberField, RangeField, Section, Switch } from "./ui";
import { confirmAction, promptAction } from "./Confirm";
import { toast, toastResult } from "./Toast";
import { IconCheck, IconDownload, IconPlus, IconTrash, IconUpload } from "./icons";

export interface PersonaEditorProps {
  boot: BootstrapState;
  onBootstrap: (state: BootstrapState) => void;
  onPersonas: (personas: Persona[]) => void;
}

const MODEL_PLACEHOLDER: Record<ProviderKind, string> = {
  anthropic: "provider.placeholder.anthropic",
  "open-ai-chat": "provider.placeholder.open-ai-chat",
  "open-ai-responses": "provider.placeholder.open-ai-responses",
  "codex-cli": "provider.placeholder.codex-cli",
  "claude-cli": "provider.placeholder.claude-cli",
};

function clonePersona(persona: Persona): Persona {
  return JSON.parse(JSON.stringify(persona)) as Persona;
}

/** Human-readable preview of the assembled system prompt. */
export function assembleSystemPrompt(persona: Persona): string {
  const lines: string[] = [persona.systemPrompt.trim()];
  lines.push("");
  lines.push(`# ${t("persona.traits")}`);
  lines.push(`- ${t("persona.tone")}: ${persona.traits.tone.trim() || t("persona.toneDefault")}`);
  lines.push(`- ${t("persona.verbosity")}: ${t(`persona.verbosity.${persona.traits.verbosity}`)}`);
  lines.push(
    `- ${t("persona.language")}: ${
      persona.traits.language === "auto"
        ? t("persona.language.auto")
        : persona.traits.language === "en"
          ? t("persona.language.en")
          : t("persona.language.zh")
    }`,
  );
  lines.push(
    `- ${t("persona.emoji")}: ${persona.traits.emoji ? t("common.yes") : t("common.no")}`,
  );

  lines.push("");
  lines.push(`# ${t("persona.memory")}`);
  lines.push(
    `- ${t("persona.memoryEnabled")}: ${persona.memory.enabled ? t("common.yes") : t("common.no")}`,
  );
  if (persona.memory.enabled) {
    lines.push(`- ${t("persona.memoryWindow")}: ${persona.memory.windowTurns}`);
    lines.push(
      `- ${t("persona.memoryLongTerm")}: ${persona.memory.longTerm ? t("common.yes") : t("common.no")}`,
    );
    lines.push(`- ${t("persona.summarizeAfter")}: ${persona.memory.summarizeAfterTurns}`);
  }

  if (persona.proactive.enabled) {
    lines.push("");
    lines.push(`# ${t("persona.proactive")}`);
    lines.push(`- ${t("persona.proactiveIdle")}: ${persona.proactive.idleMinutes}`);
  }

  return lines.join("\n");
}

export function PersonaEditor(props: PersonaEditorProps) {
  const { boot } = props;
  const [selectedId, setSelectedId] = useState<string | null>(
    boot.activePersona?.id ?? boot.personas[0]?.id ?? null,
  );
  const [draft, setDraft] = useState<Persona | null>(null);
  const [templates, setTemplates] = useState<Record<string, Persona> | null>(null);
  const [busy, setBusy] = useState(false);
  const [creating, setCreating] = useState(false);

  const selected = useMemo(
    () => boot.personas.find((persona) => persona.id === selectedId) ?? null,
    [boot.personas, selectedId],
  );

  useEffect(() => {
    if (creating) return;
    if (selectedId && boot.personas.some((persona) => persona.id === selectedId)) return;
    setSelectedId(boot.activePersona?.id ?? boot.personas[0]?.id ?? null);
  }, [boot.personas, boot.activePersona, selectedId, creating]);

  useEffect(() => {
    if (creating) return;
    setDraft(selected ? clonePersona(selected) : null);
  }, [selected, creating]);

  useEffect(() => {
    if (templates) return;
    void ipc.personaTemplates().then((result) => {
      if (result.ok) setTemplates(result.value);
    });
  }, [templates]);

  const dirty =
    creating || Boolean(draft && selected && JSON.stringify(draft) !== JSON.stringify(selected));

  const patch = (next: Partial<Persona>) => setDraft((current) => (current ? { ...current, ...next } : current));

  const save = async () => {
    if (!draft) return;
    if (!draft.name.trim()) {
      toast.error(t("persona.nameRequired"));
      return;
    }
    setBusy(true);
    const result = await ipc.savePersona(draft);
    setBusy(false);
    if (toastResult(result, t("persona.saved"))) {
      props.onPersonas(result.value);
      setCreating(false);
      setSelectedId(draft.id);
    }
  };

  const activate = async () => {
    if (!selected) return;
    const result = await ipc.usePersona(selected.id);
    if (toastResult(result)) props.onBootstrap(result.value);
  };

  const createFromTemplate = async () => {
    const entries = Object.entries(templates ?? {});
    const choice = await promptAction({
      title: t("persona.new"),
      label: t("persona.template"),
      value: entries[0]?.[0] ?? "default",
      selectOptions: entries.map(([key, template]) => ({ value: key, label: template.name || key })),
    });
    if (!choice) return;
    const template = templates?.[choice];
    if (!template) return;
    const created = clonePersona(template);
    created.id = uid("persona");
    created.name = `${template.name} ${t("common.copy")}`;
    created.builtin = false;
    setCreating(true);
    setSelectedId(null);
    setDraft(created);
    toast.info(t("persona.dirty"));
  };

  const duplicate = async () => {
    if (!selected) return;
    const name = await promptAction({
      label: t("persona.duplicatePrompt"),
      value: `${selected.name} ${t("common.copy")}`,
    });
    if (!name) return;
    setBusy(true);
    const result = await ipc.duplicatePersona(selected.id, uid("persona"), name);
    setBusy(false);
    if (toastResult(result, t("persona.saved"))) props.onPersonas(result.value);
  };

  const remove = async () => {
    if (!selected) return;
    if (selected.builtin) {
      toast.error(t("persona.builtinLocked"));
      return;
    }
    const confirmed = await confirmAction({
      body: t("persona.deleteConfirm", { name: selected.name }),
      danger: true,
    });
    if (!confirmed) return;
    setBusy(true);
    const result = await ipc.deletePersona(selected.id);
    setBusy(false);
    if (toastResult(result, t("memory.deleted"))) {
      props.onPersonas(result.value);
      setSelectedId(null);
    }
  };

  const importPersona = async () => {
    const path = await promptAction({ label: t("persona.importPrompt"), placeholder: "/path/to/persona.json" });
    if (!path) return;
    setBusy(true);
    const result = await ipc.importPersona(path, false);
    setBusy(false);
    if (toastResult(result, t("persona.imported"))) props.onPersonas(result.value);
  };

  const exportPersona = async () => {
    if (!selected) return;
    const out = await promptAction({ label: t("persona.exportPrompt"), value: `${selected.id}.json` });
    if (!out) return;
    const result = await ipc.exportPersona(selected.id, out);
    toastResult(result, t("persona.exported", { path: out }));
  };

  const providers = Object.values(boot.config.providers ?? {});
  const provider = draft?.model?.provider ? boot.config.providers[draft.model.provider] : undefined;
  const modelPlaceholder = provider
    ? t(MODEL_PLACEHOLDER[provider.kind] ?? "provider.placeholder.open-ai-chat")
    : t("persona.modelInherit");

  return (
    <div class="split">
      <div class="list-pane">
        <div class="row" style="gap:6px">
          <button type="button" class="btn btn-primary grow" onClick={() => void createFromTemplate()}>
            <IconPlus size={14} />
            {t("persona.new")}
          </button>
        </div>
        <div class="row" style="gap:6px">
          <button type="button" class="btn btn-sm grow" disabled={!selected || busy} onClick={() => void duplicate()}>
            {t("persona.duplicate")}
          </button>
          <button type="button" class="btn btn-sm grow" onClick={() => void importPersona()}>
            <IconUpload size={13} />
            {t("common.import")}
          </button>
          <button
            type="button"
            class="btn btn-sm grow"
            disabled={!selected}
            onClick={() => void exportPersona()}
          >
            <IconDownload size={13} />
            {t("common.export")}
          </button>
        </div>

        {boot.personas.length === 0 ? (
          <EmptyState>{t("persona.noPersonas")}</EmptyState>
        ) : (
          boot.personas.map((persona) => (
            <button
              key={persona.id}
              type="button"
              class={persona.id === selectedId && !creating ? "list-item active" : "list-item"}
              onClick={() => {
                setCreating(false);
                setSelectedId(persona.id);
              }}
            >
              <span class="list-item-main">
                <span class="list-item-title">{persona.name}</span>
                <span class="list-item-sub">
                  {persona.description?.trim() || persona.id}
                </span>
              </span>
              {persona.builtin ? <Badge>{t("persona.builtin")}</Badge> : null}
              {boot.activePersona?.id === persona.id ? (
                <Badge kind="accent">{t("persona.active")}</Badge>
              ) : null}
            </button>
          ))
        )}
      </div>

      <div class="stack">
        {!draft ? (
          <EmptyState>{t("persona.select")}</EmptyState>
        ) : (
          <>
            <div class="row-between">
              <div class="row" style="gap:8px">
                <h2>{draft.name || t("common.unnamed")}</h2>
                {draft.builtin ? <Badge>{t("persona.builtin")}</Badge> : null}
                {dirty ? <Badge kind="warn">{t("common.unsaved")}</Badge> : null}
              </div>
              <div class="row" style="gap:6px">
                {boot.activePersona?.id !== draft.id ? (
                  <button
                    type="button"
                    class="btn btn-sm"
                    disabled={busy || creating}
                    onClick={() => void activate()}
                  >
                    <IconCheck size={13} />
                    {t("persona.use")}
                  </button>
                ) : (
                  <Badge kind="accent">{t("persona.active")}</Badge>
                )}
                <button
                  type="button"
                  class="btn btn-sm btn-primary"
                  disabled={!dirty || busy}
                  onClick={() => void save()}
                >
                  {t("persona.save")}
                </button>
                <button
                  type="button"
                  class="btn btn-sm btn-danger"
                  disabled={draft.builtin || busy}
                  title={draft.builtin ? t("persona.builtinLocked") : t("persona.delete")}
                  onClick={() => void remove()}
                >
                  <IconTrash size={13} />
                </button>
              </div>
            </div>

            <Section title={t("persona.name")}>
              <div class="form-grid">
                <Field label={t("persona.name")}>
                  <input
                    class="input"
                    value={draft.name}
                    onInput={(event) => patch({ name: (event.currentTarget as HTMLInputElement).value })}
                  />
                </Field>
                <Field label={t("persona.avatar")}>
                  <select
                    class="select"
                    value={draft.avatarPet ?? ""}
                    onChange={(event) =>
                      patch({
                        avatarPet: (event.currentTarget as HTMLSelectElement).value || null,
                      })
                    }
                  >
                    <option value="">{t("persona.avatarNone")}</option>
                    {boot.pets.map((pet) => (
                      <option key={pet.id} value={pet.id}>
                        {pet.displayName}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label={t("persona.description")} class="span-2">
                  <input
                    class="input"
                    value={draft.description ?? ""}
                    onInput={(event) =>
                      patch({ description: (event.currentTarget as HTMLInputElement).value || null })
                    }
                  />
                </Field>
                <Field label={t("persona.greeting")} class="span-2">
                  <input
                    class="input"
                    value={draft.greeting ?? ""}
                    placeholder={t("persona.greetingPlaceholder")}
                    onInput={(event) =>
                      patch({ greeting: (event.currentTarget as HTMLInputElement).value || null })
                    }
                  />
                </Field>
                <Field
                  label={t("persona.systemPrompt")}
                  count={t("persona.charCount", { n: draft.systemPrompt.length })}
                  class="span-2"
                >
                  <textarea
                    class="textarea"
                    rows={6}
                    value={draft.systemPrompt}
                    placeholder={t("persona.promptPlaceholder")}
                    onInput={(event) =>
                      patch({ systemPrompt: (event.currentTarget as HTMLTextAreaElement).value })
                    }
                  />
                </Field>
              </div>
            </Section>

            <Section title={t("persona.traits")}>
              <div class="form-grid">
                <Field label={t("persona.tone")}>
                  <input
                    class="input"
                    value={draft.traits.tone}
                    placeholder={t("persona.tonePlaceholder")}
                    onInput={(event) =>
                      patch({
                        traits: {
                          ...draft.traits,
                          tone: (event.currentTarget as HTMLInputElement).value,
                        },
                      })
                    }
                  />
                </Field>
                <Field label={t("persona.verbosity")}>
                  <select
                    class="select"
                    value={draft.traits.verbosity}
                    onChange={(event) =>
                      patch({
                        traits: {
                          ...draft.traits,
                          verbosity: (event.currentTarget as HTMLSelectElement).value,
                        },
                      })
                    }
                  >
                    <option value="short">{t("persona.verbosity.short")}</option>
                    <option value="normal">{t("persona.verbosity.normal")}</option>
                    <option value="detailed">{t("persona.verbosity.detailed")}</option>
                  </select>
                </Field>
                <Field label={t("persona.language")}>
                  <select
                    class="select"
                    value={draft.traits.language}
                    onChange={(event) =>
                      patch({
                        traits: {
                          ...draft.traits,
                          language: (event.currentTarget as HTMLSelectElement).value,
                        },
                      })
                    }
                  >
                    <option value="auto">{t("persona.language.auto")}</option>
                    <option value="zh-CN">{t("persona.language.zh")}</option>
                    <option value="en">{t("persona.language.en")}</option>
                  </select>
                </Field>
                <div class="field" style="justify-content:flex-end">
                  <Switch
                    checked={draft.traits.emoji}
                    onChange={(emoji) => patch({ traits: { ...draft.traits, emoji } })}
                    label={t("persona.emoji")}
                  />
                </div>
              </div>
            </Section>

            <Section title={t("persona.sampling")}>
              <div class="form-grid">
                <RangeField
                  label={t("persona.temperature")}
                  min={0}
                  max={2}
                  step={0.05}
                  value={draft.sampling.temperature}
                  format={(value) => value.toFixed(2)}
                  onChange={(temperature) =>
                    patch({ sampling: { ...draft.sampling, temperature } })
                  }
                />
                <Field label={t("persona.maxTokens")}>
                  <NumberField
                    value={draft.sampling.maxTokens}
                    min={1}
                    max={200000}
                    step={128}
                    onChange={(maxTokens) => patch({ sampling: { ...draft.sampling, maxTokens } })}
                  />
                </Field>
                <Field label={t("persona.provider")}>
                  <select
                    class="select"
                    value={draft.model?.provider ?? ""}
                    onChange={(event) => {
                      const value = (event.currentTarget as HTMLSelectElement).value;
                      patch({ model: value ? { provider: value, model: draft.model?.model ?? null } : null });
                    }}
                  >
                    <option value="">{t("persona.modelInherit")}</option>
                    {providers.map((item) => (
                      <option key={item.id} value={item.id}>
                        {item.label}
                      </option>
                    ))}
                  </select>
                </Field>
                <Field label={t("persona.modelName")}>
                  <input
                    class="input"
                    value={draft.model?.model ?? ""}
                    placeholder={modelPlaceholder}
                    disabled={!draft.model}
                    onInput={(event) =>
                      patch({
                        model: draft.model
                          ? { ...draft.model, model: (event.currentTarget as HTMLInputElement).value || null }
                          : null,
                      })
                    }
                  />
                </Field>
              </div>
            </Section>

            <Section title={t("persona.memory")}>
              <Switch
                checked={draft.memory.enabled}
                onChange={(enabled) => patch({ memory: { ...draft.memory, enabled } })}
                label={t("persona.memoryEnabled")}
              />
              <div class="form-grid">
                <Field label={t("persona.memoryWindow")}>
                  <NumberField
                    value={draft.memory.windowTurns}
                    min={1}
                    max={200}
                    disabled={!draft.memory.enabled}
                    onChange={(windowTurns) => patch({ memory: { ...draft.memory, windowTurns } })}
                  />
                </Field>
                <Field label={t("persona.summarizeAfter")}>
                  <NumberField
                    value={draft.memory.summarizeAfterTurns}
                    min={1}
                    max={500}
                    disabled={!draft.memory.enabled}
                    onChange={(summarizeAfterTurns) =>
                      patch({ memory: { ...draft.memory, summarizeAfterTurns } })
                    }
                  />
                </Field>
              </div>
              <Switch
                checked={draft.memory.longTerm}
                disabled={!draft.memory.enabled}
                onChange={(longTerm) => patch({ memory: { ...draft.memory, longTerm } })}
                label={t("persona.memoryLongTerm")}
              />
            </Section>

            <Section title={t("persona.tts")}>
              <Switch
                checked={draft.tts.enabled}
                onChange={(enabled) => patch({ tts: { ...draft.tts, enabled } })}
                label={t("persona.ttsEnabled")}
              />
              <div class="form-grid">
                <Field label={t("persona.ttsVoice")}>
                  <input
                    class="input"
                    value={draft.tts.voice ?? ""}
                    disabled={!draft.tts.enabled}
                    placeholder={boot.config.tts.voice ?? ""}
                    onInput={(event) =>
                      patch({ tts: { ...draft.tts, voice: (event.currentTarget as HTMLInputElement).value || null } })
                    }
                  />
                </Field>
                <RangeField
                  label={t("persona.ttsRate")}
                  min={0.5}
                  max={2}
                  step={0.05}
                  value={draft.tts.rate}
                  disabled={!draft.tts.enabled}
                  format={(value) => `${value.toFixed(2)}×`}
                  onChange={(rate) => patch({ tts: { ...draft.tts, rate } })}
                />
              </div>
            </Section>

            <Section title={t("persona.proactive")}>
              <Switch
                checked={draft.proactive.enabled}
                onChange={(enabled) => patch({ proactive: { ...draft.proactive, enabled } })}
                label={t("persona.proactiveEnabled")}
              />
              <Field label={t("persona.proactiveIdle")}>
                <NumberField
                  value={draft.proactive.idleMinutes}
                  min={1}
                  max={1440}
                  disabled={!draft.proactive.enabled}
                  onChange={(idleMinutes) => patch({ proactive: { ...draft.proactive, idleMinutes } })}
                />
              </Field>
            </Section>

            <Section title={t("persona.promptPreview")} desc={t("persona.promptPreviewHint")}>
              <textarea class="textarea mono" rows={12} readOnly value={assembleSystemPrompt(draft)} />
            </Section>
          </>
        )}
      </div>
    </div>
  );
}
