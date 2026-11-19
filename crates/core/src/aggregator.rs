use std::sync::Arc;

use crate::circuit_breaker::CircuitBreaker;
use crate::model::{BatteryInfo, BatterySource, ProviderError};
use crate::provider::BatteryProvider;

/// Aggregates and merges device battery readings from multiple providers.
///
/// Ensures a single authoritative entry per physical device by:
/// 1. Correlating composite devices using Windows `ContainerId` across providers.
/// 2. Deduplicating by canonical `device_id` as fallback.
/// 3. Prioritizing sources: `OsCache` (5) > `HidStandard` (4) > `XInput` (3) > `BleGatt` (2) > `Rfcomm` (1).
/// 4. Prioritizing fine 1% readings over coarse 10% steps when appropriate.
pub struct DeviceAggregator {
    providers: Vec<Arc<dyn BatteryProvider>>,
    circuit_breaker: Option<Arc<CircuitBreaker>>,
}

impl Default for DeviceAggregator {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceAggregator {
    /// Creates an empty aggregator with no providers.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            circuit_breaker: None,
        }
    }

    /// Creates an aggregator with an attached circuit breaker.
    pub fn with_circuit_breaker(circuit_breaker: Arc<CircuitBreaker>) -> Self {
        Self {
            providers: Vec::new(),
            circuit_breaker: Some(circuit_breaker),
        }
    }

    /// Adds a battery provider to the aggregator.
    pub fn add_provider(&mut self, provider: Arc<dyn BatteryProvider>) {
        self.providers.push(provider);
    }

    /// Returns the registered providers.
    pub fn providers(&self) -> &[Arc<dyn BatteryProvider>] {
        &self.providers
    }

    /// Returns the optional attached circuit breaker.
    pub fn circuit_breaker(&self) -> Option<&Arc<CircuitBreaker>> {
        self.circuit_breaker.as_ref()
    }

    /// Determines whether an incoming candidate `BatteryInfo` should replace an existing one.
    #[inline]
    pub fn should_replace(existing: &BatteryInfo, candidate: &BatteryInfo) -> bool {
        let existing_fine = !existing.level.is_multiple_of(10);
        let candidate_fine = !candidate.level.is_multiple_of(10);

        // Precision override: Direct hardware fine reading (1%) takes precedence over coarse 10% OsCache/Rfcomm
        if candidate_fine
            && !existing_fine
            && (candidate.source == BatterySource::BleGatt
                || candidate.source == BatterySource::HidStandard)
        {
            return true;
        }
        if existing_fine
            && !candidate_fine
            && (existing.source == BatterySource::BleGatt
                || existing.source == BatterySource::HidStandard)
        {
            return false;
        }

        // Standard source tier priority: OsCache (5) > HidStandard (4) > XInput (3) > BleGatt (2) > Rfcomm (1)
        if candidate.source.priority() > existing.source.priority() {
            return true;
        }
        if candidate.source.priority() < existing.source.priority() {
            return false;
        }

        // Tie-breaker: latest timestamp wins
        candidate.last_updated > existing.last_updated
    }

    /// Merges an arbitrary collection of `BatteryInfo` entries, deduplicating across providers
    /// by Windows `ContainerId` (when present) or canonical `device_id`, enforcing priority order:
    /// `OsCache` > `HidStandard` > `XInput` > `BleGatt` > `Rfcomm`.
    pub fn merge(items: impl IntoIterator<Item = BatteryInfo>) -> Vec<BatteryInfo> {
        let mut result: Vec<BatteryInfo> = Vec::new();

        for item in items {
            let item_id = item.device_id.trim();
            let item_cid = item.container_id.as_deref().map(str::trim);

            let existing_idx = result.iter().position(|existing| {
                // 1. ContainerId match if both have non-empty container_id
                if let (Some(cid_a), Some(cid_b)) =
                    (item_cid, existing.container_id.as_deref().map(str::trim))
                {
                    if !cid_a.is_empty() && cid_a.eq_ignore_ascii_case(cid_b) {
                        return true;
                    }
                }
                // 2. Fallback: match by device_id
                existing.device_id.trim().eq_ignore_ascii_case(item_id)
            });

            match existing_idx {
                Some(idx) => {
                    if Self::should_replace(&result[idx], &item) {
                        let mut replacement = item;
                        if replacement.container_id.is_none() {
                            replacement.container_id = result[idx].container_id.clone();
                        }
                        result[idx] = replacement;
                    } else if result[idx].container_id.is_none() && item.container_id.is_some() {
                        result[idx].container_id = item.container_id;
                    }
                }
                None => {
                    result.push(item);
                }
            }
        }

        result.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        result
    }

    /// Fetches snapshots from all registered providers, filtering out any devices blocked
    /// by the circuit breaker (if present), and returns the deduplicated merged results.
    pub async fn aggregate(&self) -> Vec<BatteryInfo> {
        let (merged, _errors) = self.aggregate_with_errors().await;
        merged
    }

    /// Fetches snapshots from all registered providers, returning both the merged device list
    /// and any provider errors encountered.
    pub async fn aggregate_with_errors(&self) -> (Vec<BatteryInfo>, Vec<(usize, ProviderError)>) {
        let mut all_items = Vec::new();
        let mut errors = Vec::new();

        for (idx, provider) in self.providers.iter().enumerate() {
            match provider.fetch_snapshot().await {
                Ok(items) => {
                    all_items.extend(items);
                }
                Err(err) => {
                    errors.push((idx, err));
                }
            }
        }

        // Apply circuit breaker filtering if present
        let filtered_items: Vec<BatteryInfo> = if let Some(cb) = &self.circuit_breaker {
            all_items
                .into_iter()
                .filter(|item| cb.can_query(&item.device_id))
                .collect()
        } else {
            all_items
        };

        (Self::merge(filtered_items), errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ConnectionType, DeviceCategory, DeviceEvent};
    use async_trait::async_trait;
    use std::time::{Duration, SystemTime};
    use tokio::sync::mpsc::Sender;

    struct MockProvider {
        name: &'static str,
        devices: Vec<BatteryInfo>,
        fail: bool,
    }

    impl MockProvider {
        fn new(name: &'static str, devices: Vec<BatteryInfo>) -> Self {
            Self {
                name,
                devices,
                fail: false,
            }
        }

        fn failing(name: &'static str) -> Self {
            Self {
                name,
                devices: Vec::new(),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl BatteryProvider for MockProvider {
        fn name(&self) -> &'static str {
            self.name
        }

        async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
            if self.fail {
                Err(ProviderError::Unavailable(format!("{} error", self.name)))
            } else {
                Ok(self.devices.clone())
            }
        }

        async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
            Ok(())
        }
    }

    fn test_device(
        id: &str,
        name: &str,
        level: u8,
        source: BatterySource,
        conn: ConnectionType,
        cat: DeviceCategory,
        t: SystemTime,
    ) -> BatteryInfo {
        BatteryInfo::new(id, name, level, true, source, conn, cat, t)
    }

    #[test]
    fn test_merge_priority_order() {
        let t0 = SystemTime::now();
        let id = "AA:BB:CC:DD:EE:10";

        let rfcomm = test_device(
            id,
            "Headset",
            40,
            BatterySource::Rfcomm,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Audio,
            t0,
        );
        let ble = test_device(
            id,
            "Headset",
            50,
            BatterySource::BleGatt,
            ConnectionType::BluetoothLE,
            DeviceCategory::Audio,
            t0,
        );
        let oscache = test_device(
            id,
            "Headset",
            60,
            BatterySource::OsCache,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Audio,
            t0,
        );

        // 1. Rfcomm vs BleGatt -> BleGatt wins
        let merged1 = DeviceAggregator::merge(vec![rfcomm.clone(), ble.clone()]);
        assert_eq!(merged1.len(), 1);
        assert_eq!(merged1[0].source, BatterySource::BleGatt);
        assert_eq!(merged1[0].level, 50);

        // 2. BleGatt vs OsCache (both multiples of 10) -> OsCache wins
        let merged2 = DeviceAggregator::merge(vec![ble.clone(), oscache.clone()]);
        assert_eq!(merged2.len(), 1);
        assert_eq!(merged2[0].source, BatterySource::OsCache);
        assert_eq!(merged2[0].level, 60);

        // 3. All three regardless of insertion order -> OsCache wins
        let merged3 = DeviceAggregator::merge(vec![rfcomm.clone(), ble.clone(), oscache.clone()]);
        assert_eq!(merged3.len(), 1);
        assert_eq!(merged3[0].source, BatterySource::OsCache);

        let merged4 = DeviceAggregator::merge(vec![oscache.clone(), ble.clone(), rfcomm.clone()]);
        assert_eq!(merged4.len(), 1);
        assert_eq!(merged4[0].source, BatterySource::OsCache);
    }

    #[test]
    fn test_merge_precision_priority_rule() {
        let t0 = SystemTime::now();
        let id = "AA:BB:CC:DD:EE:15";

        // Coarse 10% reading from OsCache (e.g. 60% from HFP)
        let oscache_coarse = test_device(
            id,
            "Headset",
            60,
            BatterySource::OsCache,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Audio,
            t0,
        );
        // Fine 1% reading from BleGatt (e.g. 64% from GATT Battery Service)
        let ble_fine = test_device(
            id,
            "Headset",
            64,
            BatterySource::BleGatt,
            ConnectionType::BluetoothLE,
            DeviceCategory::Audio,
            t0,
        );

        // BleGatt fine (64%) MUST override OsCache coarse (60%)
        let merged1 = DeviceAggregator::merge(vec![oscache_coarse.clone(), ble_fine.clone()]);
        assert_eq!(merged1.len(), 1);
        assert_eq!(merged1[0].source, BatterySource::BleGatt);
        assert_eq!(merged1[0].level, 64);

        // Even if BleGatt fine is inserted first, coarse OsCache does NOT overwrite it
        let merged2 = DeviceAggregator::merge(vec![ble_fine.clone(), oscache_coarse.clone()]);
        assert_eq!(merged2.len(), 1);
        assert_eq!(merged2[0].source, BatterySource::BleGatt);
        assert_eq!(merged2[0].level, 64);

        // If OsCache also has fine precision (e.g. 65%), OsCache tier priority wins
        let oscache_fine = test_device(
            id,
            "Headset",
            65,
            BatterySource::OsCache,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Audio,
            t0,
        );
        let merged3 = DeviceAggregator::merge(vec![ble_fine.clone(), oscache_fine.clone()]);
        assert_eq!(merged3.len(), 1);
        assert_eq!(merged3[0].source, BatterySource::OsCache);
        assert_eq!(merged3[0].level, 65);
    }

    #[test]
    fn test_merge_tie_breaker_by_timestamp() {
        let t0 = SystemTime::now();
        let t1 = t0 + Duration::from_secs(60);
        let id = "AA:BB:CC:DD:EE:20";

        let older = test_device(
            id,
            "Mouse",
            80,
            BatterySource::BleGatt,
            ConnectionType::BluetoothLE,
            DeviceCategory::Mouse,
            t0,
        );
        let newer = test_device(
            id,
            "Mouse",
            75,
            BatterySource::BleGatt,
            ConnectionType::BluetoothLE,
            DeviceCategory::Mouse,
            t1,
        );

        // Newer timestamp should replace older
        let merged = DeviceAggregator::merge(vec![older.clone(), newer.clone()]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].level, 75);
        assert_eq!(merged[0].last_updated, t1);

        // Even if newer appears first in slice
        let merged_rev = DeviceAggregator::merge(vec![newer.clone(), older.clone()]);
        assert_eq!(merged_rev.len(), 1);
        assert_eq!(merged_rev[0].level, 75);
    }

    #[test]
    fn test_merge_case_insensitive_device_id() {
        let t0 = SystemTime::now();
        let dev1 = test_device(
            "aa:bb:cc:dd:ee:30",
            "Keyboard",
            90,
            BatterySource::Rfcomm,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Keyboard,
            t0,
        );
        let dev2 = test_device(
            "AA:BB:CC:DD:EE:30",
            "Keyboard",
            95,
            BatterySource::OsCache,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Keyboard,
            t0,
        );

        let merged = DeviceAggregator::merge(vec![dev1, dev2]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].source, BatterySource::OsCache);
        assert_eq!(merged[0].level, 95);
    }

    #[test]
    fn test_merge_container_id_deduplication() {
        let t0 = SystemTime::now();
        let container_uuid = "{12345678-ABCD-EF01-2345-6789ABCDEF01}";

        // Bluetooth PnP endpoint (Audio)
        let bt_dev = test_device(
            "AA:BB:CC:DD:EE:99",
            "Sony WH-1000XM4",
            80,
            BatterySource::OsCache,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Audio,
            t0,
        )
        .with_container_id(container_uuid);

        // HID Control endpoint for the same headset (different device_id, same container_id)
        let hid_dev = test_device(
            "HID\\VID_054C&PID_0CD3\\7&123456",
            "Sony WH-1000XM4 HID",
            84,
            BatterySource::HidStandard,
            ConnectionType::Wireless24Ghz,
            DeviceCategory::Audio,
            t0,
        )
        .with_container_id(container_uuid);

        // Merging should deduplicate them into 1 device using ContainerId!
        // OsCache (80%) vs HidStandard fine (84%) -> HidStandard fine wins due to 1% precision override
        let merged = DeviceAggregator::merge(vec![bt_dev, hid_dev]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].level, 84);
        assert_eq!(merged[0].source, BatterySource::HidStandard);
        assert_eq!(merged[0].container_id.as_deref(), Some(container_uuid));
    }

    #[test]
    fn test_merge_multiple_distinct_devices() {
        let t0 = SystemTime::now();
        let dev1 = test_device(
            "AA:BB:CC:DD:EE:41",
            "Device 1",
            10,
            BatterySource::BleGatt,
            ConnectionType::BluetoothLE,
            DeviceCategory::Peripheral,
            t0,
        );
        let dev2 = test_device(
            "AA:BB:CC:DD:EE:42",
            "Device 2",
            20,
            BatterySource::OsCache,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Peripheral,
            t0,
        );
        let dev3 = test_device(
            "AA:BB:CC:DD:EE:43",
            "Device 3",
            30,
            BatterySource::Rfcomm,
            ConnectionType::BluetoothClassic,
            DeviceCategory::Peripheral,
            t0,
        );

        let merged = DeviceAggregator::merge(vec![dev1, dev2, dev3]);
        assert_eq!(merged.len(), 3);
        assert_eq!(merged[0].device_id, "AA:BB:CC:DD:EE:41");
        assert_eq!(merged[1].device_id, "AA:BB:CC:DD:EE:42");
        assert_eq!(merged[2].device_id, "AA:BB:CC:DD:EE:43");
    }

    #[tokio::test]
    async fn test_aggregator_with_multiple_providers() {
        let t0 = SystemTime::now();
        let dev1_id = "AA:BB:CC:DD:EE:51";
        let dev2_id = "AA:BB:CC:DD:EE:52";

        // Provider 1 (RFCOMM)
        let p1 = Arc::new(MockProvider::new(
            "RFCOMM",
            vec![
                test_device(
                    dev1_id,
                    "Headset",
                    30,
                    BatterySource::Rfcomm,
                    ConnectionType::BluetoothClassic,
                    DeviceCategory::Audio,
                    t0,
                ),
                test_device(
                    dev2_id,
                    "Speaker",
                    60,
                    BatterySource::Rfcomm,
                    ConnectionType::BluetoothClassic,
                    DeviceCategory::Audio,
                    t0,
                ),
            ],
        ));

        // Provider 2 (BleGatt with 1% fine precision 35%)
        let p2 = Arc::new(MockProvider::new(
            "BleGatt",
            vec![test_device(
                dev1_id,
                "Headset",
                35,
                BatterySource::BleGatt,
                ConnectionType::BluetoothLE,
                DeviceCategory::Audio,
                t0,
            )],
        ));

        // Provider 3 (Failing)
        let p3 = Arc::new(MockProvider::failing("FailingProvider"));

        let mut aggregator = DeviceAggregator::new();
        aggregator.add_provider(p1);
        aggregator.add_provider(p2);
        aggregator.add_provider(p3);

        let (results, errors) = aggregator.aggregate_with_errors().await;
        assert_eq!(errors.len(), 1);
        assert_eq!(results.len(), 2);

        // dev1: BleGatt (35%) wins over Rfcomm (30%)
        let dev1 = results.iter().find(|d| d.device_id == dev1_id).unwrap();
        assert_eq!(dev1.source, BatterySource::BleGatt);
        assert_eq!(dev1.level, 35);

        // dev2: Only present in Rfcomm
        let dev2 = results.iter().find(|d| d.device_id == dev2_id).unwrap();
        assert_eq!(dev2.source, BatterySource::Rfcomm);
        assert_eq!(dev2.level, 60);
    }

    #[tokio::test]
    async fn test_aggregator_with_circuit_breaker() {
        let t0 = SystemTime::now();
        let dev1_id = "AA:BB:CC:DD:EE:61";
        let dev2_id = "AA:BB:CC:DD:EE:62";

        let cb = Arc::new(CircuitBreaker::new());
        // Trip breaker for dev1 (3 failures)
        cb.record_failure(dev1_id);
        cb.record_failure(dev1_id);
        cb.record_failure(dev1_id);
        assert!(!cb.can_query(dev1_id));
        assert!(cb.can_query(dev2_id));

        let p1 = Arc::new(MockProvider::new(
            "OsCache",
            vec![
                test_device(
                    dev1_id,
                    "Blocked Device",
                    20,
                    BatterySource::OsCache,
                    ConnectionType::BluetoothClassic,
                    DeviceCategory::Peripheral,
                    t0,
                ),
                test_device(
                    dev2_id,
                    "Allowed Device",
                    80,
                    BatterySource::OsCache,
                    ConnectionType::BluetoothClassic,
                    DeviceCategory::Peripheral,
                    t0,
                ),
            ],
        ));

        let mut aggregator = DeviceAggregator::with_circuit_breaker(cb);
        aggregator.add_provider(p1);

        let results = aggregator.aggregate().await;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].device_id, dev2_id);
    }
}
