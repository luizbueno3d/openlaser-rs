# OpenLaser diagnostics for Windows. Read-only: it installs nothing and changes no driver, setting or file. The one
# thing it writes is its report, OpenLaser-diagnostics.txt on the Desktop (also copied to the clipboard, so it can be
# pasted into a chat or a remote-desktop session).
#
# It checks Windows (and that the download fits it: 64-bit, or the 32-bit windows-x86 package), the unzipped OpenLaser files, the Visual C++ runtime, the laser board on USB (9588:9899) and the
# driver Windows gave it, and OpenLaser's own folders. It ends with a SUMMARY and the next step.
#
# Start it with "OpenLaser diagnostics.cmd" in this folder. Plain ASCII on purpose: Windows PowerShell 5.1 reads a
# script without a byte-order mark as the local code page.

param(
    # Do not start openlaser-hw.exe (GitHub's runners use this: they never run the hardware binaries).
    [switch]$SkipPrograms,
    # No clipboard, no Notepad window at the end.
    [switch]$Quiet,
    [string]$OutFile = ''
)

$ErrorActionPreference = 'Continue'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (-not $OutFile) {
    $desktop = [Environment]::GetFolderPath('Desktop')
    if (-not $desktop -or -not (Test-Path -LiteralPath $desktop)) { $desktop = $here }
    $OutFile = Join-Path $desktop 'OpenLaser-diagnostics.txt'
}

$details = New-Object System.Collections.Generic.List[string]
$summary = New-Object System.Collections.Generic.List[string]
$next = New-Object System.Collections.Generic.List[string]

function Say([string]$text) { $details.Add($text); Write-Host $text }
function Section([string]$title) { Say ''; Say "== $title ==" }
function Good([string]$text) { $summary.Add("OK       $text") }
function Note([string]$text) { $summary.Add("NOTE     $text") }
function Problem([string]$text) { $summary.Add("PROBLEM  $text") }
function Step([string]$text) { $next.Add($text) }
function Guard([string]$name, [scriptblock]$block) {
    try { & $block } catch { Say "  ($name could not be checked: $($_.Exception.Message))"; Note "$name could not be checked (see details)" }
}

# Runs openlaser-hw.exe with a 20 s limit and returns @{ Code; Out } ($null when it could not run).
function Invoke-Hw([string]$arguments, [bool]$hardware) {
    $exe = Join-Path $here 'openlaser-hw.exe'
    if (-not (Test-Path -LiteralPath $exe)) { Say '  openlaser-hw.exe is not in this folder'; return $null }
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = $exe
    $psi.Arguments = $arguments
    $psi.WorkingDirectory = $here
    $psi.UseShellExecute = $false
    $psi.CreateNoWindow = $true
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.EnvironmentVariables.Remove('OPENLASER_NO_HW')
    $psi.EnvironmentVariables.Remove('OPENLASER_HW_PRESENT')
    # `detect` only lists USB devices (it never opens the board); without this variable it refuses to look at all.
    if ($hardware) { $psi.EnvironmentVariables['OPENLASER_HW_PRESENT'] = '1' }
    $p = [System.Diagnostics.Process]::Start($psi)
    $out = $p.StandardOutput.ReadToEndAsync()
    $err = $p.StandardError.ReadToEndAsync()
    if (-not $p.WaitForExit(20000)) {
        try { $p.Kill() } catch { }
        Say "  openlaser-hw $arguments gave no answer in 20 s and was stopped"
        return $null
    }
    $text = ($out.Result + $err.Result).Trim()
    Say "  openlaser-hw $arguments  (exit code $($p.ExitCode))"
    foreach ($line in ($text -split "`r?`n")) { if ($line) { Say "    $line" } }
    return @{ Code = $p.ExitCode; Out = $text }
}

# 'x86', 'x64', 'arm64' or '' : the CPU a Windows program is built for (the Machine field of its PE header).
function Get-ExeArch([string]$path) {
    if (-not (Test-Path -LiteralPath $path)) { return '' }
    $bytes = [System.IO.File]::ReadAllBytes($path)
    if ($bytes.Length -lt 64) { return '' }
    $pe = [BitConverter]::ToInt32($bytes, 60)
    if ($pe -lt 0 -or $pe + 6 -gt $bytes.Length) { return '' }
    switch ([BitConverter]::ToUInt16($bytes, $pe + 4)) {
        0x014c { return 'x86' }
        0x8664 { return 'x64' }
        0xaa64 { return 'arm64' }
        default { return '' }
    }
}

