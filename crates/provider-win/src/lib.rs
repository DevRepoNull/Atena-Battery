//! Windows Plug-and-Play (PnP) Battery Provider (Tier 1).
//!
//! Uses official Microsoft `windows` crate bindings to query SetupAPI
//! device information sets and Windows PnP property stores for
//! `DEVPKEY_Bluetooth_Battery` (`{104EA319-6EE2-4701-BD47-8DDBF425BBE5}, 2`).

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, ConnectionType, DeviceCategory, DeviceEvent,
    ProviderError,
};
use tokio::sync::mpsc::Sender;

pub mod ble_gatt;
pub mod hid;
pub mod pnp_watcher;
pub mod rfcomm_at;
pub mod xinput;

pub use ble_gatt::BleGattProvider;
pub use hid::Win32HidProvider;
pub use pnp_watcher::{spawn_pnp_watcher, SafeHdevnotify};
pub use rfcomm_at::LegacyRfcommProvider;
pub use xinput::Win32XInputProvider;

use windows::core::PCWSTR;
use windows::Win32::Devices::Bluetooth::{
    GUID_BLUETOOTHLE_DEVICE_INTERFACE, GUID_BTHPORT_DEVICE_INTERFACE,
};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, DIGCF_ALLCLASSES,
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, GUID_DEVCLASS_BLUETOOTH, HDEVINFO,
    SETUP_DI_GET_CLASS_DEVS_FLAGS, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{
    DEVPKEY_Device_DeviceDesc, DEVPKEY_Device_FriendlyName, DEVPKEY_Device_IsPresent, DEVPROPKEY,
    DEVPROPTYPE,
};
use windows::Win32::Foundation::HWND;

/// `DEVPKEY_Bluetooth_Battery` property key:
/// GUID: `{104EA319-6EE2-4701-BD47-8DDBF425BBE5}`, PID: 2
pub const DEVPKEY_BLUETOOTH_BATTERY: DEVPROPKEY = DEVPROPKEY {
    fmtid: windows::core::GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5),
    pid: 2,
};

/// `DEVPKEY_Bluetooth_DeviceAddress` property key:
/// GUID: `{2BD67D8B-8BEB-48D5-87E0-6CDA3428040A}`, PID: 1
pub const DEVPKEY_BLUETOOTH_DEVICE_ADDRESS: DEVPROPKEY = DEVPROPKEY {
    fmtid: windows::core::GUID::from_u128(0x2bd67d8b_8beb_48d5_87e0_6cda3428040a),
    pid: 1,
};

/// `DEVPKEY_Device_IsConnected` property key:
/// GUID: `{83DA6326-97A6-4088-9453-A1923F573B29}`, PID: 15
pub const DEVPKEY_DEVICE_IS_CONNECTED: DEVPROPKEY = DEVPROPKEY {
    fmtid: windows::core::GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29),
    pid: 15,
};

/// `DEVPKEY_Device_ContainerId` property key:
/// GUID: `{8C7FCF52-82FB-4C3A-A002-58C041018235}`, PID: 2
pub const DEVPKEY_DEVICE_CONTAINER_ID: DEVPROPKEY = DEVPROPKEY {
    fmtid: windows::core::GUID::from_u128(0x8c7fcf52_82fb_4c3a_a002_58c041018235),
    pid: 2,
};

/// Safe RAII wrapper around `HDEVINFO` ensuring `SetupDiDestroyDeviceInfoList`
/// is automatically invoked when dropped, preventing handle leaks.
pub struct DeviceInfoList(HDEVINFO);

impl DeviceInfoList {
    /// Creates a device information list using `SetupDiGetClassDevsW`.
    pub fn get_class_devs(
        class_guid: Option<&windows::core::GUID>,
        flags: SETUP_DI_GET_CLASS_DEVS_FLAGS,
    ) -> Result<Self, ProviderError> {
        // SAFETY: `SetupDiGetClassDevsW` is called with either a valid pointer to a GUID or null,
        // an empty enumerator filter, null window handle, and standard flags per Win32 documentation.
        let handle = unsafe {
            SetupDiGetClassDevsW(
                class_guid.map(|g| g as *const _),
                None,
                HWND(std::ptr::null_mut()),
                flags,
            )
        }
        .map_err(|e| ProviderError::Unavailable(format!("SetupDiGetClassDevsW failed: {e}")))?;

        Ok(Self(handle))
    }

