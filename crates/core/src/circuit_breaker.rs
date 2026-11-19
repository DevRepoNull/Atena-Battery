use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Operational state of a per-device circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation: all queries are permitted.
    Closed,
    /// Device failing: queries blocked until backoff period elapses.
    Open,
    /// Backoff elapsed: exactly one trial probe is permitted.
    HalfOpen,
}

/// Configuration for the per-device circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Consecutive failures required to trip the breaker. Default: 3.
    pub failure_threshold: u32,
    /// Initial backoff period upon tripping. Default: 5 minutes (300s).
    pub initial_backoff: Duration,
    /// Multiplier applied on each subsequent failure after threshold. Default: 2.0.
    pub backoff_factor: f64,
    /// Maximum backoff duration cap. Default: 24 hours.
    pub max_backoff: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            initial_backoff: Duration::from_secs(5 * 60),
            backoff_factor: 2.0,
            max_backoff: Duration::from_secs(24 * 60 * 60),
        }
    }
}

#[derive(Debug, Clone)]
struct DeviceState {
    consecutive_failures: u32,
    open_until: Option<Instant>,
    #[allow(dead_code)]
    current_backoff: Duration,
}

impl DeviceState {
    fn new(initial_backoff: Duration) -> Self {
        Self {
            consecutive_failures: 0,
            open_until: None,
            current_backoff: initial_backoff,
        }
    }
}

