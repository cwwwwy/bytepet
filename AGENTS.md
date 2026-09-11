# BytePet 项目须知（给 AI 协作者）

## 这是什么

BytePet 是一个 Windows / macOS 桌宠：**Rust-only**（eframe/egui + winit + tray-icon + ureq），
没有 Node、WebView 或 Tauri 运行时。它读取 Codex 宠物包（`pet.json` + 8×9 / 8×11 图集），
按官方动画表播放，支持人格、轻量 JSON 记忆、DeepSeek 短问候，以及一个本地状态协议。

## 目录

| 路径 | 内容 |
|---|---|
| `crates/bytepet-core/` | 宠物格式与动画引擎、人格、记忆、DeepSeek 客户端、本地状态协议 |
| `crates/bytepet-app/` | egui 应用；窗口/托盘/菜单/输入都在 `src/app.rs`，Win32 调用在 `src/platform.rs` |
| `legacy/` | 重构前的 Tauri 应用与前端，**仅作参考**，不参与构建 |
| `docs/PET_NATIVE.md` | Codex 原生宠物复刻的实测记录（帧表、行语义、验收步骤） |

## 常用命令

```powershell
cargo run -p bytepet-app
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

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
- **尽量不新增第三方依赖**：能用标准库 / 已有 `windows-sys` 解决的，不要引新 crate。
- 用户看不到你的屏幕、你也截不了图；但可以用 PowerShell + Win32 读窗口样式/位置（见下）。
  涉及窗口、托盘、菜单的改动，交付时写清“需要用户实机确认什么”。
- 改动较大时先说清思路再动手；改完至少跑 `fmt` / `clippy` / `test`。

## 当前状态（2026-09-11）

- `main` 已合并 Rust-only 版本（顶端提交 `611da58`），测试 58 个通过（core 53 + app 5）。
- 已完成：宠物格式与动画（含 V2 look 行“注视”）、宠物库（本地库 + 只读引用 `~/.codex/pets`、
  `~/.unipet/pets`，支持导入文件夹/zip、导出、删除，内置 ByteBot 常驻本地库）、人格 + JSON 记忆 +
  DeepSeek 问候、托盘（**不用原生菜单**，点击托盘图标弹自己的菜单窗口）、设置窗口（可滚动、原生边框、
  用宠物首帧当图标）、本地状态协议、真·无边框宠物窗口、点击/拖动/双击/右键/看向光标/活动提醒行走。
- **待办**：下面这批修复曾在一次误删中丢失，需要重做（用户已知情并表示需要）。

### 待办清单（按优先级）

1. **消除偶发闪 + 宠物窗口紧贴精灵**
   - 现象：点击 / 右键 / 打开设置的瞬间，宠物周围偶尔闪出一个 Windows 边框。
   - 原因：`cursor_hittest(false)`（= `WS_EX_TRANSPARENT`）在 winit 里会**连带增删 `WS_EX_LAYERED`**，
     按光标在“宠物 / 空白”之间反复切换这个样式就会让窗口重绘。
   - 方案：宠物窗口去掉气泡预留区，做成**正好等于精灵尺寸**（按 1/4 档 scale 向上取整），
     常驻可交互、**不再切穿透**；只有隐藏 / 显示宠物时才切一次。
   - 代价（用户已接受）：精灵矩形内的透明角落会挡住点击，不再穿透到桌面。
2. **气泡独立成窗口**
   - 透明、点击穿透、`with_active(false)`、无边框；固定在宠物正上方、随宠物移动；
     没有气泡时不画任何东西。这样宠物窗口才能做小，气泡也不会被裁切。
3. **菜单定位用物理像素 + 光标所在显示器的工作区**
   - 现象：多屏 / 混合 DPI 下菜单跑回旧屏幕，托盘菜单常常看不见。
   - 方案：用 `MonitorFromPoint` + `GetMonitorInfoW(rcWork)` 取工作区，锚点用 `GetCursorPos`
     的**物理坐标**，夹取后用 `SetWindowPos` 精确落位（绕开 DPI 换算）。
4. **记住窗口位置**
   - 拖动结束后把窗口物理坐标写进 `config.json` 的 `window.startPosition`；启动时精确还原；
     保存的显示器不存在时回落到默认。首次运行（或设置里“重置位置”后）放**主屏工作区右下角**（留 ~32px）。
5. **重力开关**（设置 → 宠物行为，默认关）
   - 松手后按 ~2600 px/s² 加速下落（上限 ~1800 px/s），落到当前显示器工作区底部，
     落地播放一次 `jumping`。拖动中 / 活动提醒行走时不生效。

## 已踩过的坑（别再重新推导）

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
- **状态协议曾经的“偶发空响应”**：Windows 上 `accept()` 得到的 socket 会继承监听 socket 的非阻塞模式，
  读请求时 `WouldBlock` 就把连接丢掉。修法：accept 后显式 `set_nonblocking(false)`（见 `state_server.rs`
  的回归测试 `every_request_gets_a_response`）。
- **HTTRANSPARENT 的坑**：`WM_NCHITTEST` 返回 `HTTRANSPARENT` 只在**同线程**窗口间可靠转发鼠标，
  跨进程要靠 `WS_EX_TRANSPARENT`（但那个会引入上面的抖动）。也就是“像素级穿透”和“零闪烁”要权衡。

## 关键事实速查

- 数据目录：`%APPDATA%\BytePet`（mac：`~/Library/Application Support/BytePet`），可用环境变量
  `BYTEPET_HOME` 覆盖（测试 / 便携用）。本地宠物库在 `...\BytePet\pets`，另有只读引用
  `~/.codex/pets`、`~/.unipet/pets`；同 id 时本地库优先。
- 状态协议：`127.0.0.1:17872`，`POST /state`（`{source,state,message,ttlMs}`）、`GET /health`、
  `GET /pets`；`ttlMs: 0` 表示不过期。状态名见 `docs/PET_NATIVE.md`。
- 发行版宠物实测：V2 = 8×11；**idle 画了 7 帧**（官方时长表写 6），引擎按“真正画了内容的格子”取帧数；
  第 9 / 10 行是“转过去再转回来”的一次性动作，实测**第 9 行 = 转向右，第 10 行 = 转向左**
  （用头部深色像素重心相对头部中心测得，先拿 row1 / row2 校准过）。详见 `docs/PET_NATIVE.md`。
- 窗口样式自查（PowerShell + Win32，比截图可靠）：用 `EnumWindows` 找本进程里标题为 `BytePet` /
  `BytePet 气泡` / `BytePet 菜单` 的窗口，再用 `GetWindowLongPtrW(hwnd, GWL_STYLE / GWL_EXSTYLE)`
  看 `POPUP` / `CAPTION` / `TRANSPARENT` / `NOACTIVATE` 位。

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
