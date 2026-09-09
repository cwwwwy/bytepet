# Release BytePet

安装包通过 GitHub Releases 分发。**最终用户不需要 Rust / Node / pnpm**——他们只下载安装包。

## 版本号

版本号存放在三处，用脚本统一修改，不要手改：

```bash
bash scripts/bump-version.sh 0.2.0            # 改文件
bash scripts/bump-version.sh 0.2.0 --tag      # 改文件 + 提交 + 打 tag
```

脚本会同步 `src-tauri/tauri.conf.json`、workspace `Cargo.toml`、`package.json`，并在 `--tag` 时创建
`v0.2.0` 提交与标签。改完版本后第一次 `cargo build` 会顺带刷新 `Cargo.lock`，记得一起提交。

## 构建产物

| 平台 | 命令 | 产物 |
|---|---|---|
| macOS（arm64 + Intel 通用） | `bash scripts/release-macos.sh` | `release/BytePet_<ver>_universal.dmg`、`SHA256SUMS` |
| Windows（x64） | `powershell -File scripts\release-windows.ps1` | `release\BytePet_<ver>_x64-setup.exe`、`SHA256SUMS` |

Windows 安装包必须在 Windows 上构建（本机无 MSVC 工具链）；macOS 产物在任意 Apple Silicon Mac 上
交叉编译即可，无需 Intel 机器。

## 发布

```bash
# 两个平台的产物都放进 release/ 后
bash scripts/publish-release.sh 0.2.0
```

脚本调用 `gh release create`，把 `release/` 下的所有文件作为附件上传，并把 `SHA256SUMS` 内容写进
发布说明。仓库必须公开，否则别人无法下载。

## 安装体验与已知提示

- **macOS（未签名）**：首次打开需要右键 → 打开；或执行
  `xattr -dr com.apple.quarantine /Applications/BytePet.app`。系统提示“无法验证开发者”属于预期。
- **Windows（未签名）**：SmartScreen 提示“未知发布者”，点「更多信息 → 仍要运行」。
- **Windows 运行时**：需要 WebView2。Windows 11 已预装；Windows 10 首次安装会联网下载
  （Tauri 默认 `downloadBootstrapper`）。如需完全离线安装，可在 `tauri.conf.json` 的
  `bundle.windows.webviewInstallMode` 改为 `{ "type": "offlineInstaller" }`（安装包约 +127 MB）。

## 发布前检查清单

1. `bash scripts/verify-macos.sh` 全绿（Windows 侧跑 `verify-windows.ps1`）。
2. 干净用户账号下启动，确认**内置宠物直接出现**、首启引导卡片只出现一次。
3. 清空 `PATH` 启动 `.app`，确认默认路径不依赖任何外部命令。
4. `hdiutil verify` / Windows 安装包能正常安装与卸载。
5. 发布说明里带上 `SHA256SUMS`。

## 自动化（可选）

`.github/workflows/release.yml` 可在仓库公开后按 tag 自动构建三平台产物并上传 Release，
届时无需本机或 Windows 机器参与。当前仓库为私有，Actions 额度已耗尽，故默认走本地脚本。
