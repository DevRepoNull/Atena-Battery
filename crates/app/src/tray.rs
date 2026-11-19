//! Native System Tray and Context Menu Management.
//!
//! Uses `tray-icon` and `muda` to build a native Windows system tray icon
//! with a dynamic context menu containing:
//! - Real-time list of connected devices with battery levels and source tags
//! - "Refresh Now" action
//! - "Low Battery Threshold" submenu (10% / 20% / 30%)
//! - "Start with Windows" toggle check item
//! - "Open Logs Folder" action
//! - "Quit" action
//! - Dynamic tooltip and icon reflecting the lowest-battery peripheral.

use atena_core::{AppConfig, BatteryInfo};
use muda::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::autostart;
use crate::icon::create_tray_icon;

pub const ID_REFRESH: &str = "action_refresh";
pub const ID_THRESH_10: &str = "action_thresh_10";
pub const ID_THRESH_20: &str = "action_thresh_20";
pub const ID_THRESH_30: &str = "action_thresh_30";
pub const ID_AUTOSTART: &str = "action_autostart";
pub const ID_OPEN_LOGS: &str = "action_open_logs";
pub const ID_QUIT: &str = "action_quit";

/// High-level System Tray Manager.
pub struct TrayManager {
    tray_icon: TrayIcon,
    current_threshold: u8,
}

impl TrayManager {
    /// Builds and initializes the native system tray icon and menu.
    pub fn new(
        initial_devices: &[BatteryInfo],
        config: &AppConfig,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let threshold = config.notifications.warning_threshold;
        let lowest = get_primary_or_lowest_device(initial_devices);

        let initial_level = lowest.map(|d| d.level).unwrap_or(100);
        let is_connected = lowest.map(|d| d.is_connected).unwrap_or(false);

        let icon = create_tray_icon(initial_level, is_connected)?;
        let menu = build_context_menu(initial_devices, threshold)?;
        let tooltip = format_tooltip(initial_devices);

        let tray_icon = TrayIconBuilder::new()
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .with_tooltip(tooltip)
            .build()?;

        Ok(Self {
            tray_icon,
            current_threshold: threshold,
        })
    }

    /// Updates the system tray state with the latest device list.
    pub fn update_devices(
        &mut self,
        devices: &[BatteryInfo],
    ) -> Result<(), Box<dyn std::error::Error>> {
        // 1. Update context menu
        let menu = build_context_menu(devices, self.current_threshold)?;
        self.tray_icon.set_menu(Some(Box::new(menu)));

        // 2. Update tooltip
        let tooltip = format_tooltip(devices);
        let _ = self.tray_icon.set_tooltip(Some(tooltip));

        // 3. Update icon color/level
        let lowest = get_primary_or_lowest_device(devices);
        let level = lowest.map(|d| d.level).unwrap_or(100);
        let is_connected = lowest.map(|d| d.is_connected).unwrap_or(false);
        if let Ok(icon) = create_tray_icon(level, is_connected) {
            let _ = self.tray_icon.set_icon(Some(icon));
        }

        Ok(())
    }

    /// Updates the active low battery warning threshold and refreshes the context menu.
    pub fn set_threshold(
        &mut self,
        devices: &[BatteryInfo],
        new_threshold: u8,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.current_threshold = new_threshold;
        let menu = build_context_menu(devices, new_threshold)?;
        self.tray_icon.set_menu(Some(Box::new(menu)));
        Ok(())
    }

    /// Accessor for current threshold.
    pub fn threshold(&self) -> u8 {
        self.current_threshold
    }
}

/// Identifies the lowest-battery connected peripheral or first device.
pub fn get_primary_or_lowest_device(devices: &[BatteryInfo]) -> Option<&BatteryInfo> {
    devices
        .iter()
        .filter(|d| d.is_connected)
        .min_by_key(|d| d.level)
        .or_else(|| devices.first())
}

/// Formats the tray tooltip string based on connected devices.
pub fn format_tooltip(devices: &[BatteryInfo]) -> String {
    let connected: Vec<&BatteryInfo> = devices.iter().filter(|d| d.is_connected).collect();
    if connected.is_empty() {
        "Atena Battery: No devices connected".to_string()
    } else if connected.len() == 1 {
        let dev = connected[0];
        format!("Atena Battery: {} ({}%)", dev.name, dev.level)
    } else {
        let lowest = connected.iter().min_by_key(|d| d.level);
        if let Some(low) = lowest {
            format!(
                "Atena Battery: {} device(s) connected\nLowest: {} ({}%)",
                connected.len(),
                low.name,
                low.level
            )
        } else {
            format!("Atena Battery: {} device(s) connected", connected.len())
        }
    }
}

