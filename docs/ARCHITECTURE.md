# 架构说明

## 分层

```
┌──────────────────────────────────────────────────────────────┐
│ 前端（WebView）                                                │
│  pet.html  →  canvas 逐帧播放图集，只负责画与报告指针意图         │
│  chat.html →  Preact 聊天 + 设置界面，只负责展示与调用命令         │
└───────────────────────────┬──────────────────────────────────┘
                            │ Tauri IPC（命令 + 事件）
┌───────────────────────────▼──────────────────────────────────┐
│ src-tauri（应用壳，Rust）                                       │
│  state.rs         AppState：配置、宠物库、人格、密钥、记忆、运行时 │
│  window/          宠物窗几何、像素级穿透、自动行走                 │
│  chat.rs          会话运行时：流式转发、持久化、摘要触发            │
│  agent_bridge.rs  本地状态协议 → 宠物状态机                       │
│  tts.rs           系统语音播放与中断                             │
│  tray.rs          托盘菜单                                     │
│  commands.rs      IPC 命令面                                    │
└───────────────────────────┬──────────────────────────────────┘
                            │ 纯 Rust 调用
┌───────────────────────────▼──────────────────────────────────┐
│ bytepet-core（无 UI 依赖，可独立测试）                               │
│  pet/      清单解析、图集解码、几何校验、alpha 掩码、库扫描、状态机  │
│  llm/      5 种通道 + SSE 解析                                  │
│  chat/     提示词拼装、回合执行、摘要生成                          │
│  persona/  人格模型与存储                                       │
│  memory/   SQLite + FTS5 记忆                                  │
│  agent/    HTTP/WS 协议 + hook 安装器                            │
│  config.rs / secrets.rs                                        │
└──────────────────────────────────────────────────────────────┘
```

设计原则：

1. **Rust 拥有全部业务状态**，前端无业务逻辑，因此换 UI 不影响正确性；
2. **bytepet-core 不依赖 Tauri**，可以用 `cargo test -p bytepet-core` 秒级验证；
3. **所有平台差异集中在 `src-tauri/window/`**，核心逻辑保持跨平台。

## 一次对话的数据流

```
用户输入
  → send_message(conversationId, text)
  → chat.rs 查人格 / 模型服务 / 记忆配置
  → memory.append_message(user)
  → memory.build_context(...)          最近若干轮 + 摘要 + 事实 + FTS 召回
  → persona.effective_system_prompt()  人格 + 语气 + 宠物上下文
  → provider.stream(request, tx, cancel)
       → ChatDelta 逐条 emit 到前端（text / reasoning / status）
  → 成功：memory.append_message(assistant)，emit chat://done
     若未摘要轮次超阈值 → 后台 summarize_conversation()
  → 失败：emit chat://error（消息已脱敏），不落库半截回答
```

宠物状态同步：开始生成 → `running`；结束 → `review`（3 秒）；出错 → `failed`；取消 → `idle`。

## 像素级点击穿透

1. Rust 解码图集时按 1/4 降采样生成每帧 alpha 掩码（`AlphaMask`）；
2. 前端每帧把当前 sprite 索引通过 `pet_sprite_index` 回报；
3. 后台线程 30Hz 读取全局光标位置，换算成窗口逻辑坐标，再除以显示缩放得到单元格像素；
4. 命中掩码则 `set_ignore_cursor_events(false)`，否则 `true`，仅在结果变化时调用系统 API。

三种模式：`auto`（像素级，默认）、`rect`（整窗可点）、`passthrough`（完全穿透，用托盘/热键操作）。

## 自动行走

- `running-left` / `running-right` 是**基础状态**，聊天、agent、一次性动作等更高优先级事件会自然打断它；
- 后台线程 60Hz 推进窗口位置，到屏幕边缘反向并休息若干秒；
- 用户拖动后进入宽限期，不自动走动。

## 记忆

```sql
conversations(id, persona_id, title, created_at, updated_at)
messages(id, conversation_id, role, content, provider, model, tokens, created_at)
summaries(id, conversation_id, through_message_id, content, created_at)   -- 每会话一条滚动摘要
facts(id, persona_id, key, value, confidence, source_message_id, updated_at)
messages_fts / facts_fts  -- FTS5 trigram（中文可子串检索）
```

召回顺序：最近 N 轮原文 → 最新摘要 → 事实 → FTS 相关片段，总预算约 2000 token，超出按顺序裁剪。

## 安全

- API Key 只进系统钥匙串，配置里只有引用；
- 图集路径做目录穿越校验，zip 导入限制大小与条目数；
- hook 安装器先备份、只动自己的条目、可逐字节还原；
- 日志统一脱敏。

## 线程模型

| 线程/任务 | 频率 | 职责 |
|---|---|---|
| Tauri 主线程 | — | 窗口与 UI 事件 |
| hit-test 线程 | 30Hz | 光标命中与穿透切换 |
| walk 线程 | 60Hz | 自动行走 |
| tokio 运行时 | 事件驱动 | 模型流式、agent 服务、记忆读写 |
| TTS 监听线程 | 8Hz | 等待语音播完并上报状态 |
