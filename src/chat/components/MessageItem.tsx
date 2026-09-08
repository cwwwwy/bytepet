/**
 * One chat bubble. Assistant/user content is Markdown rendered through the
 * sanitizing pipeline; links are handed to the OS browser and code blocks get a
 * delegated copy button. The streaming variant appends a blinking caret.
 */

import { useMemo } from "preact/hooks";
import { openUrl } from "@tauri-apps/plugin-opener";
import { renderMarkdown } from "../../shared/markdown";
import { copyText, estimateTokens, formatTime, formatTokens } from "../../shared/format";
import { t } from "../../shared/i18n";
import type { ChatMessageView } from "../../shared/types";
import { IconSparkles } from "./icons";

export interface MessageUsage {
  inputTokens: number;
  outputTokens: number;
}

export interface MessageItemProps {
  role: ChatMessageView["role"];
  content: string;
  createdAt?: number;
  tokens?: number;
  model?: string | null;
  provider?: string | null;
  streaming?: boolean;
  reasoning?: string | null;
  reasoningOpen?: boolean;
  status?: string | null;
  usage?: MessageUsage | null;
  cancelled?: boolean;
}

function handleMarkdownClick(event: MouseEvent): void {
  const target = event.target as HTMLElement | null;
  if (!target) return;

  const copyButton = target.closest<HTMLElement>(".md-copy");
  if (copyButton) {
    event.preventDefault();
    const code = copyButton.closest(".md-code")?.querySelector("code")?.textContent ?? "";
    void copyText(code).then((ok) => {
      if (!ok) return;
      copyButton.classList.add("copied");
      const label = copyButton.querySelector(".md-copy-label");
      const previous = label?.textContent ?? "";
      if (label) label.textContent = t("common.copied");
      window.setTimeout(() => {
        copyButton.classList.remove("copied");
        if (label) label.textContent = previous;
      }, 1400);
    });
    return;
  }

  const anchor = target.closest<HTMLAnchorElement>("a[href]");
  if (anchor) {
    const href = anchor.getAttribute("href") ?? "";
    event.preventDefault();
    if (/^(https?|mailto):/i.test(href)) void openUrl(href);
  }
}

export function MessageItem(props: MessageItemProps) {
  const { role, content, streaming } = props;
  const html = useMemo(() => renderMarkdown(content), [content]);
  const estimated = streaming ? estimateTokens(content) : 0;

  if (role === "system") {
    return (
      <div class="msg msg-system">
        <div class="msg-system-note">{content}</div>
      </div>
    );
  }

  const isUser = role === "user";
  const usage = props.usage;
  const showUsage = Boolean(usage && (usage.inputTokens > 0 || usage.outputTokens > 0));

  return (
    <div class={isUser ? "msg msg-user" : "msg msg-assistant"}>
      {!isUser && props.reasoning ? (
        <details class="reasoning" open={props.reasoningOpen}>
          <summary>
            <IconSparkles size={13} />
            <span>{t("chat.reasoning")}</span>
          </summary>
          <div class="reasoning-body">{props.reasoning}</div>
        </details>
      ) : null}

      {!isUser && streaming && props.status ? (
        <div class="msg-meta" style="margin-bottom:4px">
          <span>{props.status}</span>
        </div>
      ) : null}

      <div class={isUser ? "bubble bubble-user" : "bubble bubble-assistant"}>
        <div class="md" onClick={handleMarkdownClick} dangerouslySetInnerHTML={{ __html: html }} />
        {streaming ? <span class="caret" aria-hidden="true" /> : null}
      </div>

      <div class="msg-meta">
        {isUser ? <span>{t("chat.you")}</span> : <span>{t("chat.assistant")}</span>}
        {props.createdAt ? (
          <>
            <span class="dot-sep" />
            <span>{formatTime(props.createdAt)}</span>
          </>
        ) : null}
        {!isUser && props.model ? (
          <>
            <span class="dot-sep" />
            <span class="truncate" style="max-width:200px">
              {props.model}
            </span>
          </>
        ) : null}
        {streaming ? (
          <>
            <span class="dot-sep" />
            <span>{t("chat.estimatedTokens", { n: formatTokens(estimated) })}</span>
          </>
        ) : null}
        {!streaming && showUsage && usage ? (
          <>
            <span class="dot-sep" />
            <span>
              {t("chat.tokensIn")} {formatTokens(usage.inputTokens)} · {t("chat.tokensOut")}{" "}
              {formatTokens(usage.outputTokens)}
            </span>
          </>
        ) : null}
        {!streaming && !showUsage && !isUser && props.tokens ? (
          <>
            <span class="dot-sep" />
            <span>{formatTokens(props.tokens)} tokens</span>
          </>
        ) : null}
        {props.cancelled ? (
          <>
            <span class="dot-sep" />
            <span>{t("chat.cancelled")}</span>
          </>
        ) : null}
      </div>
    </div>
  );
}
