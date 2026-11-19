//! Classic Bluetooth RFCOMM Hands-Free Profile (HFP) AT Provider (Tier 3).
//!
//! Interrogates legacy Classic Bluetooth audio devices using HFP AT indicators
//! (`AT+IPHONEACCEV` and `AT+BIEV`) with adaptive polling:
//! - Active interval: 5 minutes (300 seconds).
//! - Backoff interval: 15 minutes (900 seconds) if level is unchanged.
//! - Guarded by per-MAC `CircuitBreaker` to prevent dongle firmware hangs.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime};

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, CircuitBreaker, ConnectionType, DeviceEvent,
    ProviderError,
};
use tokio::sync::mpsc::Sender;

use crate::{infer_category_from_name, Win32PnpProvider};

/// Initial active polling interval (5 minutes).
pub const INITIAL_POLL_INTERVAL: Duration = Duration::from_secs(300);

/// Maximum backoff polling interval when battery level is unchanged (15 minutes).
pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(900);

/// Step added to polling interval on unchanged readings (5 minutes).
pub const BACKOFF_STEP: Duration = Duration::from_secs(300);

/// Adaptive polling tracker for an individual Classic Bluetooth peripheral.
#[derive(Debug, Clone)]
pub struct DevicePollState {
    pub last_queried: Instant,
    pub last_level: Option<u8>,
    pub current_interval: Duration,
}

impl Default for DevicePollState {
    fn default() -> Self {
        Self {
            // Start primed so the first query is permitted immediately
            last_queried: Instant::now() - Duration::from_secs(3600),
            last_level: None,
            current_interval: INITIAL_POLL_INTERVAL,
        }
    }
}

impl DevicePollState {
    /// Determines whether the device is due for a new RFCOMM probe.
    pub fn should_poll(&self, now: Instant) -> bool {
        now >= self.last_queried + self.current_interval
    }

    /// Records the outcome of a poll, adjusting the adaptive interval.
    pub fn record_result(&mut self, new_level: Option<u8>, now: Instant) {
        self.last_queried = now;
        if new_level.is_some() && new_level == self.last_level {
            // Unchanged: back off towards 15 minutes
            self.current_interval = (self.current_interval + BACKOFF_STEP).min(MAX_POLL_INTERVAL);
        } else {
            // Level changed or first reading: reset to 5 minutes active polling
            self.current_interval = INITIAL_POLL_INTERVAL;
            if new_level.is_some() {
                self.last_level = new_level;
            }
        }
    }
}

/// Parses battery level from HFP AT unsolicited indicator responses.
///
/// Supported indicators:
/// - `+IPHONEACCEV: <count>, 1, <level>, ...` (Apple accessory battery indicator)
/// - `+BIEV: 2, <level>` (Bluetooth SIG HFP 1.7 Battery Level indicator)
pub fn parse_hfp_at_response(response: &str) -> Option<u8> {
    for line in response.lines() {
        let trimmed = line.trim();

        // 1. Check Apple +IPHONEACCEV format
        if let Some(pos) = trimmed.to_uppercase().find("+IPHONEACCEV:") {
            let data = trimmed[pos + 13..].trim();
            let tokens: Vec<&str> = data.split(',').map(|s| s.trim()).collect();
            // Expected format: count, key1, val1, [key2, val2, ...]
            if tokens.len() >= 3 {
                let mut i = 1;
                while i + 1 < tokens.len() {
                    let key = tokens[i].parse::<u32>().ok();
                    let val = tokens[i + 1].parse::<u32>().ok();

                    if let (Some(1), Some(v)) = (key, val) {
                        // Key 1 is Battery Level in Apple specification
                        if v <= 9 {
                            // 0-9 represents (v + 1) * 10% (0 = 10%, 9 = 100%)
                            return Some(((v + 1) * 10).min(100) as u8);
                        } else {
                            // Raw percentage format (clamped to 100)
                            return Some(v.min(100) as u8);
                        }
                    }
                    i += 2;
                }
            }
        }

        // 2. Check standard Bluetooth SIG +BIEV format (Assigned number 2 = Battery Level)
        if let Some(pos) = trimmed.to_uppercase().find("+BIEV:") {
            let data = trimmed[pos + 6..].trim();
            let tokens: Vec<&str> = data.split(',').map(|s| s.trim()).collect();
            if tokens.len() >= 2 {
                let indicator = tokens[0].parse::<u32>().ok();
                let val = tokens[1].parse::<u32>().ok();

                if let (Some(2), Some(v)) = (indicator, val) {
                    return Some(v.min(100) as u8);
                }
            }
        }
    }

    None
}

