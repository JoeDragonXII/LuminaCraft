@echo off
cd /d "%~dp0"
if exist "dist\LuminaCraft\LuminaCraft.exe" (
  start "" "dist\LuminaCraft\LuminaCraft.exe" %*
  exit /b
)
if exist "target\debug\luminacraft.exe" (
  start "" "target\debug\luminacraft.exe" %*
  exit /b
)
echo LuminaCraft executable is missing. Run scripts\cargo.ps1 build first.
pause
