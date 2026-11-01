@echo off
rem Read-only check of this PC for OpenLaser: Windows, the unzipped files, the laser board on USB and its driver.
rem It changes nothing; it writes OpenLaser-diagnostics.txt on the Desktop and copies it to the clipboard.
cd /d "%~dp0"
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0OpenLaser-diagnostics.ps1"
pause
