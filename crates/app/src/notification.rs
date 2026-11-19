//! Windows Native Toast Notification Manager with Discharge Cycle Tracking.
//!
//! Employs Windows WinRT `ToastNotificationManager` to display native OS toasts.
//! Tracks per-device state (`DischargeTracker`) so low battery notifications fire
//! only once per discharge cycle, re-arming only when the battery level recovers
//! above the threshold (plus 5% hysteresis).

use std::collections::HashMap;
use windows::core::HSTRING;
use windows::Data::Xml::Dom::XmlDocument;
use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

const APP_USER_MODEL_ID: &str = "Atena.Battery.App";
const HYSTERESIS_MARGIN: u8 = 5;

/// State tracker for an individual wireless peripheral's discharge cycle.
#[derive(Debug, Clone, Default)]
pub struct DeviceDischargeState {
    pub notified_low: bool,
    pub last_reported_level: u8,
}

/// Tracks low-battery notification states for all peripherals to ensure notifications
/// fire exactly once per discharge cycle.
#[derive(Debug, Default)]
pub struct NotificationManager {
    device_states: HashMap<String, DeviceDischargeState>,
}

impl NotificationManager {
    /// Creates a new `NotificationManager`.
    pub fn new() -> Self {
        Self {
            device_states: HashMap::new(),
        }
    }

    /// Evaluates device battery level against threshold and triggers a toast notification
    /// if the threshold is crossed during the current discharge cycle.
    pub fn check_and_notify(&mut self, device_id: &str, name: &str, level: u8, threshold: u8) {
        let key = device_id.trim().to_uppercase();
        let state = self
            .device_states
            .entry(key)
            .or_insert(DeviceDischargeState {
                notified_low: false,
                last_reported_level: level,
            });

        state.last_reported_level = level;

        if level <= threshold {
            if !state.notified_low {
                state.notified_low = true;
                send_toast_notification(name, level);
            }
        } else if level > threshold.saturating_add(HYSTERESIS_MARGIN) {
            // Battery recharged above threshold + hysteresis: re-arm notification
            state.notified_low = false;
        }
    }

    /// Queries the discharge state for a given device ID.
    #[cfg(test)]
    pub fn get_state(&self, device_id: &str) -> Option<&DeviceDischargeState> {
        let key = device_id.trim().to_uppercase();
        self.device_states.get(&key)
    }
}

