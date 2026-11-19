mod common;

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, CircuitBreaker, ConnectionType, DeviceAggregator,
    DeviceCategory, DeviceEvent, ProviderError,
};
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
            Err(ProviderError::Unavailable(format!(
                "{} mock failure",
                self.name
            )))
        } else {
            Ok(self.devices.clone())
        }
    }

    async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        Ok(())
    }
}

fn make_device(
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

#[tokio::test]
async fn test_priority_merging_oscache_beats_blegatt_and_rfcomm() {
    let t0 = SystemTime::now();
    let mac = "00:11:22:33:44:55";

    let rfcomm = make_device(
        mac,
        "Headset",
        40,
        BatterySource::Rfcomm,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    let ble = make_device(
        mac,
        "Headset",
        50,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t0,
    );
    let oscache = make_device(
        mac,
        "Headset",
        60,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );

    let p_rfcomm = Arc::new(MockProvider::new("RFCOMM", vec![rfcomm]));
    let p_ble = Arc::new(MockProvider::new("BleGatt", vec![ble]));
    let p_oscache = Arc::new(MockProvider::new("OsCache", vec![oscache]));

    let mut agg = DeviceAggregator::new();
    agg.add_provider(p_rfcomm);
    agg.add_provider(p_ble);
    agg.add_provider(p_oscache);

    let results = agg.aggregate().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, BatterySource::OsCache);
    assert_eq!(results[0].level, 60);
}

#[tokio::test]
async fn test_blegatt_beats_rfcomm() {
    let t0 = SystemTime::now();
    let mac = "00:11:22:33:44:56";

    let rfcomm = make_device(
        mac,
        "Earbuds",
        30,
        BatterySource::Rfcomm,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    let ble = make_device(
        mac,
        "Earbuds",
        40,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t0,
    );

    let mut agg = DeviceAggregator::new();
    agg.add_provider(Arc::new(MockProvider::new("RFCOMM", vec![rfcomm])));
    agg.add_provider(Arc::new(MockProvider::new("BleGatt", vec![ble])));

    let results = agg.aggregate().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, BatterySource::BleGatt);
    assert_eq!(results[0].level, 40);
}

#[tokio::test]
async fn test_precision_priority_one_percent_overrides_ten_percent() {
    let t0 = SystemTime::now();
    let mac = "00:11:22:33:44:57";

    // OsCache reports coarse 70%
    let oscache = make_device(
        mac,
        "Sony WH-1000XM4",
        70,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    // BleGatt reports precise 73% (1% step)
    let ble = make_device(
        mac,
        "Sony WH-1000XM4",
        73,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t0,
    );

    let mut agg = DeviceAggregator::new();
    agg.add_provider(Arc::new(MockProvider::new("OsCache", vec![oscache])));
    agg.add_provider(Arc::new(MockProvider::new("BleGatt", vec![ble])));

    let results = agg.aggregate().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].source, BatterySource::BleGatt);
    assert_eq!(results[0].level, 73);
}

#[tokio::test]
async fn test_timestamp_tie_breaker_same_tier() {
    let t0 = SystemTime::now();
    let t1 = t0 + Duration::from_secs(30);
    let mac = "00:11:22:33:44:58";

    let older = make_device(
        mac,
        "Mouse",
        85,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Mouse,
        t0,
    );
    let newer = make_device(
        mac,
        "Mouse",
        82,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Mouse,
        t1,
    );

    let merged = DeviceAggregator::merge(vec![older, newer]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].level, 82);
    assert_eq!(merged[0].last_updated, t1);
}

#[tokio::test]
async fn test_aggregator_fault_tolerance_with_failing_provider() {
    let t0 = SystemTime::now();
    let mac = "00:11:22:33:44:59";

    let healthy = make_device(
        mac,
        "Keyboard",
        90,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Keyboard,
        t0,
    );

    let mut agg = DeviceAggregator::new();
    agg.add_provider(Arc::new(MockProvider::new("Healthy", vec![healthy])));
    agg.add_provider(Arc::new(MockProvider::failing("Broken")));

    let (results, errors) = agg.aggregate_with_errors().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].device_id, mac);
    assert_eq!(errors.len(), 1);
}

