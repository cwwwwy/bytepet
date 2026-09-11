# macOS 实机验证清单

这份清单用来在 macOS 上人工验收 BytePet。CI 的 macOS job 只跑 `fmt` / `clippy` / `test`，
只能证明“能编译”，不能证明“能用”；真正的结论以这份清单为准。

当前已知状态（2026-09-11）：

- ✅ 窗口显示、托盘、设置窗口、宠物库、状态协议、持久化等**基础功能**理论上可用，待实测。
- ⛔ 点击、双击、右键、拖拽、转头、点击穿透、no-activate **尚未实现**：`platform.rs` 的
  非 Windows 实现全部返回 `None` / `false`，`app.rs` 的交互函数会直接跳过。
- ⚠️ 空闲重绘目前是 ~60 FPS（`app.rs:2132`），Activity Monitor 里 CPU 可能不为 0。
- ⛔ 单实例、开机自启、文件日志、`.app` 打包尚未做。

最小可用判定：**B1（左键）、B3（拖拽）、B5（转头）、B6（穿透）、B11（空闲 CPU）全部通过**，
macOS 才算真正可用。

## 0. 准备

```bash
# 只需一次
xcode-select --install

# 在仓库根目录
cargo test --workspace
cargo run -p bytepet-app
```

- 用独立数据目录，避免污染真实数据：

```bash
BYTEPET_HOME="$HOME/.bytepet-mac-test" cargo run -p bytepet-app
```

- 不设置 `BYTEPET_HOME` 时，数据在 `~/Library/Application Support/BytePet/`。
- 终端运行才能看到 `tracing` 日志；目前没有文件日志。建议：

```bash
RUST_LOG=debug cargo run -p bytepet-app 2>&1 | tee /tmp/bytepet-mac.log
```

- 当前没有单实例保护：测试时不要同时开多个实例，否则状态协议端口会冲突。

## A. 基础回归（现在应该能通过）

| # | 操作 | 预期结果 | 状态 |
|---|---|---|---|
| A1 | 启动 | 宠物窗口出现、无边框、透明背景（不是黑底/白底）、置顶 | 待实测 |
| A2 | 看菜单栏 | 出现 BytePet 托盘图标 | 待实测 |
| A3 | 点击托盘图标 | 弹出应用自己的菜单窗口：打开设置 / 显示隐藏宠物 / 退出 | 待实测 |
| A4 | 打开设置 | 能打开、滚动；缩放、穿透、状态协议端口、自动行走等控件可操作 | 待实测 |
| A5 | 切换宠物 | 本地库 + `~/.codex/pets` + `~/.unipet/pets` 都能列出；切换后动画和窗口/托盘图标更新 | 待实测 |
| A6 | 导入/导出 | 导入文件夹或 `.zip`（含拖放到窗口）；导出 Codex 上传格式；删除本地副本需确认 | 待实测 |
| A7 | 状态协议 POST | `waiting` / `failed` / `review` / `running` 能切换动画；带 `message` 时显示气泡；`ttlMs` 到期回 base | 待实测 |
| A8 | 状态协议 GET | `GET /health` 返回当前 pet/persona/state；`GET /pets` 返回 id 列表 | 待实测 |
| A9 | 重启持久化 | 当前宠物、人格、缩放等写入 `config.json`，重启后保持 | 待实测 |
| A10 | 托盘隐藏/显示/退出 | 隐藏后窗口消失，托盘可恢复；退出后进程真的结束 | 待实测 |
| A11 | DeepSeek keychain | 设置里保存 API key 后，Keychain 出现 BytePet 条目；重启后仍能读取（无 key 时回落固定问候） | 待实测 |

状态协议命令：

```bash
curl -s http://127.0.0.1:17872/health
curl -s http://127.0.0.1:17872/pets
curl -XPOST http://127.0.0.1:17872/state \
  -H 'content-type: application/json' \
  -d '{"source":"mac-verify","state":"waiting","message":"macOS 验证","ttlMs":10000}'
```

