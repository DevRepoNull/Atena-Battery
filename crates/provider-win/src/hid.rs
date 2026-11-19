//! Windows Human Interface Device (HID) Battery Provider (Tier 2).
//!
//! Enumerate present HID devices via SetupAPI (`GUID_DEVINTERFACE_HID`),
//! queries standard HID Battery reports (Usage Page 0x06 / Usage 0x20 Battery Strength,
//! and Usage Page 0x84 / 0x85 Power/Battery System), and safely extracts battery levels
//! for 2.4GHz USB-dongle wireless peripherals (mice, keyboards, headsets).

use std::collections::HashMap;
use std::time::SystemTime;

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, ConnectionType, DeviceCategory, DeviceEvent,
    ProviderError,
};
use tokio::sync::mpsc::Sender;

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiEnumDeviceInterfaces, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE,
    DIGCF_PRESENT, HDEVINFO, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    SP_DEVINFO_DATA,
};
use windows::Win32::Devices::HumanInterfaceDevice::{
    HidD_FreePreparsedData, HidD_GetFeature, HidD_GetPreparsedData, HidD_GetProductString,
    HidP_Feature, HidP_GetCaps, HidP_GetUsageValue, HidP_GetValueCaps, HidP_Input, HIDP_CAPS,
    HIDP_STATUS_SUCCESS, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_GENERIC_READ, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

use crate::pnp_watcher::GUID_DEVINTERFACE_HID;
use crate::{
    clean_device_name, extract_container_id, extract_device_name, infer_category_from_name,
    DeviceInfoList,
};

/// Safe RAII wrapper for an open Win32 file handle.
pub struct SafeFileHandle(pub HANDLE);

impl Drop for SafeFileHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is guaranteed to be an open file handle uniquely owned by `SafeFileHandle`.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Safe RAII wrapper for HID preparsed data allocated by `HidD_GetPreparsedData`.
pub struct SafePreparsedData(pub PHIDP_PREPARSED_DATA);

impl Drop for SafePreparsedData {
    fn drop(&mut self) {
        if self.0 .0 != 0 {
            // SAFETY: `self.0` is a non-null preparsed data block returned by `HidD_GetPreparsedData`
            // and must be freed using `HidD_FreePreparsedData`.
            unsafe {
                let _ = HidD_FreePreparsedData(self.0);
            }
        }
    }
}

/// Windows HID Battery Provider for 2.4GHz wireless dongles and USB peripherals.
#[derive(Debug, Default, Clone)]
pub struct Win32HidProvider;

impl Win32HidProvider {
    /// Creates a new `Win32HidProvider` instance.
    pub fn new() -> Self {
        Self
    }

    /// Synchronously scans for connected HID devices that expose battery capabilities.
    pub fn scan_hid_devices(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        let dev_info_list = DeviceInfoList::get_class_devs(
            Some(&GUID_DEVINTERFACE_HID),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )?;

        let mut devices_by_id: HashMap<String, BatteryInfo> = HashMap::new();
        let mut index = 0u32;
        let now = SystemTime::now();

        loop {
            let mut iface_data = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };

            // SAFETY: `SetupDiEnumDeviceInterfaces` enumerates registered device interfaces
            // with a valid HDEVINFO handle and pointer to initialized SP_DEVICE_INTERFACE_DATA.
            let ok = unsafe {
                SetupDiEnumDeviceInterfaces(
                    dev_info_list.raw(),
                    None,
                    &GUID_DEVINTERFACE_HID,
                    index,
                    &mut iface_data,
                )
            };

            if ok.is_err() {
                break;
            }
            index += 1;

            // Retrieve required size for interface detail
            let mut required_size = 0u32;
            // SAFETY: First call with null detail pointer to determine required buffer size.
            unsafe {
                let _ = SetupDiGetDeviceInterfaceDetailW(
                    dev_info_list.raw(),
                    &iface_data,
                    None,
                    0,
                    Some(&mut required_size),
                    None,
                );
            }

            if required_size == 0 {
                continue;
            }

            let mut detail_buffer = vec![0u8; required_size as usize];
            let detail_ptr = detail_buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            // SAFETY: Setting cbSize to the architecture-specific size of SP_DEVICE_INTERFACE_DETAIL_DATA_W.
            unsafe {
                (*detail_ptr).cbSize =
                    std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            }

            let mut dev_info_data = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };

            // SAFETY: Second call retrieves the actual interface device path and devinfo data.
            let detail_ok = unsafe {
                SetupDiGetDeviceInterfaceDetailW(
                    dev_info_list.raw(),
                    &iface_data,
                    Some(detail_ptr),
                    required_size,
                    Some(&mut required_size),
                    Some(&mut dev_info_data),
                )
            };

            if detail_ok.is_err() {
                continue;
            }

            // Extract null-terminated device path from detail buffer
            // SAFETY: `detail_buffer` is allocated with `required_size` bytes. `path_start` points to
            // the beginning of `DevicePath`. Scanning is strictly bounded by `buffer_end` to prevent overruns.
            let device_path = unsafe {
                let path_start = (*detail_ptr).DevicePath.as_ptr();
                let buffer_end = detail_buffer.as_ptr().add(detail_buffer.len()) as *const u16;
                let mut len = 0;
                while path_start.add(len) < buffer_end && *path_start.add(len) != 0 {
                    len += 1;
                }
                let slice = std::slice::from_raw_parts(path_start, len);
                String::from_utf16_lossy(slice)
            };

            if device_path.is_empty() {
                continue;
            }

            // Attempt to query battery report from the HID device
            if let Some((name, level, category, conn_type)) =
                query_hid_battery(dev_info_list.raw(), &dev_info_data, &device_path)
            {
                let container_id = extract_container_id(dev_info_list.raw(), &dev_info_data);
                let normalized_id = normalize_hid_id(&device_path);

                let mut info = BatteryInfo::new(
                    normalized_id.clone(),
                    name,
                    level,
                    true,
                    BatterySource::HidStandard,
                    conn_type,
                    category,
                    now,
                );

                if let Some(cid) = container_id {
                    info = info.with_container_id(cid);
                }

                devices_by_id.entry(normalized_id).or_insert(info);
            }
        }

        let mut results: Vec<BatteryInfo> = devices_by_id.into_values().collect();
        results.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        Ok(results)
    }
}

