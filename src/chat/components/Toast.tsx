/**
 * Minimal toast system: a module-level store + one `<ToastHost />` mounted by
 * the app. Commands report through `toast.fromResult(...)` so every failure is
 * surfaced exactly once.
 */

import { useEffect, useState } from "preact/hooks";
import type { IpcResult } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import { IconAlert, IconCheck, IconClose, IconInfo } from "./icons";

export type ToastKind = "success" | "error" | "info";

export interface ToastItem {
  id: number;
  kind: ToastKind;
  message: string;
}

const DEFAULT_DURATION = 4200;
const MAX_TOASTS = 4;

let nextId = 1;
let items: ToastItem[] = [];
const listeners = new Set<(next: ToastItem[]) => void>();
const timers = new Map<number, number>();

function emit(): void {
  for (const listener of listeners) listener(items);
}

function schedule(id: number, durationMs: number): void {
  if (durationMs <= 0) return;
  const timer = window.setTimeout(() => dismissToast(id), durationMs);
  timers.set(id, timer);
}

export function pushToast(message: string, kind: ToastKind = "info", durationMs = DEFAULT_DURATION): number {
  const id = nextId++;
  items = [...items, { id, kind, message }].slice(-MAX_TOASTS);
  emit();
  schedule(id, durationMs);
  return id;
}

export function dismissToast(id: number): void {
  const timer = timers.get(id);
  if (timer !== undefined) {
    window.clearTimeout(timer);
    timers.delete(id);
  }
  items = items.filter((item) => item.id !== id);
  emit();
}

export function clearToasts(): void {
  for (const timer of timers.values()) window.clearTimeout(timer);
  timers.clear();
  items = [];
  emit();
}

export const toast = {
  success: (message: string) => pushToast(message, "success"),
  error: (message: string) => pushToast(message, "error", 6000),
  info: (message: string) => pushToast(message, "info"),
};

/** Report an `IpcResult`: success toast with `okMessage`, failure with the error. */
export function toastResult<T>(result: IpcResult<T>, okMessage?: string): result is { ok: true; value: T } {
  if (result.ok) {
    if (okMessage) toast.success(okMessage);
    return true;
  }
  toast.error(result.error);
  return false;
}

export function ToastHost() {
  const [visible, setVisible] = useState<ToastItem[]>(items);

  useEffect(() => {
    const listener = (next: ToastItem[]) => setVisible(next);
    listeners.add(listener);
    setVisible(items);
    return () => {
      listeners.delete(listener);
    };
  }, []);

  if (visible.length === 0) return null;

  return (
    <div class="toast-host" role="status" aria-live="polite">
      {visible.map((item) => (
        <div key={item.id} class={`toast ${item.kind}`}>
          <span style="margin-top:1px">
            {item.kind === "success" ? (
              <IconCheck size={15} />
            ) : item.kind === "error" ? (
              <IconAlert size={15} />
            ) : (
              <IconInfo size={15} />
            )}
          </span>
          <span class="grow">{item.message}</span>
          <button
            type="button"
            class="icon-btn toast-close"
            aria-label={t("common.close")}
            onClick={() => dismissToast(item.id)}
          >
            <IconClose size={13} />
          </button>
        </div>
      ))}
    </div>
  );
}
