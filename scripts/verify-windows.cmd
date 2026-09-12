@echo off
setlocal

cd /d "%~dp0.."
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0verify-windows.ps1"
set "exit_code=%ERRORLEVEL%"

echo.
if "%exit_code%"=="0" (
    echo Windows verification passed.
) else (
    echo Windows verification failed with exit code %exit_code%.
)
pause
exit /b %exit_code%