## B. 交互修复验收（当前预期失败，修好后逐条打勾）

| # | 操作 | 预期结果 | 当前状态 |
|---|---|---|---|
| B1 | 左键单击宠物 | 挥手 + 气泡；320ms 内的第二次点击不应先触发单击 | ⛔ 无反应 |
| B2 | 快速双击 | 跳一下；不先触发单击 | ⛔ 无反应 |
| B3 | 按住拖动 | 宠物跟手移动；左右移动时播放 running-left/right；松手停下且不触发单击 | ⛔ 拖不动 |
| B4 | 右键 | 菜单出现在光标位置；点菜单外或按 Esc 关闭 | ⛔ 不出/不关 |
| B5 | 鼠标在宠物左右两侧移动 | row9/row10 转头动作各播放一次；0.9s 冷却；正前方死区不触发 | ⛔ 不转头 |
| B6 | 开启 `click_through` | 透明像素点击落到桌面；不透明精灵像素仍能点击；关闭时整个窗口可交互 | ⛔ 可能挡住桌面点击 |
| B7 | 在 TextEdit/浏览器输入时点宠物 | 前台焦点不被打断，输入继续进入原应用 | ⛔ no-activate 是 no-op |
| B8 | 点击 / 右键 / 打开设置 | 宠物周围不出现任何边框闪烁 | 待实测（macOS 理论上无 Windows 那个问题） |
| B9 | 用状态协议发带 message 的 state | 气泡完整不被裁切；显示/消失时宠物不移动、窗口不闪烁 | 待实测 |
| B10 | 在副屏右键 | 菜单出现在光标所在显示器，且被夹在工作区内 | ⛔ 依赖全局光标 |
| B11 | 空闲时看 Activity Monitor | BytePet 空闲 CPU 接近 0–1%（允许偶发波动） | ⚠️ 当前可能 >3%（~60 FPS 重绘） |
| B12 | 启动第二个实例 | 不出现第二只宠物；要么退出，要么唤起已有实例 | ⛔ 未实现 |
| B13 | 注销再登录 / 重启 | 宠物自动出现 | ⛔ 未实现（需要 .app + Login Item/LaunchAgent） |

## C. 多显示器 / Spaces / 窗口系统

| # | 操作 | 预期结果 | 状态 |
|---|---|---|---|
| C1 | 把宠物拖到副屏（修好拖拽后） | 位置正确，缩放正确，不跳回主屏 | 待实测 |
| C2 | Retina + 非 Retina（或不同缩放）混用 | 精灵清晰、尺寸不跳、点击命中位置不漂 | 待实测 |
| C3 | 切换 Space / 进入全屏 App | 置顶行为符合预期；明确宠物是只在当前 Space 还是所有 Space | 待实测 |
| C4 | Mission Control / Stage Manager | 宠物不干扰窗口管理，托盘菜单仍可用 | 待实测 |
| C5 | 深色 / 浅色菜单栏 | 托盘图标在两种模式下都清晰可见 | 待实测 |
| C6 | 隐藏/显示后 | 窗口位置、层级、穿透状态保持一致 | 待实测 |

## D. 打包 / 发布（后续）

- [ ] `cargo build --release` 产物直接运行正常
- [ ] 生成 `.app` bundle（Info.plist、bundle id、图标）
- [ ] 如不需要 Dock 图标：`LSUIElement = true` 或 activation policy = accessory
- [ ] Login Item / LaunchAgent 开机自启
- [ ] 签名 + notarization（对外分发时）
- [ ] `.dmg` / `.zip` 发布产物
- [ ] 单实例保护（macOS 与 Windows 都要）

## E. 记录模板

| 日期 | macOS 版本 | 芯片 | 构建方式 | 结论 | 备注 / 日志 |
|---|---|---|---|---|---|
|  |  |  | `cargo run -p bytepet-app` |  |  |
