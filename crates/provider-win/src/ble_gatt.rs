//! Windows Bluetooth Low Energy (BLE) GATT Battery Service Provider (Tier 2).
//!
//! Queries standard GATT Battery Service (`0x180F`) and Characteristic (`0x2A19`)
//! using official Microsoft WinRT APIs (`windows::Devices::Bluetooth::GenericAttributeProfile`).
//!
//! Strictly acts as a fallback: only queries devices that did NOT report battery via
//! Tier 1 (`OsCache`) and are not currently tripped in `CircuitBreaker`.

use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, CircuitBreaker, ConnectionType, DeviceEvent,
    ProviderError,
};
use tokio::sync::mpsc::Sender;

use windows::Devices::Bluetooth::BluetoothLEDevice;
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristicUuids, GattCommunicationStatus, GattServiceUuids,
};
use windows::Storage::Streams::DataReader;

use crate::{infer_category_from_name, Win32PnpProvider};

/// Tier 2 BLE GATT Battery Service provider.
#[derive(Debug, Clone, Default)]
pub struct BleGattProvider {
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    pnp_provider: Win32PnpProvider,
}

impl BleGattProvider {
    /// Creates a new `BleGattProvider`.
    pub fn new() -> Self {
        Self {
            circuit_breaker: None,
            pnp_provider: Win32PnpProvider::new(),
        }
    }

    /// Creates a new `BleGattProvider` with an attached circuit breaker.
    pub fn with_circuit_breaker(circuit_breaker: Arc<CircuitBreaker>) -> Self {
        Self {
            circuit_breaker: Some(circuit_breaker),
            pnp_provider: Win32PnpProvider::new(),
        }
    }

    /// Converts a canonical MAC address string (e.g. `XX:XX:XX:XX:XX:XX`) to u64 for WinRT.
    pub fn mac_to_u64(mac: &str) -> Option<u64> {
        let clean: String = mac.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if clean.len() == 12 {
            u64::from_str_radix(&clean, 16).ok()
        } else {
            None
        }
    }

    /// Reads the BLE battery level directly via WinRT GATT for the given 64-bit Bluetooth address.
    pub fn read_gatt_battery_by_address(addr: u64) -> Result<Option<u8>, ProviderError> {
        let service_uuid = GattServiceUuids::Battery().map_err(|e| {
            ProviderError::Unavailable(format!("GattServiceUuids::Battery failed: {e}"))
        })?;
        let char_uuid = GattCharacteristicUuids::BatteryLevel().map_err(|e| {
            ProviderError::Unavailable(format!("GattCharacteristicUuids::BatteryLevel failed: {e}"))
        })?;

        let op = BluetoothLEDevice::FromBluetoothAddressAsync(addr).map_err(|e| {
            ProviderError::Communication(format!("FromBluetoothAddressAsync failed: {e}"))
        })?;
        let dev = match op.get() {
            Ok(d) => d,
            Err(e) => {
                return Err(ProviderError::Communication(format!(
                    "BluetoothLEDevice.get failed: {e}"
                )))
            }
        };

        let services_res = match dev.GetGattServicesForUuidAsync(service_uuid) {
            Ok(op) => match op.get() {
                Ok(res) => res,
                Err(_) => return Ok(None),
            },
            Err(_) => return Ok(None),
        };

        if services_res
            .Status()
            .unwrap_or(GattCommunicationStatus::Unreachable)
            != GattCommunicationStatus::Success
        {
            return Ok(None);
        }

        let services = match services_res.Services() {
            Ok(s) => s,
            Err(_) => return Ok(None),
        };

        for service in services {
            let chars_res = match service.GetCharacteristicsForUuidAsync(char_uuid) {
                Ok(op) => match op.get() {
                    Ok(res) => res,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };

            if chars_res
                .Status()
                .unwrap_or(GattCommunicationStatus::Unreachable)
                != GattCommunicationStatus::Success
            {
                continue;
            }

            let characteristics = match chars_res.Characteristics() {
                Ok(c) => c,
                Err(_) => continue,
            };

            for characteristic in characteristics {
                let read_res = match characteristic.ReadValueAsync() {
                    Ok(op) => match op.get() {
                        Ok(res) => res,
                        Err(_) => continue,
                    },
                    Err(_) => continue,
                };

                if read_res
                    .Status()
                    .unwrap_or(GattCommunicationStatus::Unreachable)
                    != GattCommunicationStatus::Success
                {
                    continue;
                }

                if let Ok(buffer) = read_res.Value() {
                    if let Ok(reader) = DataReader::FromBuffer(&buffer) {
                        if let Ok(len) = reader.UnconsumedBufferLength() {
                            if len > 0 {
                                if let Ok(val) = reader.ReadByte() {
                                    if val <= 100 {
                                        return Ok(Some(val));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(None)
    }

    /// Queries a single device by MAC address via BLE GATT, honoring CircuitBreaker status.
    pub fn query_device(&self, mac: &str, name: &str) -> Option<BatteryInfo> {
        // Enforce CircuitBreaker gate
        if let Some(cb) = &self.circuit_breaker {
            if !cb.can_query(mac) {
                return None;
            }
        }

        let addr = Self::mac_to_u64(mac)?;
        match Self::read_gatt_battery_by_address(addr) {
            Ok(Some(level)) => {
                if let Some(cb) = &self.circuit_breaker {
                    cb.record_success(mac);
                }
                let category = infer_category_from_name(name);
                Some(BatteryInfo::new(
                    mac,
                    name,
                    level,
                    true,
                    BatterySource::BleGatt,
                    ConnectionType::BluetoothLE,
                    category,
                    SystemTime::now(),
                ))
            }
            Ok(None) => None,
            Err(_) => {
                if let Some(cb) = &self.circuit_breaker {
                    cb.record_failure(mac);
                }
                None
            }
        }
    }
}

#[async_trait]
impl BatteryProvider for BleGattProvider {
    fn name(&self) -> &'static str {
        "WinRT-BleGatt"
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        // Retrieve present devices that did NOT report battery via Tier 1
        let candidates = self.pnp_provider.scan_candidates_without_tier1()?;
        let mut results = Vec::new();

        for (mac, name) in candidates {
            if let Some(info) = self.query_device(&mac, &name) {
                results.push(info);
            }
        }

        results.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        Ok(results)
    }

    async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mac_to_u64_conversion() {
        assert_eq!(
            BleGattProvider::mac_to_u64("E3:48:D8:11:12:9E"),
            Some(0xe348d811129e)
        );
        assert_eq!(
            BleGattProvider::mac_to_u64("00:11:22:33:44:55"),
            Some(0x001122334455)
        );
        assert_eq!(BleGattProvider::mac_to_u64("invalid"), None);
    }

    #[test]
    fn test_circuit_breaker_blocking_in_ble_provider() {
        let cb = Arc::new(CircuitBreaker::new());
        let mac = "AA:BB:CC:DD:EE:77";
        // Trip circuit breaker
        cb.record_failure(mac);
        cb.record_failure(mac);
        cb.record_failure(mac);

        let provider = BleGattProvider::with_circuit_breaker(cb);
        assert!(provider.query_device(mac, "Test Device").is_none());
    }
}
