//! Configuration and App Data Store for Atena-Battery.
//!
//! Manages persistence in `%APPDATA%\AtenaBattery\config.toml` and log directory
//! paths `%APPDATA%\AtenaBattery\logs`.

use atena_core::AppConfig;
use std::path::PathBuf;

const APP_SUBDIR: &str = "AtenaBattery";
const CONFIG_FILE_NAME: &str = "config.toml";
const LOGS_DIR_NAME: &str = "logs";

/// Resolves the root application data directory (`%APPDATA%\AtenaBattery`).
pub fn get_app_dir() -> PathBuf {
    if let Ok(appdata) = std::env::var("APPDATA") {
        PathBuf::from(appdata).join(APP_SUBDIR)
    } else if let Ok(userprofile) = std::env::var("USERPROFILE") {
        PathBuf::from(userprofile)
            .join("AppData")
            .join("Roaming")
            .join(APP_SUBDIR)
    } else {
        PathBuf::from(".").join(APP_SUBDIR)
    }
}

/// Resolves the path to the configuration file (`%APPDATA%\AtenaBattery\config.toml`).
pub fn get_config_path() -> PathBuf {
    get_app_dir().join(CONFIG_FILE_NAME)
}

/// Resolves the path to the logs directory (`%APPDATA%\AtenaBattery\logs`).
pub fn get_logs_dir() -> PathBuf {
    let logs_dir = get_app_dir().join(LOGS_DIR_NAME);
    let _ = std::fs::create_dir_all(&logs_dir);
    logs_dir
}

/// Loads the application configuration from `%APPDATA%\AtenaBattery\config.toml`.
/// Returns default configuration if the file does not exist or fails to parse.
pub fn load_config() -> AppConfig {
    let path = get_config_path();
    if path.exists() {
        AppConfig::load_from_file(&path).unwrap_or_default()
    } else {
        let config = AppConfig::default();
        let _ = config.save_to_file(&path);
        config
    }
}

/// Persists the application configuration to `%APPDATA%\AtenaBattery\config.toml`.
pub fn save_config(config: &AppConfig) -> Result<(), std::io::Error> {
    let path = get_config_path();
    config.save_to_file(path)
}

/// Opens the application logs folder in Windows Explorer.
pub fn open_logs_folder() {
    let logs_dir = get_logs_dir();
    let _ = std::process::Command::new("explorer")
        .arg(&logs_dir)
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_paths_resolution() {
        let app_dir = get_app_dir();
        assert!(app_dir.to_string_lossy().contains("AtenaBattery"));

        let config_path = get_config_path();
        assert!(config_path.to_string_lossy().ends_with("config.toml"));

        let logs_dir = get_logs_dir();
        assert!(logs_dir.to_string_lossy().ends_with("logs"));
    }
}
