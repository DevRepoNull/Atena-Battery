# Atena Battery

> An ultra-lightweight, event-driven Universal Wireless Battery Monitor for Windows, engineered in Rust.

[![CI](https://github.com/DevRepoNull/Atena-Battery/actions/workflows/ci.yml/badge.svg?branch=Rewriting-Rust)](https://github.com/DevRepoNull/Atena-Battery/actions/workflows/ci.yml)
[![Platform](https://img.shields.io/badge/Platform-Windows%2010%20%2F%2011-0078D6.svg?logo=windows)](https://microsoft.com/windows)
[![Rust](https://img.shields.io/badge/Engineered%20in-Rust%202021-DEA584.svg?logo=rust)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Release Size](https://img.shields.io/badge/Binary%20Size-1.09%20MB-brightgreen.svg)](https://github.com/DevRepoNull/Atena-Battery/releases)

---

## Overview

**Atena Battery** is a high-performance, native Windows system tray utility that monitors real-time battery levels across all connected wireless peripherals with an almost invisible system footprint: **2.61 MB RAM**, **~0.0% idle CPU**, and a **1.09 MB single standalone executable** with zero external dependencies.

Unlike legacy utilities that poll radios or wrap web browsers, Atena Battery uses an **event-driven architecture** hooked directly into the Windows Win32 message loop (`WM_DEVICECHANGE`), sleeping with 0 CPU cycles until an actual device event occurs.

---

## Python vs. Rust: Performance & Architecture Comparison

| Metric / Dimension | Atena Battery (Rust) | Legacy Python Prototype | Engineering Advantage |
| :--- | :--- | :--- | :--- |
| **Idle Memory (Working Set)** | **2.61 MB** | ~35.0 – 60.0 MB | **~23× lower RAM footprint** (stack buffers + trim) |
| **Idle CPU Utilization** | **~0.0% (0.00s delta)** | 1.0 – 3.0% (Polling spikes) | **Zero polling loops** (`WM_DEVICECHANGE` event-driven) |
| **Executable Footprint** | **1.09 MB (1,144,832 B)** | ~60.0 MB (PyInstaller bundle) | **Single standalone `.exe`** with zero runtime dependencies |
| **Cold Startup Latency** | **< 20 ms** | 1,500 – 3,000 ms | Instantaneous launch, zero console window flashing |
| **Hardware Compatibility** | **Universal Multi-Protocol** | BLE-only / basic registry | Bluetooth Classic, BLE, 2.4GHz USB HID, XInput Gamepads |
| **Hardware Correlation** | **ContainerId Deduplication** | None (duplicate tray entries) | Unifies composite devices into a single physical entity |
| **Failure Protection** | **Circuit Breaker per MAC** | Unhandled exceptions / hangs | Exponential backoff prevents radio and dongle deadlocks |
| **Automated Testing** | **73 / 73 Unit & Integration** | Zero automated tests | 100% verified test suite with zero clippy warnings |

> *Historical Note:* The original Python prototype and its migration notes are archived in [`oldcode/`](oldcode/README.md) for historical reference.

---

## Supported Wireless Devices

Atena Battery correlates and monitors battery levels across 4 primary wireless categories:

- **Bluetooth Classic Audio Headsets & Speakers:** Sony (WH-1000XM4/XM5, WF-1000XM4), Apple (AirPods, AirPods Pro, AirPods Max), Bose (QuietComfort 35/45/Ultra), Sennheiser (Momentum), JBL, Galaxy Buds, Soundcore, and Marshall (via Win32 PnP cache & RFCOMM HFP AT commands).
- **Bluetooth Low Energy (BLE) Accessories:** Styluses (Surface Pen), wireless mice, keyboards, and presentation remotes (via WinRT GATT Battery Service `0x180F`).
- **2.4GHz Proprietary USB Dongle Peripherals:** Low-latency gaming mice, wireless mechanical keyboards, and headsets using high-speed 2.4GHz RF dongles (Logitech Lightspeed, Razer HyperSpeed, Keychron Wireless, SteelSeries, Corsair Slipstream via HID Battery Strength `0x06:0x20` and `0x84`/`0x85`).
- **Wireless Gamepads:** Xbox Wireless Controllers (Series X/S, Xbox One, Elite Series 2) across controller slots 0 through 4 (via XInput).

---

## 3-Step Quick Start

1. **Download:** Grab the latest `atena-battery-windows-x64.zip` from [GitHub Releases](https://github.com/DevRepoNull/Atena-Battery/releases).
2. **Run:** Extract and double-click `atena-battery.exe`. It launches silently in the background with zero terminal flashing and places a dynamic battery gauge icon in your System Tray next to the clock.
3. **Control:** **Right-click** or click the tray icon to view all connected devices, refresh levels on demand, switch low-battery warning thresholds (10%, 20%, 30%), or toggle "Start with Windows".

---

## Universal 5-Tier Discovery Engine

Atena Battery queries battery levels through a prioritized multi-engine discovery pipeline:

```
[Device Event / PnP Arrival]
            │
            ▼
┌──────────────────────────────────────┐   Available?   ┌──────────────────────────┐
│ Tier 1: Win32 PnP Device Store Cache ├───────────────►│ Return Battery Percentage│
└──────────────────┬───────────────────┘     (YES)      └──────────────────────────┘
                   │ (NO)
                   ▼
┌──────────────────────────────────────┐   Available?   ┌──────────────────────────┐
│ Tier 2: 2.4GHz HID Battery Reports   ├───────────────►│ Return Battery Percentage│
└──────────────────┬───────────────────┘     (YES)      └──────────────────────────┘
                   │ (NO)
                   ▼
┌──────────────────────────────────────┐   Connected?   ┌──────────────────────────┐
│ Tier 3: XInput Gamepad Status (0..4) ├───────────────►│ Return Battery Percentage│
└──────────────────┬───────────────────┘     (YES)      └──────────────────────────┘
                   │ (NO)
                   ▼
┌──────────────────────────────────────┐   Connected?   ┌──────────────────────────┐
│ Tier 4: WinRT BLE GATT Service 0x180F├───────────────►│ Return Battery Percentage│
└──────────────────┬───────────────────┘     (YES)      └──────────────────────────┘
                   │ (NO)
                   ▼
┌──────────────────────────────────────┐   Breaker OK?  ┌──────────────────────────┐
│ Tier 5: RFCOMM Hands-Free Profile AT ├───────────────►│ Return Battery Percentage│
└──────────────────┬───────────────────┘     (YES)      └──────────────────────────┘
                   │ (TRIPPED / FAIL)
                   ▼
┌──────────────────────────────────────┐
│ Circuit Breaker Backoff Isolation    │
└──────────────────────────────────────┘
```

- **Two-Phase Container Correlation:** Multi-interface devices (e.g. a headset presenting both a 2.4GHz wireless dongle and a Bluetooth audio endpoint) are automatically consolidated into one entry via Windows `ContainerId` GUID matching.
- **Precision Precedence:** Fine 1% readings automatically override coarse 10% stepping readings.
- **Smart Discharge Cycle Alerting:** Toast alerts fire **exactly once** per discharge cycle when crossing below the threshold, suppressing duplicate alerts from voltage fluctuations until recharged.

---

## System Tray Controls

- **Tray Icon Indicator:**
  - 🟢 **Green:** Battery level optimal (> 50%)
  - 🟡 **Yellow / Orange:** Moderate battery level (20% – 50%)
  - 🔴 **Red:** Low battery alert state (≤ 20%)
  - ⚪ **Gray:** No active wireless peripherals connected
- **Hover Tooltip:** Shows total connected count and the lowest-battery device.
- **Context Menu:**
  - `[Category] Device Name — Percentage%`
  - `🔄 Refresh Now` (Dispatches an immediate scan across all 5 engines)
  - `⚡ Low Battery Threshold (10% / 20% / 30%)`
  - `🚀 Start with Windows` (Registers native autostart in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`)
  - `📁 Open Logs Folder` (`%APPDATA%\AtenaBattery\logs\`)
  - `❌ Quit Atena Battery`

---

## Documentation

- **[User Guide](docs/user-guide.md):** Complete guide for end users covering tray controls, notification thresholds, autostart, configuration, and hardware troubleshooting.
- **[Developer Guide](docs/developer-guide.md):** Deep-dive into the Cargo Workspace, async traits, Win32 RAII handles, Circuit Breaker state machine, and contributing `crates/provider-linux`.
- **[Performance Report](docs/audits/performance-report.md):** Formal benchmarks, memory working set measurements, and binary breakdown.
- **[Test Report](docs/audits/test-report.md):** Full QA audit report with 73/73 passed test cases.
- **[Contributing](CONTRIBUTING.md):** Contributor guidelines, safety policies, and coding standards.

---

## Building from Source

### Prerequisites
- Windows 10 (version 1809+) or Windows 11 (x64 / ARM64).
- [Rust toolchain](https://rustup.rs/) (1.80+ recommended).

### Build Commands
```powershell
# 1. Clone the repository
git clone -b Rewriting-Rust https://github.com/DevRepoNull/Atena-Battery.git
cd Atena-Battery

# 2. Run the unit and integration test suite (73 tests)
cargo test --workspace

# 3. Verify strict Clippy compliance
cargo clippy --workspace --all-targets -- -D warnings

# 4. Verify documentation build
cargo doc --no-deps --workspace

# 5. Build the optimized stripped release binary
cargo build --release --workspace

# Output executable located at:
# target/release/atena-app.exe (1.09 MB)
```

---

## License

This project is licensed under the [MIT License](LICENSE).
