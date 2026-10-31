OpenLaser
=========

Laser design and control software. Galvo fiber lasers on JCZ/BJJCZ LMC boards (USB 9588:9899) first.
Source, issues and full documentation: https://github.com/luizbueno3d/openlaser-rs

EARLY SOFTWARE. It has been run on one machine so far (a JPT M7 MOPA). The software is not a safety
system: wear laser goggles rated for your laser, and keep the E-stop and the key switch within reach.


Two ways to start
-----------------

  OpenLaser                        Design, import, preview and the built-in simulator. The real laser stays off.
  Start OpenLaser with laser       Opens a small terminal window that starts a 2-hour "bench session", then opens
                                   OpenLaser with the real laser enabled. After 2 hours,
                                   start it again.


macOS (Apple Silicon and Intel, macOS 11 or later)
--------------------------------------------------

1. Drag OpenLaser into Applications. Keep "Start OpenLaser with laser" somewhere handy (Desktop is fine).
2. The app is not signed with an Apple developer certificate yet, so the first time macOS says it cannot verify it:
   open System Settings > Privacy & Security, scroll down and click "Open Anyway". Do the same for
   "Start OpenLaser with laser" the first time you open it.
3. No driver is needed for the laser board.


Windows (10 and 11, 64-bit)
---------------------------

1. Unzip the folder anywhere (for example Documents). Keep the files together.
2. If Windows shows "Windows protected your PC", click "More info", then "Run anyway".
3. One-time driver change, needed for the laser board:
   a. Download Zadig from https://zadig.akeo.ie and open it, with the laser plugged in and powered.
   b. Options > List All Devices. Pick the device whose USB ID is 9588 9899.
   c. Choose WinUSB as the driver and click "Replace Driver" (or "Install Driver").
   This replaces EZCAD's driver: EZCAD will not see the board on this PC until you reinstall its driver
   (Device Manager > the device > Update driver).


First time on your laser
------------------------

- Connect, then use Frame (red light only) before burning anything. If the frame is rotated or mirrored against
  the design, click Disconnect, open Device Settings and change "Galvo 1 is X Axis" or "Reverse X/Y direction",
  then Save and connect again.
- Check the field size and the lens settings in Device Settings against your machine before a real job.


License: MIT or Apache-2.0, at your option.
