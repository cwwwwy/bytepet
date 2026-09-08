/**
 * Chat pane: message history + streaming turn.
 *
 * Streaming events are coalesced into one state update per animation frame so a
 * fast provider cannot thrash the renderer. Auto-scroll only follows the bottom
 * while the user is already there; otherwise a "jump to latest" pill appears.
 */

import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import type { UnlistenFn } from "@tauri-apps/api/event";
import { events, ipc } from "../../shared/ipc";
import { t } from "../../shared/i18n";
import type { AppConfig, ChatMessageView, Persona } from "../../shared/types";
import { MessageItem, type MessageUsage } from "./MessageItem";
import { Composer } from "./Composer";
import { toast } from "./Toast";
import { IconAlert, IconArrowDown, IconClose } from "./icons";

interface StreamState {
  conversationId: string;
  text: string;
  reasoning: string;
  status: string | null;
  startedAt: number;
}

interface BufferedDelta {
  text: string;
  reasoning: string;
  status: string | null;
}

export interface ChatPaneProps {
  conversationId: string | null;
  persona: Persona | null;
  config: AppConfig;
  /** Create (or reuse) a conversation and make it active. */
  onRequestConversation: () => Promise<string | null>;
  /** Conversation list needs a refresh (new message, title change). */
  onConversationTouched: (conversationId: string) => void;
}

