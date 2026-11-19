use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::SystemTime;

/// Canonical 48-bit Bluetooth MAC Address (EUI-48).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MacAddress(pub [u8; 6]);

impl MacAddress {
    /// Parses a 12-character hex or colon-delimited MAC string into `MacAddress`.
    pub fn parse(s: &str) -> Result<Self, &'static str> {
        // Enforce that input only contains valid hex characters or standard delimiters
        for c in s.chars() {
            if !c.is_ascii_hexdigit() && c != ':' && c != '-' && !c.is_ascii_whitespace() {
                return Err("Invalid character in MAC address string");
            }
        }
        let cleaned: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        if cleaned.len() != 12 {
            return Err("Invalid MAC address length: expected 12 hex characters");
        }
        let mut bytes = [0u8; 6];
        for i in 0..6 {
            bytes[i] = u8::from_str_radix(&cleaned[i * 2..i * 2 + 2], 16)
                .map_err(|_| "Invalid hex character in MAC address")?;
        }
        Ok(Self(bytes))
    }

    /// Formats the address into canonical uppercase `XX:XX:XX:XX:XX:XX`.
    pub fn to_canonical_string(&self) -> String {
        format!(
            "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }

    /// Returns the raw 6-byte array representation.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 6] {
        &self.0
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_canonical_string())
    }
}

/// Physical transport or connection mechanism used by the peripheral.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ConnectionType {
    /// Bluetooth Classic (BR/EDR, RFCOMM, Hands-Free Profile).
    BluetoothClassic,
    /// Bluetooth Low Energy (GATT Battery Service 0x180F).
    BluetoothLE,
    /// 2.4GHz proprietary USB-dongle wireless peripheral (HID reports).
    Wireless24Ghz,
    /// Direct wireless gamepad controller (e.g. Xbox Wireless Controller via XInput).
    Gamepad,
    /// Other / unrecognized wireless or wired connection.
    Other,
}

impl fmt::Display for ConnectionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BluetoothClassic => write!(f, "Bluetooth Classic"),
            Self::BluetoothLE => write!(f, "Bluetooth LE"),
            Self::Wireless24Ghz => write!(f, "2.4GHz Wireless"),
            Self::Gamepad => write!(f, "Gamepad (XInput)"),
            Self::Other => write!(f, "Other"),
        }
    }
}

/// High-level device peripheral classification for tray presentation and iconography.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DeviceCategory {
    Audio,
    Mouse,
    Keyboard,
    Gamepad,
    Peripheral,
}

impl DeviceCategory {
    /// Returns the standard display icon/emoji for Tray UI listings.
    pub const fn icon(&self) -> &'static str {
        match self {
            Self::Audio => "🎧",
            Self::Mouse => "🖱️",
            Self::Keyboard => "⌨️",
            Self::Gamepad => "🎮",
            Self::Peripheral => "🔋",
        }
    }

    /// Returns the text badge prefix.
    pub const fn tag(&self) -> &'static str {
        match self {
            Self::Audio => "[Audio]",
            Self::Mouse => "[Mouse]",
            Self::Keyboard => "[Keyboard]",
            Self::Gamepad => "[Gamepad]",
            Self::Peripheral => "[Peripheral]",
        }
    }
}

impl fmt::Display for DeviceCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.tag())
    }
}

/// Represents the origin source of a device's battery reading.
///
/// Priority hierarchy:
/// `OsCache` (5) > `HidStandard` (4) > `XInput` (3) > `BleGatt` (2) > `Rfcomm` (1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BatterySource {
    /// Operating System cached battery value (highest priority).
    OsCache,
    /// Standard Human Interface Device (HID) battery report (Usage Page 0x06 / 0x84/0x85).
    HidStandard,
    /// Xbox Wireless Gamepad querying via XInput.
    XInput,
    /// Direct Bluetooth Low Energy GATT service query.
    BleGatt,
    /// RFCOMM / Classic Bluetooth protocol (e.g. AT commands / Hands-Free profile).
    Rfcomm,
}

