/** Provider manager: list + per-kind editor (API providers and CLI wrappers). */

import { useEffect, useMemo, useState } from "preact/hooks";
import { ipc, type ProviderTestResult } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import { uid } from "../../shared/format";
import type { AppConfig, ProviderConfig, ProviderKind } from "../../shared/types";
import { Badge, EmptyState, Field, Section, Switch } from "./ui";
import { confirmAction } from "./Confirm";
import { toastResult } from "./Toast";
import { IconPlus, IconRefresh, IconTrash } from "./icons";

export interface ProviderManagerProps {
  boot: {
    config: AppConfig;
  };
  onProviders: (providers: ProviderConfig[]) => void;
  onConfig: (config: AppConfig) => void;
}

const KINDS: ProviderKind[] = [
  "anthropic",
  "open-ai-chat",
  "open-ai-responses",
  "codex-cli",
  "claude-cli",
];

const CLI_KINDS: ProviderKind[] = ["codex-cli", "claude-cli"];

const KIND_DEFAULTS: Record<ProviderKind, { baseUrl: string | null; model: string }> = {
  anthropic: { baseUrl: "https://api.anthropic.com", model: "" },
  "open-ai-chat": { baseUrl: "https://api.openai.com/v1", model: "" },
  "open-ai-responses": { baseUrl: "https://api.openai.com/v1", model: "" },
  "codex-cli": { baseUrl: null, model: "" },
  "claude-cli": { baseUrl: null, model: "" },
};

interface HeaderRow {
  id: number;
  key: string;
  value: string;
}

function readOption(options: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = options[key];
    if (typeof value === "string") return value;
    if (typeof value === "number") return String(value);
  }
  return "";
}

function readArgs(options: Record<string, unknown>): string {
  const value = options.extraArgs;
  if (Array.isArray(value)) return value.filter((item): item is string => typeof item === "string").join(" ");
  if (typeof value === "string") return value;
  return "";
}

function newProvider(): ProviderConfig {
  return {
    id: uid("provider"),
    label: t("provider.new"),
    kind: "anthropic",
    baseUrl: KIND_DEFAULTS.anthropic.baseUrl,
    model: "",
    apiKeyRef: null,
    extraHeaders: {},
    enabled: true,
    options: {},
  };
}

