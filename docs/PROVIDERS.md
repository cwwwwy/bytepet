# 模型通道

五种通道共用同一个接口：

```rust
#[async_trait]
pub trait ChatProvider: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> ProviderKind;
    async fn stream(
        &self,
        request: ChatRequest,
        tx: mpsc::Sender<ChatDelta>,
        cancel: CancellationToken,
    ) -> Result<()>;
    async fn probe(&self) -> Result<()>;   // 「测试」按钮
}
```

`ChatDelta` 只有五种：`text`、`reasoning`、`status`、`usage`、`done`。前端不关心协议差异。

## 1. Anthropic（Claude API）

- `POST {base_url}/v1/messages`
- Header：`x-api-key`、`anthropic-version: 2023-06-01`
- 系统提示走独立 `system` 字段；`max_tokens` 必填（未设置时用 1024）
- 解析 SSE：`message_start`（输入 token）、`content_block_delta`（`text_delta` / `thinking_delta`）、`message_delta`（输出 token、`stop_reason`）、`message_stop`

## 2. OpenAI 兼容 `/chat/completions`

适用：DeepSeek、OpenAI、OpenRouter、Together、Ollama、LM Studio、任何兼容网关。

- `POST {base_url}/chat/completions`，`Authorization: Bearer <key>`
- `stream: true` + `stream_options.include_usage`
- `delta.content` → 正文；`delta.reasoning_content`（DeepSeek）或 `delta.reasoning` → 思考过程
- 处理末尾的 `data: [DONE]`

## 3. OpenAI `/responses`

适用：OpenAI 新接口，以及 **Codex CLI 自定义 provider 的 `wire_api = "responses"`**（本机 `~/.codex/config.toml` 就是这样把 Codex 指向 DeepSeek 的）。

- `POST {base_url}/responses`，`Authorization: Bearer <key>`
- 请求体：`instructions`（系统提示）、`input: [{role, content:[{type, text}]}]`、`max_output_tokens`
- 解析 SSE：`response.output_text.delta`、`response.reasoning_summary_text.delta`、`response.completed`（用量），并用 `response.output_item.done` 兜底防止丢字

## 4. Codex CLI（本地复用，无需 Key）

- 命令：`codex exec --json --model <model> [--sandbox <mode>] <extra args> -`，提示词从 stdin 传入
- 续聊：`codex exec resume <session_id> ...`
- 逐行解析 JSONL：会话开始、`item.started/updated/completed`（`agent_message` / `reasoning` / `command_execution` / `mcp_tool_call` / `file_change`）、`turn.completed`
- 未知事件类型会被忽略而不是报错，避免 CLI 升级后直接崩
- 取消时立即结束子进程；输出有上限保护
- 默认使用最保守的沙箱模式（`read-only`），可在服务配置里改

> 注意：CLI 通道会启动本机 Codex agent，它会按自身配置读取工作目录。请确认这是你想要的。

## 5. Claude Code CLI（本地复用，无需 Key）

- 命令：`claude -p --output-format stream-json --include-partial-messages --verbose [--model] [--permission-mode]`，提示词从 stdin 传入
- 解析 JSONL：`system/init`（会话 id）、`stream_event`（内含 Anthropic 事件，取增量文本/思考）、`assistant`（整条兜底）、`result`（用量与结束）
- 续聊：`--resume <session_id>`

## 服务配置

```json
{
  "id": "deepseek",
  "label": "DeepSeek",
  "kind": "openai-responses",
  "baseUrl": "https://api.deepseek.com",
  "model": "deepseek-chat",
  "apiKeyRef": "provider/deepseek",
  "extraHeaders": {},
  "enabled": true,
  "options": {}
}
```

> CLI 通道（`codex-cli` / `claude-cli`）的 `model` 可以留空：留空时不传 `--model`，直接使用 CLI 自身的默认模型。例如 Codex 会采用 `~/.codex/config.toml` 里的 `model`，因此把 Codex 指向 DeepSeek 的用户无需在这里再填一次。

CLI 通道额外支持 `options`：

```json
{
  "binary": "/usr/local/bin/codex",
  "sandbox": "read-only",
  "permissionMode": "default",
  "extraArgs": ["-c", "model_reasoning_effort=high"],
  "timeoutSecs": 300
}
```

## 密钥

- 只写入 macOS 钥匙串 / Windows 凭据管理器；配置文件里只有 `provider/<id>` 引用；
- 钥匙串不可用时回退到 `secrets.json`（Unix 下 0600 权限）并在界面显著提示；
- 日志与错误信息会统一脱敏（`sk-`、`Bearer`、`x-api-key` 一律替换为 `***`）。

## 错误处理

- 非 2xx：读取响应体并给出「状态码 + 截断后的错误文本」，绝不回显密钥；
- 429 / 5xx：指数退避重试（最多 3 次，尊重 `Retry-After`）；
- 连接超时 20s，整体超时 300s，空闲超时 120s；
- 取消：`CancellationToken` 逐会话生效，HTTP 请求与子进程都会立即终止。

## 人格如何选择模型

人格里的 `model: {provider, model}` 优先；未设置时用设置里的默认服务。`model` 为空则使用服务自身的默认模型。
