use atena_core::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
use std::time::{Duration, Instant};

#[test]
fn test_default_config_values() {
    let cfg = CircuitBreakerConfig::default();
    assert_eq!(cfg.failure_threshold, 3);
    assert_eq!(cfg.initial_backoff, Duration::from_secs(300));
    assert!((cfg.backoff_factor - 2.0).abs() < f64::EPSILON);
    assert_eq!(cfg.max_backoff, Duration::from_secs(86400));
}

#[test]
fn test_state_transitions_closed_to_open_to_half_open() {
    let cb = CircuitBreaker::new();
    let mac = "11:22:33:44:55:66";
    let t0 = Instant::now();

    // Initially Closed
    assert_eq!(cb.state_at(mac, t0), CircuitState::Closed);
    assert!(cb.can_query_at(mac, t0));

    // Failure 1 -> still Closed
    assert_eq!(cb.record_failure_at(mac, t0), None);
    assert_eq!(cb.state_at(mac, t0), CircuitState::Closed);

    // Failure 2 -> still Closed
    assert_eq!(cb.record_failure_at(mac, t0), None);
    assert_eq!(cb.state_at(mac, t0), CircuitState::Closed);

    // Failure 3 -> Trips to Open! Initial backoff = 300s (5 minutes)
    let backoff = cb.record_failure_at(mac, t0);
    assert_eq!(backoff, Some(Duration::from_secs(300)));
    assert_eq!(cb.state_at(mac, t0), CircuitState::Open);
    assert!(!cb.can_query_at(mac, t0));

    // At 299s -> Still Open
    let t_during = t0 + Duration::from_secs(299);
    assert_eq!(cb.state_at(mac, t_during), CircuitState::Open);
    assert!(!cb.can_query_at(mac, t_during));

    // At 300s -> Transitions to HalfOpen (trial probe allowed)
    let t_half_open = t0 + Duration::from_secs(300);
    assert_eq!(cb.state_at(mac, t_half_open), CircuitState::HalfOpen);
    assert!(cb.can_query_at(mac, t_half_open));

    // Trial probe success -> transitions back to Closed
    cb.record_success(mac);
    assert_eq!(cb.state_at(mac, t_half_open), CircuitState::Closed);
    assert_eq!(cb.consecutive_failures(mac), 0);
}

#[test]
fn test_half_open_failure_doubles_backoff() {
    let cb = CircuitBreaker::new();
    let mac = "22:33:44:55:66:77";
    let t0 = Instant::now();

    // 3 failures trip breaker (5 min backoff)
    cb.record_failure_at(mac, t0);
    cb.record_failure_at(mac, t0);
    cb.record_failure_at(mac, t0);
    assert_eq!(cb.state_at(mac, t0), CircuitState::Open);

    // Advance to HalfOpen
    let t1 = t0 + Duration::from_secs(300);
    assert_eq!(cb.state_at(mac, t1), CircuitState::HalfOpen);

    // Trial probe fails -> backoff doubles to 600s (10 minutes)
    let backoff2 = cb.record_failure_at(mac, t1);
    assert_eq!(backoff2, Some(Duration::from_secs(600)));
    assert_eq!(cb.state_at(mac, t1), CircuitState::Open);
    assert!(!cb.can_query_at(mac, t1));

    // At 599s after t1 -> Still Open
    assert_eq!(
        cb.state_at(mac, t1 + Duration::from_secs(599)),
        CircuitState::Open
    );

    // At 600s after t1 -> HalfOpen again
    let t2 = t1 + Duration::from_secs(600);
    assert_eq!(cb.state_at(mac, t2), CircuitState::HalfOpen);

    // Trial probe fails again -> backoff doubles to 1200s (20 minutes)
    let backoff3 = cb.record_failure_at(mac, t2);
    assert_eq!(backoff3, Some(Duration::from_secs(1200)));
    assert_eq!(cb.state_at(mac, t2), CircuitState::Open);
}

