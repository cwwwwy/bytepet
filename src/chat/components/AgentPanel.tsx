/** Agent panel: status server config, coding-agent hooks, diagnostics. */

import { useEffect, useState } from "preact/hooks";
import { ipc, type HookStatus } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import { copyText } from "../../shared/format";
import type { AppConfig } from "../../shared/types";
import { Badge, EmptyState, Field, NumberField, Section, Switch } from "./ui";
import { toast, toastResult } from "./Toast";
import { IconCopy, IconDownload, IconFolder, IconRefresh } from "./icons";

export interface AgentPanelProps {
  agentUrl: string;
  dataDir: string;
  config: AppConfig;
  onConfig: (config: AppConfig) => void;
}

export function AgentPanel(props: AgentPanelProps) {
  const [draft, setDraft] = useState<AppConfig["agent"]>(props.config.agent);
  const [hooks, setHooks] = useState<HookStatus[]>([]);
  const [busy, setBusy] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setDraft(props.config.agent);
  }, [props.config.agent]);

  const loadHooks = async () => {
    setLoading(true);
    const result = await ipc.hooksStatus();
    setLoading(false);
    if (result.ok) setHooks(result.value);
    else toast.error(result.error);
  };

  useEffect(() => {
    void loadHooks();
  }, []);

  const dirty = JSON.stringify(draft) !== JSON.stringify(props.config.agent);

  const save = async () => {
    setBusy("save");
    const result = await ipc.saveSettings({ ...props.config, agent: draft });
    setBusy(null);
    if (toastResult(result, t("settings.saved"))) props.onConfig(result.value);
  };

  const runHooks = async (kind: HookStatus["kind"], install: boolean) => {
    setBusy(`${kind}-${install ? "install" : "uninstall"}`);
    const result = install ? await ipc.installHooks(kind) : await ipc.uninstallHooks(kind);
    setBusy(null);
    if (!result.ok) {
      toast.error(result.error);
      return;
    }
    const messages = result.value.messages ?? [];
    const summary = install
      ? t("agent.hooksInstalled", { kind })
      : t("agent.hooksUninstalled", { kind });
    if (result.value.ok === false) {
      toast.error(messages[0] ?? t("common.failed"));
    } else {
      toast.success(messages.length > 0 ? `${summary} · ${messages.join(" ")}` : summary);
    }
    void loadHooks();
  };

  const copyUrl = () => {
    void copyText(props.agentUrl).then((ok) => {
      if (ok) toast.success(t("agent.copiedUrl"));
    });
  };

  const exportLogs = async () => {
    const result = await ipc.exportLogs();
    if (result.ok) toast.success(t("agent.logsExported", { path: result.value.path }));
    else toast.error(result.error);
  };

  return (
    <div class="stack">
      <Section
        title={t("agent.server")}
        desc={t("agent.url")}
        actions={
          <Badge kind="ok">
            <span class="pet-dot" style="margin-right:2px" />
            {t("agent.running")}
          </Badge>
        }
      >
        <div class="row" style="gap:6px">
          <input class="input mono grow" readOnly value={props.agentUrl} aria-label={t("agent.url")} />
          <button type="button" class="icon-btn" aria-label={t("common.copy")} onClick={copyUrl}>
            <IconCopy size={14} />
          </button>
        </div>
      </Section>

      <Section
        title={t("agent.title")}
        actions={
          <button
            type="button"
            class="btn btn-sm btn-primary"
            disabled={!dirty || busy !== null}
            onClick={() => void save()}
          >
            {t("agent.save")}
          </button>
        }
      >
        <Switch
          checked={draft.enabled}
          onChange={(enabled) => setDraft({ ...draft, enabled })}
          label={t("agent.enabled")}
        />
        <div class="form-grid">
          <Field label={t("agent.port")}>
            <NumberField
              value={draft.port}
              min={1}
              max={65535}
              disabled={!draft.enabled}
              onChange={(port) => setDraft({ ...draft, port })}
            />
          </Field>
          <Field label={t("agent.defaultTtl")}>
            <NumberField
              value={draft.defaultTtlSeconds}
              min={1}
              max={86400}
              disabled={!draft.enabled}
              onChange={(defaultTtlSeconds) => setDraft({ ...draft, defaultTtlSeconds })}
            />
          </Field>
        </div>
        <Switch
          checked={draft.autoInstallHooks}
          disabled={!draft.enabled}
          onChange={(autoInstallHooks) => setDraft({ ...draft, autoInstallHooks })}
          label={t("agent.autoInstallHooks")}
        />
      </Section>

      <Section
        title={t("agent.hooks")}
        desc={t("agent.hooksHint")}
        actions={
          <button type="button" class="icon-btn" aria-label={t("common.refresh")} onClick={() => void loadHooks()}>
            <IconRefresh size={14} />
          </button>
        }
      >
        {loading ? (
          <div class="small muted">{t("common.loading")}</div>
        ) : hooks.length === 0 ? (
          <EmptyState>{t("agent.hooksEmpty")}</EmptyState>
        ) : (
          <div class="card scroll-x">
            <table class="table">
              <thead>
                <tr>
                  <th>{t("agent.hooksKind")}</th>
                  <th>{t("agent.configPath")}</th>
                  <th>{t("agent.exists")}</th>
                  <th>{t("agent.installed")}</th>
                  <th>{t("agent.detail")}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {hooks.map((hook) => (
                  <tr key={hook.kind}>
                    <td class="mono">{hook.kind}</td>
                    <td class="mono small" style="overflow-wrap:anywhere;max-width:260px">
                      {hook.configPath}
                    </td>
                    <td>
                      <Badge kind={hook.exists ? "ok" : "warn"}>
                        {hook.exists ? t("common.yes") : t("common.no")}
                      </Badge>
                    </td>
                    <td>
                      <Badge kind={hook.installed ? "ok" : "default"}>
                        {hook.installed ? t("common.yes") : t("common.no")}
                      </Badge>
                    </td>
                    <td class="small muted" style="overflow-wrap:anywhere;max-width:280px">
                      {hook.detail}
                    </td>
                    <td style="width:1%">
                      {hook.installed ? (
                        <button
                          type="button"
                          class="btn btn-sm"
                          disabled={busy !== null}
                          onClick={() => void runHooks(hook.kind, false)}
                        >
                          {t("agent.uninstall")}
                        </button>
                      ) : (
                        <button
                          type="button"
                          class="btn btn-sm btn-primary"
                          disabled={busy !== null}
                          onClick={() => void runHooks(hook.kind, true)}
                        >
                          {t("agent.install")}
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Section>

      <Section title={t("agent.dataDir")} desc={props.dataDir}>
        <div class="row" style="gap:6px;flex-wrap:wrap">
          <button
            type="button"
            class="btn"
            onClick={() => {
              void ipc.openDataDir().then((result) => {
                if (!result.ok) toast.error(result.error);
              });
            }}
          >
            <IconFolder size={14} />
            {t("agent.openDataDir")}
          </button>
          <button type="button" class="btn" onClick={() => void exportLogs()}>
            <IconDownload size={14} />
            {t("agent.exportLogs")}
          </button>
        </div>
      </Section>
    </div>
  );
}
