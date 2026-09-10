/**
 * First-run onboarding card.
 *
 * Shown once (config.firstRun), it explains that the pet is already on screen
 * and offers the only step that actually unlocks chatting: connecting a model.
 */

import { t } from "../../shared/i18n";
import { ipc } from "../../shared/ipc";
import { IconClose } from "./icons";

interface Props {
  onOpenProviders: () => void;
  onDismiss: () => void;
}

export function Onboarding({ onOpenProviders, onDismiss }: Props) {
  const finish = async (next?: () => void) => {
    // Persist first, then navigate: a failure still lets the user continue.
    await ipc.completeOnboarding();
    onDismiss();
    next?.();
  };

  return (
    <div
      class="modal-backdrop"
      role="dialog"
      aria-modal="true"
      aria-labelledby="onboarding-title"
    >
      <div class="modal onboarding">
        <div class="onboarding-head">
          <h2 id="onboarding-title">{t("onboarding.title")}</h2>
          <button
            type="button"
            class="icon-btn"
            aria-label={t("common.close")}
            title={t("common.close")}
            onClick={() => void finish()}
          >
            <IconClose size={15} />
          </button>
        </div>
        <p class="muted onboarding-subtitle">{t("onboarding.subtitle")}</p>

        <ol class="onboarding-steps">
          <li>
            <strong>{t("onboarding.step1.title")}</strong>
            <span class="muted">{t("onboarding.step1.body")}</span>
          </li>
          <li>
            <strong>{t("onboarding.step2.title")}</strong>
            <span class="muted">{t("onboarding.step2.body")}</span>
          </li>
          <li>
            <strong>{t("onboarding.step3.title")}</strong>
            <span class="muted">{t("onboarding.step3.body")}</span>
          </li>
        </ol>

        <div class="modal-actions">
          <button type="button" class="btn" onClick={() => void finish()}>
            {t("onboarding.later")}
          </button>
          <button
            type="button"
            class="btn btn-primary"
            onClick={() => void finish(onOpenProviders)}
          >
            {t("onboarding.connect")}
          </button>
        </div>
      </div>
    </div>
  );
}