/// Lightweight thread-safe circuit breaker tracking consecutive device query failures
/// and enforcing exponential backoff per device identifier (`device_id`).
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    devices: RwLock<HashMap<String, DeviceState>>,
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with default settings (3 failures, 5 min initial backoff).
    pub fn new() -> Self {
        Self::with_config(CircuitBreakerConfig::default())
    }

    /// Creates a circuit breaker with custom configuration.
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        Self {
            config,
            devices: RwLock::new(HashMap::new()),
        }
    }

    #[inline]
    fn normalize_id(id: &str) -> String {
        id.trim().to_uppercase()
    }

    /// Returns the current circuit state for a given device ID.
    pub fn state(&self, device_id: &str) -> CircuitState {
        self.state_at(device_id, Instant::now())
    }

    /// Returns the circuit state for a given device ID at a specific instant.
    pub fn state_at(&self, device_id: &str, now: Instant) -> CircuitState {
        let key = Self::normalize_id(device_id);
        let guard = match self.devices.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        if let Some(state) = guard.get(&key) {
            if state.consecutive_failures < self.config.failure_threshold {
                CircuitState::Closed
            } else if let Some(open_until) = state.open_until {
                if now >= open_until {
                    CircuitState::HalfOpen
                } else {
                    CircuitState::Open
                }
            } else {
                CircuitState::Closed
            }
        } else {
            CircuitState::Closed
        }
    }

    /// Checks if a query is allowed for the given device ID at current time.
    pub fn can_query(&self, device_id: &str) -> bool {
        self.can_query_at(device_id, Instant::now())
    }

    /// Checks if a query is allowed for the given device ID at a specific instant.
    pub fn can_query_at(&self, device_id: &str, now: Instant) -> bool {
        match self.state_at(device_id, now) {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => false,
        }
    }

    /// Records a failed query for the given device ID at current time.
    /// Returns `Some(backoff)` if the circuit breaker is now tripped/open, `None` if under threshold.
    pub fn record_failure(&self, device_id: &str) -> Option<Duration> {
        self.record_failure_at(device_id, Instant::now())
    }

    /// Records a failed query for the given device ID at a specific instant.
    pub fn record_failure_at(&self, device_id: &str, now: Instant) -> Option<Duration> {
        let key = Self::normalize_id(device_id);
        let mut guard = match self.devices.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        let initial_backoff = self.config.initial_backoff;
        let state = guard
            .entry(key)
            .or_insert_with(|| DeviceState::new(initial_backoff));

        state.consecutive_failures = state.consecutive_failures.saturating_add(1);

        if state.consecutive_failures >= self.config.failure_threshold {
            let exponent = state.consecutive_failures - self.config.failure_threshold;
            let factor = self.config.backoff_factor.powi(exponent as i32);
            let backoff =
                (self.config.initial_backoff.mul_f64(factor)).min(self.config.max_backoff);

            state.current_backoff = backoff;
            state.open_until = Some(now + backoff);
            Some(backoff)
        } else {
            None
        }
    }

    /// Records a successful query for the given device ID, resetting its circuit state to Closed.
    pub fn record_success(&self, device_id: &str) {
        let key = Self::normalize_id(device_id);
        let mut guard = match self.devices.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        if let Some(state) = guard.get_mut(&key) {
            state.consecutive_failures = 0;
            state.open_until = None;
            state.current_backoff = self.config.initial_backoff;
        }
    }

    /// Returns the number of consecutive failures recorded for the device ID.
    pub fn consecutive_failures(&self, device_id: &str) -> u32 {
        let key = Self::normalize_id(device_id);
        let guard = match self.devices.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        guard.get(&key).map(|s| s.consecutive_failures).unwrap_or(0)
    }

    /// Checks if the circuit breaker is currently open (blocking queries).
    pub fn is_open(&self, device_id: &str) -> bool {
        self.state(device_id) == CircuitState::Open
    }

    /// Checks if the circuit breaker is open at a specific instant.
    pub fn is_open_at(&self, device_id: &str, now: Instant) -> bool {
        self.state_at(device_id, now) == CircuitState::Open
    }

    /// Returns the remaining backoff duration, if the breaker is open.
    pub fn remaining_backoff_at(&self, device_id: &str, now: Instant) -> Option<Duration> {
        let key = Self::normalize_id(device_id);
        let guard = match self.devices.read() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };

        guard.get(&key).and_then(|state| {
            if state.consecutive_failures >= self.config.failure_threshold {
                state.open_until.and_then(
                    |until| {
                        if until > now {
                            Some(until - now)
                        } else {
                            None
                        }
                    },
                )
            } else {
                None
            }
        })
    }

    /// Resets the circuit breaker state for a specific device ID.
    pub fn reset(&self, device_id: &str) {
        let key = Self::normalize_id(device_id);
        let mut guard = match self.devices.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.remove(&key);
    }

    /// Clears all tracked device states.
    pub fn clear(&self) {
        let mut guard = match self.devices.write() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state_allows_query() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.state("AA:BB:CC:DD:EE:01"), CircuitState::Closed);
        assert!(cb.can_query("AA:BB:CC:DD:EE:01"));
        assert_eq!(cb.consecutive_failures("AA:BB:CC:DD:EE:01"), 0);
        assert!(!cb.is_open("AA:BB:CC:DD:EE:01"));
    }

    #[test]
    fn test_trip_after_three_failures_with_five_minute_backoff() {
        let cb = CircuitBreaker::new();
        let id = "AA:BB:CC:DD:EE:02";
        let t0 = Instant::now();

        // 1st failure: allowed (Closed)
        let backoff = cb.record_failure_at(id, t0);
        assert_eq!(backoff, None);
        assert_eq!(cb.state_at(id, t0), CircuitState::Closed);
        assert!(cb.can_query_at(id, t0));
        assert_eq!(cb.consecutive_failures(id), 1);

        // 2nd failure: allowed (Closed)
        let backoff = cb.record_failure_at(id, t0);
        assert_eq!(backoff, None);
        assert_eq!(cb.state_at(id, t0), CircuitState::Closed);
        assert!(cb.can_query_at(id, t0));
        assert_eq!(cb.consecutive_failures(id), 2);

        // 3rd failure: trips! 5 minutes backoff (Open)
        let backoff = cb.record_failure_at(id, t0);
        assert_eq!(backoff, Some(Duration::from_secs(300)));
        assert_eq!(cb.consecutive_failures(id), 3);
        assert_eq!(cb.state_at(id, t0), CircuitState::Open);
        assert!(cb.is_open_at(id, t0));
        assert!(!cb.can_query_at(id, t0));

        // During backoff period (299s later) -> Still Open
        let t_during = t0 + Duration::from_secs(299);
        assert_eq!(cb.state_at(id, t_during), CircuitState::Open);
        assert!(!cb.can_query_at(id, t_during));
        assert!(cb.is_open_at(id, t_during));
        let rem = cb.remaining_backoff_at(id, t_during).unwrap();
        assert_eq!(rem, Duration::from_secs(1));

        // Exactly at 5 minutes (300s): HalfOpen trial probe allowed
        let t_expired = t0 + Duration::from_secs(300);
        assert_eq!(cb.state_at(id, t_expired), CircuitState::HalfOpen);
        assert!(cb.can_query_at(id, t_expired));
        assert!(!cb.is_open_at(id, t_expired));
    }

    #[test]
    fn test_exponential_backoff_on_subsequent_failures() {
        let cb = CircuitBreaker::new();
        let id = "HID_DONGLE_03";
        let t0 = Instant::now();

        // 3 initial failures -> 5 minutes (300s)
        cb.record_failure_at(id, t0);
        cb.record_failure_at(id, t0);
        let b3 = cb.record_failure_at(id, t0);
        assert_eq!(b3, Some(Duration::from_secs(300)));

        // Probe after 5 minutes fails -> 4th consecutive failure: 10 minutes (600s)
        let t1 = t0 + Duration::from_secs(300);
        assert_eq!(cb.state_at(id, t1), CircuitState::HalfOpen);
        let b4 = cb.record_failure_at(id, t1);
        assert_eq!(b4, Some(Duration::from_secs(600)));
        assert_eq!(cb.state_at(id, t1), CircuitState::Open);
        assert!(!cb.can_query_at(id, t1));
        assert!(!cb.can_query_at(id, t1 + Duration::from_secs(599)));
        assert!(cb.can_query_at(id, t1 + Duration::from_secs(600)));

        // Probe after 10 minutes fails -> 5th consecutive failure: 20 minutes (1200s)
        let t2 = t1 + Duration::from_secs(600);
        let b5 = cb.record_failure_at(id, t2);
        assert_eq!(b5, Some(Duration::from_secs(1200)));
        assert_eq!(cb.state_at(id, t2), CircuitState::Open);
        assert!(!cb.can_query_at(id, t2));
        assert!(cb.can_query_at(id, t2 + Duration::from_secs(1200)));
    }

    #[test]
    fn test_success_resets_breaker() {
        let cb = CircuitBreaker::new();
        let id = "XInput#0";
        let t0 = Instant::now();

        // 2 failures
        cb.record_failure_at(id, t0);
        cb.record_failure_at(id, t0);
        assert_eq!(cb.consecutive_failures(id), 2);

        // Success resets counter
        cb.record_success(id);
        assert_eq!(cb.consecutive_failures(id), 0);
        assert_eq!(cb.state(id), CircuitState::Closed);
        assert!(cb.can_query(id));

        // Now requires 3 new failures to trip
        cb.record_failure_at(id, t0);
        cb.record_failure_at(id, t0);
        assert!(cb.can_query_at(id, t0));

        let b3 = cb.record_failure_at(id, t0);
        assert_eq!(b3, Some(Duration::from_secs(300)));
        assert_eq!(cb.state_at(id, t0), CircuitState::Open);
        assert!(!cb.can_query_at(id, t0));

        // After trip and probe time, success resets again
        let t_expired = t0 + Duration::from_secs(300);
        assert_eq!(cb.state_at(id, t_expired), CircuitState::HalfOpen);
        assert!(cb.can_query_at(id, t_expired));
        cb.record_success(id);
        assert_eq!(cb.consecutive_failures(id), 0);
        assert_eq!(cb.state_at(id, t_expired), CircuitState::Closed);
        assert!(cb.can_query(id));
    }

    #[test]
    fn test_mac_normalization_and_isolation() {
        let cb = CircuitBreaker::new();
        let dev1 = "aa:bb:cc:dd:ee:05 ";
        let dev1_upper = "AA:BB:CC:DD:EE:05";
        let dev2 = "XInput#1";
        let t0 = Instant::now();

        cb.record_failure_at(dev1, t0);
        cb.record_failure_at(dev1_upper, t0);
        cb.record_failure_at(dev1, t0);

        // dev1 should be tripped (Open)
        assert_eq!(cb.state_at(dev1_upper, t0), CircuitState::Open);
        assert!(!cb.can_query_at(dev1_upper, t0));
        assert_eq!(cb.consecutive_failures(dev1_upper), 3);

        // dev2 should be unaffected (Closed)
        assert_eq!(cb.state_at(dev2, t0), CircuitState::Closed);
        assert!(cb.can_query_at(dev2, t0));
        assert_eq!(cb.consecutive_failures(dev2), 0);
    }
}