#[tokio::test]
async fn test_aggregator_circuit_breaker_isolation() {
    let t0 = SystemTime::now();
    let mac1 = "AA:BB:CC:DD:EE:01";
    let mac2 = "AA:BB:CC:DD:EE:02";

    let cb = Arc::new(CircuitBreaker::new());
    // Trip circuit breaker for mac1
    cb.record_failure(mac1);
    cb.record_failure(mac1);
    cb.record_failure(mac1);

    let mut agg = DeviceAggregator::with_circuit_breaker(cb);
    agg.add_provider(Arc::new(MockProvider::new(
        "Provider",
        vec![
            make_device(
                mac1,
                "Broken Headset",
                10,
                BatterySource::Rfcomm,
                ConnectionType::BluetoothClassic,
                DeviceCategory::Audio,
                t0,
            ),
            make_device(
                mac2,
                "Working Mouse",
                95,
                BatterySource::OsCache,
                ConnectionType::BluetoothClassic,
                DeviceCategory::Mouse,
                t0,
            ),
        ],
    )));

    let results = agg.aggregate().await;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].device_id, mac2);
}

#[tokio::test]
async fn test_universal_wireless_six_plus_devices_aggregation() {
    let t0 = SystemTime::now();

    // 1. Bluetooth Classic Headset (reported by OsCache and RFCOMM)
    let bt_headset_mac = "AA:BB:CC:11:22:33";
    let d1_oscache = make_device(
        bt_headset_mac,
        "Sony WH-1000XM4",
        70,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    let d1_rfcomm = make_device(
        bt_headset_mac,
        "Sony WH-1000XM4",
        60,
        BatterySource::Rfcomm,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );

    // 2. Bluetooth LE Earbuds (reported by BleGatt)
    let ble_earbuds_mac = "AA:BB:CC:44:55:66";
    let d2_ble = make_device(
        ble_earbuds_mac,
        "Soundcore R50i",
        85,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t0,
    );

    // 3. 2.4GHz Wireless Mouse (via HID standard report)
    let mouse_id = "VID_046D&PID_C539";
    let d3_mouse = make_device(
        mouse_id,
        "Logitech G Pro Wireless",
        72,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Mouse,
        t0,
    )
    .with_container_id("{11111111-2222-3333-4444-555555555555}");

    // 4. 2.4GHz Wireless Keyboard (via HID standard report)
    let kbd_id = "VID_3434&PID_0200";
    let d4_kbd = make_device(
        kbd_id,
        "Keychron K3 Pro",
        90,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Keyboard,
        t0,
    )
    .with_container_id("{22222222-3333-4444-5555-666666666666}");

    // 5. Composite 2.4GHz Gaming Headset with dual presence (Bluetooth audio + HID dongle battery)
    // Both share the same Windows ContainerId. HID provides fine 63% reading, OsCache coarse 60%.
    let headset_container = "{33333333-4444-5555-6666-777777777777}";
    let d5_bt = make_device(
        "AA:BB:CC:77:88:99",
        "SteelSeries Arctis 7",
        60,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    )
    .with_container_id(headset_container);
    let d5_hid = make_device(
        "VID_1038&PID_12AD",
        "SteelSeries Arctis 7",
        63,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Audio,
        t0,
    )
    .with_container_id(headset_container);

    // 6. Wireless Gamepad #1 (via XInput)
    let d6_gamepad1 = make_device(
        "XInput#0",
        "Xbox Wireless Controller",
        50,
        BatterySource::XInput,
        ConnectionType::Gamepad,
        DeviceCategory::Gamepad,
        t0,
    );

    // 7. Wireless Gamepad #2 (via XInput)
    let d7_gamepad2 = make_device(
        "XInput#1",
        "Xbox Wireless Controller (Player 2)",
        100,
        BatterySource::XInput,
        ConnectionType::Gamepad,
        DeviceCategory::Gamepad,
        t0,
    );

    // Build providers
    let p_oscache = Arc::new(MockProvider::new("OsCache", vec![d1_oscache, d5_bt]));
    let p_hid = Arc::new(MockProvider::new("HID", vec![d3_mouse, d4_kbd, d5_hid]));
    let p_xinput = Arc::new(MockProvider::new("XInput", vec![d6_gamepad1, d7_gamepad2]));
    let p_ble = Arc::new(MockProvider::new("BleGatt", vec![d2_ble]));
    let p_rfcomm = Arc::new(MockProvider::new("RFCOMM", vec![d1_rfcomm]));

    let mut agg = DeviceAggregator::new();
    agg.add_provider(p_oscache);
    agg.add_provider(p_hid);
    agg.add_provider(p_xinput);
    agg.add_provider(p_ble);
    agg.add_provider(p_rfcomm);

    let devices = agg.aggregate().await;

    // Verify exactly 7 unique physical devices after deduplication of 9 reported nodes
    assert_eq!(
        devices.len(),
        7,
        "Expected 7 physical devices aggregated from 9 reported nodes"
    );

    // 1) Verify BT headset deduplicated: OsCache (70%) won over RFCOMM (60%)
    let d1 = devices
        .iter()
        .find(|d| d.device_id == bt_headset_mac)
        .expect("BT headset missing");
    assert_eq!(d1.level, 70);
    assert_eq!(d1.source, BatterySource::OsCache);
    assert_eq!(d1.category, DeviceCategory::Audio);

    // 2) Verify BLE earbuds
    let d2 = devices
        .iter()
        .find(|d| d.device_id == ble_earbuds_mac)
        .expect("BLE earbuds missing");
    assert_eq!(d2.level, 85);
    assert_eq!(d2.source, BatterySource::BleGatt);
    assert_eq!(d2.connection_type, ConnectionType::BluetoothLE);

    // 3) Verify 2.4GHz Mouse
    let d3 = devices
        .iter()
        .find(|d| d.device_id == mouse_id)
        .expect("Mouse missing");
    assert_eq!(d3.level, 72);
    assert_eq!(d3.category, DeviceCategory::Mouse);
    assert_eq!(d3.connection_type, ConnectionType::Wireless24Ghz);

    // 4) Verify 2.4GHz Keyboard
    let d4 = devices
        .iter()
        .find(|d| d.device_id == kbd_id)
        .expect("Keyboard missing");
    assert_eq!(d4.level, 90);
    assert_eq!(d4.category, DeviceCategory::Keyboard);

    // 5) Verify Composite SteelSeries Arctis 7: ContainerId matched!
    // HidStandard 63% has 1% fine precision which overrides OsCache 60% coarse precision
    let d5 = devices
        .iter()
        .find(|d| d.name == "SteelSeries Arctis 7")
        .expect("Composite headset missing");
    assert_eq!(
        d5.level, 63,
        "1% fine precision from HID must override 10% coarse OsCache"
    );
    assert_eq!(d5.source, BatterySource::HidStandard);

    // 6) Verify Gamepads
    let gp1 = devices
        .iter()
        .find(|d| d.device_id == "XInput#0")
        .expect("Gamepad 0 missing");
    assert_eq!(gp1.level, 50);
    assert_eq!(gp1.category, DeviceCategory::Gamepad);

    let gp2 = devices
        .iter()
        .find(|d| d.device_id == "XInput#1")
        .expect("Gamepad 1 missing");
    assert_eq!(gp2.level, 100);
    assert_eq!(gp2.category, DeviceCategory::Gamepad);
}

#[tokio::test]
async fn test_container_id_case_insensitivity_and_whitespace() {
    let t0 = SystemTime::now();
    let dev1 = make_device(
        "DEV_01",
        "Mouse Dongle",
        72,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Mouse,
        t0,
    )
    .with_container_id(" {A1B2C3D4-E5F6-7890-ABCD-EF0123456789} ");

    let dev2 = make_device(
        "DEV_02",
        "Mouse Bluetooth",
        65,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Mouse,
        t0,
    )
    .with_container_id("{a1b2c3d4-e5f6-7890-abcd-ef0123456789}");

    let merged = DeviceAggregator::merge(vec![dev1, dev2]);
    assert_eq!(
        merged.len(),
        1,
        "Devices with case-different or padded ContainerIds must deduplicate"
    );
    assert_eq!(merged[0].source, BatterySource::HidStandard);
    assert_eq!(merged[0].level, 72);
}

#[tokio::test]
async fn test_container_id_preservation_when_higher_tier_lacks_it() {
    let t0 = SystemTime::now();
    let mac = "00:11:22:33:44:AA";

    // Higher tier (OsCache) has no container_id
    let dev_oscache = make_device(
        mac,
        "Headset",
        80,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );

    // Lower tier (HID) has a container_id
    let dev_hid = make_device(
        mac,
        "Headset",
        70,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Audio,
        t0,
    )
    .with_container_id("{99999999-8888-7777-6666-555555555555}");

    let merged = DeviceAggregator::merge(vec![dev_hid, dev_oscache]);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].source, BatterySource::OsCache);
    assert_eq!(merged[0].level, 80);
    assert_eq!(
        merged[0].container_id.as_deref(),
        Some("{99999999-8888-7777-6666-555555555555}"),
        "ContainerId from lower tier must be preserved when higher tier lacks one"
    );
}

