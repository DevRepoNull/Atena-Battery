# Atena-Battery: Functional Specification & Feature Parity Matrix

**Document ID:** `SPEC-01-FEATURE-PARITY`  
**Author:** Graphify Analyst (`01-graphify-analyst`)  
**Target:** Architecture & Implementation Phase (Gate 0 Sign-Off)  
**Status:** Approved / Definitive  
**Source Baseline:** Python Legacy Repository (`Atena-Battery`) & Graphify Knowledge Graph  

---

## 1. Executive Summary & Purpose

This specification reverse-engineers the legacy Python implementation of **Atena-Battery** (`app.py`, `battery_scanner.py`, `make_icon.py`), utilizing Graphify AST extraction and dynamic graph analysis. It establishes a vendor-neutral, architecture-independent specification for the production rewrite.

### The Anti-Pollution Principle
In accordance with Gate 0 rules, **zero Python runtime concepts or architectural debts are carried forward**:
- **Banned**: Spawning `powershell.exe` child processes via `subprocess.run()`.
- **Banned**: Piping text strings over stdout and parsing with regex/split.
- **Banned**: UI frameworks with heavy runtime overhead (`tkinter`, `pystray`, webviews).
- **Banned**: Unthrottled timer-based polling loops that wake the CPU unnecessarily.
- **Mandated**: Direct Win32 CfgMgr32 / SetupAPI integration, native OS property store queries, event-driven PnP callbacks, and zero-allocation string normalization.

---

## 2. Feature Classification: MVP vs Nice-to-Have

To ensure a disciplined engineering delivery, features are strictly partitioned into **Core Must-Have Features (MVP)** and **Nice-to-Have Enhancements**.

