# 在 Windows 实机上验证与使用 BytePet

> **定位**：Windows 机器用于**验证与日常使用**，不是开发机；开发以 macOS 为主。
> 因此这里只包含环境准备、门禁脚本与人工验证清单，不涉及跨机开发流程。

macOS 侧已经覆盖了核心逻辑、真实 Codex/Claude CLI 通道、hook 往返与打包；本机还通过
`cargo xwin check/clippy --target x86_64-pc-windows-msvc` 验证了 Windows 目标能编译、lint 干净。
但下面这些**只能在 Windows 实机上确认**，这也是这份清单存在的原因：

- 透明窗 + 像素级点击穿透的真实手感（WebView2 与 `WS_EX_TRANSPARENT` 行为）
- npm 安装的 `codex` / `claude` 是 `.cmd` shim，Windows 下必须经 `cmd /C` 启动
- hook 包装器 `.cmd` / `.ps1` 的真实执行，以及路径含空格时的表现
- 托盘、单实例、开机自启、多显示器 / 高 DPI、Windows 凭据管理器、SAPI 语音
- NSIS 安装包的构建与安装后行为

## 1. 一次性环境准备

| 依赖 | 说明 |
|---|---|
| Rust | `rustup` + **MSVC** 工具链（安装 Visual Studio Build Tools 的“使用 C++ 的桌面开发”） |
| Node | 22 LTS |
| pnpm | **12.x**（与仓库 `pnpm-lock.yaml` 一致） |
| WebView2 | Windows 11 自带；Windows 10 装 Edge WebView2 Runtime |
| GitHub 认证 | 仓库是私有的：`gh auth login` 或配置 PAT/SSH |

建议再做四件事：

```powershell
# 1. 长路径支持（Rust 的 target 目录很容易超过 260 字符）
git config --system core.longpaths true
# 并在“设置 → 系统 → 开发者选项”里打开长路径支持

# 2. 打开开发者模式（pnpm 需要创建符号链接）
#    设置 → 系统 → 开发者选项 → 开发人员模式

# 3. Windows Defender 排除仓库目录与 target\（构建会快很多，也避免误报）
#    设置 → 病毒和威胁防护 → 排除项

# 4. 克隆并安装
gh repo clone cwwwwy/bytepet
cd bytepet
pnpm install
```

> 不要在本机设置 `CARGO_HOME`/`BYTEPET_CONFIG_DIR` 之类的东西——那是 macOS 沙箱环境的产物。
> 仓库里的 `.npmrc` 把 pnpm store 指向仓库内的相对路径，Windows 上同样适用。

## 2. 一键跑 CI 的全部关卡

```powershell
powershell -ExecutionPolicy Bypass -File scripts\verify-windows.ps1
```

它按 `.github/workflows/ci.yml` 的顺序执行：`pnpm install --frozen-lockfile` → `pnpm build`
→ `cargo test --workspace` → `cargo clippy --workspace --all-targets -- -D warnings`
→ `pnpm test` → `pnpm tauri build --bundles nsis`。

> 注意：`src-tauri/tests/chat_e2e.rs` 在 Windows 上是**故意跳过**的——Tauri 的 mock 运行时在测试
> 二进制里缺少 Common-Controls v6 manifest，会以 `STATUS_ENTRYPOINT_NOT_FOUND` 退出
> （tauri-apps/tauri#11028）。命令层的等价覆盖在 macOS 上跑。

## 3. 人工验证清单（按风险从高到低）

调试日志：`$env:BYTEPET_LOG="debug"`，然后从终端启动 `pnpm tauri dev`。
数据目录：`%APPDATA%\com.bytepet.desktop`（配置、`pet.db`、`hooks\`、`logs\`）。
状态服务：`http://127.0.0.1:17872/health`。

### A. CLI 通道（风险最高：`cmd /C` 改动是盲写的）

```powershell
# 先用独立探针，绕开 UI
cargo run -p bytepet-core --example cli_probe -- codex "只回复四个字：你好世界"
cargo run -p bytepet-core --example cli_probe -- claude "只回复四个字：你好世界"
```

- 预期：`STATUS session:<id>`、`TEXT ...`、`USAGE ...`、`DONE`，最后打印最终文本。
- 失败时看 `%APPDATA%\com.bytepet.desktop\logs` 与 `BYTEPET_LOG=debug` 输出；
  如果报 “program not found”，说明 `.cmd` 解析仍不对。
