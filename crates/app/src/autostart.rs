//! Windows Autostart Management via CurrentVersion\\Run Registry Key.
//!
//! Controls whether Atena-Battery launches on Windows user logon by manipulating:
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`
//! Value Name: `"AtenaBattery"`
//!
//! Employs a strict RAII `SafeHkey` guard to guarantee `RegCloseKey` is always called.

use std::path::Path;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW, HKEY,
    HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, REG_VALUE_TYPE,
};

const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: PCWSTR = w!("AtenaBattery");

/// Safe RAII wrapper for Win32 `HKEY` handle.
pub struct SafeHkey(HKEY);

impl Drop for SafeHkey {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is a valid opened HKEY that must be closed.
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

/// Checks whether Atena-Battery is currently registered for autostart in `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
pub fn is_autostart_enabled() -> bool {
    let mut raw_key = HKEY::default();
    // SAFETY: We provide valid pointer to receive opened key. Uses least-privilege KEY_QUERY_VALUE.
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            0,
            KEY_QUERY_VALUE,
            &mut raw_key,
        )
    };

    if status.is_err() {
        return false;
    }

    let _safe_key = SafeHkey(raw_key);
    let mut val_type = REG_VALUE_TYPE::default();
    let mut data_len = 0u32;

    // SAFETY: Querying value length and existence with NULL buffer.
    let query_res = unsafe {
        RegQueryValueExW(
            raw_key,
            VALUE_NAME,
            None,
            Some(&mut val_type),
            None,
            Some(&mut data_len),
        )
    };

    query_res.is_ok() && data_len > 0
}

/// Enables or disables autostart in the registry.
pub fn set_autostart_enabled(enabled: bool) -> Result<(), WIN32_ERROR> {
    let mut raw_key = HKEY::default();
    // SAFETY: Opening key with least-privilege write permission (KEY_SET_VALUE).
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            0,
            KEY_SET_VALUE,
            &mut raw_key,
        )
    };

    if let Err(e) = status.ok() {
        return Err(WIN32_ERROR(e.code().0 as u32));
    }

    let _safe_key = SafeHkey(raw_key);

    if enabled {
        let exe_path = std::env::current_exe().map_err(|_| WIN32_ERROR(1))?;
        set_autostart_for_path(raw_key, &exe_path)
    } else {
        // SAFETY: Deleting value from opened registry key.
        let del_res = unsafe { RegDeleteValueW(raw_key, VALUE_NAME) };
        // If the value did not exist, it's not an error
        let code = del_res.0;
        if code == 0 || code == 2 {
            Ok(())
        } else {
            Err(del_res)
        }
    }
}

fn set_autostart_for_path(raw_key: HKEY, path: &Path) -> Result<(), WIN32_ERROR> {
    // Prevent Unquoted Path Vulnerability: always enclose executable path in double quotes
    let path_str = path.to_string_lossy();
    let formatted = if path_str.starts_with('"') && path_str.ends_with('"') {
        path_str.into_owned()
    } else {
        format!("\"{path_str}\"")
    };

    let mut wide_data: Vec<u16> = formatted.encode_utf16().collect();
    wide_data.push(0); // null terminator

    // SAFETY:
    // 1. `wide_data` is a heap-allocated `Vec<u16>` that remains live for the duration of this call.
    // 2. The pointer is non-null, properly aligned, and spans exactly `wide_data.len() * 2` bytes.
    let byte_slice = unsafe {
        std::slice::from_raw_parts(
            wide_data.as_ptr() as *const u8,
            wide_data.len() * std::mem::size_of::<u16>(),
        )
    };

    // SAFETY: Writing string value to opened registry key.
    let set_res = unsafe { RegSetValueExW(raw_key, VALUE_NAME, 0, REG_SZ, Some(byte_slice)) };

    if set_res.0 == 0 {
        Ok(())
    } else {
        Err(set_res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autostart_query_does_not_panic() {
        // Querying should execute cleanly and return bool without crashing
        let _ = is_autostart_enabled();
    }
}
