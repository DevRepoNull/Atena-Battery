# Atena Battery: User Guide

This guide provides end-to-end instructions for installing, configuring, operating, and troubleshooting **Atena Battery** on Windows 10 and Windows 11.

---

## 1. Quick Start (Zero-Install Execution)

Atena Battery is distributed as a single standalone executable requiring **no installer**, no administrator privileges, and no background runtimes:

1. **Download:** Obtain `atena-battery-windows-x64.zip` from the latest [GitHub Releases](https://github.com/AtenaBattery/Atena-Battery/releases).
2. **Extract & Run:** Extract `atena-battery.exe` to a folder of your choice (e.g. `C:\Users\<User>\bin\` or your portable tools folder) and double-click to launch.
3. **Tray Interaction:** The application starts silently with zero console window flashing and places a dynamic battery gauge icon in your Windows Notification Area (System Tray), located next to the clock in the bottom-right corner of your taskbar.

---

## 2. System Tray Interface

### Tray Icon Colors & Gauge Indicators
The tray icon dynamically renders the battery gauge of your primary (or lowest-battery) connected device:

| Icon Indicator | Status Range | Description |
| :---: | :---: | :--- |
| 🟢 **Green** | **51% – 100%** | Battery level is optimal. |
| 🟡 **Yellow / Orange** | **20% – 50%** | Battery level is moderate. |
| 🔴 **Red** | **0% – 19%** | Battery level is critical; recharging is recommended. |
| ⚪ **Gray** | **Disconnected** | No wireless devices are connected or reporting battery. |

### Tooltip Status Summary
Hovering your mouse cursor over the system tray icon displays an instantaneous status tooltip without opening the menu:

- **Single Connected Device**:
  ```text
  Atena Battery: Sony WH-1000XM4 (70%)
  ```
- **Multiple Connected Devices**:
  ```text
  Atena Battery: 3 device(s) connected
  Lowest: Logitech G Pro Wireless (18%)
  ```
- **No Active Devices**:
  ```text
  Atena Battery: No devices connected
  ```

---

## 3. Context Menu Navigation

**Click** or **Right-Click** the system tray icon to reveal the context menu:

```text
+---------------------------------------------+
|  🟢 [Audio] Sony WH-1000XM4 — 70%            |
|  🟡 [Mouse] Logitech G Pro X — 35%          |
|  🟢 [Gamepad] Xbox Controller — 90%         |
|  -----------------------------------------  |
|  🔄 Refresh Now                             |
|  ⚡ Low Battery Threshold                ►  |
|  🚀 Start with Windows                 [✓]  |
|  📁 Open Logs Folder                        |
|  -----------------------------------------  |
|  ❌ Quit Atena Battery                      |
+---------------------------------------------+
```

### Context Menu Actions Explained

1. **Active Device Listing**:
   - Informational entries grouped by category (`[Audio]`, `[Mouse]`, `[Keyboard]`, `[Gamepad]`, `[Peripheral]`), displaying real-time battery percentages and health indicators.
2. **🔄 Refresh Now**:
   - Manually triggers an immediate query across all 5 discovery engines (Win32 PnP Cache, 2.4GHz HID, XInput, WinRT BLE GATT, and RFCOMM HFP AT) to refresh peripheral levels on demand.
3. **⚡ Low Battery Threshold Submenu**:
   - Allows choosing the warning alert threshold:
     - **10% (Critical Alert)**: For users who prefer late warnings.
     - **20% (Default Recommendation)**: Optimal balance for daily peripheral charging.
     - **30% (Early Warning)**: Ideal for long gaming sessions or remote work.
   - Selecting any option immediately updates your configuration and saves it atomically to disk.
4. **🚀 Start with Windows Toggle**:
   - Toggles automatic startup upon Windows user login.
   - When checked, Atena Battery writes a clean entry to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`. Unchecking cleanly removes the registry key. Does **not** require administrator privileges.
5. **📁 Open Logs Folder**:
   - Opens the rolling log directory `%APPDATA%\AtenaBattery\logs\` in Windows File Explorer.
6. **❌ Quit Atena Battery**:
   - Cleanly closes the application, unregisters Win32 PnP device notification listeners, flushes logs to disk, and frees all memory.

---

## 4. Native Windows Toast Notifications

Atena Battery generates native Windows 10/11 Toast notifications when any connected wireless device drops to or below your configured alert threshold.

### Smart Discharge Cycle Tracking (Anti-Spam)
Unlike basic polling scripts that trigger repetitive notification sounds every few minutes, Atena Battery features an integrated state machine:
- **Fires Once Per Discharge**: When a device drops to or below the alert threshold (e.g. 20%), a notification is dispatched **once**.
- **Suppressed on Voltage Fluctuations**: Minor voltage drops or rebounds (e.g. 19% $\leftrightarrow$ 22%) will **not** trigger duplicate alerts.
- **Automatic Re-Arming with Hysteresis**: The notification system re-arms **only** after the peripheral is charged above the threshold plus a 5% margin (e.g., above 25% for a 20% threshold).
- **Subsequent Cycle Alert**: Once recharged and later discharged back below the threshold, the notification will fire again for the new cycle.

---

## 5. Configuration File (`config.toml`)

Your preferences are automatically saved in:
```text
%APPDATA%\AtenaBattery\config.toml
```
*(Typically located at `C:\Users\<YourUsername>\AppData\Roaming\AtenaBattery\config.toml`)*

### Configuration Schema Reference
```toml
# Atena Battery Configuration File
version = 1
selected_device_mac = ""

[notifications]
low_battery_enabled = true
warning_threshold = 20
critical_threshold = 10

[overlay]
pos_x = 1737
pos_y = 50
always_on_top = true
scale = 1.0
refresh_interval_secs = 30
```

- **Atomic File Writing**: All updates are written to a temporary file (`config.toml.tmp`) first and renamed into place, guaranteeing zero risk of file corruption during unexpected power losses or system reboots.

---

## 6. Diagnostic Logs

Atena Battery records non-blocking rolling logs inside:
```text
%APPDATA%\AtenaBattery\logs\atena-battery.log.YYYY-MM-DD
```

### Log File Characteristics
- **Daily Rotation**: A new log file is automatically generated each day, avoiding unbounded disk growth.
- **Privacy & Security**: Logs contain only device names, sanitized hardware IDs, battery levels, and error codes. No private personal data or keystrokes are ever captured.
- **Non-Blocking Execution**: File writing runs asynchronously on a dedicated worker thread via `tracing-appender` without causing UI stutter or message loop delays.

---

## 7. Hardware Troubleshooting Guide

### A. Device Is Connected but Battery Reading Does Not Appear
1. **Peripheral Sleep Mode**: 2.4GHz wireless mice and keyboards often turn off reporting radios after 3–5 minutes of inactivity to conserve battery. Move the mouse or press a key to awaken the device; the battery level will appear within 1–2 seconds.
2. **Older Classic Bluetooth Headsets**: Older Bluetooth headsets might not publish their battery level to the Windows PnP store immediately upon pairing. Click **Refresh Now** in the tray menu to dispatch a Tier 5 RFCOMM HFP query.
3. **Third-Party USB Bluetooth Dongle Drivers (CSR8510 / Broadcom BCM20702)**:
   - Inexpensive USB Bluetooth dongles often ship with outdated mini-CD software (such as BlueSoleil or Harmony Stack) that bypasses standard Windows Bluetooth APIs.
   - Open **Device Manager**, expand **Bluetooth**, and verify the adapter is using Microsoft's standard `Generic Bluetooth Adapter` driver.
   - Uninstall third-party Bluetooth management software to enable standard Windows SetupAPI queries.

### B. Xbox Controller Battery Missing
- Verify the controller is connected wirelessly (via the official Xbox Wireless Adapter or Bluetooth) and displays a solid Xbox guide button light.
- Wired USB controllers do not provide battery readings over XInput and are filtered out intentionally.

### C. Toast Alerts Not Appearing
- Open Windows **Settings** $\rightarrow$ **System** $\rightarrow$ **Notifications**.
- Verify that notifications are set to **On** and that **Atena Battery** is allowed to send notifications.
- Ensure Windows **Focus Assist** / **Do Not Disturb** is disabled.
