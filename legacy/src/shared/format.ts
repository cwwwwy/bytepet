/** Small pure formatting/coercion helpers shared by the chat and settings UI. */

import { getLanguage, t } from "./i18n";

export function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}

/** Parse a number input, returning `fallback` for empty/invalid text. */
export function toNumber(value: string, fallback: number): number {
  const parsed = Number(value.trim());
  return Number.isFinite(parsed) ? parsed : fallback;
}

/** Integer coercion for settings inputs, clamped to a sane range. */
export function toInt(value: string, fallback: number, min = 0, max = 1_000_000): number {
  return Math.round(clamp(toNumber(value, fallback), min, max));
}

export function formatTime(ts: number): string {
  if (!ts) return "";
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function formatDateTime(ts: number): string {
  if (!ts) return "";
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return "";
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}`;
}

/** Locale-aware coarse relative time ("3 分钟前" / "3 min ago"). */
export function formatRelative(ts: number, now = Date.now()): string {
  if (!ts) return "";
  const diff = Math.max(0, now - ts);
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (diff < minute) return t("time.justNow");
  if (diff < hour) return t("time.minutesAgo", { n: Math.floor(diff / minute) });
  if (diff < day) return t("time.hoursAgo", { n: Math.floor(diff / hour) });
  if (diff < 7 * day) return t("time.daysAgo", { n: Math.floor(diff / day) });
  return formatDateTime(ts);
}

/** Compact token counts: 950 -> "950", 12400 -> "12.4k". */
export function formatTokens(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return "0";
  if (n < 1000) return String(Math.round(n));
  if (n < 100_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, "")}k`;
  return `${Math.round(n / 1000)}k`;
}

/** Rough token estimate used for the live counter while streaming. */
export function estimateTokens(text: string): number {
  if (!text) return 0;
  let ascii = 0;
  let wide = 0;
  for (const char of text) {
    if (char.codePointAt(0)! > 0x2e80) wide += 1;
    else ascii += 1;
  }
  return Math.ceil(ascii / 4) + wide;
}

export function truncate(value: string, max: number): string {
  if (value.length <= max) return value;
  return `${value.slice(0, Math.max(0, max - 1))}…`;
}

export function uid(prefix = "id"): string {
  const cryptoApi = globalThis.crypto;
  if (cryptoApi && typeof cryptoApi.randomUUID === "function") {
    return `${prefix}-${cryptoApi.randomUUID()}`;
  }
  return `${prefix}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** Best-effort clipboard copy that works in the Tauri webview and tests. */
export async function copyText(text: string): Promise<boolean> {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return true;
    }
  } catch {
    /* fall through to the legacy path */
  }
  try {
    const area = document.createElement("textarea");
    area.value = text;
    area.setAttribute("readonly", "");
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    const ok = document.execCommand("copy");
    document.body.removeChild(area);
    return ok;
  } catch {
    return false;
  }
}

/** Localised display name for a pet source badge. */
export function petSourceLabel(root: string): string {
  const key = `pet.source.${root}`;
  const label = t(key);
  return label === key ? t("pet.source.custom") : label;
}

/** Locale tag for `Intl`/`toLocaleString`. */
export function localeTag(): string {
  return getLanguage() === "en" ? "en-US" : "zh-CN";
}

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}
