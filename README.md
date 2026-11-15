# 🔵 Atena Battery

<p align="center">
  <img src="app_icon.ico" width="80" height="80" alt="Bluetooth Battery Overlay Icon" />
</p>

<p align="center">
  <strong>A lightweight, frameless floating circular desktop widget for Windows that displays real-time battery levels for connected Bluetooth peripherals and audio devices.</strong>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Platform-Windows-0078D6?style=flat&logo=windows&logoColor=white" alt="Windows" />
  <img src="https://img.shields.io/badge/Python-3.10+-3776AB?style=flat&logo=python&logoColor=white" alt="Python" />
  <img src="https://img.shields.io/badge/Dependencies-Zero-26D07C?style=flat" alt="Zero Dependencies" />
  <img src="https://img.shields.io/badge/License-MIT-blue.svg?style=flat" alt="License" />
</p>

---

## 🌟 Overview

Windows often buries Bluetooth device battery percentages deep within the Settings panel. **Atena Battery** brings that information directly to your desktop as a sleek, minimalist floating HUD ring.

Whether you use Bluetooth earbuds (AirPods, QCY, Sony, Galaxy Buds), wireless headphones, mice, keyboards, or game controllers, this overlay keeps track of your remaining charge with a color-coded gauge that changes smoothly from green to yellow to red.

---

## ✨ Features

- **🎯 Accurate Windows Battery Retrieval**:
  - Interrogates Windows Plug and Play (`PnP`) properties for Bluetooth audio devices (**Hands-Free AG / AVRCP profiles**).
  - Queries the Windows Registry for HID peripherals (**Bluetooth Keyboards, Mice, Gamepads**).
- **🎨 Dynamic Circular Progress Gauge**:
  - Smooth HSV-to-RGB color gradient calculated mathematically based on battery percentage:
    - 🟢 **Emerald Green** (60% – 100%)
    - 🟡 **Amber / Orange** (21% – 59%)
    - 🔴 **Crimson Red** (0% – 20%)
  - Center readout with crisp bold percentage.
- **🪟 True Transparent Floating Overlay**:
  - 100% borderless and transparent background — only the circular gauge and device badge float on your desktop wallpaper.
  - **Always-on-Top**: Stays visible across work sessions (toggleable via menu).
  - **Draggable**: Click and drag from anywhere on the widget to reposition it freely.
  - **Position Memory**: Automatically remembers where you placed it and restores its position on reboot.
- **🚀 Auto-Start with Windows**:
  - One-click toggle in the right-click menu to register/unregister the app in Windows Startup.
- **⚡ Zero External Dependencies for Runtime**:
  - Written purely with Python standard libraries (`tkinter`, `subprocess`, `threading`, `colorsys`, `winreg`).
  - No bloated webview or heavy frameworks needed.
- **🔄 Asynchronous Non-Blocking Engine**:
  - Background daemon thread auto-refreshes every 30 seconds.
  - Double-click widget or use right-click menu to refresh instantly without GUI stutter.
- **🎧 Multi-Device Support**:
  - Cycle through multiple connected devices using `◀` and `▶` controls.

---

## 📸 Preview

```
       .---.
     /   90% \        <- Dynamic colored arc
    |  BATTERY|       <- Percentage readout
     \       /
       '---'
   [ ● Atena Battery ]    <- Compact device pill directly below
```

---

## 🚀 Quick Start

### Option 1: Standalone Portable Executable (No Python Required)
1. Download `Atena_Battery.exe` from the latest release or build it yourself.
2. Run `Atena_Battery.exe`.
3. Right-click the widget and check **✔ Start with Windows** to keep it running automatically.

### Option 2: Running from Source
Ensure Python 3.10+ is installed on Windows:

```bash
# Clone the repository
git clone https://github.com/yourusername/Atena-Battery.git
cd Atena_Battery

# Run directly
python app.py
```

*Tip: You can also double-click `run.bat` to launch the application silently in the background with `pythonw` (without opening a terminal console).*

---

## 🖱️ Controls & Shortcuts

| Action | Function |
| :--- | :--- |
| **Left Click + Drag** | Move widget to any location on your screen |
| **Double Click** | Trigger an immediate battery re-scan |
| **Right Click** | Open Context Menu (Refresh, Start with Windows, Always on Top, Close) |
| **Arrow Buttons (`◀` / `▶`)** | Switch between multiple connected Bluetooth devices |

---


## 📁 Repository Structure

```
Atena_Battery/
├── app.py                # Main application UI, canvas drawing, and event loop
├── battery_scanner.py    # Windows PnP and registry battery detection module
├── make_icon.py          # Icon generator script (multi-resolution .ico)
├── app_icon.ico          # Application icon
├── run.bat               # Windows batch launcher (runs silently with pythonw)
├── requirements.txt      # Optional dependencies for building the executable
└── README.md             # Project documentation
```

---

## ⚙️ How It Works

Windows stores Bluetooth audio battery data differently depending on the device's driver and profile:
1. **Hands-Free Profile (HFP / AVRCP)**: Earphones and headsets report battery events via AT commands, which Windows caches inside the Plug-and-Play property store (`DEVPKEY_Device_BatteryLevel` / `{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2`). `battery_scanner.py` queries these properties asynchronously.
2. **HID Peripherals**: Standard Bluetooth mice and keyboards typically store their power states under `HKLM\SYSTEM\CurrentControlSet\Enum\BTHENUM`. A fast registry fallback parses these keys.

---

## 📜 License

This project is licensed under the [MIT License](LICENSE). Feel free to use, modify, and distribute.
