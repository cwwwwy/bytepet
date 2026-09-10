/** Settings shell: tab navigation + the seven panes. */

import { t } from "../../shared/i18n";
import type { AppConfig, BootstrapState, Persona, ProviderConfig } from "../../shared/types";
import { SETTINGS_TABS, settingsTabLabelKey, type SettingsTab, type View } from "../view";
import { AboutPanel } from "./AboutPanel";
import { AgentPanel } from "./AgentPanel";
import { GeneralSettings } from "./GeneralSettings";
import { MemoryManager } from "./MemoryManager";
import { PersonaEditor } from "./PersonaEditor";
import { PetPicker } from "./PetPicker";
import { ProviderManager } from "./ProviderManager";

export interface SettingsViewProps {
  boot: BootstrapState;
  view: Extract<View, { kind: "settings" }>;
  onTab: (tab: SettingsTab) => void;
  onBootstrap: (state: BootstrapState) => void;
  onConfig: (config: AppConfig) => void;
  onPersonas: (personas: Persona[]) => void;
  onProviders: (providers: ProviderConfig[]) => void;
  onConversationsChanged: () => void;
}

export function SettingsView(props: SettingsViewProps) {
  const { boot, view } = props;

  return (
    <div class="settings">
      <div class="settings-inner">
        <div class="settings-nav">
          <div class="tabs" role="tablist" aria-label={t("settings.title")}>
            {SETTINGS_TABS.map((tab) => (
              <button
                key={tab}
                type="button"
                class="tab"
                role="tab"
                aria-selected={view.tab === tab}
                onClick={() => props.onTab(tab)}
              >
                {t(settingsTabLabelKey(tab))}
              </button>
            ))}
          </div>
        </div>

        <div role="tabpanel" aria-label={t(settingsTabLabelKey(view.tab))} class="stack">
          {view.tab === "pet" ? <PetPicker boot={boot} onBootstrap={props.onBootstrap} /> : null}

          {view.tab === "persona" ? (
            <PersonaEditor
              boot={boot}
              onBootstrap={props.onBootstrap}
              onPersonas={props.onPersonas}
            />
          ) : null}

          {view.tab === "provider" ? (
            <ProviderManager
              boot={boot}
              onProviders={props.onProviders}
              onConfig={props.onConfig}
            />
          ) : null}

          {view.tab === "memory" ? (
            <MemoryManager boot={boot} onConversationsChanged={props.onConversationsChanged} />
          ) : null}

          {view.tab === "general" ? (
            <GeneralSettings config={boot.config} onConfig={props.onConfig} />
          ) : null}

          {view.tab === "agent" ? (
            <AgentPanel
              agentUrl={boot.agentUrl}
              dataDir={boot.dataDir}
              config={boot.config}
              onConfig={props.onConfig}
            />
          ) : null}

          {view.tab === "about" ? <AboutPanel boot={boot} /> : null}
        </div>
      </div>
    </div>
  );
}
