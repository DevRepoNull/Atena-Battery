use async_trait::async_trait;
use tokio::sync::mpsc::Sender;

use crate::model::{BatteryInfo, DeviceEvent, ProviderError};

/// An asynchronous battery information provider for devices.
///
/// Providers can query OS caches, BLE GATT characteristics, or RFCOMM AT commands.
#[async_trait]
pub trait BatteryProvider: Send + Sync {
    /// Friendly diagnostic provider name.
    fn name(&self) -> &'static str {
        "Unknown"
    }

    /// Fetches a current snapshot of battery readings across all discovered devices.
    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError>;

    /// Subscribes to real-time device battery and connection events.
    async fn subscribe_events(&self, tx: Sender<DeviceEvent>) -> Result<(), ProviderError>;
}
