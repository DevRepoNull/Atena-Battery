# Contributing to Atena Battery

Thank you for contributing to **Atena Battery**! This project is engineered with high standards for systems programming, memory safety, and performance.

---

## 1. Development Principles

All contributions must adhere to the following rules:

1. **Zero `unwrap()` / `expect()` Policy**:
   - Production code (`crates/*/src/`) must contain **zero** instances of `.unwrap()` or `.expect()`.
   - All errors must be propagated using `Result<T, ProviderError>` or handled with fallback options.
2. **Mandatory `// SAFETY:` Invariant Comments**:
   - Every `unsafe` block must be immediately preceded by a structured `// SAFETY:` comment proving:
     1. Pointer validity and non-null guarantees.
     2. Proper alignment and size matching of allocated buffers.
     3. Complete RAII cleanup of any returned handles under all code paths, including early returns (`?`).
3. **Single-Threaded Concurrency**:
   - Do not introduce thread pools or multi-threaded runtimes into `crates/app`. Atena Battery runs on Tokio's `current_thread` runtime to maintain thread affinity for Win32 message queues.
4. **Strict Resource Budgets**:
   - Stripped release binary size must remain **< 3.0 MB** (currently 1.09 MB).
   - Background idle memory must remain **< 5.0 MB RAM** (currently 2.61 MB).
   - Idle CPU utilization must remain **~0.0%**.

---

## 2. Tutorial: How to Implement a New `BatteryProvider`

To add a new platform provider (e.g. Linux BlueZ, macOS IOKit, or a custom hardware driver):

### Step 1: Implement the `BatteryProvider` Trait
In your crate, implement `atena_core::BatteryProvider`:

```rust
use async_trait::async_trait;
use atena_core::{BatteryInfo, BatteryProvider, DeviceEvent, ProviderError};
use tokio::sync::mpsc::Sender;

#[derive(Debug, Default)]
pub struct CustomHardwareProvider;

#[async_trait]
impl BatteryProvider for CustomHardwareProvider {
    fn name(&self) -> &'static str {
        "Custom-Hardware-Provider"
    }

    /// Fetches an on-demand snapshot of all connected peripherals.
    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        let mut devices = Vec::new();
        // 1. Query hardware status...
        // 2. Build BatteryInfo instances...
        Ok(devices)
    }

    /// Subscribes to real-time asynchronous device connection and battery events.
    async fn subscribe_events(&self, tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        // Spawn background listener task sending DeviceEvent::BatteryUpdated, etc.
        Ok(())
    }
}
```

### Step 2: Register with `DeviceAggregator`
In `crates/app/src/main.rs`, instantiate your provider and register it:

```rust
let custom_provider = Arc::new(CustomHardwareProvider::new());
aggregator.add_provider(custom_provider as Arc<dyn BatteryProvider>);
```

---

## 3. Mock Testing Guidelines

When adding features or testing edge cases, never depend on live Bluetooth hardware or specific host OS devices. Use `MockBatteryProvider` (located in `crates/core/tests/common/mod.rs`):

```rust
use std::sync::Arc;
use std::time::SystemTime;
use atena_core::{BatteryInfo, BatterySource, ConnectionType, DeviceCategory, DeviceAggregator};

#[tokio::test]
async fn test_custom_provider_aggregation() {
    let t0 = SystemTime::now();
    let dev = BatteryInfo::new(
        "AA:BB:CC:11:22:33",
        "Test Headset",
        75,
        true,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );

    let provider = Arc::new(MockBatteryProvider::new("Mock", BatterySource::OsCache, vec![dev]));
    let mut agg = DeviceAggregator::new();
    agg.add_provider(provider);

    let results = agg.aggregate().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].level, 75);
}
```

---

## 4. Pre-Commit Verification Checklist

Before submitting a Pull Request, verify that all 5 quality gates pass locally:

```powershell
# 1. Verify code formatting
cargo fmt --all -- --check

# 2. Strict Clippy check with zero warnings
cargo clippy --workspace --all-targets -- -D warnings

# 3. Full test suite execution (must be 100% green)
cargo test --workspace

# 4. Documentation build check (zero warnings)
cargo doc --workspace --no-deps

# 5. Build optimized stripped release binary
cargo build --release --workspace
```

---

## 5. Submitting Pull Requests

1. Fork the repository and create a feature branch (`git checkout -b feature/my-feature`).
2. Commit your changes with clear, descriptive commit messages.
3. Push to your branch and open a Pull Request against `main`.
4. The CI pipeline will automatically run formatting, clippy, unit tests, and cross-compilation checks.