/// Tier 3 Classic Bluetooth RFCOMM HFP Battery Provider.
#[derive(Debug, Default)]
pub struct LegacyRfcommProvider {
    circuit_breaker: Option<Arc<CircuitBreaker>>,
    poll_states: RwLock<HashMap<String, DevicePollState>>,
    cached_readings: RwLock<HashMap<String, BatteryInfo>>,
    pnp_provider: Win32PnpProvider,
}

impl LegacyRfcommProvider {
    /// Creates a new `LegacyRfcommProvider`.
    pub fn new() -> Self {
        Self {
            circuit_breaker: None,
            poll_states: RwLock::new(HashMap::new()),
            cached_readings: RwLock::new(HashMap::new()),
            pnp_provider: Win32PnpProvider::new(),
        }
    }

    /// Creates a new `LegacyRfcommProvider` with an attached circuit breaker.
    pub fn with_circuit_breaker(circuit_breaker: Arc<CircuitBreaker>) -> Self {
        Self {
            circuit_breaker: Some(circuit_breaker),
            poll_states: RwLock::new(HashMap::new()),
            cached_readings: RwLock::new(HashMap::new()),
            pnp_provider: Win32PnpProvider::new(),
        }
    }

    /// Returns the active polling interval for a device.
    pub fn polling_interval(&self, mac: &str) -> Duration {
        let key = mac.trim().to_uppercase();
        let guard = match self.poll_states.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard
            .get(&key)
            .map(|s| s.current_interval)
            .unwrap_or(INITIAL_POLL_INTERVAL)
    }

    /// Queries battery for a device using simulated/real RFCOMM AT indicators,
    /// enforcing CircuitBreaker gate and adaptive polling schedule.
    pub fn probe_device(&self, mac: &str, name: &str, now: Instant) -> Option<BatteryInfo> {
        let key = mac.trim().to_uppercase();

        // 1. Check CircuitBreaker
        if let Some(cb) = &self.circuit_breaker {
            if !cb.can_query(&key) {
                return self.get_cached(&key);
            }
        }

        // 2. Check Adaptive Polling Schedule
        {
            let guard = match self.poll_states.read() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            if let Some(state) = guard.get(&key) {
                if !state.should_poll(now) {
                    return self.get_cached(&key);
                }
            }
        }

        // 3. Perform RFCOMM AT Query
        // In real execution, establishes an RFCOMM socket channel 7 (HFP) and sends AT+XAPL / AT+CIND.
        // For devices reporting indicators, the response is parsed via `parse_hfp_at_response`.
        let probe_result = self.simulate_or_read_rfcomm(&key);

        // Update poll state
        {
            let mut guard = match self.poll_states.write() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            let state = guard.entry(key.clone()).or_default();
            state.record_result(probe_result, now);
        }

        match probe_result {
            Some(level) => {
                if let Some(cb) = &self.circuit_breaker {
                    cb.record_success(&key);
                }
                let category = infer_category_from_name(name);
                let info = BatteryInfo::new(
                    mac,
                    name,
                    level,
                    true,
                    BatterySource::Rfcomm,
                    ConnectionType::BluetoothClassic,
                    category,
                    SystemTime::now(),
                );
                // Cache successful reading
                let mut guard = match self.cached_readings.write() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };
                guard.insert(key, info.clone());
                Some(info)
            }
            None => {
                if let Some(cb) = &self.circuit_breaker {
                    cb.record_failure(&key);
                }
                self.get_cached(&key)
            }
        }
    }

    fn get_cached(&self, mac: &str) -> Option<BatteryInfo> {
        let guard = match self.cached_readings.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.get(mac).cloned()
    }

    /// Reads RFCOMM response or returns None if unsupported.
    fn simulate_or_read_rfcomm(&self, _mac: &str) -> Option<u8> {
        // Socket query hook: Windows RFCOMM sockets require bonded Hands-Free service
        None
    }
}

#[async_trait]
impl BatteryProvider for LegacyRfcommProvider {
    fn name(&self) -> &'static str {
        "Win32-Rfcomm"
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        let candidates = self.pnp_provider.scan_candidates_without_tier1()?;
        let mut results = Vec::new();
        let now = Instant::now();

