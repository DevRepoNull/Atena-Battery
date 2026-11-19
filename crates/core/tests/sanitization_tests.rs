use atena_core::{BatteryInfo, BatterySource, ConnectionType, DeviceCategory, MacAddress};
use std::time::SystemTime;

#[test]
fn test_battery_level_clamping() {
    let t0 = SystemTime::now();

    // Out-of-range battery level (> 100) must be clamped to 100
    let dev_255 = BatteryInfo::new(
        "DEV_01",
        "Test Device",
        255,
        true,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    assert_eq!(dev_255.level, 100, "Level 255 must be clamped to 100");

    let dev_101 = BatteryInfo::new(
        "DEV_02",
        "Test Device",
        101,
        true,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    assert_eq!(dev_101.level, 100, "Level 101 must be clamped to 100");

    // Boundary 100 remains 100
    let dev_100 = BatteryInfo::new(
        "DEV_03",
        "Test Device",
        100,
        true,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    assert_eq!(dev_100.level, 100);

    // Boundary 0 remains 0
    let dev_0 = BatteryInfo::new(
        "DEV_04",
        "Test Device",
        0,
        true,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    assert_eq!(dev_0.level, 0);
}

#[test]
fn test_mac_address_parser_valid_and_hostile_inputs() {
    // Standard colon-separated
    let mac = MacAddress::parse("00:11:22:33:44:55").unwrap();
    assert_eq!(mac.to_canonical_string(), "00:11:22:33:44:55");
    assert_eq!(mac.as_bytes(), &[0x00, 0x11, 0x22, 0x33, 0x44, 0x55]);

    // Hyphen-separated
    let mac2 = MacAddress::parse("AA-BB-CC-DD-EE-FF").unwrap();
    assert_eq!(mac2.to_canonical_string(), "AA:BB:CC:DD:EE:FF");

    // Bare 12-char hex string, lowercase
    let mac3 = MacAddress::parse("0123456789ab").unwrap();
    assert_eq!(mac3.to_canonical_string(), "01:23:45:67:89:AB");

    // Mixed casing with spaces and weird formatting
    let mac4 = MacAddress::parse(" e3:48:d8 : 11:12:9e ").unwrap();
    assert_eq!(mac4.to_canonical_string(), "E3:48:D8:11:12:9E");

    // Hostile inputs:
    // 1. Too short
    assert!(MacAddress::parse("00:11:22").is_err());
    // 2. Too long
    assert!(MacAddress::parse("00:11:22:33:44:55:66").is_err());
    // 3. Non-hex characters
    assert!(MacAddress::parse("00:11:22:33:44:ZZ").is_err());
    // 4. Empty string
    assert!(MacAddress::parse("").is_err());
    // 5. Null bytes injection
    assert!(MacAddress::parse("00:11:22\0:33:44:55").is_err());
    // 6. Whitespace only
    assert!(MacAddress::parse("          ").is_err());
}

#[test]
fn test_battery_info_mac_helper() {
    let t0 = SystemTime::now();

    // Bluetooth Classic has MAC
    let bt = BatteryInfo::new(
        "AA:BB:CC:11:22:33",
        "Headphones",
        50,
        true,
        BatterySource::OsCache,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t0,
    );
    assert_eq!(bt.mac_address(), Some("AA:BB:CC:11:22:33"));

    // Bluetooth LE has MAC
    let ble = BatteryInfo::new(
        "AA:BB:CC:44:55:66",
        "Earbuds",
        80,
        true,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t0,
    );
    assert_eq!(ble.mac_address(), Some("AA:BB:CC:44:55:66"));

    // 2.4GHz HID does NOT return MAC
    let hid = BatteryInfo::new(
        "VID_046D&PID_C539",
        "Mouse",
        90,
        true,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        DeviceCategory::Mouse,
        t0,
    );
    assert_eq!(hid.mac_address(), None);

    // Gamepad does NOT return MAC
    let gp = BatteryInfo::new(
        "XInput#0",
        "Controller",
        100,
        true,
        BatterySource::XInput,
        ConnectionType::Gamepad,
        DeviceCategory::Gamepad,
        t0,
    );
    assert_eq!(gp.mac_address(), None);
}
