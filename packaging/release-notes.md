**Early release.** OpenLaser has been run on one machine so far (JPT M7 MOPA, LMC board `9588:9899`). The software is
not a safety system: wear goggles rated for your laser and keep the E-stop and the key switch within reach.

## Download

- **macOS** (Apple Silicon and Intel, macOS 11+): `OpenLaser-…-macos.dmg`. Drag OpenLaser into Applications. It is not
  signed by Apple yet: the first time, open System Settings › Privacy & Security and click **Open Anyway**.
- **Windows** (10/11, 64-bit): `OpenLaser-…-windows.zip`. Unzip it and keep the files together. The laser board needs
  the WinUSB driver once, via [Zadig](https://zadig.akeo.ie) (USB ID `9588 9899`). This replaces EZCAD's driver on that PC.

`README.txt` inside each download has the step-by-step guide.

## Starting

- **OpenLaser**: design, import, preview and the simulator. The real laser stays off.
- **Start OpenLaser with laser**: starts a 2-hour bench session in a terminal window, then opens OpenLaser with the laser
  enabled. Start burns at the power you set; it does not need a red-light frame first.