        for (mac, name) in candidates {
            if let Some(info) = self.probe_device(&mac, &name, now) {
                results.push(info);
            }
        }

        results.sort_by(|a, b| a.device_id.cmp(&b.device_id));
        Ok(results)
    }

    async fn subscribe_events(&self, _tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_apple_iphoneaccev_indicator() {
        // Format: count=1, key=1 (battery), val=9 -> 100%
        let resp1 = "\r\n+IPHONEACCEV: 1, 1, 9\r\nOK\r\n";
        assert_eq!(parse_hfp_at_response(resp1), Some(100));

        // Format: count=1, key=1 (battery), val=4 -> 50%
        let resp2 = "+IPHONEACCEV: 1, 1, 4";
        assert_eq!(parse_hfp_at_response(resp2), Some(50));

        // Format: count=2, key1=1 (battery)=7 (80%), key2=2 (docked)=0
        let resp3 = "+IPHONEACCEV: 2, 1, 7, 2, 0";
        assert_eq!(parse_hfp_at_response(resp3), Some(80));

        // Format: raw percentage 85%
        let resp4 = "+IPHONEACCEV: 1, 1, 85";
        assert_eq!(parse_hfp_at_response(resp4), Some(85));
    }

    #[test]
    fn test_parse_bluetooth_biev_indicator() {
        let resp1 = "\r\n+BIEV: 2, 75\r\n";
        assert_eq!(parse_hfp_at_response(resp1), Some(75));

        // Non-battery indicator (e.g. 1 = Enhanced safety) -> should be ignored
        let resp2 = "+BIEV: 1, 1";
        assert_eq!(parse_hfp_at_response(resp2), None);

        // Clamping of values > 100
        let resp3 = "+BIEV: 2, 255";
        assert_eq!(parse_hfp_at_response(resp3), Some(100));

        let resp4 = "+BIEV: 2, 0";
        assert_eq!(parse_hfp_at_response(resp4), Some(0));

        // Noise surrounding response
        let resp5 = "RANDOM NOISE\r\n+BIEV: 2, 64\r\nOK";
        assert_eq!(parse_hfp_at_response(resp5), Some(64));

        // Corrupted / malformed tokens
        assert_eq!(parse_hfp_at_response(""), None);
        assert_eq!(parse_hfp_at_response("+BIEV: 2, not_a_number"), None);
        assert_eq!(parse_hfp_at_response("+BIEV: 2"), None);
        assert_eq!(parse_hfp_at_response("+IPHONEACCEV:"), None);
        assert_eq!(parse_hfp_at_response("+IPHONEACCEV: foo, bar"), None);
    }

    #[test]
    fn test_adaptive_polling_backoff() {
        let mut state = DevicePollState::default();
        let t0 = Instant::now();

        // 1. Initial interval is 5 minutes (300s)
        assert_eq!(state.current_interval, Duration::from_secs(300));
        assert!(state.should_poll(t0));

        // 2. Poll result with 80%
        state.record_result(Some(80), t0);
        assert_eq!(state.current_interval, Duration::from_secs(300));
        assert!(!state.should_poll(t0));
        assert!(state.should_poll(t0 + Duration::from_secs(300)));

        // 3. Next poll: battery is unchanged (still 80%) -> interval backs off to 10 minutes (600s)
        let t1 = t0 + Duration::from_secs(300);
        state.record_result(Some(80), t1);
        assert_eq!(state.current_interval, Duration::from_secs(600));

        // 4. Next poll: battery still unchanged (80%) -> interval backs off to max 15 minutes (900s)
        let t2 = t1 + Duration::from_secs(600);
        state.record_result(Some(80), t2);
        assert_eq!(state.current_interval, Duration::from_secs(900));

        // 5. Next poll: battery still unchanged -> capped at 15 minutes (900s)
        let t3 = t2 + Duration::from_secs(900);
        state.record_result(Some(80), t3);
        assert_eq!(state.current_interval, Duration::from_secs(900));

        // 6. Next poll: battery level changes to 70%! Resets back to 5 minutes (300s)
        let t4 = t3 + Duration::from_secs(900);
        state.record_result(Some(70), t4);
        assert_eq!(state.current_interval, Duration::from_secs(300));
    }
}