- 然后在应用里选 `codex-cli` / `claude-cli` 服务聊一句，确认 UI 流式渲染正常。

### B. Hook 往返（会写你的真实 agent 配置，先备份）

```powershell
target\debug\bytepet.exe hooks install all
target\debug\bytepet.exe hooks status
# 跑一次真实会话，例如：
codex exec --json --skip-git-repo-check "回复：你好"
curl http://127.0.0.1:17872/health     # 期望 state 变成 review
target\debug\bytepet.exe hooks uninstall all
```

- 检查 `%USERPROFILE%\.codex\config.toml` 的 `notify` 在安装后指向
  `%APPDATA%\com.bytepet.desktop\hooks\codex-notify.cmd`，卸载后**逐字节还原**（先 `copy` 一份原文件再 `fc` 对比）。
- `%USERPROFILE%\.claude\settings.json` 同理，注意其它 `env`/`hooks` 条目必须原样保留。
- **重点场景**：把 `BYTEPET_CONFIG_DIR` 指向一个含空格的目录（例如
  `C:\Users\me\My Pets`）再安装一次，确认 Claude hook 仍能触发。若失败，需要在
  `command` 里加引号——这是目前已知的未验证风险。

### C. 窗口行为

- 透明、无边框、置顶、可拖动（拖拽后重启，位置应保留）。
- **像素级穿透**：点宠物身旁的透明区域，应能点到底下的窗口（例如桌面图标）；点宠物本体应命中。
- 托盘菜单：打开聊天、显示/隐藏、设置、退出。
- 单实例：再启动一次 `bytepet-app.exe`，应唤起已存在的实例并打开聊天窗。
- 开机自启：设置里打开，注销/重启后确认生效。

### D. 多显示器与高 DPI

把宠物拖到副屏：尺寸应保持一致、自动行走在副屏边界内转身、穿透坐标不偏移（点击命中点与视觉一致）。
若错位，记录两块屏的分辨率与缩放比例。

### E. 钥匙串（Windows 凭据管理器）

在设置里保存一个 API Key → 打开「控制面板 → 用户帐户 → 凭据管理器 → Windows 凭据」，
应能看到 `com.bytepet.desktop` 条目；重启应用后 Key 仍可用（不需要重填）。

### F. TTS

开启 TTS → 点「试听」。实现走 `powershell.exe` + `System.Speech`（Windows PowerShell 5.1），
不是 `pwsh`；如果没声音，先确认系统里有可用语音包。

### G. 安装包

`pnpm tauri build --bundles nsis` 产物在 `target\release\bundle\nsis\`。
安装后验证：能从开始菜单启动、托盘正常、数据目录仍是 `%APPDATA%\com.bytepet.desktop`。
安装包未签名，SmartScreen 会提示“未知发布者”，选择“仍要运行”。

## 4. 已知的预期差异

- macOS 上用的是 `macOSPrivateApi` 透明窗；Windows 走 WebView2 的透明背景，视觉细节可能略有不同。
- 窗口圆角/阴影：macOS 关掉了阴影以避免鬼影；Windows 上不受影响。
- 自动行走的边界使用显示器物理坐标，任务栏区域没有扣除（会走到任务栏上方边缘）。

## 5. 记录与回报

跑完 `verify-windows.ps1` 后，把这些贴给我：

1. 脚本输出的**最后 40 行**（哪个关卡失败、报错原文）。
2. 相关日志：`%APPDATA%\com.bytepet.desktop\logs\`，或 `BYTEPET_LOG=debug` 的终端输出。
3. 失败场景的最小复现步骤（例如“用户名含空格 + Claude hook 不触发”）。

更高效的方式：在 Windows 上开一个分支提交修复/改动并推上来

```powershell
git checkout -b windows-verify
# ...修改...
git commit -am "fix(windows): ..."
git push -u origin windows-verify
```

我这边有仓库访问权限，可以直接 `git fetch origin windows-verify` 拉下来复现、修好再推回去。
由于 GitHub Actions 的私有仓库额度已耗尽，`verify-windows.ps1` 目前就是本仓库在 Windows 上的
CI 等价物。