export function ProviderManager(props: ProviderManagerProps) {
  const providers = useMemo(
    () => Object.values(props.boot.config.providers ?? {}),
    [props.boot.config.providers],
  );
  const [selectedId, setSelectedId] = useState<string | null>(providers[0]?.id ?? null);
  const [draft, setDraft] = useState<ProviderConfig | null>(providers[0] ?? null);
  const [headerRows, setHeaderRows] = useState<HeaderRow[]>([]);
  const [apiKey, setApiKey] = useState("");
  const [hasKey, setHasKey] = useState(false);
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ProviderTestResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [creating, setCreating] = useState(false);

  const selected = useMemo(
    () => providers.find((provider) => provider.id === selectedId) ?? null,
    [providers, selectedId],
  );

  useEffect(() => {
    if (creating) return;
    if (selectedId && providers.some((provider) => provider.id === selectedId)) return;
    setSelectedId(providers[0]?.id ?? null);
  }, [providers, selectedId, creating]);

  useEffect(() => {
    if (creating) return;
    setDraft(selected ? { ...selected, options: { ...selected.options }, extraHeaders: { ...selected.extraHeaders } } : null);
    setTestResult(null);
    setApiKey("");
  }, [selected, creating]);

  useEffect(() => {
    if (!draft) {
      setHeaderRows([]);
      return;
    }
    setHeaderRows(
      Object.entries(draft.extraHeaders).map(([key, value], index) => ({ id: index, key, value })),
    );
  }, [draft?.id]);

  useEffect(() => {
    if (!draft || CLI_KINDS.includes(draft.kind)) {
      setHasKey(false);
      return;
    }
    let cancelled = false;
    void ipc.hasApiKey(draft.id).then((result) => {
      if (!cancelled && result.ok) setHasKey(result.value);
    });
    return () => {
      cancelled = true;
    };
  }, [draft?.id, draft?.kind]);

  if (!draft) {
    return (
      <div class="stack">
        <div class="row">
          <button
            type="button"
            class="btn btn-primary"
            onClick={() => {
              setCreating(true);
              setDraft(newProvider());
            }}
          >
            <IconPlus size={14} />
            {t("provider.new")}
          </button>
        </div>
        <EmptyState>{t("provider.noProviders")}</EmptyState>
      </div>
    );
  }

  const isCli = CLI_KINDS.includes(draft.kind);
  const patch = (next: Partial<ProviderConfig>) =>
    setDraft((current) => (current ? { ...current, ...next } : current));
  const patchOption = (key: string, value: unknown) =>
    setDraft((current) =>
      current ? { ...current, options: { ...current.options, [key]: value } } : current,
    );

  const applyHeaders = (rows: HeaderRow[]) => {
    setHeaderRows(rows);
    const record: Record<string, string> = {};
    for (const row of rows) {
      const key = row.key.trim();
      if (key) record[key] = row.value;
    }
    patch({ extraHeaders: record });
  };

  const changeKind = (kind: ProviderKind) => {
    const defaults = KIND_DEFAULTS[kind];
    patch({ kind, baseUrl: defaults.baseUrl, model: defaults.model, options: {} });
    applyHeaders([]);
    setTestResult(null);
  };

  const save = async () => {
    if (!draft) return;
    setBusy(true);
    const result = await ipc.saveProvider(draft);
    setBusy(false);
    if (toastResult(result, t("provider.saved"))) {
      props.onProviders(result.value);
      setCreating(false);
      setSelectedId(draft.id);
    }
  };

  const remove = async () => {
    if (!draft) return;
    const confirmed = await confirmAction({
      body: t("provider.deleteConfirm", { label: draft.label }),
      danger: true,
    });
    if (!confirmed) return;
    setBusy(true);
    const result = await ipc.deleteProvider(draft.id);
    setBusy(false);
    if (toastResult(result, t("provider.deleted"))) {
      props.onProviders(result.value);
      setCreating(false);
      setSelectedId(null);
    }
  };

  const saveApiKey = async () => {
    if (!draft || !apiKey.trim()) return;
    const result = await ipc.setApiKey(draft.id, apiKey.trim());
    if (toastResult(result, t("provider.apiKeySaved"))) {
      setHasKey(true);
      setApiKey("");
    }
  };

  const test = async () => {
    if (!draft) return;
    setTesting(true);
    setTestResult(null);
    const result = await ipc.testProvider(draft.id);
    setTesting(false);
    if (result.ok) setTestResult(result.value);
    else setTestResult({ ok: false, message: result.error });
  };

  const setDefault = async () => {
    if (!draft) return;
    const result = await ipc.saveSettings({ ...props.boot.config, defaultProvider: draft.id });
    if (toastResult(result, t("provider.defaultSet"))) props.onConfig(result.value);
  };

  const persisted = Boolean(props.boot.config.providers[draft.id]);
  const isDefault = props.boot.config.defaultProvider === draft.id;

  return (
    <div class="split">
      <div class="list-pane">
        <button
          type="button"
          class="btn btn-primary"
          onClick={() => {
            setCreating(true);
            setDraft(newProvider());
            setSelectedId(null);
            setHeaderRows([]);
            setTestResult(null);
          }}
        >
          <IconPlus size={14} />
          {t("provider.new")}
        </button>
        {providers.length === 0 ? <EmptyState>{t("provider.noProviders")}</EmptyState> : null}
        {providers.map((provider) => (
          <button
            key={provider.id}
            type="button"
            class={provider.id === selectedId && !creating ? "list-item active" : "list-item"}
            onClick={() => {
              setCreating(false);
              setSelectedId(provider.id);
            }}
          >
            <span class="list-item-main">
              <span class="list-item-title">{provider.label}</span>
              <span class="list-item-sub">{t(`provider.kind.${provider.kind}`)}</span>
            </span>
            {props.boot.config.defaultProvider === provider.id ? (
              <Badge kind="accent">{t("provider.isDefault")}</Badge>
            ) : null}
            {provider.enabled ? null : <Badge>{t("common.disabled")}</Badge>}
          </button>
        ))}
      </div>

      <div class="stack">
        <div class="row-between">
          <div class="row" style="gap:8px">
            <h2>{draft.label || t("common.unnamed")}</h2>
            {isDefault ? <Badge kind="accent">{t("provider.isDefault")}</Badge> : null}
            {persisted ? null : <Badge kind="warn">{t("common.unsaved")}</Badge>}
          </div>
          <div class="row" style="gap:6px">
            {persisted && !isDefault ? (
              <button type="button" class="btn btn-sm" onClick={() => void setDefault()}>
                {t("provider.setDefault")}
              </button>
            ) : null}
            <button type="button" class="btn btn-sm btn-primary" disabled={busy} onClick={() => void save()}>
              {t("provider.save")}
            </button>
            <button
              type="button"
              class="btn btn-sm btn-danger"
              disabled={busy || !persisted}
              onClick={() => void remove()}
            >
              <IconTrash size={13} />
            </button>
          </div>
        </div>

        <Section title={t("provider.kind")} desc={t(`provider.kindHint.${draft.kind}`)}>
          <div class="form-grid">
            <Field label={t("provider.label")}>
              <input
                class="input"
                value={draft.label}
                onInput={(event) => patch({ label: (event.currentTarget as HTMLInputElement).value })}
              />
            </Field>
            <Field label={t("provider.kind")}>
              <select
                class="select"
                value={draft.kind}
                onChange={(event) =>
                  changeKind((event.currentTarget as HTMLSelectElement).value as ProviderKind)
                }
              >
                {KINDS.map((kind) => (
                  <option key={kind} value={kind}>
                    {t(`provider.kind.${kind}`)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={t("provider.model")}>
              <input
                class="input"
                value={draft.model}
                placeholder={t(`provider.placeholder.${draft.kind}`)}
                onInput={(event) => patch({ model: (event.currentTarget as HTMLInputElement).value })}
              />
            </Field>
            <div class="field" style="justify-content:flex-end">
              <Switch
                checked={draft.enabled}
                onChange={(enabled) => patch({ enabled })}
                label={t("provider.enable")}
              />
            </div>
          </div>
        </Section>

        {isCli ? (
          <Section title={t("provider.cliOptions")} desc={t("provider.cliNoKey")}>
            <div class="form-grid">
              <Field label={t("provider.binaryPath")}>
                <input
                  class="input mono"
                  value={readOption(draft.options, ["binaryPath", "binary", "command"])}
                  placeholder={draft.kind === "codex-cli" ? "codex" : "claude"}
                  onInput={(event) =>
                    patchOption("binaryPath", (event.currentTarget as HTMLInputElement).value)
                  }
                />
              </Field>
              <Field label={t("provider.sandboxMode")}>
                <input
                  class="input mono"
                  value={readOption(draft.options, ["sandboxMode", "permissionMode", "sandbox"])}
                  placeholder={t("provider.sandboxModePlaceholder")}
                  onInput={(event) =>
                    patchOption("sandboxMode", (event.currentTarget as HTMLInputElement).value)
                  }
                />
              </Field>
              <Field label={t("provider.extraArgs")} class="span-2">
                <input
                  class="input mono"
                  value={readArgs(draft.options)}
                  onInput={(event) =>
                    patchOption(
                      "extraArgs",
                      (event.currentTarget as HTMLInputElement)
                        .value.split(/\s+/)
                        .filter(Boolean),
                    )
                  }
                />
              </Field>
              <Field label={t("provider.timeout")}>
                <input
                  class="input"
                  type="number"
                  min={0}
                  value={readOption(draft.options, ["timeoutSeconds", "timeout"])}
                  onInput={(event) =>
                    patchOption(
                      "timeoutSeconds",
                      Number((event.currentTarget as HTMLInputElement).value) || 0,
                    )
                  }
                />
              </Field>
            </div>
          </Section>
        ) : (
          <>
            <Section title={t("provider.baseUrl")} desc={t("provider.baseUrlDefault")}>
              <Field label={t("provider.baseUrl")}>
                <input
                  class="input mono"
                  value={draft.baseUrl ?? ""}
                  placeholder={KIND_DEFAULTS[draft.kind].baseUrl ?? ""}
                  onInput={(event) =>
                    patch({ baseUrl: (event.currentTarget as HTMLInputElement).value || null })
                  }
                />
              </Field>
            </Section>

            <Section
              title={t("provider.apiKey")}
              desc={t("provider.apiKeyHint")}
              actions={
                <Badge kind={hasKey ? "ok" : "warn"}>
                  {hasKey ? t("provider.apiKeySaved") : t("provider.apiKeyMissing")}
                </Badge>
              }
            >
              <div class="form-row">
                <input
                  class="input grow"
                  type="password"
                  autoComplete="off"
                  value={apiKey}
                  placeholder={hasKey ? "••••••••" : "sk-…"}
                  aria-label={t("provider.apiKey")}
                  onInput={(event) => setApiKey((event.currentTarget as HTMLInputElement).value)}
                />
                <button
                  type="button"
                  class="btn"
                  disabled={!apiKey.trim() || !persisted}
                  onClick={() => void saveApiKey()}
                >
                  {t("provider.apiKeySave")}
                </button>
              </div>
            </Section>

            <Section title={t("provider.extraHeaders")}>
              <div class="stack" style="gap:6px">
                {headerRows.map((row, index) => (
                  <div class="kv-row" key={row.id}>
                    <input
                      class="input mono"
                      value={row.key}
                      placeholder={t("provider.headerKey")}
                      aria-label={t("provider.headerKey")}
                      onInput={(event) => {
                        const next = [...headerRows];
                        next[index] = { ...row, key: (event.currentTarget as HTMLInputElement).value };
                        applyHeaders(next);
                      }}
                    />
                    <input
                      class="input mono"
                      value={row.value}
                      placeholder={t("provider.headerValue")}
                      aria-label={t("provider.headerValue")}
                      onInput={(event) => {
                        const next = [...headerRows];
                        next[index] = { ...row, value: (event.currentTarget as HTMLInputElement).value };
                        applyHeaders(next);
                      }}
                    />
                    <button
                      type="button"
                      class="icon-btn danger"
                      aria-label={t("common.remove")}
                      onClick={() => applyHeaders(headerRows.filter((item) => item.id !== row.id))}
                    >
                      <IconTrash size={13} />
                    </button>
                  </div>
                ))}
                <button
                  type="button"
                  class="btn btn-sm"
                  onClick={() =>
                    applyHeaders([
                      ...headerRows,
                      { id: (headerRows.at(-1)?.id ?? 0) + 1, key: "", value: "" },
                    ])
                  }
                >
                  <IconPlus size={13} />
                  {t("common.add")}
                </button>
              </div>
            </Section>
          </>
        )}

        <Section
          title={t("provider.test")}
          actions={
            <button
              type="button"
              class="btn btn-sm"
              disabled={testing || !persisted}
              onClick={() => void test()}
            >
              <IconRefresh size={13} />
              {testing ? t("provider.testing") : t("provider.test")}
            </button>
          }
        >
          {!persisted ? (
            <div class="small muted">{t("common.unsaved")}</div>
          ) : testResult ? (
            <div class={testResult.ok ? "alert alert-ok" : "alert alert-danger"}>
              <span class="grow">
                {testResult.ok
                  ? t("provider.testOk", { ms: testResult.latencyMs ?? 0 })
                  : `${t("provider.testFailed")} · ${testResult.message}`}
              </span>
            </div>
          ) : (
            <div class="small muted">{t("common.test")}</div>
          )}
        </Section>
      </div>
    </div>
  );
}
