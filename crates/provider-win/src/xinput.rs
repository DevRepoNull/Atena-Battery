//! Windows XInput Gamepad Battery Provider (Tier 3).
//!
//! Uses `XInputGetBatteryInformation` to query battery levels for wireless Xbox gamepads
//! across user indices `0..XUSER_MAX_COUNT` with near-zero overhead.

use std::time::SystemTime;

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, ConnectionType, DeviceCategory, DeviceEvent,
    ProviderError,
};
use tokio::sync::mpsc::Sender;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::UI::Input::XboxController::{
    XInputGetBatteryInformation, BATTERY_DEVTYPE_GAMEPAD, BATTERY_LEVEL_EMPTY, BATTERY_LEVEL_FULL,
    BATTERY_LEVEL_LOW, BATTERY_LEVEL_MEDIUM, BATTERY_TYPE_DISCONNECTED, BATTERY_TYPE_WIRED,
    XINPUT_BATTERY_INFORMATION, XUSER_MAX_COUNT,
};

/// Provider querying battery status of wireless gamepads via XInput.
#[derive(Debug, Default, Clone)]
pub struct Win32XInputProvider;

impl Win32XInputProvider {
    /// Creates a new `Win32XInputProvider` instance.
    pub fn new() -> Self {
        Self
    }

    /// Queries battery for all connected wireless gamepads (user indices 0..4).
    pub fn scan_gamepads(&self) -> Vec<BatteryInfo> {
        let mut results = Vec::new();
        let now = SystemTime::now();

        for user_index in 0..XUSER_MAX_COUNT {
            let mut info = XINPUT_BATTERY_INFORMATION::default();
            // SAFETY: `XInputGetBatteryInformation` is called with valid user index (0..4),
            // constant device type `BATTERY_DEVTYPE_GAMEPAD`, and a valid stack pointer to `info`.
            let res = unsafe {
                XInputGetBatteryInformation(user_index, BATTERY_DEVTYPE_GAMEPAD, &mut info)
            };

            if res == ERROR_SUCCESS.0 {
                // Filter out disconnected controllers and wired-only controllers
                if info.BatteryType == BATTERY_TYPE_DISCONNECTED
                    || info.BatteryType == BATTERY_TYPE_WIRED
                {
                    continue;
                }

                let level = match info.BatteryLevel {
                    BATTERY_LEVEL_EMPTY => 10,
                    BATTERY_LEVEL_LOW => 30,
                    BATTERY_LEVEL_MEDIUM => 60,
                    BATTERY_LEVEL_FULL => 100,
                    other => other.0.min(100),
                };

                let device_id = format!("XInput#{}", user_index);
                let name = format!("Xbox Controller #{}", user_index + 1);

                results.push(BatteryInfo::new(
                    device_id,
                    name,
                    level,
                    true,
                    BatterySource::XInput,
                    ConnectionType::Gamepad,
                    DeviceCategory::Gamepad,
                    now,
                ));
            }
        }

        results
    }
}

#[async_trait]
impl BatteryProvider for Win32XInputProvider {
    fn name(&self) -> &'static str {
        "Win32-XInput"
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        Ok(self.scan_gamepads())
    }

    async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scan_gamepads_does_not_panic() {
        let provider = Win32XInputProvider::new();
        let results = provider.scan_gamepads();
        // Should execute smoothly without panicking regardless of connected gamepads
        for dev in &results {
            assert!(dev.level <= 100);
            assert_eq!(dev.source, BatterySource::XInput);
            assert_eq!(dev.category, DeviceCategory::Gamepad);
            assert_eq!(dev.connection_type, ConnectionType::Gamepad);
        }
    }
}