#[async_trait]
impl BatteryProvider for Win32HidProvider {
    fn name(&self) -> &'static str {
        "Win32-HID"
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        self.scan_hid_devices()
    }

    async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// Normalizes raw Windows device path (e.g., `\\?\hid#vid_046d&pid_c539...`) into a compact identifier.
fn normalize_hid_id(path: &str) -> String {
    let lower = path.trim().to_uppercase();
    if let Some(start) = lower.find("VID_") {
        let tail = &lower[start..];
        if let Some(end) = tail.find('#') {
            return tail[..end].to_string();
        }
    }
    lower
}

/// Queries battery details from an open HID device node.
fn query_hid_battery(
    dev_info: HDEVINFO,
    dev_info_data: &SP_DEVINFO_DATA,
    device_path: &str,
) -> Option<(String, u8, DeviceCategory, ConnectionType)> {
    let path_wide: Vec<u16> = device_path
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: CreateFileW opens the HID device interface for querying preparsed descriptor reports.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_wide.as_ptr()),
            FILE_GENERIC_READ.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            HANDLE::default(),
        )
    };

    if handle.is_err() {
        return None;
    }
    let safe_handle = SafeFileHandle(match handle {
        Ok(h) => h,
        Err(_) => return None,
    });

    let mut preparsed = PHIDP_PREPARSED_DATA::default();
    // SAFETY: HidD_GetPreparsedData allocates preparsed data for the opened device handle.
    let ok = unsafe { HidD_GetPreparsedData(safe_handle.0, &mut preparsed) };
    if !ok.as_bool() || preparsed.0 == 0 {
        return None;
    }
    let safe_preparsed = SafePreparsedData(preparsed);

    let mut caps = HIDP_CAPS::default();
    // SAFETY: HidP_GetCaps fills HIDP_CAPS using valid preparsed data.
    let status = unsafe { HidP_GetCaps(safe_preparsed.0, &mut caps) };
    if status != HIDP_STATUS_SUCCESS {
        return None;
    }

    // Determine category and transport connection type
    let category = determine_hid_category(caps.UsagePage, caps.Usage, device_path);
    let conn_type = if device_path.to_ascii_lowercase().contains("bth") {
        ConnectionType::BluetoothLE
    } else {
        ConnectionType::Wireless24Ghz
    };

    // 1. Inspect Feature Value Caps for Battery Reports (Usage Page 0x06 Usage 0x20, or Usage Page 0x84/0x85)
    let mut num_feature_caps = caps.NumberFeatureValueCaps;
    let mut feature_caps = vec![HIDP_VALUE_CAPS::default(); num_feature_caps as usize];
    if num_feature_caps > 0 {
        // SAFETY: HidP_GetValueCaps reads feature value descriptors into a pre-allocated buffer.
        let _ = unsafe {
            HidP_GetValueCaps(
                HidP_Feature,
                feature_caps.as_mut_ptr(),
                &mut num_feature_caps,
                safe_preparsed.0,
            )
        };
        feature_caps.truncate(num_feature_caps as usize);
    }

    // 2. Inspect Input Value Caps
    let mut num_input_caps = caps.NumberInputValueCaps;
    let mut input_caps = vec![HIDP_VALUE_CAPS::default(); num_input_caps as usize];
    if num_input_caps > 0 {
        // SAFETY: HidP_GetValueCaps reads input value descriptors into a pre-allocated buffer.
        let _ = unsafe {
            HidP_GetValueCaps(
                HidP_Input,
                input_caps.as_mut_ptr(),
                &mut num_input_caps,
                safe_preparsed.0,
            )
        };
        input_caps.truncate(num_input_caps as usize);
    }

    // Look for matching Battery usage in Feature or Input caps
    let target_cap = feature_caps
        .iter()
        .find(|vc| is_battery_usage(vc))
        .or_else(|| input_caps.iter().find(|vc| is_battery_usage(vc)))?;

    // Attempt to read the value report
    let level = read_hid_value(safe_handle.0, safe_preparsed.0, &caps, target_cap)?;

    // Extract device name (via HidD_GetProductString or SetupAPI)
    let raw_name = read_hid_product_name(safe_handle.0).unwrap_or_else(|| {
        let n = extract_device_name(dev_info, dev_info_data);
        if n.is_empty() {
            format!("Wireless {}", category)
        } else {
            n
        }
    });
    let name = clean_device_name(&raw_name);

    Some((name, level, category, conn_type))
}

