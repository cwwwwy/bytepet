/** General settings: pet window, chat prefs, TTS, interface, autostart. */

import { useEffect, useState } from "preact/hooks";
import { disable as autostartDisable, enable as autostartEnable, isEnabled as autostartIsEnabled } from "@tauri-apps/plugin-autostart";
import { ipc } from "../../shared/ipc";
import { setLanguage, t } from "../../shared/i18n";
import type { AppConfig } from "../../shared/types";
import { Field, NumberField, RadioCards, RangeField, Section, Switch } from "./ui";
import { toast, toastResult } from "./Toast";
import { IconPlay } from "./icons";

export interface GeneralSettingsProps {
  config: AppConfig;
  onConfig: (config: AppConfig) => void;
}

function cloneConfig(config: AppConfig): AppConfig {
  return JSON.parse(JSON.stringify(config)) as AppConfig;
}

export function GeneralSettings(props: GeneralSettingsProps) {
  const [draft, setDraft] = useState<AppConfig>(() => cloneConfig(props.config));
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setDraft(cloneConfig(props.config));
  }, [props.config]);

  useEffect(() => {
    let cancelled = false;
    void autostartIsEnabled()
      .then((enabled) => {
        if (!cancelled) setAutostart(enabled);
      })
      .catch(() => {
        if (!cancelled) setAutostart(null);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const dirty = JSON.stringify(draft) !== JSON.stringify(props.config);

  const patch = (next: Partial<AppConfig>) => setDraft((current) => ({ ...current, ...next }));

  const toggleAutostart = async (enabled: boolean) => {
    setAutostart(enabled);
    patch({ ui: { ...draft.ui, launchAtLogin: enabled } });
    try {
      if (enabled) await autostartEnable();
      else await autostartDisable();
    } catch (error) {
      setAutostart(!enabled);
      patch({ ui: { ...draft.ui, launchAtLogin: !enabled } });
      toast.error(`${t("settings.autostartFailed")}: ${String(error)}`);
    }
  };

  const save = async () => {
    setBusy(true);
    const result = await ipc.saveSettings(draft);
    setBusy(false);
    if (toastResult(result, t("settings.saved"))) {
      setLanguage(result.value.ui.language);
      props.onConfig(result.value);
    }
  };

  const preview = () => {
    void ipc.speak(t("settings.previewText")).then((result) => {
      if (!result.ok) toast.error(result.error);
    });
  };

  const providers = Object.values(props.config.providers ?? {});

  return (
    <div class="stack">
      <div class="row-between">
        <div class="row" style="gap:8px">
          <h2>{t("settings.title")}</h2>
          {dirty ? <span class="badge badge-warn">{t("settings.dirty")}</span> : null}
        </div>
        <div class="row" style="gap:6px">
          <button type="button" class="btn btn-sm" disabled={!dirty} onClick={() => setDraft(cloneConfig(props.config))}>
            {t("settings.discard")}
          </button>
          <button type="button" class="btn btn-sm btn-primary" disabled={!dirty || busy} onClick={() => void save()}>
            {t("settings.save")}
          </button>
        </div>
      </div>

      <Section title={t("settings.petWindow")}>
        <div class="form-grid">
          <RangeField
            label={t("settings.scale")}
            min={0.5}
            max={3}
            step={0.05}
            value={draft.pet.scale}
            format={(value) => `${value.toFixed(2)}×`}
            onChange={(scale) => patch({ pet: { ...draft.pet, scale } })}
          />
          <RangeField
            label={t("settings.opacity")}
            min={0.2}
            max={1}
            step={0.05}
            value={draft.pet.opacity}
            format={(value) => `${Math.round(value * 100)}%`}
            onChange={(opacity) => patch({ pet: { ...draft.pet, opacity } })}
          />
        </div>
        <Switch
          checked={draft.pet.alwaysOnTop}
          onChange={(alwaysOnTop) => patch({ pet: { ...draft.pet, alwaysOnTop } })}
          label={t("settings.alwaysOnTop")}
        />
        <div class="field">
          <span class="field-label">
            <span>{t("settings.clickThrough")}</span>
          </span>
          <RadioCards
            name="click-through"
            value={draft.pet.clickThrough}
            onChange={(clickThrough) => patch({ pet: { ...draft.pet, clickThrough } })}
            options={[
              {
                value: "auto",
                title: t("settings.clickThrough.auto"),
                hint: t("settings.clickThrough.autoHint"),
              },
              {
                value: "rect",
                title: t("settings.clickThrough.rect"),
                hint: t("settings.clickThrough.rectHint"),
              },
              {
                value: "passthrough",
                title: t("settings.clickThrough.passthrough"),
                hint: t("settings.clickThrough.passthroughHint"),
              },
            ]}
          />
        </div>
        <div class="divider" />
        <Switch
          checked={draft.pet.autoWalk.enabled}
          onChange={(enabled) => patch({ pet: { ...draft.pet, autoWalk: { ...draft.pet.autoWalk, enabled } } })}
          label={t("settings.autoWalk")}
        />
        <div class="form-grid">
          <Field label={t("settings.speed")}>
            <NumberField
              value={draft.pet.autoWalk.speedPxS}
              min={1}
              max={2000}
              disabled={!draft.pet.autoWalk.enabled}
              onChange={(speedPxS) =>
                patch({ pet: { ...draft.pet, autoWalk: { ...draft.pet.autoWalk, speedPxS } } })
              }
            />
          </Field>
          <Field label={t("settings.pauseSeconds")}>
            <NumberField
              value={draft.pet.autoWalk.pauseSeconds}
              min={0}
              max={600}
              disabled={!draft.pet.autoWalk.enabled}
              onChange={(pauseSeconds) =>
                patch({ pet: { ...draft.pet, autoWalk: { ...draft.pet.autoWalk, pauseSeconds } } })
              }
            />
          </Field>
          <Field label={t("settings.edgeMargin")}>
            <NumberField
              value={draft.pet.autoWalk.edgeMargin}
              min={0}
              max={2000}
              disabled={!draft.pet.autoWalk.enabled}
              onChange={(edgeMargin) =>
                patch({ pet: { ...draft.pet, autoWalk: { ...draft.pet.autoWalk, edgeMargin } } })
              }
            />
          </Field>
          <Field label={t("settings.userGrace")}>
            <NumberField
              value={draft.pet.autoWalk.userGraceSeconds}
              min={0}
              max={600}
              disabled={!draft.pet.autoWalk.enabled}
              onChange={(userGraceSeconds) =>
                patch({
                  pet: { ...draft.pet, autoWalk: { ...draft.pet.autoWalk, userGraceSeconds } },
                })
              }
            />
          </Field>
        </div>
      </Section>

      <Section title={t("settings.chat")}>
        <Switch
          checked={draft.chat.sendOnEnter}
          onChange={(sendOnEnter) => patch({ chat: { ...draft.chat, sendOnEnter } })}
          label={t("settings.sendOnEnter")}
          hint={t("settings.sendOnEnterHint")}
        />
        <Switch
          checked={draft.chat.showReasoning}
          onChange={(showReasoning) => patch({ chat: { ...draft.chat, showReasoning } })}
          label={t("settings.showReasoning")}
        />
        <Switch
          checked={draft.chat.speakReplies}
          onChange={(speakReplies) => patch({ chat: { ...draft.chat, speakReplies } })}
          label={t("settings.speakReplies")}
        />
        <div class="form-grid">
          <Field label={t("settings.maxContextTurns")}>
            <NumberField
              value={draft.chat.maxContextTurns}
              min={1}
              max={200}
              onChange={(maxContextTurns) => patch({ chat: { ...draft.chat, maxContextTurns } })}
            />
          </Field>
          <Field label={t("settings.defaultProvider")}>
            <select
              class="select"
              value={draft.defaultProvider ?? ""}
              onChange={(event) =>
                patch({ defaultProvider: (event.currentTarget as HTMLSelectElement).value || null })
              }
            >
              <option value="">{t("common.none")}</option>
              {providers.map((provider) => (
                <option key={provider.id} value={provider.id}>
                  {provider.label}
                </option>
              ))}
            </select>
          </Field>
        </div>
      </Section>

      <Section
        title={t("settings.tts")}
        actions={
          <button type="button" class="btn btn-sm" onClick={preview}>
            <IconPlay size={13} />
            {t("settings.preview")}
          </button>
        }
      >
        <Switch
          checked={draft.tts.enabled}
          onChange={(enabled) => patch({ tts: { ...draft.tts, enabled } })}
          label={t("settings.ttsEnabled")}
        />
        <div class="form-grid">
          <Field label={t("settings.ttsVoice")}>
            <input
              class="input"
              value={draft.tts.voice ?? ""}
              disabled={!draft.tts.enabled}
              onInput={(event) =>
                patch({ tts: { ...draft.tts, voice: (event.currentTarget as HTMLInputElement).value || null } })
              }
            />
          </Field>
          <RangeField
            label={t("settings.ttsRate")}
            min={0.5}
            max={2}
            step={0.05}
            value={draft.tts.rate}
            disabled={!draft.tts.enabled}
            format={(value) => `${value.toFixed(2)}×`}
            onChange={(rate) => patch({ tts: { ...draft.tts, rate } })}
          />
          <Field label={t("settings.ttsMaxChars")}>
            <NumberField
              value={draft.tts.maxChars}
              min={1}
              max={10000}
              disabled={!draft.tts.enabled}
              onChange={(maxChars) => patch({ tts: { ...draft.tts, maxChars } })}
            />
          </Field>
        </div>
      </Section>

      <Section title={t("settings.ui")}>
        <div class="form-grid">
          <Field label={t("settings.language")}>
            <select
              class="select"
              value={draft.ui.language}
              onChange={(event) =>
                patch({ ui: { ...draft.ui, language: (event.currentTarget as HTMLSelectElement).value } })
              }
            >
              <option value="zh-CN">简体中文</option>
              <option value="en">English</option>
            </select>
          </Field>
          <Field label={t("settings.theme")}>
            <select
              class="select"
              value={draft.ui.theme}
              onChange={(event) =>
                patch({ ui: { ...draft.ui, theme: (event.currentTarget as HTMLSelectElement).value } })
              }
            >
              <option value="system">{t("settings.theme.system")}</option>
              <option value="dark">{t("settings.theme.dark")}</option>
              <option value="light">{t("settings.theme.light")}</option>
            </select>
          </Field>
        </div>
        <Switch
          checked={autostart ?? draft.ui.launchAtLogin}
          disabled={autostart === null}
          onChange={(enabled) => void toggleAutostart(enabled)}
          label={t("settings.launchAtLogin")}
          hint={t("settings.launchAtLoginHint")}
        />
      </Section>
    </div>
  );
}