export function ChatPane(props: ChatPaneProps) {
  const { conversationId, persona, config } = props;

  const [messages, setMessages] = useState<ChatMessageView[]>([]);
  const [draft, setDraft] = useState("");
  const [stream, setStream] = useState<StreamState | null>(null);
  const [usage, setUsage] = useState<MessageUsage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [atBottom, setAtBottom] = useState(true);

  const scrollRef = useRef<HTMLDivElement>(null);
  const conversationRef = useRef<string | null>(conversationId);
  const streamRef = useRef<StreamState | null>(null);
  const atBottomRef = useRef(true);
  const busyConversationRef = useRef<string | null>(null);
  const creatingRef = useRef(false);
  const bufferRef = useRef<BufferedDelta>({ text: "", reasoning: "", status: null });
  const rafRef = useRef(0);
  const lastAttemptRef = useRef<{ conversationId: string; text: string } | null>(null);
  const propsRef = useRef(props);
  const personaRef = useRef(persona);

  conversationRef.current = conversationId;
  streamRef.current = stream;
  propsRef.current = props;
  personaRef.current = persona;

  const scrollToBottom = useCallback((smooth: boolean) => {
    const element = scrollRef.current;
    if (!element) return;
    element.scrollTo({ top: element.scrollHeight, behavior: smooth ? "smooth" : "auto" });
    atBottomRef.current = true;
    setAtBottom(true);
  }, []);

  const flush = useCallback(() => {
    rafRef.current = 0;
    const buffered = bufferRef.current;
    bufferRef.current = { text: "", reasoning: "", status: null };
    if (!buffered.text && !buffered.reasoning && buffered.status === null) return;
    setStream((previous) => {
      if (!previous) return previous;
      return {
        ...previous,
        text: previous.text + buffered.text,
        reasoning: previous.reasoning + buffered.reasoning,
        status: buffered.status ?? previous.status,
      };
    });
  }, []);

  const scheduleFlush = useCallback(() => {
    if (rafRef.current !== 0) return;
    rafRef.current = window.requestAnimationFrame(flush);
  }, [flush]);

  const appendAssistant = useCallback((message: ChatMessageView) => {
    setMessages((previous) => {
      const withoutLocalDuplicates = previous.filter(
        (item) => !(item.id < 0 && item.role === "assistant" && item.content === message.content),
      );
      if (withoutLocalDuplicates.some((item) => item.id === message.id)) return withoutLocalDuplicates;
      return [...withoutLocalDuplicates, message];
    });
  }, []);

  // --- load history --------------------------------------------------------
  useEffect(() => {
    if (!conversationId) {
      setMessages([]);
      setStream(null);
      setUsage(null);
      setError(null);
      setLoading(false);
      return;
    }
    // A live turn owns this conversation's in-memory list.
    if (busyConversationRef.current === conversationId || creatingRef.current) return;
    const requested = conversationId;
    let cancelled = false;
    setLoading(true);
    setError(null);
    setUsage(null);
    setStream(null);
    void ipc.getMessages(requested, 400).then((result) => {
      if (cancelled || conversationRef.current !== requested) return;
      setLoading(false);
      if (!result.ok) {
        setError(result.error);
        return;
      }
      setMessages(result.value);
      atBottomRef.current = true;
      setAtBottom(true);
    });
    return () => {
      cancelled = true;
    };
  }, [conversationId]);

  // --- event wiring --------------------------------------------------------
  useEffect(() => {
    let disposed = false;
    let unlisten: UnlistenFn[] = [];

    const handleDelta = (payload: { conversationId: string; kind: string; text: string }) => {
      if (payload.conversationId !== conversationRef.current) return;
      if (payload.kind === "reasoning") bufferRef.current.reasoning += payload.text;
      else if (payload.kind === "status") bufferRef.current.status = payload.text;
      else bufferRef.current.text += payload.text;
      scheduleFlush();
    };

    const handleDone = (payload: {
      conversationId: string;
      messageId: number;
      content: string;
      inputTokens: number;
      outputTokens: number;
      finishReason: string | null;
    }) => {
      if (payload.conversationId !== conversationRef.current) return;
      if (rafRef.current !== 0) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = 0;
      }
      bufferRef.current = { text: "", reasoning: "", status: null };
      busyConversationRef.current = null;
      creatingRef.current = false;
      setStream(null);
      setUsage({ inputTokens: payload.inputTokens, outputTokens: payload.outputTokens });
      lastAttemptRef.current = null;
      appendAssistant({
        id: payload.messageId,
        role: "assistant",
        content: payload.content,
        provider: null,
        model: personaRef.current?.model?.model ?? null,
        tokens: payload.outputTokens,
        createdAt: Date.now(),
      });
      propsRef.current.onConversationTouched(payload.conversationId);
      void ipc.getMessages(payload.conversationId, 400).then((result) => {
        if (!result.ok) return;
        if (conversationRef.current !== payload.conversationId) return;
        if (result.value.some((message) => message.id === payload.messageId)) {
          setMessages(result.value);
        }
      });
    };

    const handleError = (payload: { conversationId: string; message: string }) => {
      if (payload.conversationId !== conversationRef.current) return;
      if (rafRef.current !== 0) {
        window.cancelAnimationFrame(rafRef.current);
        rafRef.current = 0;
      }
      bufferRef.current = { text: "", reasoning: "", status: null };
      busyConversationRef.current = null;
      creatingRef.current = false;
      setStream(null);
      setError(payload.message);
    };

    void (async () => {
      const offs = await Promise.all([
        events.onChatDelta(handleDelta),
        events.onChatDone(handleDone),
        events.onChatError(handleError),
      ]);
      if (disposed) {
        for (const off of offs) off();
      } else {
        unlisten = offs;
      }
    })().catch(() => {
      /* event bridge unavailable (e.g. plain browser dev server) */
    });

    return () => {
      disposed = true;
      if (rafRef.current !== 0) window.cancelAnimationFrame(rafRef.current);
      for (const off of unlisten) off();
    };
  }, []);

  // --- keep the viewport pinned to the bottom ------------------------------
  const streamLength = stream ? stream.text.length + stream.reasoning.length : 0;
  useEffect(() => {
    if (atBottomRef.current) {
      const element = scrollRef.current;
      if (element) element.scrollTop = element.scrollHeight;
    }
  }, [messages.length, streamLength, loading]);

  useEffect(() => {
    if (!conversationId) return;
    atBottomRef.current = true;
    setAtBottom(true);
  }, [conversationId]);

  const onScroll = () => {
    const element = scrollRef.current;
    if (!element) return;
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    const next = distance < 56;
    if (next !== atBottomRef.current) {
      atBottomRef.current = next;
      setAtBottom(next);
    }
  };

  // --- sending -------------------------------------------------------------
  const send = useCallback(
    async (rawText: string) => {
      const text = rawText.trim();
      if (!text) return;
      if (!persona) {
        toast.error(t("chat.emptyNoPersona"));
        return;
      }

      let target = conversationRef.current;
      if (!target) {
        creatingRef.current = true;
        target = await props.onRequestConversation();
        creatingRef.current = false;
        if (!target) return;
      }

      const conversation = target;
      busyConversationRef.current = conversation;
      const optimistic: ChatMessageView = {
        id: -Date.now(),
        role: "user",
        content: text,
        provider: null,
        model: null,
        tokens: 0,
        createdAt: Date.now(),
      };
      setMessages((previous) => [...previous, optimistic]);
      setDraft("");
      setError(null);
      setUsage(null);
      atBottomRef.current = true;
      setAtBottom(true);
      lastAttemptRef.current = { conversationId: conversation, text };
      setStream({
        conversationId: conversation,
        text: "",
        reasoning: "",
        status: null,
        startedAt: Date.now(),
      });

      const result = await ipc.sendMessage(conversation, text);
      if (!result.ok) {
        busyConversationRef.current = null;
        setStream(null);
        setMessages((previous) => previous.filter((message) => message.id !== optimistic.id));
        setError(result.error);
        return;
      }
      props.onConversationTouched(conversation);
    },
    [persona, props],
  );

  const cancel = useCallback(() => {
    const conversation = streamRef.current?.conversationId ?? conversationRef.current;
    if (!conversation) return;
    void ipc.cancelMessage(conversation).then((result) => {
      if (!result.ok) toast.error(result.error);
    });
    const partial = streamRef.current;
    busyConversationRef.current = null;
    setStream(null);
    if (partial && partial.text.trim()) {
      setMessages((previous) => [
        ...previous,
        {
          id: -Date.now(),
          role: "assistant",
          content: partial.text,
          provider: null,
          model: persona?.model?.model ?? null,
          tokens: 0,
          createdAt: Date.now(),
        },
      ]);
    }
  }, [persona]);

  const retry = () => {
    const attempt = lastAttemptRef.current;
    if (!attempt) return;
    setError(null);
    void send(attempt.text);
  };

  const showEmpty = !loading && messages.length === 0 && !stream;
  const greeting = persona?.greeting?.trim() || t("chat.emptyTitle");
  const composerPlaceholder = persona ? t("chat.placeholder") : t("chat.emptyNoPersona");

  return (
    <div class="chat">
      <div class="chat-body">
        <div class="chat-scroll" ref={scrollRef} onScroll={onScroll}>
          <div
            class="chat-inner"
            role="log"
            aria-live="polite"
            aria-relevant="additions"
            aria-busy={Boolean(stream)}
          >
            {loading ? <div class="small muted" style="text-align:center">{t("common.loading")}</div> : null}

            {showEmpty ? (
              <div class="chat-empty">
                <div class="chat-empty-mark" aria-hidden="true" />
                <h2>{persona ? greeting : t("chat.emptyNoPersona")}</h2>
                {persona?.description ? <p>{persona.description}</p> : null}
                <p>{t("chat.emptyHint")}</p>
              </div>
            ) : null}

            {messages.map((message, index) => (
              <MessageItem
                key={message.id}
                role={message.role}
                content={message.content}
                createdAt={message.createdAt}
                tokens={message.tokens}
                model={message.model}
                provider={message.provider}
                usage={
                  usage && message.role === "assistant" && index === messages.length - 1
                    ? usage
                    : null
                }
              />
            ))}

            {stream ? (
              <MessageItem
                role="assistant"
                content={stream.text}
                streaming
                reasoning={stream.reasoning || null}
                reasoningOpen={config.chat.showReasoning}
                status={stream.status ?? t("chat.thinking")}
                model={persona?.model?.model ?? null}
                usage={usage}
              />
            ) : null}
          </div>
        </div>

        {!atBottom && (messages.length > 0 || stream) ? (
          <button type="button" class="jump-pill" onClick={() => scrollToBottom(true)}>
            <IconArrowDown size={14} />
            {t("chat.jumpToLatest")}
          </button>
        ) : null}
      </div>

      {error ? (
        <div style="padding:0 12px 8px">
          <div class="alert alert-danger" role="alert">
            <IconAlert size={15} />
            <span class="grow">
              <strong>{t("chat.error")}</strong>
              <span class="small" style="display:block;margin-top:2px;overflow-wrap:anywhere">
                {error}
              </span>
            </span>
            {lastAttemptRef.current ? (
              <button type="button" class="btn btn-sm" onClick={retry}>
                {t("common.retry")}
              </button>
            ) : null}
            <button
              type="button"
              class="icon-btn"
              aria-label={t("common.close")}
              onClick={() => setError(null)}
            >
              <IconClose size={13} />
            </button>
          </div>
        </div>
      ) : null}

      <Composer
        value={draft}
        onInput={setDraft}
        onSend={() => void send(draft)}
        onCancel={cancel}
        streaming={Boolean(stream)}
        sendOnEnter={config.chat.sendOnEnter}
        disabled={!persona}
        placeholder={composerPlaceholder}
      />
    </div>
  );
}