/// Checks if an HID Value Cap represents a battery measurement.
fn is_battery_usage(vc: &HIDP_VALUE_CAPS) -> bool {
    // SAFETY: Accessing union field `Anonymous.Range` or `Anonymous.NotRange` according to discriminative `vc.IsRange`.
    let usage = unsafe {
        if vc.IsRange.as_bool() {
            vc.Anonymous.Range.UsageMin
        } else {
            vc.Anonymous.NotRange.Usage
        }
    };

    // Generic Device Controls (0x06) -> Battery Strength (0x20)
    if vc.UsagePage == 0x06 && usage == 0x20 {
        return true;
    }
    // Power Device (0x84) or Battery System (0x85)
    if vc.UsagePage == 0x84 || vc.UsagePage == 0x85 {
        return true;
    }

    false
}

/// Reads and parses an HID Feature or Input battery value.
fn read_hid_value(
    handle: HANDLE,
    preparsed: PHIDP_PREPARSED_DATA,
    caps: &HIDP_CAPS,
    vc: &HIDP_VALUE_CAPS,
) -> Option<u8> {
    let report_len = caps.FeatureReportByteLength as usize;
    if report_len == 0 {
        return None;
    }

    let mut report_buf = vec![0u8; report_len];
    report_buf[0] = vc.ReportID;

    // SAFETY: HidD_GetFeature queries the feature report buffer from the open device handle.
    let ok =
        unsafe { HidD_GetFeature(handle, report_buf.as_mut_ptr() as *mut _, report_len as u32) };

    if !ok.as_bool() {
        return None;
    }

    // SAFETY: Reading union field based on discriminative `vc.IsRange`.
    let usage = unsafe {
        if vc.IsRange.as_bool() {
            vc.Anonymous.Range.UsageMin
        } else {
            vc.Anonymous.NotRange.Usage
        }
    };

    let mut raw_val = 0u32;
    // SAFETY: HidP_GetUsageValue parses raw report buffer using preparsed data and usage identifiers.
    let status = unsafe {
        HidP_GetUsageValue(
            HidP_Feature,
            vc.UsagePage,
            0,
            usage,
            &mut raw_val,
            preparsed,
            &report_buf,
        )
    };

    if status != HIDP_STATUS_SUCCESS {
        return None;
    }

    Some(normalize_battery(raw_val, vc.LogicalMin, vc.LogicalMax))
}