impl BatterySource {
    /// Returns numerical priority: OsCache (5) > HidStandard (4) > XInput (3) > BleGatt (2) > Rfcomm (1).
    #[inline]
    pub const fn priority(&self) -> u8 {
        match self {
            Self::OsCache => 5,
            Self::HidStandard => 4,
            Self::XInput => 3,
            Self::BleGatt => 2,
            Self::Rfcomm => 1,
        }
    }
}

impl PartialOrd for BatterySource {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BatterySource {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.priority().cmp(&other.priority())
    }
}

impl fmt::Display for BatterySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OsCache => write!(f, "OsCache"),
            Self::HidStandard => write!(f, "HID"),
            Self::XInput => write!(f, "XInput"),
            Self::BleGatt => write!(f, "BleGatt"),
            Self::Rfcomm => write!(f, "Rfcomm"),
        }
    }
}

/// Contains battery and connectivity status for any monitored wireless device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatteryInfo {
    /// Unified identifier: MAC address for Bluetooth, ContainerId/VID:PID:Serial for HID, XInput#N for gamepads.
    pub device_id: String,
    pub name: String,
    pub level: u8,
    pub is_connected: bool,
    pub source: BatterySource,
    pub connection_type: ConnectionType,
    pub category: DeviceCategory,
    /// Windows Device ContainerId (UUID) for hardware grouping and cross-provider deduplication.
    pub container_id: Option<String>,
    pub last_updated: SystemTime,
}

impl BatteryInfo {
    /// Creates a new `BatteryInfo` instance.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device_id: impl Into<String>,
        name: impl Into<String>,
        level: u8,
        is_connected: bool,
        source: BatterySource,
        connection_type: ConnectionType,
        category: DeviceCategory,
        last_updated: SystemTime,
    ) -> Self {
        Self {
            device_id: device_id.into(),
            name: name.into(),
            level: level.min(100),
            is_connected,
            source,
            connection_type,
            category,
            container_id: None,
            last_updated,
        }
    }

    /// Attaches an optional Windows ContainerId.
    pub fn with_container_id(mut self, container_id: impl Into<String>) -> Self {
        self.container_id = Some(container_id.into());
        self
    }

    /// Backward compatibility helper returning the MAC address if the device is Bluetooth.
    pub fn mac_address(&self) -> Option<&str> {
        match self.connection_type {
            ConnectionType::BluetoothClassic | ConnectionType::BluetoothLE => Some(&self.device_id),
            _ => None,
        }
    }
}

/// Device events that can be streamed through providers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeviceEvent {
    /// Battery information for a device was updated.
    BatteryUpdated(BatteryInfo),
    /// A device connected.
    DeviceConnected { device_id: String, name: String },
    /// A device disconnected.
    DeviceDisconnected { device_id: String },
}

/// Errors produced by battery providers and core services.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    DeviceNotFound(String),
    Unavailable(String),
    Communication(String),
    ChannelClosed,
    CircuitBreakerOpen {
        device_id: String,
        backoff_secs: u64,
    },
    ConfigError(String),
    Other(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DeviceNotFound(id) => write!(f, "Device not found: {id}"),
            Self::Unavailable(msg) => write!(f, "Provider unavailable: {msg}"),
            Self::Communication(msg) => write!(f, "Communication error: {msg}"),
            Self::ChannelClosed => write!(f, "Event channel closed"),
            Self::CircuitBreakerOpen {
                device_id,
                backoff_secs,
            } => {
                write!(
                    f,
                    "Circuit breaker open for {device_id}, retry after {backoff_secs}s"
                )
            }
            Self::ConfigError(msg) => write!(f, "Configuration error: {msg}"),
            Self::Other(msg) => write!(f, "Provider error: {msg}"),
        }
    }
}

impl std::error::Error for ProviderError {}