    /// Creates a device information list filtering by enumerator (e.g. "BTHENUM", "BTHLE") using `SetupDiGetClassDevsW`.
    pub fn get_class_devs_with_enumerator(
        class_guid: Option<&windows::core::GUID>,
        enumerator: Option<&str>,
        flags: SETUP_DI_GET_CLASS_DEVS_FLAGS,
    ) -> Result<Self, ProviderError> {
        let enum_wide: Option<Vec<u16>> =
            enumerator.map(|e| e.encode_utf16().chain(std::iter::once(0)).collect());
        let enum_pcwstr = match &enum_wide {
            Some(w) => PCWSTR(w.as_ptr()),
            None => PCWSTR(std::ptr::null()),
        };

        let handle = unsafe {
            SetupDiGetClassDevsW(
                class_guid.map(|g| g as *const _),
                enum_pcwstr,
                HWND(std::ptr::null_mut()),
                flags,
            )
        }
        .map_err(|e| ProviderError::Unavailable(format!("SetupDiGetClassDevsW failed: {e}")))?;

        Ok(Self(handle))
    }

    /// Returns the inner raw `HDEVINFO` handle.
    #[inline]
    pub fn raw(&self) -> HDEVINFO {
        self.0
    }
}

impl Drop for DeviceInfoList {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is guaranteed to be a non-invalid `HDEVINFO` handle created by
            // `SetupDiGetClassDevsW` and uniquely owned by this `DeviceInfoList` instance.
            unsafe {
                let _ = SetupDiDestroyDeviceInfoList(self.0);
            }
        }
    }
}

/// Windows Bluetooth Plug-and-Play (PnP) battery provider (Tier 1).
///
/// Discovers connected Bluetooth devices via SetupAPI and queries
/// OS-cached battery levels and device metadata with microsecond latency.
#[derive(Debug, Default, Clone)]
pub struct Win32PnpProvider;

impl Win32PnpProvider {
    /// Creates a new `Win32PnpProvider` instance.
    pub fn new() -> Self {
        Self
    }

