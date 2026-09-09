<#
.SYNOPSIS
    Mirrors the GitHub Actions CI pipeline on a local Windows machine.

.DESCRIPTION
    Runs the same gates as .github/workflows/ci.yml:
      pnpm install --frozen-lockfile -> pnpm build -> cargo test -> cargo clippy
      -> pnpm test -> pnpm tauri build --bundles nsis

    Requires: Rust (MSVC toolchain), Node 22+, pnpm 12, WebView2 (preinstalled
    on Windows 11 / Windows 10 with Edge).

.EXAMPLE
    pwsh -File scripts\verify-windows.ps1
    powershell -ExecutionPolicy Bypass -File scripts\verify-windows.ps1
#>
#requires -Version 5.1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

function Step($name) {
    Write-Host ""
    Write-Host "=== $name ===" -ForegroundColor Cyan
}

function Fail($message) {
    Write-Host ""
    Write-Host "FAILED: $message" -ForegroundColor Red
    exit 1
}

Step 'Toolchain'
foreach ($tool in 'rustc', 'cargo', 'node', 'pnpm') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Fail "$tool is not on PATH. See docs/WINDOWS.md for setup."
    }
}
rustc --version
node --version
pnpm --version

Step 'clippy component'
rustup component add clippy 2>$null
if (-not (Get-Command cargo-clippy -ErrorAction SilentlyContinue)) {
    Fail "clippy is unavailable; run 'rustup component add clippy'."
}

Step 'pnpm install --frozen-lockfile'
pnpm install --frozen-lockfile
if ($LASTEXITCODE -ne 0) { Fail 'pnpm install' }

Step 'pnpm build (tsc + vite)'
pnpm build
if ($LASTEXITCODE -ne 0) { Fail 'pnpm build' }

Step 'cargo test --workspace'
cargo test --workspace
if ($LASTEXITCODE -ne 0) { Fail 'cargo test' }

Step 'cargo clippy -D warnings'
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { Fail 'cargo clippy' }

Step 'pnpm test (vitest)'
pnpm test
if ($LASTEXITCODE -ne 0) { Fail 'pnpm test' }

Step 'pnpm tauri build --bundles nsis'
pnpm tauri build --bundles nsis
if ($LASTEXITCODE -ne 0) { Fail 'tauri build' }

Write-Host ""
Write-Host "All gates passed." -ForegroundColor Green
Write-Host "Installer: target\release\bundle\nsis\" -ForegroundColor Green
