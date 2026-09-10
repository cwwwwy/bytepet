/** Pet library: import/validate by path, live preview grid, activate a skin. */

import { useState } from "preact/hooks";
import { ipc } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import { petSourceLabel } from "../../shared/format";
import type { BootstrapState, PetEntry, ValidationReport } from "../../shared/types";
import { PetPreview } from "./PetPreview";
import { Badge, EmptyState, Section, Switch } from "./ui";
import { confirmAction, promptAction } from "./Confirm";
import { toast, toastResult } from "./Toast";
import { IconDownload, IconRefresh, IconTrash, IconUpload } from "./icons";

export interface PetPickerProps {
  boot: BootstrapState;
  onBootstrap: (state: BootstrapState) => void;
}

function ValidationPanel({ report }: { report: ValidationReport }) {
  return (
    <div class="stack" style="gap:8px">
      <div class="row" style="flex-wrap:wrap">
        <Badge kind={report.ok ? "ok" : "danger"}>
          {report.ok ? t("pet.validationOk") : t("pet.validationFailed")}
        </Badge>
        {report.displayName ? <Badge>{report.displayName}</Badge> : null}
        <Badge>{report.id}</Badge>
        <Badge>
          {t("pet.frameSpec", {
            w: report.frame.width,
            h: report.frame.height,
            cols: report.frame.columns,
            rows: report.frame.rows,
          })}
        </Badge>
        {report.imageSize ? (
          <Badge>
            {t("pet.imageSize")} {report.imageSize[0]}×{report.imageSize[1]}
          </Badge>
        ) : null}
      </div>

      {report.errors.length > 0 ? (
        <div class="alert alert-danger">
          <div class="grow">
            <strong>{t("pet.errors")}</strong>
            <ul>
              {report.errors.map((message) => (
                <li key={message}>{message}</li>
              ))}
            </ul>
          </div>
        </div>
      ) : null}

      {report.warnings.length > 0 ? (
        <div class="alert alert-warn">
          <div class="grow">
            <strong>{t("pet.warnings")}</strong>
            <ul>
              {report.warnings.map((message) => (
                <li key={message}>{message}</li>
              ))}
            </ul>
          </div>
        </div>
      ) : null}

      {report.animations.length > 0 ? (
        <div class="card scroll-x">
          <table class="table">
            <thead>
              <tr>
                <th>{t("pet.animationState")}</th>
                <th>{t("pet.animationFrames")}</th>
                <th>{t("pet.animationLoop")}</th>
                <th>{t("pet.animationFallback")}</th>
              </tr>
            </thead>
            <tbody>
              {report.animations.map((animation) => (
                <tr key={animation.state}>
                  <td class="mono">{animation.state}</td>
                  <td>{animation.frames}</td>
                  <td>{animation.loopAnim ? t("common.yes") : t("common.no")}</td>
                  <td class="mono muted">{animation.fallback}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </div>
  );
}

export function PetPicker(props: PetPickerProps) {
  const { boot } = props;
  const [path, setPath] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [report, setReport] = useState<ValidationReport | null>(null);
  const [busy, setBusy] = useState<string | null>(null);

  const activeId = boot.activePet?.id ?? null;

  const validate = async () => {
    if (!path.trim()) return;
    setBusy("validate");
    const result = await ipc.validatePet(path.trim());
    setBusy(null);
    if (result.ok) {
      setReport(result.value);
      if (result.value.ok) toast.success(t("pet.validationOk"));
      else toast.error(result.value.errors[0] ?? t("pet.validationFailed"));
    } else {
      toast.error(result.error);
    }
  };

  const importPet = async () => {
    if (!path.trim()) return;
    setBusy("import");
    const result = await ipc.importPet(path.trim(), overwrite);
    setBusy(null);
    if (result.ok) {
      props.onBootstrap(result.value);
      setReport(null);
      const imported = result.value.pets.find(
        (pet) => pet.dir === path.trim() || pet.spritesheet.startsWith(path.trim()),
      );
      toast.success(t("pet.imported", { name: imported?.displayName ?? path.trim() }));
    } else {
      toast.error(result.error);
    }
  };

  const usePet = async (pet: PetEntry) => {
    if (pet.id === activeId) return;
    setBusy(pet.id);
    const result = await ipc.usePet(pet.id);
    setBusy(null);
    if (toastResult(result, t("pet.using", { name: pet.displayName }))) {
      props.onBootstrap(result.value);
    }
  };

  const removePet = async (pet: PetEntry) => {
    const confirmed = await confirmAction({
      body: t("pet.removeConfirm", { name: pet.displayName }),
      danger: true,
    });
    if (!confirmed) return;
    setBusy(pet.id);
    const result = await ipc.removePet(pet.id);
    setBusy(null);
    if (toastResult(result, t("pet.removed", { name: pet.displayName }))) {
      props.onBootstrap(result.value);
    }
  };

  const exportPet = async (pet: PetEntry) => {
    const out = await promptAction({
      label: t("pet.exportPath"),
      value: `${pet.id}.pet`,
    });
    if (!out) return;
    const result = await ipc.exportPet(pet.id, out);
    toastResult(result, t("pet.exported", { path: out }));
  };

  return (
    <div class="stack">
      <Section title={t("pet.import")} desc={t("pet.importPath")}>
        <div class="form-row">
          <input
            class="input grow"
            value={path}
            placeholder={t("pet.importPathPlaceholder")}
            aria-label={t("pet.importPath")}
            onInput={(event) => setPath((event.currentTarget as HTMLInputElement).value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void validate();
            }}
          />
          <button type="button" class="btn" disabled={busy !== null} onClick={() => void validate()}>
            <IconRefresh size={14} />
            {busy === "validate" ? t("common.loading") : t("pet.validate")}
          </button>
          <button
            type="button"
            class="btn btn-primary"
            disabled={busy !== null || !path.trim()}
            onClick={() => void importPet()}
          >
            <IconUpload size={14} />
            {busy === "import" ? t("common.loading") : t("common.import")}
          </button>
        </div>
        <Switch checked={overwrite} onChange={setOverwrite} label={t("pet.overwrite")} />
        {report ? <ValidationPanel report={report} /> : null}
      </Section>

      <Section
        title={t("pet.title")}
        desc={t("pet.libraryRoots")}
        actions={<Badge kind="accent">{boot.pets.length}</Badge>}
      >
        <div class="row" style="flex-wrap:wrap;gap:6px">
          {boot.libraryRoots.map((root) => (
            <span class="badge" key={`${root.kind}-${root.path}`} title={root.path}>
              {petSourceLabel(root.kind)}
              {root.writable ? "" : " · readonly"}
            </span>
          ))}
        </div>

        {boot.pets.length === 0 ? (
          <EmptyState>{t("pet.noPets")}</EmptyState>
        ) : (
          <div class="pet-grid">
            {boot.pets.map((pet) => (
              <div key={pet.id} class={pet.id === activeId ? "pet-card active" : "pet-card"}>
                <PetPreview pet={pet} />
                <div class="pet-card-title">
                  <span class="pet-card-name" title={pet.displayName}>
                    {pet.displayName}
                  </span>
                  {pet.id === activeId ? <Badge kind="accent">{t("pet.active")}</Badge> : null}
                </div>
                <div class="pet-meta">
                  <span class="row" style="gap:6px">
                    <Badge>{petSourceLabel(pet.root)}</Badge>
                    {pet.linked ? <Badge kind="ok">{t("pet.linked")}</Badge> : null}
                  </span>
                  <span>
                    {t("pet.frameSpec", {
                      w: pet.frame.width,
                      h: pet.frame.height,
                      cols: pet.frame.columns,
                      rows: pet.frame.rows,
                    })}
                  </span>
                  <span>
                    {t("pet.animations", { n: Object.keys(pet.animations).length })}
                  </span>
                </div>
                <div class="pet-card-actions">
                  <button
                    type="button"
                    class="btn btn-sm grow"
                    disabled={pet.id === activeId || busy !== null}
                    onClick={() => void usePet(pet)}
                  >
                    {pet.id === activeId ? t("pet.active") : t("pet.use")}
                  </button>
                  <button
                    type="button"
                    class="icon-btn"
                    aria-label={t("pet.export")}
                    title={t("pet.export")}
                    onClick={() => void exportPet(pet)}
                  >
                    <IconDownload size={14} />
                  </button>
                  <button
                    type="button"
                    class="icon-btn danger"
                    aria-label={t("pet.remove")}
                    title={t("pet.remove")}
                    disabled={busy !== null}
                    onClick={() => void removePet(pet)}
                  >
                    <IconTrash size={14} />
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}
      </Section>
    </div>
  );
}