    /// Synchronously scans Bluetooth devices and retrieves battery snapshots.
    pub fn scan_devices(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        let mut devices_by_mac: HashMap<String, BatteryInfo> = HashMap::new();

        let lists = [
            // Standard Bluetooth Setup Class
            DeviceInfoList::get_class_devs(Some(&GUID_DEVCLASS_BLUETOOTH), DIGCF_PRESENT).ok(),
            // Bluetooth LE interface class
            DeviceInfoList::get_class_devs(
                Some(&GUID_BLUETOOTHLE_DEVICE_INTERFACE),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .ok(),
            // Bluetooth Port interface class
            DeviceInfoList::get_class_devs(
                Some(&GUID_BTHPORT_DEVICE_INTERFACE),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .ok(),
            // All present BTHENUM nodes (captures HFP audio gateways, AVRCP, SPP, etc.)
            DeviceInfoList::get_class_devs_with_enumerator(
                None,
                Some("BTHENUM"),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            )
            .ok(),
            // All present BTHLE nodes (captures BLE peripheral endpoints)
            DeviceInfoList::get_class_devs_with_enumerator(
                None,
                Some("BTHLE"),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            )
            .ok(),
            // All present BTH radio/adapter nodes
            DeviceInfoList::get_class_devs_with_enumerator(
                None,
                Some("BTH"),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            )
            .ok(),
        ];

        let valid_lists: Vec<DeviceInfoList> = lists.into_iter().flatten().collect();

        // Pass 1: Identify all MAC addresses and Container IDs that are actively connected
        let mut connected_macs: HashSet<String> = HashSet::new();
        let mut connected_cids: HashSet<String> = HashSet::new();

        for dev_info_list in &valid_lists {
            let mut index = 0u32;
            loop {
                let mut dev_info_data = SP_DEVINFO_DATA {
                    cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                    ..Default::default()
                };

                // SAFETY: `SetupDiEnumDeviceInfo` enumerates device information elements for the valid
                // `dev_info_list.raw()` handle using an initialized `SP_DEVINFO_DATA` struct with `cbSize` set.
                let enum_ok = unsafe {
                    SetupDiEnumDeviceInfo(dev_info_list.raw(), index, &mut dev_info_data)
                };

                if enum_ok.is_err() {
                    break;
                }
                index += 1;

                if check_is_connected(dev_info_list.raw(), &dev_info_data) {
                    let mac = extract_mac_address(dev_info_list.raw(), &dev_info_data);
                    if mac != "00:00:00:00:00:00" && !mac.is_empty() {
                        connected_macs.insert(mac);
                    }
                    if let Some(cid) = extract_container_id(dev_info_list.raw(), &dev_info_data) {
                        connected_cids.insert(cid);
                    }
                }
            }
        }

        // Pass 2: Extract battery readings
        for dev_info_list in &valid_lists {
            scan_device_info_set_with_connections(
                dev_info_list,
                &connected_macs,
                &connected_cids,
                &mut devices_by_mac,
            );
        }

        let mut results: Vec<BatteryInfo> = devices_by_mac.into_values().collect();
        results.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        Ok(results)
    }

    /// Scans connected Bluetooth devices that did NOT report battery via Tier 1.
    /// Returns a list of `(mac_address, friendly_name)` tuples.
    pub fn scan_candidates_without_tier1(&self) -> Result<Vec<(String, String)>, ProviderError> {
        // Collect MAC addresses of devices that already reported battery via Tier 1
        let tier1_devices = self.scan_devices().unwrap_or_default();
        let tier1_macs: HashSet<String> = tier1_devices.into_iter().map(|d| d.device_id).collect();

        let mut candidates_by_mac: HashMap<String, String> = HashMap::new();

        let lists = [
            DeviceInfoList::get_class_devs(Some(&GUID_DEVCLASS_BLUETOOTH), DIGCF_PRESENT).ok(),
            DeviceInfoList::get_class_devs(
                Some(&GUID_BLUETOOTHLE_DEVICE_INTERFACE),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .ok(),
            DeviceInfoList::get_class_devs(
                Some(&GUID_BTHPORT_DEVICE_INTERFACE),
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .ok(),
            DeviceInfoList::get_class_devs_with_enumerator(
                None,
                Some("BTHENUM"),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            )
            .ok(),
            DeviceInfoList::get_class_devs_with_enumerator(
                None,
                Some("BTHLE"),
                DIGCF_ALLCLASSES | DIGCF_PRESENT,
            )
            .ok(),
        ];

        let valid_lists: Vec<DeviceInfoList> = lists.into_iter().flatten().collect();

        // Collect all connected MACs
        let mut connected_macs: HashSet<String> = HashSet::new();
        for dev_info_list in &valid_lists {
            let mut index = 0u32;
            loop {
                let mut dev_info_data = SP_DEVINFO_DATA {
                    cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                    ..Default::default()
                };
                // SAFETY: `dev_info_list.raw()` is a valid HDEVINFO handle, and `dev_info_data` is initialized.
                if unsafe { SetupDiEnumDeviceInfo(dev_info_list.raw(), index, &mut dev_info_data) }
                    .is_err()
                {
                    break;
                }
                index += 1;
                if check_is_connected(dev_info_list.raw(), &dev_info_data) {
                    let mac = extract_mac_address(dev_info_list.raw(), &dev_info_data);
                    if mac != "00:00:00:00:00:00" && !mac.is_empty() {
                        connected_macs.insert(mac);
                    }
                }
            }
        }

        for dev_info_list in &valid_lists {
            let mut index = 0u32;
            loop {
                let mut dev_info_data = SP_DEVINFO_DATA {
                    cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                    ClassGuid: windows::core::GUID::default(),
                    DevInst: 0,
                    Reserved: 0,
                };

                // SAFETY: `SetupDiEnumDeviceInfo` is called with valid HDEVINFO and initialized SP_DEVINFO_DATA.
                let enum_ok = unsafe {
                    SetupDiEnumDeviceInfo(dev_info_list.raw(), index, &mut dev_info_data)
                };

                if enum_ok.is_err() {
                    break;
                }

                index += 1;

                // If device already reported battery via Tier 1, skip it!
                if read_battery_level(dev_info_list.raw(), &dev_info_data).is_some() {
                    continue;
                }

                let mac = extract_mac_address(dev_info_list.raw(), &dev_info_data);
                if mac == "00:00:00:00:00:00" || mac.is_empty() {
                    continue;
                }

                // If this device MAC already reported battery via Tier 1, do not probe it!
                if tier1_macs.contains(&mac) {
                    continue;
                }

                // Check connectivity
                let is_connected = check_is_connected(dev_info_list.raw(), &dev_info_data)
                    || connected_macs.contains(&mac);
                if !is_connected {
                    continue;
                }

                let raw_name = extract_device_name(dev_info_list.raw(), &dev_info_data);
                let name = clean_device_name(&raw_name);

                candidates_by_mac.entry(mac).or_insert(name);
            }
        }

        let mut results: Vec<(String, String)> = candidates_by_mac.into_iter().collect();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }
}

#[async_trait]
impl BatteryProvider for Win32PnpProvider {
    fn name(&self) -> &'static str {
        "Win32-PnP"
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        self.scan_devices()
    }

    async fn subscribe_events(&self, tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        pnp_watcher::spawn_pnp_watcher(self.clone(), tx)
    }
}

/// Fallback alias to preserve backwards compatibility.
pub type WindowsBatteryProvider = Win32PnpProvider;

/// Scans all device nodes within a `DeviceInfoList` and inserts discovered battery info into `devices_by_mac`.
fn scan_device_info_set_with_connections(
    dev_info_list: &DeviceInfoList,
    connected_macs: &HashSet<String>,
    connected_cids: &HashSet<String>,
    devices_by_mac: &mut HashMap<String, BatteryInfo>,
) {
    let mut index = 0u32;

    loop {
        let mut dev_info_data = SP_DEVINFO_DATA {
            cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
            ClassGuid: windows::core::GUID::default(),
            DevInst: 0,
            Reserved: 0,
        };

        // SAFETY: `dev_info_list.raw()` is a valid non-invalid `HDEVINFO` handle, `index` increments
        // sequentially, and `dev_info_data` has its `cbSize` initialized properly to the struct size.
        let enum_ok =
            unsafe { SetupDiEnumDeviceInfo(dev_info_list.raw(), index, &mut dev_info_data) };

        if enum_ok.is_err() {
            // Enumeration complete or error encountered
            break;
        }

        index += 1;

        // Attempt to read the battery percentage
        let level = match read_battery_level(dev_info_list.raw(), &dev_info_data) {
            Some(lvl) => lvl,
            None => continue,
        };

        let mac_address = extract_mac_address(dev_info_list.raw(), &dev_info_data);
        if mac_address == "00:00:00:00:00:00" || mac_address.is_empty() {
            continue;
        }

        let container_id = extract_container_id(dev_info_list.raw(), &dev_info_data);
        let raw_name = extract_device_name(dev_info_list.raw(), &dev_info_data);
        let name = clean_device_name(&raw_name);

        let is_connected = check_is_connected(dev_info_list.raw(), &dev_info_data)
            || connected_macs.contains(&mac_address)
            || container_id
                .as_ref()
                .is_some_and(|cid| connected_cids.contains(cid));

        let category = infer_category_from_name(&name);

        match devices_by_mac.get_mut(&mac_address) {
            Some(existing) => {
                // If existing entry was marked disconnected but this one is connected, promote it!
                if is_connected {
                    existing.is_connected = true;
                }
                // Update battery reading
                existing.level = level;
                existing.last_updated = SystemTime::now();

                // If existing name is generic, update with better name
                if (existing.name.contains("Unknown") || existing.name.contains("Bluetooth Device"))
                    && !name.contains("Unknown")
                {
                    existing.name = name;
                    existing.category = category;
                }
                if existing.container_id.is_none() && container_id.is_some() {
                    existing.container_id = container_id;
                }
            }
            None => {
                let mut info = BatteryInfo::new(
                    mac_address.clone(),
                    name,
                    level,
                    is_connected,
                    BatterySource::OsCache,
                    ConnectionType::BluetoothClassic,
                    category,
                    SystemTime::now(),
                );

                if let Some(cid) = container_id {
                    info = info.with_container_id(cid);
                }

                devices_by_mac.insert(mac_address, info);
            }
        }
    }
}

/// Reads a raw device property using two-pass buffer allocation with `SetupDiGetDevicePropertyW`.
fn query_device_property(
    dev_info: HDEVINFO,
    dev_info_data: &SP_DEVINFO_DATA,
    prop_key: &DEVPROPKEY,
) -> Option<(DEVPROPTYPE, Vec<u8>)> {
    let mut prop_type = DEVPROPTYPE(0);
    let mut required_size = 0u32;

    // Pass 1: Query required buffer size
    // SAFETY: We pass None as the buffer to query `required_size` safely without buffer overrun.
    let _ = unsafe {
        SetupDiGetDevicePropertyW(
            dev_info,
            dev_info_data as *const _,
            prop_key as *const _,
            &mut prop_type,
            None,
            Some(&mut required_size),
            0,
        )
    };

    if required_size == 0 {
        return None;
    }

    let mut buffer = vec![0u8; required_size as usize];

    // Pass 2: Fetch actual property data
    // SAFETY: Buffer is sized exactly to `required_size` bytes allocated on the heap.
    let res = unsafe {
        SetupDiGetDevicePropertyW(
            dev_info,
            dev_info_data as *const _,
            prop_key as *const _,
            &mut prop_type,
            Some(&mut buffer),
            Some(&mut required_size),
            0,
        )
    };

    if res.is_ok() {
        Some((prop_type, buffer))
    } else {
        None
    }
}

/// Reads the battery percentage via `DEVPKEY_BLUETOOTH_BATTERY`.
fn read_battery_level(dev_info: HDEVINFO, dev_info_data: &SP_DEVINFO_DATA) -> Option<u8> {
    let (_prop_type, buffer) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_BLUETOOTH_BATTERY)?;
    match buffer.len() {
        0 => None,
        1 => {
            let val = buffer[0];
            if val <= 100 {
                Some(val)
            } else {
                None
            }
        }
        2 => {
            let val = u16::from_le_bytes([buffer[0], buffer[1]]);
            if val <= 100 {
                Some(val as u8)
            } else {
                None
            }
        }
        4.. => {
            let val = u32::from_le_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
            if val <= 100 {
                Some(val as u8)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Extracts the device friendly name safely, falling back to device description or default string.
pub fn extract_device_name(dev_info: HDEVINFO, dev_info_data: &SP_DEVINFO_DATA) -> String {
    // 1. Try DEVPKEY_Device_FriendlyName
    if let Some((_type, buf)) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_Device_FriendlyName)
    {
        if let Some(name) = decode_utf16_string(&buf) {
            return name;
        }
    }

    // 2. Try DEVPKEY_Device_DeviceDesc
    if let Some((_type, buf)) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_Device_DeviceDesc)
    {
        if let Some(desc) = decode_utf16_string(&buf) {
            return desc;
        }
    }

    "Unknown Bluetooth Device".to_string()
}

/// Extracts the MAC address safely from properties or device instance ID.
fn extract_mac_address(dev_info: HDEVINFO, dev_info_data: &SP_DEVINFO_DATA) -> String {
    // 1. Try DEVPKEY_BLUETOOTH_DEVICE_ADDRESS
    if let Some((_type, buf)) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_BLUETOOTH_DEVICE_ADDRESS)
    {
        if let Some(str_val) = decode_utf16_string(&buf) {
            if let Some(formatted) = format_mac_from_hex(&str_val) {
                return formatted;
            }
        }
    }

    // 2. Try Device Instance ID (e.g. BTHENUM\DEV_XXXXXXXXXXXX\... or BTHLE\DEV_XXXXXXXXXXXX\...)
    if let Some(instance_id) = query_device_instance_id(dev_info, dev_info_data) {
        if let Some(mac) = extract_mac_from_instance_id(&instance_id) {
            return mac;
        }
    }

    "00:00:00:00:00:00".to_string()
}

/// Reads the device instance ID using `SetupDiGetDeviceInstanceIdW`.
fn query_device_instance_id(dev_info: HDEVINFO, dev_info_data: &SP_DEVINFO_DATA) -> Option<String> {
    let mut required_size = 0u32;

    // Pass 1: Query required character count
    // SAFETY: We pass None as buffer to safely retrieve `required_size`.
    let _ = unsafe {
        SetupDiGetDeviceInstanceIdW(
            dev_info,
            dev_info_data as *const _,
            None,
            Some(&mut required_size),
        )
    };

    if required_size == 0 {
        return None;
    }

    let mut buffer = vec![0u16; required_size as usize];

    // Pass 2: Retrieve instance ID string
    // SAFETY: Buffer is sized to `required_size` u16 characters.
    let res = unsafe {
        SetupDiGetDeviceInstanceIdW(
            dev_info,
            dev_info_data as *const _,
            Some(&mut buffer),
            Some(&mut required_size),
        )
    };

    if res.is_ok() {
        let s = String::from_utf16_lossy(&buffer);
        let trimmed = s.trim_matches(char::from(0)).trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        None
    }
}

/// Checks device connectivity via `DEVPKEY_Device_IsConnected` or `DEVPKEY_Device_IsPresent`.
fn check_is_connected(dev_info: HDEVINFO, dev_info_data: &SP_DEVINFO_DATA) -> bool {
    // 1. Try explicit DEVPKEY_DEVICE_IS_CONNECTED
    if let Some((_type, buf)) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_DEVICE_IS_CONNECTED)
    {
        if !buf.is_empty() {
            return buf[0] != 0;
        }
    }

    // 2. Fallback to DEVPKEY_Device_IsPresent
    if let Some((_type, buf)) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_Device_IsPresent)
    {
        if !buf.is_empty() {
            return buf[0] != 0;
        }
    }

    // Since DIGCF_PRESENT was specified in SetupDiGetClassDevsW, default to true
    true
}

/// Formats a 16-byte raw GUID buffer into a standard canonical `{XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX}` string.
pub fn format_guid_bytes(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 16 {
        return None;
    }
    let data1 = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let data2 = u16::from_le_bytes([bytes[4], bytes[5]]);
    let data3 = u16::from_le_bytes([bytes[6], bytes[7]]);
    Some(format!(
        "{{{data1:08X}-{data2:04X}-{data3:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        bytes[8], bytes[9], bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    ))
}

/// Extracts the Windows Device ContainerId (UUID) from the SetupAPI property store.
pub fn extract_container_id(dev_info: HDEVINFO, dev_info_data: &SP_DEVINFO_DATA) -> Option<String> {
    let (_, raw_bytes) =
        query_device_property(dev_info, dev_info_data, &DEVPKEY_DEVICE_CONTAINER_ID)?;
    format_guid_bytes(&raw_bytes)
}

/// Infers the high-level `DeviceCategory` from peripheral name heuristics.
pub fn infer_category_from_name(name: &str) -> DeviceCategory {
    let lower = name.to_ascii_lowercase();
    if lower.contains("mouse") {
        DeviceCategory::Mouse
    } else if lower.contains("keyboard") || lower.contains("keypad") {
        DeviceCategory::Keyboard
    } else if lower.contains("gamepad")
        || lower.contains("controller")
        || lower.contains("joystick")
    {
        DeviceCategory::Gamepad
    } else if lower.contains("headset")
        || lower.contains("headphone")
        || lower.contains("earbud")
        || lower.contains("buds")
        || lower.contains("audio")
        || lower.contains("speaker")
        || lower.contains("sound")
        || lower.contains("airpods")
    {
        DeviceCategory::Audio
    } else {
        DeviceCategory::Peripheral
    }
}

/// Decodes a UTF-16LE byte buffer into a clean Rust `String` without panicking on malformed bytes.
fn decode_utf16_string(buf: &[u8]) -> Option<String> {
    if buf.is_empty() {
        return None;
    }

    // Safe truncation to even length if odd byte count encountered
    let even_len = buf.len() - (buf.len() % 2);
    if even_len == 0 {
        return None;
    }

    let (chunks, _) = buf[..even_len].as_chunks::<2>();
    let u16_slice: Vec<u16> = chunks
        .iter()
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .take_while(|&val| val != 0)
        .collect();

    let s = String::from_utf16_lossy(&u16_slice);
    let trimmed = s.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Cleans and sanitizes raw device names according to security and UI specifications.
///
/// Strips:
/// - Leading/trailing driver annotations and transport suffixes
/// - Non-printable ASCII control characters (`\x00`..=`\x1F`, `\x7F`)
/// - Unicode bidirectional override characters (`\u{200E}`, `\u{200F}`, `\u{202A}`..=`\u{202E}`, `\u{2066}`..=`\u{2069}`) to prevent UI spoofing
/// - Enforces maximum length of 64 characters to prevent UI buffer expansion.
pub fn clean_device_name(raw: &str) -> String {
    // 1. Sanitize untrusted input: strip control characters and Bidi overrides
    let sanitized: String = raw
        .chars()
        .filter(|&c| {
            // Strip ASCII control characters
            if c.is_ascii_control() {
                return false;
            }
            // Strip Unicode BiDi overrides / formatting controls
            !matches!(
                c,
                '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
            )
        })
        .collect();

    let mut name = sanitized.trim();

    // 2. Strip leading GATT descriptor
    if let Some(stripped) = name.strip_prefix("Bluetooth LE GATT compliant ") {
        name = stripped.trim();
    } else if let Some(stripped) = name.strip_prefix("bluetooth le gatt compliant ") {
        name = stripped.trim();
    }

    // 3. Strip trailing driver endpoint annotations
    let suffixes = [
        " Hands-Free AG Audio",
        " Hands-Free AG",
        " Hands-Free Audio",
        " Hands-Free",
        " Avrcp Transport",
        " Bluetooth LE Device",
        " Bluetooth Device",
        " Stereo",
        " Audio",
        " Headphones",
        " Headset",
        " GATT",
        " Service",
        " (BLE)",
    ];

    let mut changed = true;
    while changed {
        changed = false;
        for suffix in &suffixes {
            if let Some(stripped) = name.strip_suffix(suffix) {
                name = stripped.trim();
                changed = true;
                break;
            } else if name.len() >= suffix.len() {
                let start = name.len() - suffix.len();
                if name.is_char_boundary(start) && name[start..].eq_ignore_ascii_case(suffix) {
                    name = name[..start].trim();
                    changed = true;
                    break;
                }
            }
        }
    }

    // 4. Enforce max length of 64 characters
    let truncated: String = name.chars().take(64).collect();

    if truncated.trim().is_empty() {
        "Unknown Bluetooth Device".to_string()
    } else {
        truncated.trim().to_string()
    }
}

/// Formats a raw hex string of 12 hex digits into standard `AA:BB:CC:DD:EE:FF` format.
fn format_mac_from_hex(input: &str) -> Option<String> {
    let clean: String = input.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if clean.len() == 12 {
        let mut formatted = String::with_capacity(17);
        for (i, ch) in clean.chars().enumerate() {
            if i > 0 && i.is_multiple_of(2) {
                formatted.push(':');
            }
            formatted.push(ch.to_ascii_uppercase());
        }
        Some(formatted)
    } else {
        None
    }
}

/// Searches for a 12-digit hex MAC address inside a device instance ID.
fn extract_mac_from_instance_id(instance_id: &str) -> Option<String> {
    let upper = instance_id.to_uppercase();

    // Check for "DEV_" prefix common in BTHENUM and BTHLE instance IDs
    if let Some(pos) = upper.find("DEV_") {
        let candidate = &upper[pos + 4..];
        let hex_part: String = candidate
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if hex_part.len() == 12 {
            return format_mac_from_hex(&hex_part);
        }
    }

    // Check for "BLUETOOTHDEVICE_" prefix
    if let Some(pos) = upper.find("BLUETOOTHDEVICE_") {
        let candidate = &upper[pos + 16..];
        let hex_part: String = candidate
            .chars()
            .take_while(|c| c.is_ascii_hexdigit())
            .collect();
        if hex_part.len() == 12 {
            return format_mac_from_hex(&hex_part);
        }
    }

    // Fallback: look for any 12-character hex sequence bordered by delimiters
    for segment in upper.split(|c: char| !c.is_ascii_hexdigit()) {
        if segment.len() == 12 {
            return format_mac_from_hex(segment);
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_mac_from_hex() {
        assert_eq!(
            format_mac_from_hex("e348d811129e"),
            Some("E3:48:D8:11:12:9E".to_string())
        );
        assert_eq!(
            format_mac_from_hex("E3:48:D8:11:12:9E"),
            Some("E3:48:D8:11:12:9E".to_string())
        );
        assert_eq!(format_mac_from_hex("short"), None);
    }

    #[test]
    fn test_extract_mac_from_instance_id() {
        let id1 = r"BTHLE\DEV_E348D811129E\8&27BB0ACF&0&E348D811129E";
        assert_eq!(
            extract_mac_from_instance_id(id1),
            Some("E3:48:D8:11:12:9E".to_string())
        );

        let id2 = r"BTHENUM\DEV_DD688054D8F1\8&2D58A30&0&BLUETOOTHDEVICE_DD688054D8F1";
        assert_eq!(
            extract_mac_from_instance_id(id2),
            Some("DD:68:80:54:D8:F1".to_string())
        );
    }

    #[test]
    fn test_scan_devices_does_not_panic() {
        let provider = Win32PnpProvider::new();
        let devices = provider.scan_devices().unwrap();
        assert!(devices.iter().all(|d| d.level <= 100));
    }

    #[test]
    fn test_clean_device_name() {
        assert_eq!(
            clean_device_name("Sony WH-1000XM4 Hands-Free AG Audio"),
            "Sony WH-1000XM4"
        );
        assert_eq!(
            clean_device_name("Logitech MX Master 3S Bluetooth LE Device"),
            "Logitech MX Master 3S"
        );
        assert_eq!(
            clean_device_name("Bluetooth LE GATT compliant HID Mouse"),
            "HID Mouse"
        );
        assert_eq!(clean_device_name("AirPods Pro (BLE)"), "AirPods Pro");
        // Test Bidi override removal (RTL override '\u{202E}')
        assert_eq!(
            clean_device_name("Legit\u{202E}txt.exe Headset"),
            "Legittxt.exe"
        );
        // Test ASCII control characters removal
        assert_eq!(
            clean_device_name("Bad\x00\x07Device\x1F Name"),
            "BadDevice Name"
        );
        // Test truncation to 64 chars
        let long_name = "A".repeat(100);
        assert_eq!(clean_device_name(&long_name).len(), 64);
        // Test empty name fallback
        assert_eq!(clean_device_name("   "), "Unknown Bluetooth Device");
        assert_eq!(clean_device_name("Creative Audio"), "Creative");
        assert_eq!(clean_device_name("   "), "Unknown Bluetooth Device");
    }

    #[test]
    fn test_decode_utf16_string_safe() {
        // Normal string: "Test" in UTF-16LE
        let bytes = vec![b'T', 0, b'e', 0, b's', 0, b't', 0, 0, 0];
        assert_eq!(decode_utf16_string(&bytes), Some("Test".to_string()));

        // Odd length bytes (truncated byte at end)
        let odd_bytes = vec![b'H', 0, b'i', 0, 42];
        assert_eq!(decode_utf16_string(&odd_bytes), Some("Hi".to_string()));

        // Embedded null terminator: terminates at first null
        let null_embedded = vec![b'A', 0, 0, 0, b'B', 0, 0, 0];
        assert_eq!(decode_utf16_string(&null_embedded), Some("A".to_string()));

        // Only null bytes
        let only_nulls = vec![0, 0, 0, 0];
        assert_eq!(decode_utf16_string(&only_nulls), None);

        // Single isolated byte
        assert_eq!(decode_utf16_string(&[65]), None);

        // Empty bytes
        assert_eq!(decode_utf16_string(&[]), None);
    }

    #[test]
    fn test_clean_device_name_unicode_and_hostile() {
        // Only control chars falls back to Unknown
        assert_eq!(
            clean_device_name("\x00\x01\x02\x03"),
            "Unknown Bluetooth Device"
        );

        // Non-ASCII unicode characters safe truncation without char boundary panics
        let arabic = "هدفون بی سیم بلوتوث مدل پیشرفته ".repeat(10);
        let cleaned = clean_device_name(&arabic);
        assert!(cleaned.chars().count() <= 64);
        assert!(!cleaned.is_empty());

        let persian_repeat = "هدفون".repeat(20);
        assert_eq!(clean_device_name(&persian_repeat).chars().count(), 64);
    }
}
