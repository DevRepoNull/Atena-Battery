//! Linux BlueZ D-Bus provider stub for Atena Battery.

use async_trait::async_trait;
use atena_core::{BatteryInfo, BatteryProvider, DeviceEvent, ProviderError};
use tokio::sync::mpsc::Sender;

/// Stub Linux BlueZ D-Bus provider.
#[derive(Debug, Default)]
pub struct LinuxBluezProvider;

impl LinuxBluezProvider {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BatteryProvider for LinuxBluezProvider {
    fn name(&self) -> &'static str {
        "Linux-BlueZ-DBus"
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        // Stub: Linux BlueZ implementation using zbus will be added here
        Ok(Vec::new())
    }

    async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        Ok(())
    }
}
