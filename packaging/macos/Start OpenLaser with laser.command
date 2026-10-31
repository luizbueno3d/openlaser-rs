#!/bin/bash
# Starts OpenLaser with the real laser enabled: a 2-hour bench session (the first emission is capped at 10 % power),
# then the app. The software is not a safety system: the E-stop and the key switch are.
APP=/Applications/OpenLaser.app/Contents/MacOS
[ -x "$APP/openlaser" ] || APP="$(cd "$(dirname "$0")" && pwd)/OpenLaser.app/Contents/MacOS"
export OPENLASER_HW_PRESENT=1
unset OPENLASER_NO_HW
if ! "$APP/openlaser-hw" bench start; then
  echo
  read -r -p "The bench session did not start (see README.txt). Press Enter to close."
  exit 1
fi
"$APP/openlaser" >/dev/null 2>&1 &
disown
echo "OpenLaser is open. You can close this window."
