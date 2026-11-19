# Legacy Python Prototype (Archived)

> **Notice:** The code and scripts in this directory are preserved strictly as a historical reference for the original Python prototype of **Atena Battery**.

This Python prototype is **no longer maintained**, supported, or developed.

### Migration Notice
Atena Battery has been completely re-engineered from the ground up in **Rust** as a high-performance, native Windows system utility with:
- **~25× Lower Memory**: 2.61 MB RAM (compared to ~45–80 MB in Python).
- **Zero Polling Overhead**: Pure Win32 event-driven architecture (~0.0% idle CPU).
- **Universal Multi-Protocol Support**: Bluetooth Classic (PnP + RFCOMM AT), Bluetooth LE (GATT), 2.4GHz USB HID dongles, and Xbox Gamepads (XInput).
- **Single Native Executable**: 1.09 MB portable binary without Python runtime or interpreter dependencies.

Please refer to the root [README.md](../README.md) for current production documentation, build instructions, and release downloads.