/// Normalizes raw integer value using logical minimum and maximum bounds.
fn normalize_battery(raw: u32, logical_min: i32, logical_max: i32) -> u8 {
    if logical_max > logical_min && logical_max > 0 {
        let min = logical_min.max(0) as f64;
        let max = logical_max as f64;
        let val = (raw as f64).clamp(min, max);
        let pct = ((val - min) / (max - min) * 100.0).round();
        (pct as u8).min(100)
    } else if raw <= 100 {
        raw as u8
    } else {
        100
    }
}

/// Reads the product name string via `HidD_GetProductString`.
fn read_hid_product_name(handle: HANDLE) -> Option<String> {
    let mut buf = [0u16; 128];
    // SAFETY: HidD_GetProductString writes wide characters into the stack-allocated buffer.
    let ok = unsafe {
        HidD_GetProductString(handle, buf.as_mut_ptr() as *mut _, (buf.len() * 2) as u32)
    };

    if ok.as_bool() {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        if len > 0 {
            return Some(String::from_utf16_lossy(&buf[..len]));
        }
    }
    None
}

/// Determines the high-level device category based on HID Desktop Usage or path heuristics.
fn determine_hid_category(usage_page: u16, usage: u16, path: &str) -> DeviceCategory {
    // Generic Desktop Page (0x01)
    if usage_page == 0x01 {
        match usage {
            0x02 => return DeviceCategory::Mouse,
            0x06 => return DeviceCategory::Keyboard,
            0x04 | 0x05 => return DeviceCategory::Gamepad,
            _ => {}
        }
    }
    // Consumer Controls Page (0x0C)
    if usage_page == 0x0C {
        return DeviceCategory::Audio;
    }

    infer_category_from_name(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_battery() {
        assert_eq!(normalize_battery(50, 0, 100), 50);
        assert_eq!(normalize_battery(0, 0, 100), 0);
        assert_eq!(normalize_battery(100, 0, 100), 100);
        assert_eq!(normalize_battery(150, 0, 100), 100);

        // Logical scale: 0 to 255
        assert_eq!(normalize_battery(128, 0, 255), 50);
        assert_eq!(normalize_battery(255, 0, 255), 100);
        assert_eq!(normalize_battery(300, 0, 255), 100);

        // Binary 0..1 battery indicator
        assert_eq!(normalize_battery(0, 0, 1), 0);
        assert_eq!(normalize_battery(1, 0, 1), 100);

        // Corrupted descriptors: min >= max
        assert_eq!(normalize_battery(50, 100, 100), 50);
        assert_eq!(normalize_battery(150, 100, 100), 100);
        assert_eq!(normalize_battery(60, 100, 50), 60);
    }

    #[test]
    fn test_normalize_hid_id() {
        let path = r"\\?\hid#vid_046d&pid_c539&mi_02#7&1a2b3c4d&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let id = normalize_hid_id(path);
        assert_eq!(id, "VID_046D&PID_C539&MI_02");
    }
}
