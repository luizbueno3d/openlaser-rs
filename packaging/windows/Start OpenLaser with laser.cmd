@echo off
rem Starts OpenLaser with the real laser enabled: a 2-hour bench session (the first emission is capped at 10 % power),
rem then the app. The software is not a safety system: the E-stop and the key switch are.
cd /d "%~dp0"
set OPENLASER_HW_PRESENT=1
set OPENLASER_NO_HW=
openlaser-hw.exe bench start
if errorlevel 1 (
  echo.
  echo The bench session did not start. See README.txt.
  pause
  exit /b 1
)
start "" openlaser.exe
