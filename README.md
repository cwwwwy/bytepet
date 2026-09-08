# 桌宠 · Pet

一款用 Rust 开发的跨平台桌面宠物（Windows / macOS）。它兼容 **Codex 桌宠图集格式**，能直接使用你 `~/.codex/pets/` 里已有的宠物；同时可以接入 Claude、OpenAI、DeepSeek 等大模型，也能直接复用本机已登录的 `codex` / `claude` 命令行，并支持完全自定义人格。

```
┌──────────────────────────────┐        ┌─────────────────────────┐
│ 透明置顶宠物窗（WebView）      │        │ 聊天 / 设置窗（Preact）  │
│ canvas 逐帧播放 Codex 图集    │        │ 流式对话 · 人格 · 模型    │
└───────────────┬──────────────┘        └────────────┬────────────┘
                │  Tauri IPC / events                │
┌───────────────▼────────────────────────────────────▼────────────┐
│ src-tauri（Rust 应用壳）                                          │
│ 窗口 · 托盘 · 像素级点击穿透 · 自动行走 · TTS · IPC 命令层          │
└───────────────┬─────────────────────────────────────────────────┘
                │
┌───────────────▼─────────────────────────────────────────────────┐
│ pet-core（纯 Rust 核心，无 UI 依赖）                              │
│ 宠物图集/校验/状态机 · 5 种模型通道 · 人格 · 记忆 · Agent 状态协议   │
└─────────────────────────────────────────────────────────────────┘
```

## 功能

- **Codex 桌宠兼容**：直接读取并渲染 `~/.codex/pets/<id>/pet.json` + `spritesheet.webp`，支持官方 8×9（1536×1872）与 V2 8×11（1536×2288，含注视行）图集，并兼容 UniPet 的 `frame`/`animations` 扩展字段。导入/导出 `.zip` 包可直接分享或上传。
- **透明悬浮 + 像素级点击穿透**：窗口无边框、置顶、可拖动；透明区域点击会穿透到下层应用，只有宠物实际画出来的像素才响应鼠标。
- **流式对话**：Markdown / 代码块渲染、中文输入法可用、随时中断、Token 用量显示。
- **五种模型通道**：
  - Anthropic Messages API（Claude）
  - OpenAI 兼容 `/chat/completions`（DeepSeek、OpenRouter、Ollama…）
  - OpenAI `/responses`（Codex CLI 自定义 provider 常用的 `wire_api = "responses"`）
  - 本地 `codex exec --json`（复用你已有的 Codex 登录，无需 API Key）
  - 本地 `claude -p --output-format stream-json`（复用 Claude Code 登录）
- **自定义人格**：系统提示 + 语气/长度/语言 + 采样参数 + 绑定宠物皮肤 + 绑定模型 + 独立记忆与 TTS 设置；内置 3 个模板，可导入导出。
- **长期记忆**：SQLite + FTS5（trigram，中文可检索），滚动摘要 + 事实记忆 + 相关片段召回，全部在本地。
- **Agent 状态联动**：本地 HTTP/WebSocket 协议 + Codex / Claude Code hook 安装器，让宠物跟随 AI 的工作状态（思考 / 等待 / 失败 / 完成）。协议与 UniPet 字段兼容。
- **自动行走**：宠物在屏幕内散步、到边缘转身，聊天或 agent 工作时自动停下。
- **TTS 朗读**：使用系统语音（macOS `say` / Windows SAPI / Linux `spd-say`），可中断、可换音色语速。
- **托盘 / 单实例 / 开机自启**。

## 快速开始

前置：Rust ≥ 1.85、Node ≥ 20、pnpm。

```bash
pnpm install
pnpm tauri dev          # 开发模式（宠物窗 + 托盘 + 聊天窗）
pnpm tauri build        # 打包 .app/.dmg（macOS）或 .msi/.exe（Windows）
```

首次启动会自动扫描 `~/.codex/pets` 与 `~/.unipet/pets`，把找到的第一只宠物显示出来。在托盘菜单或聊天窗里选择宠物、配置模型、创建人格。

### 配置模型

在「设置 → 模型服务」里新增一个服务：