Say "OpenLaser diagnostics, $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')"
Say "Folder: $here"

# ---- Windows -------------------------------------------------------------------------------------------------------
Section 'Windows'
Guard 'Windows version' {
    $os = Get-CimInstance Win32_OperatingSystem
    Say "  $($os.Caption), version $($os.Version), $($os.OSArchitecture)"
    Say "  PowerShell $($PSVersionTable.PSVersion)"
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    Say "  Running as administrator: $admin (not needed for this check)"
    $script:exeArch = Get-ExeArch (Join-Path $here 'openlaser.exe')
    $bits = if ($script:exeArch -eq 'x86') { '32-bit' } else { '64-bit' }
    $package = if ($script:exeArch) { "$($script:exeArch) ($bits)" } else { 'unknown' }
    Say "  These OpenLaser programs are built for: $package"
    if ([Environment]::Is64BitOperatingSystem) {
        Good 'Windows is 64-bit'
        if ($script:exeArch -eq 'x86') { Note 'this is the 32-bit (windows-x86) download: it works here, but the 64-bit OpenLaser-...-windows.zip is the one for 64-bit Windows' }
    } elseif ($script:exeArch -eq 'x86') {
        Good 'Windows is 32-bit and this is the 32-bit (windows-x86) download of OpenLaser'
    } elseif ($script:exeArch) {
        Problem "Windows is 32-bit, but these programs are ${package}: download OpenLaser-...-windows-x86.zip instead"
    } else {
        Problem 'Windows is 32-bit: use the 32-bit download, OpenLaser-...-windows-x86.zip'
    }
}

# ---- Files ---------------------------------------------------------------------------------------------------------
Section 'OpenLaser files'
$missing = @()
$marked = @()
foreach ($name in @('openlaser.exe', 'openlaser-hw.exe', 'Start OpenLaser with laser.cmd', 'README.txt')) {
    $path = Join-Path $here $name
    if (Test-Path -LiteralPath $path) {
        $item = Get-Item -LiteralPath $path
        $zone = Get-Item -LiteralPath $path -Stream Zone.Identifier -ErrorAction SilentlyContinue
        if ($zone) { $marked += $name }
        Say ("  {0,-32} {1,12:N0} bytes{2}" -f $name, $item.Length, $(if ($zone) { '  (marked as downloaded)' } else { '' }))
    } else {
        $missing += $name
        Say "  $name is MISSING"
    }
}
if ($here -match '\.zip' -or $here -match '\\Temp\\') {
    Problem 'this runs from inside the .zip or a temporary folder: right-click the .zip, Extract All, and use the extracted folder'
} elseif ($missing.Count -gt 0) {
    Problem "files missing from this folder: $($missing -join ', '). Extract the whole .zip and keep the files together"
} else {
    Good 'all OpenLaser files are in this folder'
}
if ($marked.Count -gt 0) {
    Note 'the files carry the "downloaded from the internet" mark: the first start shows "Windows protected your PC" > More info > Run anyway (normal)'
}