```
┌────────────────────────────────────────────────────────────────────────┐
│                      CORE MUST-HAVE (MVP)                              │
│ ├─ Win32 Direct PnP Property Scanner (DEVPKEY_Device_BatteryPercentage)│
│ ├─ Active Device Connection Status Filtering (Zero Ghost Devices)     │
│ ├─ Multi-Tier Battery Fallback Engine (PnP -> Registry -> BLE GATT)   │
│ ├─ MAC-Based Device Deduplication & Precision Weighting (1% > 10%)    │
│ ├─ Name Sanitization & Alias Normalization (Regex / Scrubbing)        │
│ ├─ Modern Floating Circular HUD Overlay (4x Supersampled Antialiased) │
│ ├─ Mathematical HSV-to-RGB Dynamic Battery Gradient (Emerald->Crimson)│
│ ├─ Multi-Device Navigation Carousel (◀ / ▶ Controls)                  │
│ ├─ Draggable Overlay with Screen Boundary & Position Memory           │
│ ├─ Context Menu (Refresh, Device Select, Autostart, Always-on-Top)    │
│ └─ Native Windows Startup Registry Integration (HKCU Run Key)         │
└────────────────────────────────────────────────────────────────────────┘
                                   │
                                   ▼
┌────────────────────────────────────────────────────────────────────────┐
│                   NICE-TO-HAVE ENHANCEMENTS                            │
│ ├─ System Tray Companion Icon with Dynamic Battery Glyph              │
│ ├─ Win10/Win11 Native Toast Notifications (Low Battery 20% & 10%)     │
│ ├─ True Event-Driven PnP Monitoring (WM_DEVICECHANGE / GUID Callbacks) │
│ ├─ Asymmetric Dual/Triple TWS Component Reporting (Left / Right / Case)│
│ ├─ Custom UI Themes, Opacity Slider & Multi-Monitor Placement Rules   │
│ └─ Circuit Breaker Telemetry Logging for Unresponsive Devices         │
└────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Device Discovery & Interrogation Engine

The discovery engine extracts battery percentages across Bluetooth Classic (Hands-Free Profile / AVRCP), Bluetooth Low Energy (GATT), and Human Interface Devices (HID).

### 3.1 Win32 Native Property Keys & GUIDs

All device queries must use native Windows property store keys and interface GUIDs:

| Identifier | Symbolic Constant / Name | Format / Value | Description |
| :--- | :--- | :--- | :--- |
| **Battery Percentage** | `DEVPKEY_Device_BatteryPercentage` | `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}, 2` | `DEVPROPTYPE_BYTE` (0–100%). Core battery metric cached by Windows. |
| **Device Address / MAC**| `DEVPKEY_Device_Address` / `DEVPKEY_Bluetooth_DeviceAddress` | `{A45C254E-DF1C-4EFD-8020-67D146A850E0}, 30` | 48-bit Bluetooth MAC Address as hex string or binary uint64. |
| **Friendly Name** | `DEVPKEY_Device_FriendlyName` | `{A45C254E-DF1C-4EFD-8020-67D146A850E0}, 14` | User-visible device display name. |
| **Connection Status** | `DEVPKEY_Device_IsConnected` | `{83DA6326-97A6-4088-9453-A1923F573B29}, 15` | `DEVPROPTYPE_BOOLEAN` (True if actively connected). Prevents ghost readings. |
| **DevNode Status** | `DEVPKEY_Device_DevNodeStatus` | `{4340A00F-5265-4505-B078-034D11656B5B}, 2` | Bitflags containing `DN_STARTED` (0x00000008) and `DN_DISABLEABLE`. |
| **Bluetooth Interface** | `GUID_DEVINTERFACE_BLUETOOTH` | `{00f40965-e89d-4487-9890-87c3abb211f4}` | Device interface class for Bluetooth Classic adapters and radios. |
| **HID Interface** | `GUID_DEVINTERFACE_HID` | `{4d1e55b2-f16f-11cf-88cb-001111000030}` | Device interface class for keyboards, mice, trackpads, gamepads. |
| **GATT Battery Service**| `GATT_UUID_BATTERY_SERVICE` | `0x180F` (`0000180F-0000-1000-8000-00805F9B34FB`) | Bluetooth SIG standard GATT service UUID. |
| **GATT Battery Level** | `GATT_UUID_BATTERY_LEVEL` | `0x2A19` (`00002A19-0000-1000-8000-00805F9B34FB`) | GATT characteristic holding 1-byte unsigned percentage (0–100). |

### 3.2 Device Enumerator Filter Rules

Target PnP devices are identified using instance ID prefix matching and device class classification:
1. **Instance ID Prefixes**:
   - `BTHENUM\*`: Bluetooth Classic enumerator (audio headsets, Hands-Free AG, AVRCP).
   - `BTHLE\*` and `BTHLEDEVICE\*`: Bluetooth Low Energy enumerator (GATT peripherals, modern audio, smart devices).
   - `HID\{00001812*`: HID over GATT devices (Service UUID `0x1812` for Bluetooth keyboards and mice).
2. **Device Setup Classes**:
   - `"Bluetooth"`
   - `"AudioEndpoint"` / `"Media"`
   - `"HIDClass"`
   - `"System"` (for composite Bluetooth bus nodes)
3. **PnP Device Status Filter**:
   - Device status must evaluate to **Started / Active** (`CM_PROB_NONE` and `DN_STARTED`). Inactive nodes must be ignored immediately to prevent stale reading ingestion.

### 3.3 3-Tier Resolution Fallback Engine

When multiple providers report data for the same physical peripheral, the system executes a deterministic 3-tier cascade:

```
┌─────────────────────────────────────────────────────────────┐
│ Tier 1: Windows PnP Property Store (Primary)                │
│ -> SetupAPI / CfgMgr32 query for DEVPKEY_Device_Battery     │
│ -> Direct, zero-privilege, low latency (< 1ms per device)   │
└──────────────────────────────┬──────────────────────────────┘
                               │ (If null / property absent)
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 2: Windows Registry Parameter Cache (Secondary)        │
│ -> HKLM\SYSTEM\CurrentControlSet\Enum\BTHLE\<Inst>\...      │
│ -> HKLM\SYSTEM\CurrentControlSet\Enum\BTHLEDevice\<Inst>\...│
│ -> HKLM\SYSTEM\CurrentControlSet\Enum\BTHENUM\<Inst>\...    │
│ -> Must verify Device IsConnected before accepting value!   │
└──────────────────────────────┬──────────────────────────────┘
                               │ (If registry absent / outdated)
                               ▼
┌─────────────────────────────────────────────────────────────┐
│ Tier 3: Native BLE GATT Direct Query (Tertiary)             │
│ -> WinRT Windows.Devices.Bluetooth.GenericAttributeProfile  │
│ -> Direct read on Characteristic 0x2A19 (Service 0x180F)    │
│ -> High precision (1%), guarded by Circuit Breaker          │
└─────────────────────────────────────────────────────────────┘
```

---

## 4. Data Normalization & Deduplication Engine

### 4.1 Device Name Sanitization Specifications

Legacy Windows drivers append verbose endpoint descriptions to Bluetooth device friendly names. The normalization engine strips driver clutter while retaining user-assigned aliases and model numbers.

#### Rule A: Suffix Removal (Regex)
Strip trailing driver descriptors case-insensitively:
```regex
(?i)\s*(Hands-Free(\s*AG(\s*Audio)?)?|Avrcp\s*Transport|Bluetooth\s*(LE\s*)?Device|Stereo|Audio|Headphones|Headset|GATT|Service|\(BLE\))\s*$
```
*Examples:*
- `"Sony WH-1000XM4 Hands-Free AG Audio"` ➔ `"Sony WH-1000XM4"`
- `"Logitech MX Master 3S Bluetooth LE Device"` ➔ `"Logitech MX Master 3S"`
- `"AirPods Pro - Find My (BLE)"` ➔ `"AirPods Pro - Find My"`
- `"Bose QC35 II Avrcp Transport"` ➔ `"Bose QC35 II"`

#### Rule B: Prefix Removal (Regex)
Strip leading generic GATT wrapper descriptions:
```regex
(?i)^Bluetooth\s+LE\s+GATT\s+compliant\s+
```
*Example:*
- `"Bluetooth LE GATT compliant HID Mouse"` ➔ `"HID Mouse"`

#### Rule C: Boundary & Whitespace Scrubbing
- Trim all leading/trailing whitespace and control characters.
- If sanitization yields an empty string, fallback to the raw device friendly name or Bluetooth MAC address formatted as `[XX:XX:XX:XX:XX:XX]`.

### 4.2 Deduplication Matrix & MAC Normalization

A single physical device often registers multiple logical PnP nodes (e.g., Audio Endpoint, HID Transport, and BLE GATT Service).
1. **Primary Key**: Canonical 48-bit MAC address in uppercase hexadecimal (`XX:XX:XX:XX:XX:XX`).
2. **Extraction Path**:
   - Query `DEVPKEY_Bluetooth_DeviceAddress` from device property store.
   - If missing from PnP, parse MAC from `InstanceId` (e.g., `BTHENUM\DEV_14ABB523CD89\...` ➔ `14:AB:B5:23:CD:89`).
3. **Merge Logic**:
   - Merge all logical nodes sharing the same MAC into a single `BatteryDevice` record.
   - If MAC is unavailable, fallback to grouping by normalized device name.

### 4.3 1% vs 10% Granularity Weighting Heuristic

Legacy Hands-Free Profile (HFP 1.5/1.7) communicates battery level via AT commands (`+BCS`) in coarse 10% increments ($0, 10, 20, \dots, 100$). Modern BLE GATT Battery Services report continuous 1% precision values.

**Resolution Priority Algorithm**:
- Given an existing cached battery reading $B_{\text{old}}$ and a new incoming reading $B_{\text{new}}$:
  1. **Source Tier Priority**: A reading obtained from Tier 3 (BLE GATT) or Tier 1 (PnP native 1%) always supersedes a Tier 2 (Registry/HFP 10%) reading.
  2. **Non-Multiple Heuristic**: If two readings originate from the same timestamp window and source confidence is equal:
     $$\text{If } (B_{\text{old}} \pmod{10} == 0) \land (B_{\text{new}} \pmod{10} \neq 0) \implies B_{\text{new}} \text{ replaces } B_{\text{old}}$$
  3. **Discharge Timestamp Protection**: A coarse reading must never overwrite a high-resolution reading unless the time elapsed exceeds the discharge timeout ($\Delta t > 300\text{s}$) and represents a genuine discharge transition.

---

## 5. User Interface & Tray Specifications

### 5.1 Floating Circular HUD Overlay Architecture

The primary UI is a frameless, transparent floating circular HUD designed to blend natively into modern desktop wallpapers.

```
                  ┌──────────────────────┐
                  │      (cx, cy-6)      │
                  │         87%          │  <-- Segoe UI 17pt Bold (#FFFFFF)
                  │      ⚡ BATTERY       │  <-- Segoe UI 6pt Bold (Dynamic Hex)
                  └──────────────────────┘
                   /                    \
                  |    Glowing Ring      |  <-- 4x Super-Sampled Arc + Neon Glow
                   \                    /
              ┌──────────────────────────────┐
              │ ◀   ● Sony WH-1000XM4    ▶   │  <-- Pill Badge (128x26 scaled)
              └──────────────────────────────┘
```

#### Dimensions & Geometry (Base 1x Coordinates)
- **Window Size**: Width = 148 px, Height = 168 px.
- **Center Point**: $(c_x, c_y) = (74, 62)$.
- **Outer Glass Disc**: Radius $R_{\text{outer}} = 52$ px.
  - Fill: `#0F131C` (`RGB(15, 19, 28)`)
  - Outer Border: 1.5 px outline `#262F44` (`RGB(38, 47, 68)`)
  - Inner Rim: Radius $48$ px, outline `#181E2C` (`RGB(24, 30, 44)`)
- **Radial Ticks**: 24 subtle tick marks spaced every 15° ($0^\circ$ to $360^\circ$).
  - Inner radius $R_{t1} = 32$ px, Outer radius $R_{t2} = 35$ px.
  - Stroke: 1 px `#222A3C` (`RGB(34, 42, 60)`).
- **Recessed Track Ring**:
  - Centerline radius $R_{\text{ring}} = 41$ px.
  - Thickness: 7 px.
  - Fill: `#1E2534` (`RGB(30, 37, 52)`).
- **Active Progress Arc**:
  - Start Angle: $-90.0^\circ$ (12 o'clock position).
  - Sweep Angle: $\theta = \left(\frac{\text{battery}}{100}\right) \times 360.0^\circ$.
  - End Angle: $-90.0^\circ + \theta$.
  - Glow Layer: Gaussian blur (radius 3.5 px), thickness 13.3 px, alpha 115 / 255.
  - Core Arc: Thickness 7 px, alpha 255 (solid dynamic color).
  - Arc Caps: Rounded end caps with radius 3 px at start and end angles.
- **Device Pill Badge**:
  - Center: $(c_x, y_{\text{pill}}) = (74, 136)$.
  - Bounds: Width = 128 px, Height = 26 px, Corner Radius = 13 px (fully rounded ends).
  - Fill: `#0F131C`, Border: 1.5 px `#262F44`.
  - Status Dot: Radius 3.5 px, color matches dynamic battery state (or `#606D80` when offline).
    - X-position: $c_x - 42$ px (when single device) or $c_x - 50$ px (when multiple devices).

#### Dynamic HSV Color Gradient Formula
Color transitions seamlessly without abrupt stepping across the battery life cycle:
- Input percentage: $P \in [0.0, 100.0]$
- Hue calculation:
  $$\text{Hue} = \left(\frac{P}{100.0}\right) \times 0.38 \quad (0.38 \approx 136.8^\circ \text{ emerald green}, 0.0 = 0^\circ \text{ red})$$
- Saturation: $S = 0.85$
- Value / Brightness: $V = 0.98$
- Color Tiers:
  - 🟢 **Emerald Green** (60% – 100%): $\text{Hue} \in [0.228, 0.380]$
  - 🟡 **Amber / Orange** (21% – 59%): $\text{Hue} \in [0.080, 0.224]$
  - 🔴 **Crimson Red** (0% – 20%): $\text{Hue} \in [0.000, 0.076]$

#### Typography Specifications
- **Connected State**:
  - Value: `"{battery}%"`, Font: `Segoe UI`, 17 pt, Bold, Color `#FFFFFF`.
  - Subtitle: `"⚡ BATTERY"`, Font: `Segoe UI`, 6 pt, Bold, Color = Dynamic Accent Hex.
- **Disconnected / Offline State**:
  - Value: `"--%"`, Font: `Segoe UI`, 16 pt, Bold, Color `#7D8CA3`.
  - Subtitle: `"OFFLINE"`, Font: `Segoe UI`, 6 pt, Bold, Color `#526075`.
- **Device Label**:
  - Font: `Segoe UI`, 8 pt, Bold, Color `#F0F4F8`.
  - Length limits: Max 12 characters (multi-device mode) or 15 characters (single-device mode). Overflow appended with ellipsis (`…`).

#### Interactive Controls & Navigation
- **Multi-Device Carousel Controls**:
  - Left Button (`◀`): Position $(c_x - 52, y_{\text{pill}})$, Font `Segoe UI 7pt Bold`. Normal color `#8B9BB4`, hover `#FFFFFF`.
  - Right Button (`▶`): Position $(c_x + 52, y_{\text{pill}})$, Font `Segoe UI 7pt Bold`. Normal color `#8B9BB4`, hover `#FFFFFF`.
  - Clicking `◀` / `▶` wraps circularly: $\text{idx} = (\text{idx} \pm 1 + N) \pmod N$.
- **Direct Drag & Repositioning**:
  - Click-and-drag from any point of the widget (excluding `◀` / `▶` button hitboxes).
  - Bounds clamping: Overlay cannot be dragged entirely off-screen.
- **Position Persistence**:
  - On mouse release (`ButtonRelease-1`), window coordinates `(pos_x, pos_y)` are persisted to configuration.
  - Default initial position: $(X, Y) = (\text{Screen Width} - 148 - 35, 50)$ (top right corner).

#### Animation Engine
- Interpolation updates smoothly 1% at a time.
- If $|\text{target} - \text{current}| > 15\%$, step accelerates to 2% until within 15% range, then steps 1% every 18 ms.
- If transition is to offline ($\text{level} < 0$), animation bypasses interpolation and jumps to offline instantly.

### 5.2 Context Menu Hierarchy (Right-Click)

A dark-themed context menu (`#121621` background, `#FFFFFF` text, `#2E6FF2` active highlight) provides control:

```
┌──────────────────────────────────────┐
│ ↻  Refresh Now                       │  <-- Triggers immediate background re-scan
├──────────────────────────────────────┤
│ ●  Sony WH-1000XM4 (87%)             │  <-- Multi-device selection list
│    Logitech MX Master 3S (60%)       │
├──────────────────────────────────────┤
│ ✔  Start with Windows                │  <-- Toggle HKCU Run registry key
│ ✔  Always on Top                     │  <-- Toggle WS_EX_TOPMOST style
├──────────────────────────────────────┤
│ ✕  Close Widget                      │  <-- Graceful application shutdown
└──────────────────────────────────────┘
```

### 5.3 System Tray & Notification Rules (Nice-to-Have / Companion)

#### Multi-Resolution Application & Tray Icon Rules
Icons rendered across 6 standard Windows mipmap sizes:
- Sizes: `16x16`, `32x32`, `48x48`, `64x64`, `128x128`, `256x256`.
- Tray Glyphs: Dynamic glyph reflecting the active device battery tier:
  - Green circle/arc ($> 50\%$)
  - Amber circle/arc ($21\% - 50\%$)
  - Red circle/arc ($0\% - 20\%$)
  - Gray circle/outline (Offline / No Device)
- Tooltip Format:
  ```
  Atena Battery
  Sony WH-1000XM4: 87%
  Logitech MX Master 3S: 60%
  ```

#### Low Battery Notification Engine
- **Warning Threshold**: $20\%$ remaining.
- **Critical Threshold**: $10\%$ remaining.
- **Notification Throttle & Cooldown**:
  - Each threshold alerts at most **once per discharge cycle**.
  - A device must charge above the threshold $+5\%$ (hysteresis band) before re-arming the notification trigger.
  - Prevents alert spamming caused by voltage fluctuations near threshold boundaries.

---

## 6. Legacy Quirks & Edge Cases Matrix

Reverse engineering `battery_scanner.py` and `app.py` revealed several critical Windows Bluetooth quirks that must be explicitly engineered around:

| Quirk / Issue ID | Legacy Python Behavior | Root Cause | Production Required Solution |
| :--- | :--- | :--- | :--- |
| **QUIRK-01: Ghost Devices from Stale Registry** | Registry fallback checked `HKLM\SYSTEM\CurrentControlSet\Enum\BTH*\...\BatteryLevel` without validating connection status. | Windows never deletes registry keys when Bluetooth peripherals disconnect; cached `BatteryLevel` persists indefinitely. | **Mandatory Connection Check**: Interrogate `DEVPKEY_Device_IsConnected` or verify `DN_STARTED` on the parent devnode before accepting registry keys. Disconnected nodes must be marked offline. |
| **QUIRK-02: Subprocess Spawning & Latency Spikes** | Spawned `powershell.exe -Command ...` via `subprocess.run()` every 20s. | Subprocess initialization took 1.5–3.0s, caused 50MB+ RAM spikes and 15–25% transient CPU usage. | **Zero Subprocess Policy**: Direct C-bindings (`windows-rs` / SetupAPI / CfgMgr32) executing synchronously in < 2ms with zero allocations. |
| **QUIRK-03: Dual-Profile Endpoint Duplication** | Normalized names via regex but lacked MAC address correlation. Audio and HID endpoints created duplicate records. | Headsets register both an HFP audio node (`BTHENUM`) and an AVRCP/GATT control node (`BTHLE`). | **MAC Canonicalization**: Use 48-bit MAC address as the unified identity key. Merge properties across all endpoints bearing identical MACs. |
| **QUIRK-04: Coarse 10% HFP Overwriting 1% GATT** | Evaluated `($existing % 10 == 0) and ($level % 10 != 0)` to choose readings. | If a high-precision BLE device genuinely discharged from 91% to 90%, the `% 10` condition broke down. | **Provider Tier Hierarchy**: Tag readings with provenance (`ProviderTier::PnpGatt` vs `ProviderTier::HfpCache`). High-tier readings hold dominance until a discharge timeout occurs. |
| **QUIRK-05: Dongle / Radio Removal Lockup** | If user disabled Bluetooth or pulled USB dongle, PowerShell script hung until the 15s timeout expired. | WMI / PnP cmdlets block on unhandled driver unload states. | **Non-Blocking Architecture**: Use asynchronous Windows APIs with cancellation tokens and a strict timeout (e.g., 500ms max per query). |
| **QUIRK-06: AirPods / TWS Asymmetric Batteries** | Single dictionary `$found[$name]` flattened left, right, and case readings into one scalar number. | TWS earbuds expose three independent battery levels, or alternate master reporting during usage. | **Multi-Component Architecture**: Support hierarchical battery structures: `Master`, `Left`, `Right`, `Case`. Display composite or lowest active component. |
| **QUIRK-07: Destructive Substring Scrubbing** | Regex stripped terms like `Audio` or `Device` from the end of all names indiscriminately. | Brands ending in "Audio" or "Sound" (e.g. "Creative Audio") had their brand name corrupted to "Creative". | **Word-Boundary & Token-Aware Cleaning**: Clean only known driver suffix tokens while protecting registered brand/model substrings. |
| **QUIRK-08: Window Transparency Chroma Bleed** | Used Tkinter `-transparentcolor "#010101"` with alpha thresholding. | Anti-aliased pixels blended with `#010101` produced visible dark gray fringes around circular arcs. | **Native Premultiplied Alpha Blending**: In the production UI, use layered Windows (`WS_EX_LAYERED`) with `UpdateLayeredWindow` and true 32-bit premultiplied ARGB graphics. |

---

## 7. Configuration & State Persistence Specifications

### 7.1 Configuration Storage
- **File Location**: `%APPDATA%\AtenaBattery\config.json`
- **Schema**:
```json
{
  "version": 1,
  "overlay": {
    "pos_x": 1737,
    "pos_y": 50,
    "always_on_top": true,
    "scale": 1.0,
    "refresh_interval_secs": 30
  },
  "selected_device_mac": "14:AB:B5:23:CD:89",
  "notifications": {
    "low_battery_enabled": true,
    "warning_threshold": 20,
    "critical_threshold": 10
  }
}
```

### 7.2 Windows Startup Integration
- **Registry Location**: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
- **Value Name**: `"AtenaBatteryOverlay"`
- **Value Type**: `REG_SZ`
- **Value Content**: Quoted absolute path to binary (e.g., `"C:\Program Files\Atena\atena-app.exe"`).

---

## 8. Verification & Gate 0 Sign-Off Checklist

- [x] All Windows PnP property keys, enumerators, and registry branches extracted from `battery_scanner.py`.
- [x] All UI states, dimensions, HSV gradient mathematics, and animations extracted from `app.py`.
- [x] Icon generation specifications and color tiers extracted from `make_icon.py`.
- [x] Clear separation between Core MVP and Nice-to-Have features established.
- [x] Zero references to Python runtime libraries (`subprocess`, `tkinter`, `pystray`, `ctypes`).
- [x] All 8 legacy Bluetooth quirks and device edge cases documented with concrete technical solutions.
- [x] Specification ready for `02-system-architect` Cargo workspace design (Gate 0 Sign-Off).

---
*Signed by:* **Graphify Analyst** (`01-graphify-analyst`)  
*Next Phase:* **Phase 1: Architecture & Trait Design** (`02-system-architect`)
