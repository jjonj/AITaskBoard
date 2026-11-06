@echo off
setlocal
cd /d "%~dp0"
if exist "src-tauri\target\release\taskboard.exe" (
  powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\Ensure-TaskBoardExecutable.ps1"
  if errorlevel 1 exit /b 1
  start "" "%~dp0TaskBoard.exe"
  exit /b 0
)
echo No TaskBoard release executable. Run "npm run rebart" to build and launch it.
exit /b 1
