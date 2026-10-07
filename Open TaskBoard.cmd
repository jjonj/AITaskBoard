@echo off
setlocal
cd /d "%~dp0"
if exist "TaskBoard.exe" (
  start "" "TaskBoard.exe"
  exit /b 0
)
npm run tauri dev
