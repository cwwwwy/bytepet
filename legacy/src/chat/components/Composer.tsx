/**
 * Message composer. Enter sends (Shift+Enter newline) when `sendOnEnter` is on,
 * otherwise Ctrl/Cmd+Enter sends. IME composition is tracked explicitly so a
 * CJK candidate confirm never fires a send.
 */

import { useEffect, useRef } from "preact/hooks";
import { t } from "../../shared/i18n";
import { IconSend, IconStop } from "./icons";

export interface ComposerProps {
  value: string;
  onInput: (value: string) => void;
  onSend: () => void;
  onCancel: () => void;
  streaming: boolean;
  sendOnEnter: boolean;
  disabled?: boolean;
  placeholder?: string;
}

export function Composer(props: ComposerProps) {
  const { value, onInput, onSend, onCancel, streaming, sendOnEnter, disabled, placeholder } = props;
  const areaRef = useRef<HTMLTextAreaElement>(null);
  const composingRef = useRef(false);

  useEffect(() => {
    const area = areaRef.current;
    if (!area) return;
    area.style.height = "auto";
    const max = Math.max(72, window.innerHeight * 0.4);
    area.style.height = `${Math.min(area.scrollHeight, max)}px`;
  }, [value]);

  const canSend = !disabled && !streaming && value.trim().length > 0;

  const submit = () => {
    if (!canSend) return;
    onSend();
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key !== "Enter") return;
    // Never send while an IME candidate window is open.
    if (composingRef.current || event.isComposing || event.keyCode === 229) return;
    const withModifier = event.ctrlKey || event.metaKey;
    const shouldSend = sendOnEnter ? !event.shiftKey : withModifier;
    if (!shouldSend) return;
    event.preventDefault();
    submit();
  };

  const hint = sendOnEnter ? t("chat.composerHint") : t("chat.composerHintNoEnter");

  return (
    <div class="composer">
      <div class="composer-inner">
        <div class="composer-box">
          <textarea
            ref={areaRef}
            class="composer-input"
            rows={1}
            value={value}
            disabled={disabled}
            placeholder={placeholder ?? t("chat.placeholder")}
            aria-label={t("chat.placeholder")}
            onInput={(event) => onInput((event.currentTarget as HTMLTextAreaElement).value)}
            onKeyDown={onKeyDown}
            onCompositionStart={() => {
              composingRef.current = true;
            }}
            onCompositionEnd={(event) => {
              composingRef.current = false;
              onInput((event.currentTarget as HTMLTextAreaElement).value);
            }}
          />
          <div class="composer-actions">
            {streaming ? (
              <button
                type="button"
                class="icon-btn send-btn"
                onClick={onCancel}
                aria-label={t("chat.cancelStream")}
                title={t("chat.cancelStream")}
              >
                <IconStop size={14} />
              </button>
            ) : (
              <button
                type="button"
                class={canSend ? "icon-btn send-btn primary" : "icon-btn send-btn"}
                onClick={submit}
                disabled={!canSend}
                aria-label={t("chat.send")}
                title={t("chat.send")}
              >
                <IconSend size={16} />
              </button>
            )}
          </div>
        </div>
        <div class="composer-hint">
          <span>{hint}</span>
          <span>{streaming ? t("chat.streamingStatus") : t("chat.attachmentHint")}</span>
        </div>
      </div>
    </div>
  );
}