#[tokio::test]
async fn test_empty_container_id_does_not_cross_match() {
    let t0 = SystemTime::now();

    // Two distinct devices that both have empty or whitespace-only container_ids
    let dev1 = make_device(
        "ID_ALPHA",
        "Peripheral Alpha",
        50,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Peripheral,
        t0,
    )
    .with_container_id("");

    let dev2 = make_device(
        "ID_BETA",
        "Peripheral Beta",
        60,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Peripheral,
        t0,
    )
    .with_container_id("   ");

    let merged = DeviceAggregator::merge(vec![dev1, dev2]);
    assert_eq!(
        merged.len(),
        2,
        "Empty container IDs must not cause accidental merging"
    );
}

#[tokio::test]
async fn test_cascading_three_tier_fallback_oscache_ble_rfcomm() {
    let t0 = SystemTime::now();
    let mac1 = "11:22:33:44:55:01";
    let mac2 = "11:22:33:44:55:02";

    // Tier 1 (OsCache) is completely empty (no cached devices)
    let p_oscache = Arc::new(MockProvider::new("OsCache", vec![]));

    // Tier 2 (BleGatt) has only mac1
    let ble_dev1 = make_device(
        mac1,
        "Earbuds",
        77,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t0,
    );
    let p_ble = Arc::new(MockProvider::new("BleGatt", vec![ble_dev1]));

    // Tier 3 (Rfcomm) has mac1 (coarse 70%) and mac2 (coarse 40%)
    let rfcomm_dev1 = make_device(
        mac1,
        "Earbuds",
        70,
        BatterySource::Rfcomm,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    let rfcomm_dev2 = make_device(
        mac2,
        "Old Speaker",
        40,
        BatterySource::Rfcomm,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    let p_rfcomm = Arc::new(MockProvider::new("RFCOMM", vec![rfcomm_dev1, rfcomm_dev2]));

    let mut agg = DeviceAggregator::new();
    agg.add_provider(p_oscache);
    agg.add_provider(p_ble);
    agg.add_provider(p_rfcomm);

    let devices = agg.aggregate().await;
    assert_eq!(
        devices.len(),
        2,
        "Should cascade and aggregate both devices"
    );

    let d1 = devices.iter().find(|d| d.device_id == mac1).unwrap();
    assert_eq!(d1.source, BatterySource::BleGatt);
    assert_eq!(d1.level, 77);

    let d2 = devices.iter().find(|d| d.device_id == mac2).unwrap();
    assert_eq!(d2.source, BatterySource::Rfcomm);
    assert_eq!(d2.level, 40);
}

#[tokio::test]
async fn test_circuit_breaker_recovery_in_aggregator() {
    let t0 = SystemTime::now();
    let mac = "AA:BB:CC:DD:EE:99";

    let cb = Arc::new(CircuitBreaker::new());
    let mut agg = DeviceAggregator::with_circuit_breaker(Arc::clone(&cb));

    let dev = make_device(
        mac,
        "Flaky Headset",
        50,
        BatterySource::Rfcomm,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    agg.add_provider(Arc::new(MockProvider::new("RFCOMM", vec![dev])));

    // 1. Healthy: device is returned
    let res = agg.aggregate().await;
    assert_eq!(res.len(), 1);

    // 2. Trip circuit breaker: 3 failures
    let t_fail = Instant::now();
    cb.record_failure_at(mac, t_fail);
    cb.record_failure_at(mac, t_fail);
    cb.record_failure_at(mac, t_fail);

    // Aggregator skips tripped device
    let res = agg.aggregate().await;
    assert_eq!(
        res.len(),
        0,
        "Tripped device must be filtered out by aggregator"
    );

    // 3. Recovery: advance to HalfOpen and record successful probe
    let t_recovery = t_fail + Duration::from_secs(301);
    assert!(cb.can_query_at(mac, t_recovery));
    cb.record_success(mac);

    // Aggregator now returns the recovered device
    let res = agg.aggregate().await;
    assert_eq!(
        res.len(),
        1,
        "Recovered device must be returned by aggregator"
    );
    assert_eq!(res[0].device_id, mac);
}
