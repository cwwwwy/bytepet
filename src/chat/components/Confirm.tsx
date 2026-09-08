/**
 * Promise-based confirm/prompt dialogs. Native `window.confirm`/`prompt` are
 * unreliable inside the Tauri webview, so the app routes through one host.
 */

import { useEffect, useRef, useState } from "preact/hooks";
import { t } from "../../shared/i18n";
import { IconAlert } from "./icons";

export interface ConfirmOptions {
  title?: string;
  body: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
}

export interface PromptOptions {
  title?: string;
  label: string;
  value?: string;
  placeholder?: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** Optional select rendered instead of a free-text input. */
  selectOptions?: { value: string; label: string }[];
  /** Extra hint under the input. */
  hint?: string;
}

type Request =
  | { kind: "confirm"; options: ConfirmOptions; resolve: (value: boolean) => void }
  | { kind: "prompt"; options: PromptOptions; resolve: (value: string | null) => void };

let current: Request | null = null;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of listeners) listener();
}

function replaceCurrent(): void {
  if (!current) return;
  if (current.kind === "confirm") current.resolve(false);
  else current.resolve(null);
  current = null;
}

export function confirmAction(options: ConfirmOptions): Promise<boolean> {
  return new Promise((resolve) => {
    replaceCurrent();
    current = { kind: "confirm", options, resolve };
    notify();
  });
}

export function promptAction(options: PromptOptions): Promise<string | null> {
  return new Promise((resolve) => {
    replaceCurrent();
    current = { kind: "prompt", options, resolve };
    notify();
  });
}

export function DialogHost() {
  const [request, setRequest] = useState<Request | null>(current);
  const inputRef = useRef<HTMLInputElement | HTMLSelectElement | null>(null);
  const [value, setValue] = useState("");

  useEffect(() => {
    const listener = () => {
      setRequest(current);
      setValue(current?.kind === "prompt" ? (current.options.value ?? "") : "");
    };
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }, []);

  useEffect(() => {
    if (!request) return;
    const node = inputRef.current;
    if (node && "focus" in node) {
      node.focus();
      if (node instanceof HTMLInputElement) node.select();
    }
  }, [request]);

  if (!request) return null;

  const close = (confirmed: boolean) => {
    const active = current;
    current = null;
    notify();
    if (!active) return;
    if (active.kind === "confirm") active.resolve(confirmed);
    else active.resolve(confirmed ? value.trim() || null : null);
  };

  const title = request.options.title ?? t("common.confirm");
  const danger = request.kind === "confirm" && request.options.danger;
  const confirmLabel =
    request.options.confirmLabel ?? (danger ? t("common.delete") : t("common.confirm"));
  const cancelLabel = request.options.cancelLabel ?? t("common.cancel");

  return (
    <div
      class="modal-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) close(false);
      }}
    >
      <div
        class="modal"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.stopPropagation();
            close(false);
          } else if (event.key === "Enter" && request.kind === "confirm") {
            event.preventDefault();
            close(true);
          } else if (event.key === "Enter" && request.kind === "prompt" && !event.shiftKey) {
            event.preventDefault();
            close(true);
          }
        }}
      >
        <div class="row" style="gap:8px;align-items:flex-start">
          {danger ? (
            <span style="color:var(--danger);margin-top:1px">
              <IconAlert size={16} />
            </span>
          ) : null}
          <div class="grow">
            <h2>{title}</h2>
            <p class="small muted" style="margin-top:6px;white-space:pre-wrap">
              {request.kind === "confirm" ? request.options.body : request.options.label}
            </p>
          </div>
        </div>

        {request.kind === "prompt" ? (
          <div class="field">
            {request.options.selectOptions ? (
              <select
                class="select"
                ref={(node) => {
                  inputRef.current = node;
                }}
                value={value}
                onChange={(event) => setValue((event.currentTarget as HTMLSelectElement).value)}
              >
                {request.options.selectOptions.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            ) : (
              <input
                class="input"
                ref={(node) => {
                  inputRef.current = node;
                }}
                value={value}
                placeholder={request.options.placeholder ?? ""}
                onInput={(event) => setValue((event.currentTarget as HTMLInputElement).value)}
              />
            )}
            {request.options.hint ? <span class="small muted">{request.options.hint}</span> : null}
          </div>
        ) : null}

        <div class="modal-actions">
          <button type="button" class="btn" onClick={() => close(false)}>
            {cancelLabel}
          </button>
          <button
            type="button"
            class={danger ? "btn btn-danger" : "btn btn-primary"}
            disabled={request.kind === "prompt" && !value.trim()}
            onClick={() => close(true)}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