/// Constructs the native `muda::Menu` context menu.
pub fn build_context_menu(
    devices: &[BatteryInfo],
    active_threshold: u8,
) -> Result<Menu, Box<dyn std::error::Error>> {
    let menu = Menu::new();

    // 1. Device list section
    let connected: Vec<&BatteryInfo> = devices.iter().filter(|d| d.is_connected).collect();
    if connected.is_empty() {
        let item = MenuItem::new("No connected devices", false, None);
        menu.append(&item)?;
    } else {
        for dev in connected {
            let indicator = if dev.level > 50 {
                "🟢"
            } else if dev.level >= 20 {
                "🟡"
            } else {
                "🔴"
            };
            let label = format!(
                "{} {} {} \u{2014} {}%",
                indicator,
                dev.category.tag(),
                dev.name,
                dev.level
            );
            // Disabled item serves as clean visual indicator
            let item = MenuItem::new(&label, false, None);
            menu.append(&item)?;
        }
    }

    menu.append(&PredefinedMenuItem::separator())?;

    // 2. Action: Refresh Now
    let refresh_item = MenuItem::with_id(ID_REFRESH, "🔄 Refresh Now", true, None);
    menu.append(&refresh_item)?;

    // 3. Submenu: Low Battery Threshold
    let thresh_menu = Submenu::new("⚡ Low Battery Threshold", true);
    let t10 = CheckMenuItem::with_id(
        ID_THRESH_10,
        "10% (Critical)",
        true,
        active_threshold == 10,
        None,
    );
    let t20 = CheckMenuItem::with_id(
        ID_THRESH_20,
        "20% (Default)",
        true,
        active_threshold == 20,
        None,
    );
    let t30 = CheckMenuItem::with_id(
        ID_THRESH_30,
        "30% (Early Warning)",
        true,
        active_threshold == 30,
        None,
    );
    thresh_menu.append(&t10)?;
    thresh_menu.append(&t20)?;
    thresh_menu.append(&t30)?;
    menu.append(&thresh_menu)?;

    // 4. Toggle: Start with Windows
    let is_autostart = autostart::is_autostart_enabled();
    let autostart_item = CheckMenuItem::with_id(
        ID_AUTOSTART,
        "🚀 Start with Windows",
        true,
        is_autostart,
        None,
    );
    menu.append(&autostart_item)?;

    // 5. Action: Open Logs Folder
    let logs_item = MenuItem::with_id(ID_OPEN_LOGS, "📁 Open Logs Folder", true, None);
    menu.append(&logs_item)?;

    menu.append(&PredefinedMenuItem::separator())?;

    // 6. Action: Quit
    let quit_item = MenuItem::with_id(ID_QUIT, "❌ Quit Atena Battery", true, None);
    menu.append(&quit_item)?;

    Ok(menu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atena_core::{BatterySource, ConnectionType, DeviceCategory};
    use std::time::SystemTime;

    fn make_device(id: &str, name: &str, level: u8, connected: bool) -> BatteryInfo {
        BatteryInfo {
            device_id: id.to_string(),
            name: name.to_string(),
            level,
            is_connected: connected,
            source: BatterySource::OsCache,
            connection_type: ConnectionType::BluetoothClassic,
            category: DeviceCategory::Audio,
            container_id: None,
            last_updated: SystemTime::now(),
        }
    }

    #[test]
    fn test_format_tooltip_empty_and_multiple() {
        // 0 devices
        assert_eq!(format_tooltip(&[]), "Atena Battery: No devices connected");

        // 1 device
        let d1 = make_device("DEV_1", "Sony Headset", 80, true);
        assert_eq!(
            format_tooltip(std::slice::from_ref(&d1)),
            "Atena Battery: Sony Headset (80%)"
        );

        // Multiple devices: identifies lowest
        let d2 = make_device("DEV_2", "Logitech Mouse", 25, true);
        let d3 = make_device("DEV_3", "Xbox Controller", 90, true);
        let tooltip = format_tooltip(&[d1, d2, d3]);
        assert!(tooltip.contains("3 device(s) connected"));
        assert!(tooltip.contains("Lowest: Logitech Mouse (25%)"));
    }

    #[test]
    fn test_get_primary_or_lowest_device() {
        let d1 = make_device("DEV_1", "Headset", 80, true);
        let d2 = make_device("DEV_2", "Mouse", 15, true);
        let d3 = make_device("DEV_3", "Disconnected Keyboard", 5, false);

        // d3 is 5% but disconnected; d2 is lowest connected (15%)
        let list = [d1, d2, d3];
        let lowest = get_primary_or_lowest_device(&list).unwrap();
        assert_eq!(lowest.device_id, "DEV_2");
        assert_eq!(lowest.level, 15);
    }

    #[test]
    fn test_build_context_menu_with_six_devices() {
        let devices = vec![
            make_device("D1", "BT Headset", 70, true),
            make_device("D2", "BLE Earbuds", 85, true),
            make_device("D3", "2.4GHz Mouse", 45, true),
            make_device("D4", "2.4GHz Keyboard", 90, true),
            make_device("D5", "Gamepad P1", 30, true),
            make_device("D6", "Gamepad P2", 100, true),
        ];

        let menu = build_context_menu(&devices, 20);
        assert!(
            menu.is_ok(),
            "Context menu should build cleanly with 6 devices"
        );
    }

    #[test]
    fn test_build_context_menu_empty_devices() {
        let menu = build_context_menu(&[], 20);
        assert!(menu.is_ok());
    }
}