| 通道 | base_url | 模型 | 需要 Key |
|---|---|---|---|
| Anthropic | `https://api.anthropic.com` | `claude-sonnet-4-5` 等 | 是（存系统钥匙串） |
| OpenAI 兼容 | `https://api.deepseek.com` | `deepseek-chat` 等 | 是 |
| OpenAI Responses | `https://api.openai.com/v1` | `gpt-5` 等 | 是 |
| Codex CLI | — | 留空或 `codex` 默认模型 | 否（用本地登录） |
| Claude Code CLI | — | 留空 | 否（用本地登录） |

API Key 只写入 macOS 钥匙串 / Windows 凭据管理器，配置文件里只保存引用。

### 让宠物跟随 Codex / Claude Code

```bash
pet hooks install all      # 安装 hook（会先备份配置，可完全回滚）
pet hooks status
pet hooks uninstall all
```

也可以从任何脚本或 agent 直接调用本地协议：

```bash
curl -XPOST http://127.0.0.1:17872/state \
  -H 'content-type: application/json' \
  -d '{"source":"my-script","state":"running","message":"跑测试中","ttlMs":120000}'
```

## 仓库结构

```
crates/pet-core/    纯 Rust 核心：宠物格式、状态机、模型通道、人格、记忆、协议
crates/pet-cli/     pet 命令行（state / doctor / pet / hooks）
src-tauri/          Tauri 应用：窗口、托盘、穿透、行走、TTS、IPC
src/pet/            宠物窗渲染器（canvas，逐帧）
src/chat/           聊天与设置界面（Preact）
docs/               格式、协议、模型通道与架构文档
```

## 开发

```bash
cargo test --workspace            # Rust 测试
cargo clippy --workspace --all-targets -- -D warnings
pnpm build                        # 前端类型检查 + 打包
pnpm test                         # 前端测试
```

如果本机 `~/.cargo` 不可写（例如受限沙箱），把 `CARGO_HOME` 指向仓库内目录：

```bash
export CARGO_HOME="$PWD/.cache/cargo"
```

日志：设置环境变量 `PET_LOG=debug` 后启动。

## 文档

- [宠物格式与兼容性](docs/PET_FORMAT.md)
- [本地状态协议](docs/PROTOCOL.md)
- [模型通道](docs/PROVIDERS.md)
- [架构说明](docs/ARCHITECTURE.md)

## 已知限制

- macOS 透明窗口依赖 `macOSPrivateApi`，因此**不适合上架 Mac App Store**，请走直接分发。
- CLI 通道会启动本机 `codex` / `claude` 进程，默认使用最保守的沙箱与权限模式；请自行确认其行为符合预期。
- v1 只支持单只宠物同屏；多宠物与在线宠物市场在后续版本。
- Linux 目前未做验证。

## 验证状态（macOS 26.6.2 / Rust 1.98 / Codex CLI 0.151.0 / Claude Code 2.1.251）

| 项目 | 结果 |
|---|---|
| `cargo test --workspace` | 125 项全部通过（pet-core 88 + 集成 29 + CLI 4 + 示例） |
| `pnpm build` / `pnpm test` | 通过（前端 53 项测试） |
| 真实 Codex 宠物渲染 | `~/.codex/pets/zip`（1536×1872）逐帧播放，帧时长与官方表一致 |
| 打招呼 → 回落 | waving 行（sprite 24–27）播放后回到 idle |
| 自动行走 | 窗口在屏幕内往返、边缘转身，左右跑行（8–15 / 16–23）正确切换 |
| 本地状态协议 | `GET /health`、`POST /state` 正常，健康快照含当前宠物与状态 |
| Codex CLI 通道 | 真实会话跑通（`你好世界`，含 session id 与用量） |
| Claude CLI 通道 | 真实会话跑通（含 reasoning/text 增量与用量） |
| HTTP 通道 | 三种协议用录制 SSE 回放做集成测试（含取消与 401 脱敏） |
| 密钥存储 | 已确认写入 macOS 钥匙串（`MacCredential`） |

尚未在本机自动化验证、建议手动确认：像素级点击穿透手感、托盘菜单点击、聊天窗中文输入法、系统语音试听。

## 许可

MIT
