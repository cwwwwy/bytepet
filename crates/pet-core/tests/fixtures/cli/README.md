# CLI JSONL fixtures

## `claude-stream.jsonl` — REAL capture

Produced on this machine with Claude Code 2.1.251:

```sh
cd /tmp && echo "回复：你好" | claude -p --output-format stream-json --include-partial-messages --verbose
```

Observed event shapes (one JSON object per line on stdout):

- `{"type":"system","subtype":"init","session_id":"<uuid>",...}` — session id,
  model, tools. Other `system` subtypes seen: `status`, `thinking_tokens`.
- `{"type":"stream_event","event":{...},"session_id":...,"parent_tool_use_id":null,"uuid":...}`
  where `event` is a raw Anthropic SSE payload:
  `message_start` (usage.input_tokens), `content_block_start`
  (`{"type":"thinking"}` / `{"type":"text"}`), `content_block_delta`
  (`thinking_delta.thinking`, `text_delta.text`, `signature_delta.signature`),
  `content_block_stop`, `message_delta` (`delta.stop_reason`, `usage.output_tokens`),
  `message_stop`.
- `{"type":"assistant","message":{"content":[{"type":"thinking",...}]}}` and a
  second one with `{"type":"text","text":"..."}` — full accumulated content,
  i.e. a duplicate of what the deltas already streamed.
- `{"type":"result","subtype":"success","result":"<final text>","stop_reason":"end_turn","usage":{...},"is_error":false,...}`
  — final line; repeats the text again.

## `codex-exec.jsonl` — SYNTHETIC (capture blocked)

Codex CLI 0.151.0 is installed and configured, but `codex exec` cannot start
inside the harness sandbox on this machine. Every invocation (with or without
`--sandbox read-only`, with a workspace-local `CODEX_HOME`/`TMPDIR`, with
`--dangerously-bypass-approvals-and-sandbox`) ends with:

```
Error: failed to initialize in-process app-server client: Operation not permitted (os error 1)
```

The root cause is that the harness sandbox denies nested seatbelt sandboxes
(`sandbox-exec -p '(version 1)(allow default)' /bin/echo` also fails with
`sandbox_apply: Operation not permitted`), and Codex applies one during app
server initialization.

The fixture therefore follows the documented `codex exec --json` contract
(one thread event per line) plus the event/type literals present in the 0.151.0
binary (`thread.started`, `turn.started`, `turn.completed`, `turn.failed`,
`item.started`, `item.updated`, `item.completed`, `agent_message`,
`agent_reasoning`, `command_execution`, `mcp_tool_call`, `file_change`,
`web_search`). The first line is a `#` comment: the provider parser ignores
non-JSON lines on purpose, which also doubles as a tolerance test.
