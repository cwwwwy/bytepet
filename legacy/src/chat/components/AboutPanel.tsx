/** About tab: version, data dir, keychain state, library roots. */

import { t } from "../../shared/i18n";
import { petSourceLabel } from "../../shared/format";
import type { BootstrapState } from "../../shared/types";
import { Badge, Section } from "./ui";
import { IconFolder } from "./icons";
import { ipc } from "../../shared/ipc";
import { toast } from "./Toast";

export const APP_VERSION = "0.1.0";

export function AboutPanel(props: { boot: BootstrapState }) {
  const { boot } = props;

  return (
    <div class="stack">
      <Section title={t("about.title")} desc={t("about.description")}>
        <div class="row" style="flex-wrap:wrap;gap:8px">
          <Badge kind="accent">
            {t("about.version")} {APP_VERSION}
          </Badge>
          <Badge>Preact 10 · Vite 8 · Tauri 2</Badge>
          <Badge kind={boot.keyringAvailable ? "ok" : "warn"}>
            {t("about.keyring")}:{" "}
            {boot.keyringAvailable ? t("about.keyringAvailable") : t("about.keyringUnavailable")}
          </Badge>
        </div>
      </Section>

      <Section
        title={t("about.dataDir")}
        desc={boot.dataDir}
        actions={
          <button
            type="button"
            class="btn btn-sm"
            onClick={() => {
              void ipc.openDataDir().then((result) => {
                if (!result.ok) toast.error(result.error);
              });
            }}
          >
            <IconFolder size={13} />
            {t("agent.openDataDir")}
          </button>
        }
      >
        <div class="card scroll-x">
          <table class="table">
            <thead>
              <tr>
                <th>{t("about.libraryRoots")}</th>
                <th>{t("pet.source")}</th>
                <th>writable</th>
              </tr>
            </thead>
            <tbody>
              {boot.libraryRoots.map((root) => (
                <tr key={`${root.kind}-${root.path}`}>
                  <td class="mono small" style="overflow-wrap:anywhere">
                    {root.path}
                  </td>
                  <td>
                    <Badge>{petSourceLabel(root.kind)}</Badge>
                  </td>
                  <td>
                    <Badge kind={root.writable ? "ok" : "warn"}>
                      {root.writable ? t("common.yes") : t("common.no")}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Section>

      <Section title={t("about.builtWith")}>
        <div class="small muted">
          marked · DOMPurify · @tauri-apps/api · @tauri-apps/plugin-opener ·
          @tauri-apps/plugin-autostart · @tauri-apps/plugin-notification
        </div>
      </Section>
    </div>
  );
}