/// Sends a native Windows Toast Notification using WinRT XML template.
pub fn send_toast_notification(device_name: &str, level: u8) {
    let xml_payload = format!(
        r#"<toast duration="short">
  <visual>
    <binding template="ToastGeneric">
      <text>Atena Battery: Low Battery Alert</text>
      <text>Device '{}' has dropped to {}% battery.</text>
    </binding>
  </visual>
</toast>"#,
        device_name, level
    );

    let doc = match XmlDocument::new() {
        Ok(d) => d,
        Err(_) => return,
    };

    let xml_hstring = HSTRING::from(xml_payload);
    if doc.LoadXml(&xml_hstring).is_err() {
        return;
    }

    let toast = match ToastNotification::CreateToastNotification(&doc) {
        Ok(t) => t,
        Err(_) => return,
    };

    let aumid = HSTRING::from(APP_USER_MODEL_ID);
    if let Ok(notifier) = ToastNotificationManager::CreateToastNotifierWithId(&aumid) {
        let _ = notifier.Show(&toast);
    } else if let Ok(notifier) = ToastNotificationManager::CreateToastNotifier() {
        let _ = notifier.Show(&toast);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_discharge_cycle_tracking_fires_only_once() {
        let mut mgr = NotificationManager::new();
        let mac = "AA:BB:CC:11:22:33";
        let name = "Headset";
        let threshold = 20;

        // 1. Initial level 80% -> no notify
        mgr.check_and_notify(mac, name, 80, threshold);
        assert!(!mgr.get_state(mac).unwrap().notified_low);

        // 2. Level drops to 20% -> triggers notification & marks notified
        mgr.check_and_notify(mac, name, 20, threshold);
        assert!(mgr.get_state(mac).unwrap().notified_low);

        // 3. Level drops further to 18% -> does NOT re-notify (notified_low remains true)
        mgr.check_and_notify(mac, name, 18, threshold);
        assert!(mgr.get_state(mac).unwrap().notified_low);

        // 4. Minor fluctuation to 22% (below 20 + 5 = 25) -> still does not re-arm
        mgr.check_and_notify(mac, name, 22, threshold);
        assert!(mgr.get_state(mac).unwrap().notified_low);

        // 5. Recharged to 30% (> 25%) -> re-arms!
        mgr.check_and_notify(mac, name, 30, threshold);
        assert!(!mgr.get_state(mac).unwrap().notified_low);

        // 6. Drops again to 15% -> fires again for next discharge cycle!
        mgr.check_and_notify(mac, name, 15, threshold);
        assert!(mgr.get_state(mac).unwrap().notified_low);
    }

    #[test]
    fn test_multi_device_independent_discharge_tracking() {
        let mut mgr = NotificationManager::new();
        let threshold = 20;

        let dev_a = "AA:00:00:00:00:01";
        let dev_b = "BB:00:00:00:00:02";
        let dev_c = "VID_046D&PID_C539";
        let dev_d = "XInput#0";

        // Dev A at 15% (low), Dev B at 50% (ok), Dev C at 22% (ok), Dev D at 10% (low)
        mgr.check_and_notify(dev_a, "Headset A", 15, threshold);
        mgr.check_and_notify(dev_b, "Mouse B", 50, threshold);
        mgr.check_and_notify(dev_c, "Dongle C", 22, threshold);
        mgr.check_and_notify(dev_d, "Gamepad D", 10, threshold);

        assert!(mgr.get_state(dev_a).unwrap().notified_low);
        assert!(!mgr.get_state(dev_b).unwrap().notified_low);
        assert!(!mgr.get_state(dev_c).unwrap().notified_low);
        assert!(mgr.get_state(dev_d).unwrap().notified_low);

        // Dev A drops further to 5% -> still notified, Dev B drops to 19% -> now notified
        mgr.check_and_notify(dev_a, "Headset A", 5, threshold);
        mgr.check_and_notify(dev_b, "Mouse B", 19, threshold);

        assert!(mgr.get_state(dev_a).unwrap().notified_low);
        assert!(mgr.get_state(dev_b).unwrap().notified_low);

        // Recharging Dev A to 80% re-arms Dev A without affecting Dev B
        mgr.check_and_notify(dev_a, "Headset A", 80, threshold);
        assert!(!mgr.get_state(dev_a).unwrap().notified_low);
        assert!(mgr.get_state(dev_b).unwrap().notified_low);
    }

    #[test]
    fn test_hysteresis_boundary_fluctuation_no_flapping() {
        let mut mgr = NotificationManager::new();
        let mac = "DEV_FLAPPING";
        let threshold = 20;

        // Drop to threshold -> notified
        mgr.check_and_notify(mac, "Device", 20, threshold);
        assert!(mgr.get_state(mac).unwrap().notified_low);

        // Battery fluctuates: 21%, 23%, 24%, 25% (all <= threshold + 5)
        for jitter_level in [21, 23, 22, 24, 25, 19, 20, 25] {
            mgr.check_and_notify(mac, "Device", jitter_level, threshold);
            assert!(
                mgr.get_state(mac).unwrap().notified_low,
                "Jitter to {}% must not reset notification state",
                jitter_level
            );
        }

        // Only at 26% (> 25%) does it reset
        mgr.check_and_notify(mac, "Device", 26, threshold);
        assert!(!mgr.get_state(mac).unwrap().notified_low);
    }

    #[test]
    fn test_initial_startup_below_threshold_and_zero_percent() {
        let mut mgr = NotificationManager::new();
        let mac = "DEV_DEAD_BATTERY";

        // Startup at 0%
        mgr.check_and_notify(mac, "Critical Device", 0, 20);
        assert!(mgr.get_state(mac).unwrap().notified_low);

        // Subsequent check at 0% does not reset
        mgr.check_and_notify(mac, "Critical Device", 0, 20);
        assert!(mgr.get_state(mac).unwrap().notified_low);
    }
}
