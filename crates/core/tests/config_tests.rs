use atena_core::AppConfig;

#[test]
fn test_default_config() {
    let cfg = AppConfig::default();
    assert_eq!(cfg.version, 1);
    assert_eq!(cfg.overlay.pos_x, 1737);
    assert_eq!(cfg.overlay.pos_y, 50);
    assert!(cfg.overlay.always_on_top);
    assert!((cfg.overlay.scale - 1.0).abs() < f32::EPSILON);
    assert_eq!(cfg.overlay.refresh_interval_secs, 30);
    assert_eq!(cfg.selected_device_mac, None);
    assert!(cfg.notifications.low_battery_enabled);
    assert_eq!(cfg.notifications.warning_threshold, 20);
    assert_eq!(cfg.notifications.critical_threshold, 10);
}

#[test]
fn test_config_toml_roundtrip() {
    let mut original = AppConfig::default();
    original.overlay.pos_x = 1200;
    original.overlay.pos_y = 150;
    original.overlay.always_on_top = false;
    original.overlay.scale = 1.25;
    original.overlay.refresh_interval_secs = 45;
    original.selected_device_mac = Some("AA:BB:CC:DD:EE:FF".to_string());
    original.notifications.low_battery_enabled = true;
    original.notifications.warning_threshold = 25;
    original.notifications.critical_threshold = 15;

    let toml_string = original.to_toml_string().expect("Serialization failed");
    assert!(toml_string.contains("version = 1"));
    assert!(toml_string.contains("pos_x = 1200"));
    assert!(toml_string.contains(r#"selected_device_mac = "AA:BB:CC:DD:EE:FF""#));

    let deserialized = AppConfig::from_toml_str(&toml_string).expect("Deserialization failed");
    assert_eq!(original, deserialized);
}

#[test]
fn test_config_partial_deserialization_uses_defaults() {
    let partial_toml = r#"
        version = 1

        [overlay]
        pos_x = 800
    "#;

    let cfg = AppConfig::from_toml_str(partial_toml).expect("Partial parsing failed");
    assert_eq!(cfg.version, 1);
    assert_eq!(cfg.overlay.pos_x, 800);
    // Defaults filled in
    assert_eq!(cfg.overlay.pos_y, 50);
    assert!(cfg.overlay.always_on_top);
    assert_eq!(cfg.overlay.refresh_interval_secs, 30);
    assert_eq!(cfg.notifications.warning_threshold, 20);
    assert_eq!(cfg.notifications.critical_threshold, 10);
    assert_eq!(cfg.selected_device_mac, None);
}

#[test]
fn test_config_file_save_and_load() {
    let dir = std::env::temp_dir().join("atena_test_config");
    let file_path = dir.join("test_config.toml");

    let mut cfg = AppConfig::default();
    cfg.overlay.pos_x = 555;
    cfg.selected_device_mac = Some("11:22:33:44:55:66".to_string());

    cfg.save_to_file(&file_path)
        .expect("Failed to save config file");
    assert!(file_path.exists());

    let loaded = AppConfig::load_from_file(&file_path).expect("Failed to load config file");
    assert_eq!(cfg, loaded);

    let _ = std::fs::remove_dir_all(&dir);
}