# ---- Visual C++ runtime ----------------------------------------------------------------------------------------------
Section 'Visual C++ runtime'
$runtimeNeeded = $false
Guard 'Visual C++ runtime' {
    foreach ($name in @('openlaser.exe', 'openlaser-hw.exe')) {
        $path = Join-Path $here $name
        if (-not (Test-Path -LiteralPath $path)) { continue }
        $bytes = [System.IO.File]::ReadAllBytes($path)
        $text = [System.Text.Encoding]::ASCII.GetString($bytes)
        $needs = $text.IndexOf('VCRUNTIME140.dll', [StringComparison]::OrdinalIgnoreCase) -ge 0
        Say "  $name needs VCRUNTIME140.dll: $needs"
        if ($needs) { $script:runtimeNeeded = $true }
    }
    $present = Test-Path -LiteralPath (Join-Path $env:SystemRoot 'System32\vcruntime140.dll')
    Say "  VCRUNTIME140.dll installed: $present"
    if (-not $script:runtimeNeeded) { Good 'the programs carry their own Visual C++ runtime' }
    elseif ($present) { Good 'the Visual C++ runtime the programs need is installed' }
    else {
        $vc = if ($script:exeArch -eq 'x86') { 'x86' } else { 'x64' }
        Problem "the Visual C++ runtime is missing: install `"Microsoft Visual C++ Redistributable $vc`" from https://aka.ms/vs/17/release/vc_redist.$vc.exe"
    }
}

# ---- Graphics ------------------------------------------------------------------------------------------------------
Section 'Graphics'
Guard 'Graphics card' {
    foreach ($v in @(Get-CimInstance Win32_VideoController)) {
        Say "  $($v.Name), driver $($v.DriverVersion)"
        if ($v.Name -match 'Basic Display|Basic Render') {
            Note 'Windows uses its basic display driver: if the OpenLaser window does not open, install the graphics card driver'
        }
        if ($v.Name -match 'Intel\(R\) HD Graphics$|Intel\(R\) HD Graphics [0-9]{4}$') {
            Note "older Intel graphics ($($v.Name)): OpenLaser starts on OpenGL if DirectX 12 is not available. If its window does not open, install Intel's newest driver for it"
        }
    }
}

# ---- Laser board on USB ----------------------------------------------------------------------------------------------
Section 'Laser board (USB 9588:9899)'
$boardState = 'unknown'
Guard 'USB board' {
    $all = @(Get-CimInstance Win32_PnPEntity | Where-Object { $_.DeviceID -match 'VID_9588' })
    $signed = @(Get-CimInstance Win32_PnPSignedDriver | Where-Object { $_.DeviceID -match 'VID_9588' })
    if ($all.Count -eq 0) { Say '  no BJJCZ device (USB vendor 9588) is connected' }
    foreach ($d in $all) {
        $drv = $signed | Where-Object { $_.DeviceID -eq $d.DeviceID } | Select-Object -First 1
        Say "  $($d.DeviceID)"
        Say "    name: $($d.Name)"
        Say "    status: $($d.Status), problem code: $($d.ConfigManagerErrorCode), service (driver): '$($d.Service)'"
        if ($drv) {
            Say "    driver package: $($drv.InfName), provider: $($drv.DriverProviderName), version: $($drv.DriverVersion), date: $($drv.DriverDate)"
        }
        if ($d.DeviceID -notmatch 'PID_9899') {
            if ($d.DeviceID -match 'PID_9980') { Problem 'a clone board (9588:9980) is connected: OpenLaser does not support it' }
            else { Note "another BJJCZ device is connected ($($d.DeviceID)): OpenLaser drives only 9588:9899" }
            continue
        }
        $service = "$($d.Service)"
        if ($service -ieq 'WinUSB' -and $d.ConfigManagerErrorCode -eq 0) {
            $script:boardState = 'winusb'
            Good 'the laser board is connected and already uses WinUSB: do NOT change its driver'
        } elseif ($service -and $service -ine 'WinUSB') {
            $script:boardState = 'other'
            $inf = if ($drv) { $drv.InfName } else { '(unknown)' }
            $provider = if ($drv) { $drv.DriverProviderName } else { '(unknown)' }
            Note "the laser board uses another driver: service '$service', package $inf, provider $provider (probably EZCAD/LMC)"
            Step "Back up that driver first. In an administrator Command Prompt: pnputil /export-driver $inf C:\Backup-Driver-LMC"
            Step 'Then install WinUSB with Zadig (Options > List All Devices, USB ID 9588 9899, WinUSB, Replace Driver)'
        } else {
            $script:boardState = 'none'
            Note "the laser board is connected but has no working driver (problem code $($d.ConfigManagerErrorCode))"
            Step 'Install WinUSB with Zadig (Options > List All Devices, USB ID 9588 9899, WinUSB, Install Driver). Nothing to back up'
        }
    }
    if ($script:boardState -eq 'unknown') {
        Problem 'the laser board (9588:9899) is not seen: check that the machine is powered on, the USB cable, and try another USB port'
    }
}

if ($SkipPrograms) {
    Say '  (openlaser-hw detect skipped: -SkipPrograms)'
} else {
    Guard 'openlaser-hw detect' {
        Say ''
        Say '  What OpenLaser itself sees (it only lists USB devices; it does not open the board or start a laser):'
        $r = Invoke-Hw 'detect' $true
        if ($null -eq $r) { return }
        if ($r.Code -eq -1073741515) {
            Problem 'openlaser-hw.exe cannot start: a DLL is missing (most likely the Visual C++ runtime, see above)'
        } elseif ($r.Out -match 'on WinUSB') {
            Good 'OpenLaser sees the board on WinUSB'
        } elseif ($r.Out -match 'not on WinUSB') {
            Note 'OpenLaser sees the board, but not on WinUSB yet'
        } elseif ($r.Out -match 'No LMC board found') {
            Note 'OpenLaser sees no laser board'
        }
    }
}

# ---- OpenLaser's folders ---------------------------------------------------------------------------------------------
Section "OpenLaser's folders"
Guard 'OpenLaser folders' {
    foreach ($root in @($env:LOCALAPPDATA, $env:APPDATA)) {
        if (-not $root) { continue }
        $dir = Join-Path $root 'OpenLaser'
        if (-not (Test-Path -LiteralPath $dir)) { Say "  $dir does not exist yet (normal before the first start)"; continue }
        Say "  $dir"
        foreach ($f in @(Get-ChildItem -LiteralPath $dir -Recurse -File -ErrorAction SilentlyContinue)) {
            Say ("    {0}  {1:N0} bytes  {2}" -f $f.FullName.Substring($dir.Length + 1), $f.Length, $f.LastWriteTime.ToString('yyyy-MM-dd HH:mm'))
        }
    }
    $machineFile = Join-Path $env:LOCALAPPDATA 'OpenLaser\machines\jpt-m7-200.json'
    if (Test-Path -LiteralPath $machineFile) {
        $m = Get-Content -LiteralPath $machineFile -Raw | ConvertFrom-Json
        Say "  machine profile jpt-m7-200: swapXy=$($m.kind.cal.swapXy), scale=$($m.kind.cal.scale -join ' x '), negate=$($m.kind.cal.negate -join ',')"
        if ($m.kind.cal.swapXy) { Good 'the JPT M7 profile has swapXy = true (as on the seller''s machine)' }
        else { Note 'the JPT M7 profile has swapXy = false: the seller''s machine needs true (openlaser-hw profile set cal.swapXy true)' }
    } else {
        Note 'no saved JPT M7 profile yet: the built-in one is used (swapXy = false). This machine needs: openlaser-hw profile set cal.swapXy true'
    }
    $bench = Join-Path $env:LOCALAPPDATA 'OpenLaser\bench-session.json'
    if (Test-Path -LiteralPath $bench) { Say "  bench session file: $(Get-Content -LiteralPath $bench -Raw)" }
}

# ---- Other laser programs ---------------------------------------------------------------------------------------------
Section 'Other laser programs running'
Guard 'Other programs' {
    $others = @(Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.ProcessName -match 'ezcad|lightburn|openlaser' })
    if ($others.Count -eq 0) { Say '  none' }
    foreach ($p in $others) { Say "  $($p.ProcessName) (pid $($p.Id))" }
    if (@($others | Where-Object { $_.ProcessName -match 'ezcad|lightburn' }).Count -gt 0) {
        Note 'EZCAD or LightBurn is running: close it before connecting OpenLaser (only one program can use the board)'
    }
}

# ---- Report ----------------------------------------------------------------------------------------------------------
switch ($boardState) {
    'winusb' { Step 'Driver: nothing to change. Next: start "Start OpenLaser with laser" and connect' }
    'unknown' { Step 'Connect and power the machine, then run this check again' }
}

$report = New-Object System.Collections.Generic.List[string]
$report.Add('OPENLASER DIAGNOSTICS (read-only: nothing on this PC was changed)')
$report.Add('')
$report.Add('SUMMARY')
foreach ($s in $summary) { $report.Add("  $s") }
if ($next.Count -gt 0) {
    $report.Add('')
    $report.Add('NEXT STEP')
    foreach ($s in $next) { $report.Add("  - $s") }
}
$report.Add('')
$report.Add('DETAILS')
foreach ($s in $details) { $report.Add($s) }

$report | Out-File -LiteralPath $OutFile -Encoding utf8
Write-Host ''
Write-Host '---------------------------------------------------------------'
foreach ($s in $report) { if ($s -eq 'DETAILS') { break }; Write-Host $s }
Write-Host ''
Write-Host "Report saved: $OutFile"
if (-not $Quiet) {
    try { Set-Clipboard -Value ($report -join "`r`n"); Write-Host 'The report is also on the clipboard: paste it where you were asked to.' } catch { }
    try { Start-Process notepad.exe -ArgumentList "`"$OutFile`"" } catch { }
}