#[test]
fn test_per_mac_isolation() {
    let cb = CircuitBreaker::new();
    let mac1 = "33:44:55:66:77:88";
    let mac2 = "44:55:66:77:88:99";
    let t0 = Instant::now();

    // Trip mac1
    cb.record_failure_at(mac1, t0);
    cb.record_failure_at(mac1, t0);
    cb.record_failure_at(mac1, t0);

    // mac1 is Open, mac2 is Closed
    assert_eq!(cb.state_at(mac1, t0), CircuitState::Open);
    assert_eq!(cb.state_at(mac2, t0), CircuitState::Closed);
    assert!(!cb.can_query_at(mac1, t0));
    assert!(cb.can_query_at(mac2, t0));

    // Resetting mac1 clears only mac1
    cb.reset(mac1);
    assert_eq!(cb.state_at(mac1, t0), CircuitState::Closed);
    assert_eq!(cb.state_at(mac2, t0), CircuitState::Closed);
}

#[test]
fn test_mac_case_and_whitespace_normalization() {
    let cb = CircuitBreaker::new();
    let mac_lower = "  55:66:77:88:99:aa  ";
    let mac_upper = "55:66:77:88:99:AA";
    let t0 = Instant::now();

    cb.record_failure_at(mac_lower, t0);
    assert_eq!(cb.consecutive_failures(mac_upper), 1);

    cb.record_failure_at(mac_upper, t0);
    assert_eq!(cb.consecutive_failures(mac_lower), 2);

    cb.record_failure_at(mac_lower, t0);
    assert_eq!(cb.consecutive_failures(mac_upper), 3);
    assert_eq!(cb.state_at(mac_upper, t0), CircuitState::Open);
    assert_eq!(cb.state_at(mac_lower, t0), CircuitState::Open);
}

#[test]
fn test_open_state_rejection_latency_is_sub_microsecond() {
    let cb = CircuitBreaker::new();
    let mac = "AA:BB:CC:11:22:33";
    let t0 = Instant::now();

    // Trip the breaker
    cb.record_failure_at(mac, t0);
    cb.record_failure_at(mac, t0);
    cb.record_failure_at(mac, t0);
    assert_eq!(cb.state_at(mac, t0), CircuitState::Open);

    // Measure latency of 1,000 inquiries while Open
    let start = Instant::now();
    for _ in 0..1000 {
        let allowed = cb.can_query_at(mac, t0);
        assert!(!allowed);
    }
    let elapsed = start.elapsed();
    let per_call = elapsed / 1000;

    // Must be fast (< 10 microseconds average on any test runner, usually ~50ns)
    assert!(
        per_call < Duration::from_micros(10),
        "Per-query latency when Open was {:?}, expected < 10µs",
        per_call
    );
}

#[test]
fn test_six_plus_devices_per_device_isolation() {
    let cb = CircuitBreaker::new();
    let t0 = Instant::now();

    let devices = [
        "DEV_AUDIO_01",
        "DEV_AUDIO_02",
        "VID_046D&PID_C539",
        "VID_3434&PID_0200",
        "XINPUT#0",
        "XINPUT#1",
        "DEV_BLE_03",
    ];

    // Trip only device index 0 and 2
    for _ in 0..3 {
        cb.record_failure_at(devices[0], t0);
        cb.record_failure_at(devices[2], t0);
    }

    // Devices 0 and 2 are Open; all others are Closed
    assert_eq!(cb.state_at(devices[0], t0), CircuitState::Open);
    assert_eq!(cb.state_at(devices[2], t0), CircuitState::Open);
    assert!(!cb.can_query_at(devices[0], t0));
    assert!(!cb.can_query_at(devices[2], t0));

    assert_eq!(cb.state_at(devices[1], t0), CircuitState::Closed);
    assert_eq!(cb.state_at(devices[3], t0), CircuitState::Closed);
    assert_eq!(cb.state_at(devices[4], t0), CircuitState::Closed);
    assert_eq!(cb.state_at(devices[5], t0), CircuitState::Closed);
    assert_eq!(cb.state_at(devices[6], t0), CircuitState::Closed);

    assert!(cb.can_query_at(devices[1], t0));
    assert!(cb.can_query_at(devices[3], t0));
    assert!(cb.can_query_at(devices[4], t0));
    assert!(cb.can_query_at(devices[5], t0));
    assert!(cb.can_query_at(devices[6], t0));
}
