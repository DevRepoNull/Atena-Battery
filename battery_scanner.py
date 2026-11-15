"""
Bluetooth Battery Scanner for Windows.
Queries Windows PnP and Registry for connected Bluetooth devices and their battery status.
"""

import subprocess
import logging

# Set up logging
logging.basicConfig(level=logging.INFO, format="%(asctime)s [%(levelname)s] %(message)s")

POWERSHELL_SCRIPT = r"""
$ErrorActionPreference = 'SilentlyContinue'
$batteryKey = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2"
$devices = Get-PnpDevice -Class "System","Bluetooth" -Status "OK" | Where-Object { $_.InstanceId -like "*BTHENUM*" }

$found = @{}

# 1. PnP Property for Bluetooth Audio / Hands-Free devices (e.g. QCY, AirPods, Sony, JBL)
foreach ($dev in $devices) {
    $prop = Get-PnpDeviceProperty -InstanceId $dev.InstanceId -KeyName $batteryKey
    if ($prop -and $null -ne $prop.Data -and $prop.Data -ge 0 -and $prop.Data -le 100) {
        $name = $dev.FriendlyName -replace '(?i)\s+(Hands-Free\s*(AG)?|Avrcp\s*Transport|Bluetooth\s*Device)$', ''
        if (-not [string]::IsNullOrWhiteSpace($name) -and -not $found.ContainsKey($name)) {
            $found[$name] = [int]$prop.Data
            Write-Output "$name|$($prop.Data)"
        }
    }
}

# 2. Registry Fallback for HID Peripherals (Mice, Keyboards, Controllers)
$regPaths = @("HKLM:\SYSTEM\CurrentControlSet\Enum\BTHENUM")
foreach ($rp in $regPaths) {
    if (Test-Path $rp) {
        Get-ChildItem $rp | ForEach-Object {
            Get-ChildItem $_.PSPath | ForEach-Object {
                $p = Get-ItemProperty $_.PSPath
                if ($p -and $null -ne $p.BatteryLevel -and $p.BatteryLevel -ge 0 -and $p.BatteryLevel -le 100) {
                    $devName = if ($p.FriendlyName) { $p.FriendlyName } else { $_.PSChildName }
                    if (-not $found.ContainsKey($devName)) {
                        $found[$devName] = [int]$p.BatteryLevel
                        Write-Output "$devName|$($p.BatteryLevel)"
                    }
                }
            }
        }
    }
}
"""


def scan_devices():
    """
    Executes PowerShell script in background (no console window)
    and returns a list of dictionaries: [{'name': '...', 'battery': 70}, ...]
    """
    try:
        # Hide command prompt window on Windows
        startupinfo = None
        creationflags = 0
        import os
        if os.name == 'nt':
            startupinfo = subprocess.STARTUPINFO()
            startupinfo.dwFlags |= subprocess.STARTF_USESHOWWINDOW
            startupinfo.wShowWindow = subprocess.SW_HIDE
            creationflags = subprocess.CREATE_NO_WINDOW

        proc = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", POWERSHELL_SCRIPT],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            startupinfo=startupinfo,
            creationflags=creationflags,
            timeout=15
        )

        results = []
        if proc.returncode == 0 and proc.stdout:
            for line in proc.stdout.strip().splitlines():
                line = line.strip()
                if not line or "|" not in line:
                    continue
                parts = line.split("|", 1)
                if len(parts) == 2:
                    name = parts[0].strip()
                    try:
                        battery = int(parts[1].strip())
                        results.append({"name": name, "battery": battery})
                    except ValueError:
                        continue

        return results

    except Exception as e:
        logging.error(f"Error scanning Bluetooth devices: {e}")
        return []


if __name__ == "__main__":
    print("Scanning Bluetooth devices...")
    devs = scan_devices()
    print("Found devices:", devs)
