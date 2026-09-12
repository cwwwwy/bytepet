#requires -Version 5.1

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root

try {
    foreach ($tool in @("rustc", "cargo")) {
        if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
            Write-Host "Missing required command: $tool" -ForegroundColor Red
            exit 1
        }
    }

    Write-Host "Repository: $root" -ForegroundColor DarkGray
    rustc --version
    cargo --version

    function Invoke-Step {
        param(
            [Parameter(Mandatory = $true)]
            [string] $Name,
            [Parameter(Mandatory = $true)]
            [scriptblock] $Command
        )

        Write-Host ""
        Write-Host "=== $Name ===" -ForegroundColor Cyan
        & $Command
        $exitCode = $LASTEXITCODE
        if ($exitCode -ne 0) {
            Write-Host "FAILED: $Name (exit code $exitCode)" -ForegroundColor Red
            exit $exitCode
        }
    }

    Invoke-Step "cargo fmt" { cargo fmt --all -- --check }
    Invoke-Step "cargo clippy" { cargo clippy --workspace --all-targets --locked -- -D warnings }
    Invoke-Step "cargo test" { cargo test --workspace --locked }
    Invoke-Step "cargo build (release linker check)" { cargo build --workspace --release --locked }

    Write-Host ""
    Write-Host "All Windows Rust gates passed." -ForegroundColor Green
}
finally {
    Pop-Location
}
