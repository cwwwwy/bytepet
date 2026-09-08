# 本地状态协议

桌宠在 `127.0.0.1`（默认端口 `17872`，可在设置中修改）上暴露一个极小的 HTTP + WebSocket 服务，任何脚本、hook、插件都可以驱动它。协议字段与 UniPet 保持一致。

## 事件结构

```json
{
  "source": "codex",
  "state": "running",
  "message": "正在跑测试",
  "action": "test",
  "ttlMs": 120000
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `source` | 是 | 事件来源，≤ 64 字符，用于按来源清理 |
| `state` | 是 | 状态名，见下表 |
| `message` | 否 | 气泡文字，≤ 500 字符 |
| `action` | 否 | 附带动作标识，界面可用于展示 |
| `ttlMs` | 否 | 存活毫秒数；`0` 表示不过期；缺省用设置里的默认值（120s） |

## 状态名

| 状态 | 含义 | 对应行 |
|---|---|---|
| `idle` | 空闲 | 0 |
| `running` / `working` / `thinking` | 工作中 | 7 |
| `waiting` / `blocked` | 等待用户输入或确认 | 6 |
| `failed` / `error` | 出错 | 5 |
| `review` / `success` / `done` | 完成待检视 | 8 |
| `waving` / `hello` | 打招呼（一次性） | 3 |
| `jumping` | 悬停创意动作（一次性） | 4 |
| `running-left` / `running-right` | 自动行走 | 2 / 1 |
| `look-row-9` / `look-row-10` | 注视（V2 图集） | 9 / 10 |

## HTTP

### `POST /state`

```bash
curl -XPOST http://127.0.0.1:17872/state \
  -H 'content-type: application/json' \
  -d '{"source":"my-script","state":"waiting","message":"需要你确认","ttlMs":300000}'
```

- `202 {"ok":true}` 接受；
- `400 {"ok":false,"error":"..."}` 字段非法（例如未知状态）；
- 请求体上限 16 KiB。

### `GET /health`

```json
{
  "ok": true,
  "version": "0.1.0",
  "pet": "zip",
  "persona": "default",
  "state": "running",
  "sources": []
}
```

### `GET /pets`

返回当前宠物库的 id 列表（便于脚本探测）。

## WebSocket

`ws://127.0.0.1:17872/ws`

- 连接后先收到一条 `{"type":"health", ...}` 快照；
- 之后每个被接受的事件都会广播为 `{"type":"state", ...}`；
- 也可以向服务端发送同样结构的事件，效果等同 `POST /state`。

## 生命周期事件名

hook 里使用的事件名会被映射到状态（`AgentEvent::state_for_lifecycle`）：

| 事件 | 状态 |
|---|---|
| `SessionStart` | `waving` |
| `UserPromptSubmit` | `running` |
| `PreToolUse` | `running` |
| `PostToolUse` | `running` |
| `Notification` | `waiting` |
| `Stop` / `SessionEnd` / `turn-ended` | `review` |
| `Error` | `failed` |

## 内置安装器

```bash
pet hooks install codex         # 包装 ~/.codex/config.toml 的 notify
pet hooks install claude-code   # 合并 ~/.claude/settings.json 的 hooks
pet hooks install all
pet hooks status
pet hooks uninstall all
```

安装器规则：

1. 先备份原配置（`*.pet-backup`）；
2. 只增删属于自己的条目，绝不覆盖其它工具；
3. Codex 的 `notify` 只能有一个命令，因此会生成**链式包装脚本**：先调用你原来的 notify，再把事件转发给桌宠；
4. 卸载时按记录精确还原，可与备份逐字节比对。

## CLI

```bash
pet state running "跑测试中" --source my-script --ttl 120s
pet state waiting --source my-script
pet clear --source my-script
pet doctor
```

`pet doctor` 会检查：数据目录、宠物库、本地服务可达性、hook 安装状态、当前宠物是否有效，并给出下一步建议。
