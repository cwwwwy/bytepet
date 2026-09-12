# BytePet 项目须知（给 AI 协作者）

## 这是什么

BytePet 是一个 Windows / macOS 桌宠：**Rust-only**（eframe/egui + winit + tray-icon + ureq），
没有 Node、WebView 或 Tauri 运行时。它读取 Codex 宠物包（`pet.json` + 8×9 / 8×11 图集），
按官方动画表播放，支持人格、轻量 JSON 记忆、DeepSeek 短问候，以及一个本地状态协议。

**平台现状（重要）**：Windows 是当前实测平台；macOS 只是“能编译”，交互层尚未实现
（见「关键缺口」第 1 条和 `docs/MACOS_VERIFICATION.md`）。不要把 CI 的 macOS 绿灯当成
macOS 可用的证据。

## 目录

| 路径 | 内容 |
|---|---|
| `crates/bytepet-core/` | 宠物格式与动画引擎、人格、记忆、DeepSeek 客户端、本地状态协议 |
| `crates/bytepet-app/` | egui 应用；窗口/托盘/菜单/输入都在 `src/app.rs`，Win32 调用在 `src/platform.rs` |
| `legacy/` | 重构前的 Tauri 应用与前端，**仅作参考**，不参与构建 |
| `docs/PET_NATIVE.md` | Codex 原生宠物复刻的实测记录（帧表、行语义、验收步骤） |
| `docs/MACOS_VERIFICATION.md` | macOS 实机验收清单（基础回归 / 交互修复验收 / 多屏 / 打包） |

## 常用命令

