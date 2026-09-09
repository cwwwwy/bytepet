<#
.SYNOPSIS
    Build the Windows release installer (NSIS) and SHA-256 checksums.

.DESCRIPTION
    Produces release\BytePet_<version>_x64-setup.exe plus release\SHA256SUMS.
    Windows artifacts must be built on Windows (MSVC toolchain required).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\release-windows.ps1
#>
#requires -Version 5.1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Get-Content 'src-tauri\tauri.conf.json' -Raw | ConvertFrom-Json).version
Write-Host "== BytePet $version (Windows x64) ==" -ForegroundColor Cyan

foreach ($tool in 'rustc', 'cargo', 'node', 'pnpm') {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        throw "$tool is not on PATH. See docs/WINDOWS.md."
    }
}

pnpm install --frozen-lockfile
if ($LASTEXITCODE -ne 0) { throw 'pnpm install failed' }

pnpm build
if ($LASTEXITCODE -ne 0) { throw 'pnpm build failed' }

pnpm tauri build --bundles nsis
if ($LASTEXITCODE -ne 0) { throw 'tauri build failed' }

$out = 'release'
if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Path $out | Out-Null

$installers = Get-ChildItem 'target\release\bundle\nsis' -Filter '*.exe'
if (-not $installers) { throw 'no NSIS installer produced' }
foreach ($file in $installers) {
    Copy-Item $file.FullName $out
}

Get-ChildItem $out -Filter '*.exe' | ForEach-Object {
    $hash = (Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLower()
    "$hash  $($_.Name)"
} | Set-Content -Encoding ascii "$out\SHA256SUMS"

Write-Host ""
Write-Host "Artifacts in $out\:" -ForegroundColor Green
Get-ChildItem $out | Format-Table Name, Length
Write-Host ""
Write-Host "Next: copy release\ to the Mac (or push it) and run scripts/publish-release.sh"
