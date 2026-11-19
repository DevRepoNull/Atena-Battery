use serde::{Deserialize, Serialize};
use std::path::Path;

/// Application configuration stored in TOML.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub overlay: OverlayConfig,
    #[serde(default)]
    pub selected_device_mac: Option<String>,
    #[serde(default)]
    pub notifications: NotificationConfig,
}

const fn default_version() -> u32 {
    1
}

/// Floating HUD Overlay configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverlayConfig {
    #[serde(default = "default_pos_x")]
    pub pos_x: i32,
    #[serde(default = "default_pos_y")]
    pub pos_y: i32,
    #[serde(default = "default_always_on_top")]
    pub always_on_top: bool,
    #[serde(default = "default_scale")]
    pub scale: f32,
    #[serde(default = "default_refresh_interval")]
    pub refresh_interval_secs: u64,
}

const fn default_pos_x() -> i32 {
    1737
}

const fn default_pos_y() -> i32 {
    50
}

const fn default_always_on_top() -> bool {
    true
}

const fn default_scale() -> f32 {
    1.0
}

const fn default_refresh_interval() -> u64 {
    30
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            pos_x: default_pos_x(),
            pos_y: default_pos_y(),
            always_on_top: default_always_on_top(),
            scale: default_scale(),
            refresh_interval_secs: default_refresh_interval(),
        }
    }
}

/// Notification threshold configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationConfig {
    #[serde(default = "default_low_battery_enabled")]
    pub low_battery_enabled: bool,
    #[serde(default = "default_warning_threshold")]
    pub warning_threshold: u8,
    #[serde(default = "default_critical_threshold")]
    pub critical_threshold: u8,
}

const fn default_low_battery_enabled() -> bool {
    true
}

const fn default_warning_threshold() -> u8 {
    20
}

const fn default_critical_threshold() -> u8 {
    10
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            low_battery_enabled: default_low_battery_enabled(),
            warning_threshold: default_warning_threshold(),
            critical_threshold: default_critical_threshold(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            version: default_version(),
            overlay: OverlayConfig::default(),
            selected_device_mac: None,
            notifications: NotificationConfig::default(),
        }
    }
}

impl AppConfig {
    /// Creates a new default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Serializes configuration to a pretty TOML string.
    pub fn to_toml_string(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Deserializes configuration from a TOML string.
    pub fn from_toml_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    /// Loads configuration from a TOML file.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let content = std::fs::read_to_string(path)?;
        Self::from_toml_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    /// Saves configuration atomically to a TOML file via a temporary file rename,
    /// creating parent directories if necessary.
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        let toml_str = self
            .to_toml_string()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let target = path.as_ref();
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = target.with_extension("tmp");
        std::fs::write(&tmp_path, toml_str)?;
        std::fs::rename(&tmp_path, target)?;
        Ok(())
    }
}