```powershell
cargo run -p bytepet-app
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

两端完整验证：

```text
macOS:  bash scripts/verify-macos.sh
Windows: powershell -ExecutionPolicy Bypass -File scripts\verify-windows.ps1
```

也可以在 Finder 双击 `scripts/verify-macos.command`，或在 Windows 资源管理器双击
`scripts\verify-windows.cmd`。脚本负责格式、Clippy、测试和 release 链接；窗口、托盘、菜单与
点击穿透仍需按 `docs/MACOS_VERIFICATION.md` 或 Windows 实机操作检查。

macOS 需要 Xcode Command Line Tools（`xcode-select --install`）和 Rust stable。

Windows MSVC 目标需要 VS Build Tools（“使用 C++ 的桌面开发”）。如果 shell 里没有 `link.exe`
（自动化沙箱里常见），可以退回 GNU 工具链：

```powershell
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = "$env:USERPROFILE\.rustup\toolchains\stable-x86_64-pc-windows-gnu\lib\rustlib\x86_64-pc-windows-gnu\bin\rust-lld.exe"
$env:RUSTFLAGS = "-C link-self-contained=yes"
cargo +stable-x86_64-pc-windows-gnu test --workspace
```

## 协作偏好（重要）

- **用中文交流。**
- **不要自动 `git add/commit/push`**，需要提交时用户会明确说。
- **`AGENTS.md` 由 AI 协作者直接维护**：内容可以自己看着改，不需要先征求同意；提交仍按上一条。
- **尽量不新增第三方依赖**：能用标准库 / 已有 `windows-sys` 解决的，不要引新 crate。
  macOS 全局指针状态（`NSEvent`）是少数值得破例的地方。
- 用户看不到你的屏幕、你也截不了图；Windows 上可以用 PowerShell + Win32 读窗口样式/位置（见下），
  macOS 没有等价的自省脚本。涉及窗口、托盘、菜单的改动，交付时写清“需要用户实机确认什么”。
- 改动较大时先说清思路再动手；改完至少跑 `fmt` / `clippy` / `test`。

## 当前状态（2026-09-11）

- `main` 顶端提交 `bdb3e05`；最近一次全量测试 58 个通过（core 53 + app 5）。
- CI：`pull_request` → main、`push` tag `v*`、`workflow_dispatch`；直接推 `main` 不跑；
  同一 ref 的旧运行会被 concurrency 取消。
- 已完成（**以 Windows 实测为准**）：宠物格式与动画（含 V2 look 行“注视”）、宠物库
  （本地库 + 只读引用 `~/.codex/pets`、`~/.unipet/pets`，支持导入文件夹/zip、导出、删除，
  内置 ByteBot 常驻本地库）、人格 + JSON 记忆 + DeepSeek 问候、托盘（**不用原生菜单**，
  点击托盘图标弹自己的菜单窗口）、设置窗口（可滚动、原生边框、用宠物首帧当图标）、
  本地状态协议、真·无边框宠物窗口、点击/拖动/双击/右键/看向光标/活动提醒行走。

## 关键缺口（按优先级）

### 1. macOS 交互后端（最高优先级；用户接下来 2–3 天主力在 Mac）

代码现状：`crates/bytepet-app/src/platform.rs` 的所有非 Windows 实现都是空壳：

- `escape_pressed()` → `false`（:205）
- `primary_button_down()` → `None`（:223）
- `secondary_button_down()` → `None`（:237）
- `global_cursor_position()` → `None`（:275）
- `set_no_activate()`、`strip_frame_styles()`、`enable_transparency()`、`clear_dwm_frame()` → no-op

`crates/bytepet-app/src/app.rs` 的交互完全依赖这些函数：

- `update_pointer`（:1729、:1741）：macOS 上左右键永远 `false`，全局光标 `None` 直接 return →
  单击、双击、右键、拖拽全部无效。
- `drag_pet`（:1545）：拿不到全局光标 → `drag_grab` 永远是 `None` → 宠物拖不动。
- `update_glance`（:1591）：拿不到全局光标 → 不转头。
- `update_passthrough`（:1701）：拿不到全局光标 → 永远不会发 `ViewportCommand::MousePassthrough`；
  默认 `click_through: true`（`crates/bytepet-core/src/config.rs`:91），macOS 上宠物矩形可能变成
  挡住桌面点击的死区。
- `poll_menu`（:508）：拿不到全局光标 → 点击外部关闭菜单失效。
- `set_no_activate` 是 no-op → macOS 上点宠物可能抢走前台窗口焦点。

修复方向：抽一层平台无关的指针后端（`PointerBackend` / `PointerSource`），至少提供全局光标位置、
主/次按键状态、Escape 状态、no-activate、cursor hittest 切换：

- Windows：继续用现有 Win32（`GetCursorPos` / `GetAsyncKeyState` / `WS_EX_NOACTIVATE`）。
- macOS：`NSEvent::mouseLocation` + `NSEvent::pressedMouseButtons`（需要引入 `objc2` 之类
  macOS 专用依赖）；只靠 egui 事件拿不到窗口外的全局光标，可以先作为窗口内降级实现。
  `winit` 0.30 的 `Window::set_cursor_hittest` 在 macOS 可用，缺的是全局坐标/按键，不是穿透 API。
- `NSEvent::mouseLocation` 是屏幕坐标，原点/Y 方向与 winit 的坐标约定不同；多显示器下要通过
  winit 的 monitor 几何做映射，别直接混用。

验收：按 `docs/MACOS_VERIFICATION.md` 的 B 节逐条过。

### 2. 日常可用性（发布形态）

现在更像 `cargo run` 项目，而不是每天开机就在的应用：

- **单实例**：没有 mutex / lock。双击两次会开两只宠物；第二个实例的状态协议端口（17872）
  绑定失败，config/memory 并发写是 last-write-wins。
- **开机自启**：Windows 需要 Run 键/Startup 快捷方式；macOS 需要 `.app` + Login Item/LaunchAgent。
- **日志**：`AppPaths` 建了 `logs_dir`，但 `tracing_subscriber` 只输出到 stderr；release 是
  `panic = "abort"`（workspace `Cargo.toml`），从 Explorer/Finder 启动后出错会静默退出。
- **Windows 控制台窗口**：`crates/bytepet-app/src/main.rs` 没有
  `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`，release exe 双击会弹控制台。
  加这个属性前必须先有文件日志，否则什么日志都看不到。
- **打包/分发**：当前 `.github/workflows` 只有 `ci.yml`；Tauri 时代的 release/dmg 流程随
  Rust-only 重写删除。macOS 需要 `.app` bundle（Login Item、Dock 行为、托盘身份都依赖它），
  对外分发还需要签名/公证。

### 3. 重绘预算（省电）

- `crates/bytepet-app/src/app.rs:2132` 在 `ui()` 末尾无条件 `request_repaint_after(16ms)`，
  `:2055` 每 100ms 一次，空闲时也按 ~60 FPS 重绘。桌宠常驻一整天，这在笔记本上是实打实的耗电。
- 目标：只有动画、气泡、拖拽、自动行走、菜单打开时才 16ms；其余 500ms–1s 或事件驱动。
  状态协议来事件时要主动 wake event loop（现在靠 100ms 轮询兜底）。

> 原先“偶发闪 / 气泡独立窗口 / 菜单物理像素定位 / 记住位置 / 重力开关”那批 Windows 待办
> 已从本文件移除；需要时从 git 历史或对话里找回。

## 已踩过的坑（别再重新推导）

除特别标注外都是 Windows 结论：

- **winit 的 `decorations(false)` 不是真的无边框**：窗口仍带 `WS_CAPTION | WS_BORDER | WS_DLGFRAME |
  WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX`，只靠 `WM_NCCALCSIZE` 装样子。任何状态变化
  （激活、切样式、改尺寸）都会让 Windows 重画那个边框——这就是用户反复报告的“闪现”。
  做法：`SetWindowLongPtrW(GWL_STYLE)` 去掉这些位、加上 `WS_POPUP`，再 `SetWindowPos(SWP_FRAMECHANGED)`；
  随后必须重新 `DwmEnableBlurBehindWindow`（空 blur 区域）恢复透明，因为 frame 重算会把 winit 设的透明弄丢。
  实测：`style=0x96000000 POPUP=True CAPTION=False`。
- **不要每帧重复发 `ViewportCommand::InnerSize` / `WindowLevel`**：会反复 `SetWindowPos` → 闪。
  只在该变的时候发（`app.rs` 里有 `applied_window_size` / `applied_always_on_top` 缓存）。
- **`tray-icon` 的原生菜单会卡死整个程序**：`TrackPopupMenu` 在事件循环线程上开模态循环，
  期间宠物不重绘、“退出”也发不出去。所以托盘不挂 menu，改用 `TrayIconEvent::Click` 弹自己的窗口。
- **`WS_EX_NOACTIVATE` 很有用**：宠物 / 气泡 / 菜单都不该抢焦点（点宠物不该打断用户正在编辑的窗口）。
  macOS 上没有等价实现，需要单独处理（见「关键缺口」第 1 条）。
- **状态协议曾经的“偶发空响应”**：Windows 上 `accept()` 得到的 socket 会继承监听 socket 的非阻塞模式，
  读请求时 `WouldBlock` 就把连接丢掉。修法：accept 后显式 `set_nonblocking(false)`（见 `state_server.rs`
  的回归测试 `every_request_gets_a_response`）。
- **HTTRANSPARENT 的坑**：`WM_NCHITTEST` 返回 `HTTRANSPARENT` 只在**同线程**窗口间可靠转发鼠标，
  跨进程要靠 `WS_EX_TRANSPARENT`（但那个会引入上面的抖动）。也就是“像素级穿透”和“零闪烁”要权衡。
- **macOS 只靠 egui 事件拿不到窗口外的全局光标**：egui 只看到投递给本窗口的事件；
  转头、拖拽、点击穿透都需要全局光标/按键状态，必须走 macOS 原生 API（或先做窗口内降级）。

## 关键事实速查

- 数据目录：`%APPDATA%\BytePet`（mac：`~/Library/Application Support/BytePet`），可用环境变量
  `BYTEPET_HOME` 覆盖（测试 / 便携用）。本地宠物库在 `...\BytePet\pets`，另有只读引用
  `~/.codex/pets`、`~/.unipet/pets`；同 id 时本地库优先。
- 状态协议：`127.0.0.1:17872`，`POST /state`（`{source,state,message,ttlMs}`）、`GET /health`、
  `GET /pets`；`ttlMs: 0` 表示不过期。状态名见 `docs/PET_NATIVE.md`。
- 发行版宠物实测：V2 = 8×11；**idle 画了 7 帧**（官方时长表写 6），引擎按“真正画了内容的格子”取帧数；
  第 9 / 10 行是“转过去再转回来”的一次性动作，实测**第 9 行 = 转向右，第 10 行 = 转向左**
  （用头部深色像素重心相对头部中心测得，先拿 row1 / row2 校准过）。详见 `docs/PET_NATIVE.md`。
- 窗口样式自查（**Windows only**，比截图可靠）：用 `EnumWindows` 找本进程里标题为 `BytePet` /
  `BytePet 气泡` / `BytePet 菜单` 的窗口，再用 `GetWindowLongPtrW(hwnd, GWL_STYLE / GWL_EXSTYLE)`
  看 `POPUP` / `CAPTION` / `TRANSPARENT` / `NOACTIVATE` 位。macOS 没有等价脚本，靠
  `docs/MACOS_VERIFICATION.md` 人工验收。
- release profile：`lto = "thin"`、`codegen-units = 1`、`strip = true`、`panic = "abort"`；目前没有
  文件日志，崩溃不会留下痕迹。

## Git 工作流

个人仓库、Windows + macOS 两地开发，直接在 `main` 上做：

```powershell
git fetch --prune
git switch main
git pull --ff-only
# ...改代码...
git add -A
git commit -m "..."
git push
```

全局配置（`pull.ff=only`、`push.autoSetupRemote=true`、`fetch.prune=true`、`core.longpaths=true`）已设好。
