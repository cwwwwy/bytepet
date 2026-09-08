/**
 * Markdown -> safe HTML pipeline.
 *
 * Layers, in order:
 *   1. `marked` renders Markdown (raw HTML in the source passes through, which is
 *      exactly why step 2 exists).
 *   2. `stripDangerousHtml` removes script/style/event-handler/`javascript:`
 *      constructs — a cheap, DOM-free guard that always runs.
 *   3. DOMPurify runs in the browser as the authoritative sanitizer. When there
 *      is no DOM (vitest's node environment, SSR) step 2 is the last line.
 *
 * Rendered HTML is cached per (language, source) because streaming re-renders the
 * same growing string many times per second.
 */

import { Marked, type Tokens } from "marked";
import DOMPurify from "dompurify";
import { getLanguage, t } from "./i18n";

export type HtmlSanitizer = (html: string) => string;

const CACHE_LIMIT = 240;
const htmlCache = new Map<string, string>();

/** Attributes that may survive sanitization. */
const PURIFY_CONFIG = {
  ADD_ATTR: ["target", "rel", "loading", "decoding"],
  FORBID_TAGS: [
    "script",
    "style",
    "iframe",
    "frame",
    "frameset",
    "object",
    "embed",
    "link",
    "meta",
    "base",
    "form",
    "input",
    "textarea",
    "select",
    "option",
    "svg",
    "math",
    "template",
    "noscript",
    "marquee",
    "blink",
  ],
  FORBID_ATTR: ["style", "srcset", "formaction", "action", "srcdoc", "xlink:href"],
  ALLOW_DATA_ATTR: false,
  ALLOW_UNKNOWN_PROTOCOLS: false,
};

const marked = new Marked({ gfm: true, breaks: true });

export function escapeHtml(value: string): string {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

marked.use({
  renderer: {
    code({ text, lang }: Tokens.Code): string {
      const language = (lang ?? "").trim().split(/\s+/)[0] ?? "";
      const languageClass = language ? ` class="language-${escapeHtml(language)}"` : "";
      const label = language ? escapeHtml(language) : "";
      return (
        `<div class="md-code">` +
        `<div class="md-code-bar">` +
        `<span class="md-code-lang">${label}</span>` +
        `<button type="button" class="md-copy" aria-label="${escapeHtml(
          t("chat.copyCode"),
        )}" title="${escapeHtml(t("chat.copyCode"))}">` +
        `<span class="md-copy-icon" aria-hidden="true"></span>` +
        `<span class="md-copy-label">${escapeHtml(t("common.copy"))}</span>` +
        `</button>` +
        `</div>` +
        `<pre><code${languageClass}>${escapeHtml(text)}</code></pre>` +
        `</div>`
      );
    },
    link({ href, title, text }: Tokens.Link): string {
      const titleAttr = title ? ` title="${escapeHtml(title)}"` : "";
      return `<a href="${escapeHtml(href)}"${titleAttr} target="_blank" rel="noopener noreferrer nofollow">${text}</a>`;
    },
    image({ href, title, text }: Tokens.Image): string {
      const titleAttr = title ? ` title="${escapeHtml(title)}"` : "";
      return `<img src="${escapeHtml(href)}" alt="${escapeHtml(text)}"${titleAttr} loading="lazy" decoding="async">`;
    },
  },
});

const DANGEROUS_BLOCKS =
  /<(script|style|iframe|object|embed|link|meta|base|form|textarea|svg|math|template)\b[\s\S]*?(?:<\/\1\s*>|$)/gi;

/**
 * A complete HTML start/close tag, allowing `>` inside quoted attribute values.
 * Attribute scrubbing runs only on these matches so that *escaped* code samples
 * (`&lt;img onerror="…"&gt;`) are never touched — code blocks keep their text.
 */
const TAG_PATTERN = /<[a-zA-Z][^>"']*(?:"[^"]*"[^>"']*|'[^']*'[^>"']*)*>/g;

const EVENT_ATTR = /\son[a-z0-9_-]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi;
const STYLE_ATTR = /\s(?:style|srcdoc)\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi;
const URL_ATTR =
  /\b(href|src|xlink:href|formaction|action)\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))/gi;

function scrubAttributes(tag: string): string {
  let out = tag.replace(EVENT_ATTR, "");
  out = out.replace(URL_ATTR, (match, name: string, _raw: string, dq?: string, sq?: string, bare?: string) => {
    const value = dq ?? sq ?? bare ?? "";
    const normalized = value.replace(/[\u0000-\u0020\u00a0]/g, "").toLowerCase();
    return /^(javascript|vbscript|data):/.test(normalized) ? `${name}="#"` : match;
  });
  return out.replace(STYLE_ATTR, "");
}

/** Neutralize the classic XSS constructs without needing a DOM. */
export function stripDangerousHtml(html: string): string {
  let out = html.replace(DANGEROUS_BLOCKS, "");
  out = out.replace(TAG_PATTERN, scrubAttributes);
  // Browsers still parse a trailing unclosed tag, so scrub and close it.
  out = out.replace(/<[a-zA-Z][^>]*$/, (fragment) => `${scrubAttributes(fragment)}>`);
  return out;
}

/** DOMPurify-backed sanitizer; falls back to the regex guard without a DOM. */
export function domPurifySanitizer(html: string): string {
  if (typeof window === "undefined" || !DOMPurify.isSupported) {
    return stripDangerousHtml(html);
  }
  return String(DOMPurify.sanitize(html, { ...PURIFY_CONFIG }));
}

/** Always: regex guard first, then the injected/default sanitizer. */
export function sanitizeHtml(html: string, sanitizer: HtmlSanitizer = domPurifySanitizer): string {
  return sanitizer(stripDangerousHtml(html));
}

export interface RenderMarkdownOptions {
  /** Override the sanitizer (tests use a spy/pass-through implementation). */
  sanitizer?: HtmlSanitizer;
  /** Skip the cache (streaming uses its own throttling). */
  noCache?: boolean;
}

/** Render Markdown to sanitized HTML. */
export function renderMarkdown(source: string, options: RenderMarkdownOptions = {}): string {
  const sanitizer = options.sanitizer ?? domPurifySanitizer;
  const cacheKey = `${getLanguage()}\u0000${source}`;
  if (!options.noCache) {
    const cached = htmlCache.get(cacheKey);
    if (cached !== undefined) return cached;
  }
  const raw = marked.parse(source, { async: false });
  const html = sanitizeHtml(typeof raw === "string" ? raw : "", sanitizer);
  if (!options.noCache) {
    if (htmlCache.size >= CACHE_LIMIT) htmlCache.clear();
    htmlCache.set(cacheKey, html);
  }
  return html;
}

/** Drop the render cache (language switch, tests). */
export function clearMarkdownCache(): void {
  htmlCache.clear();
}

/**
 * Text shown in a collapsed reasoning block: strip Markdown syntax so the
 * preview is readable without rendering another tree.
 */
export function plainTextPreview(source: string, maxLength = 160): string {
  const text = source
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/`([^`]*)`/g, "$1")
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[#>*_~|-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  return text.length > maxLength ? `${text.slice(0, maxLength)}…` : text;
}
