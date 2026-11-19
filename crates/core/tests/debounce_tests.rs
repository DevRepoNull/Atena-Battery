use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use atena_core::{
    BatteryInfo, BatterySource, ConnectionType, DeviceAggregator, DeviceCategory, DeviceEvent,
};
use tokio::sync::mpsc;

fn make_test_device(id: &str, name: &str, level: u8, connected: bool) -> BatteryInfo {
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

/// Simulated debounced event coalescer mimicking Windows PnP debounce loop.
struct DebouncedEventCoalescer {
    query_count: Arc<AtomicUsize>,
}

impl DebouncedEventCoalescer {
    fn new(query_count: Arc<AtomicUsize>) -> Self {
        Self { query_count }
    }

    /// Runs the debounce loop: coalesces bursts arriving within `debounce_duration`.
    async fn run(&self, mut rx: mpsc::Receiver<DeviceEvent>, debounce_duration: Duration) {
        let sleep = tokio::time::sleep(debounce_duration);
        tokio::pin!(sleep);
        let mut active = false;

        loop {
            tokio::select! {
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(_) => {
                            sleep.as_mut().reset(tokio::time::Instant::now() + debounce_duration);
                            active = true;
                        }
                        None => break,
                    }
                }
                () = &mut sleep, if active => {
                    self.query_count.fetch_add(1, Ordering::SeqCst);
                    active = false;
                }
            }
        }
    }
}

#[tokio::test]
async fn test_pnp_event_debounce_coalesces_storm() {
    tokio::time::pause();

    let query_count = Arc::new(AtomicUsize::new(0));
    let coalescer = DebouncedEventCoalescer::new(Arc::clone(&query_count));
    let (tx, rx) = mpsc::channel(64);

    let debounce_window = Duration::from_millis(250);
    let handle = tokio::spawn(async move {
        coalescer.run(rx, debounce_window).await;
    });

    // 1. Emit 15 rapid arrival events within a 100ms window (simulating Windows PnP tree enumeration)
    for _ in 0..15 {
        let dev = make_test_device("00:11:22:33:44:55", "Sony Headset", 70, true);
        tx.send(DeviceEvent::BatteryUpdated(dev)).await.unwrap();
        tokio::time::advance(Duration::from_millis(6)).await; // 15 * 6ms = 90ms total
        tokio::task::yield_now().await;
    }

    // During the 90ms burst, debounce timer was repeatedly reset -> 0 queries dispatched
    assert_eq!(
        query_count.load(Ordering::SeqCst),
        0,
        "No query should dispatch during rapid storm"
    );

    // 2. Advance time past the 250ms quiet window (e.g. 260ms)
    tokio::time::advance(Duration::from_millis(260)).await;
    tokio::task::yield_now().await;

    // 3. Exactly ONE unified query execution should have been dispatched
    assert_eq!(
        query_count.load(Ordering::SeqCst),
        1,
        "Exactly 1 query execution must be dispatched after 250ms quiescence"
    );

    // Clean shutdown
    drop(tx);
    let _ = handle.await;
}

#[tokio::test]
async fn test_rapid_connect_disconnect_burst_deduplication() {
    let mac = "AA:BB:CC:DD:EE:01";

    // Simulate 20 alternating connect/disconnect events arriving in rapid succession
    let mut burst = Vec::new();
    for i in 0..20 {
        let is_connected = i % 2 == 1; // Last one (i=19) will be is_connected: true
        let level = 50 + (i as u8);
        burst.push(make_test_device(
            mac,
            "Flaky Peripheral",
            level,
            is_connected,
        ));
    }

    // Process through aggregator deduplication
    let merged = DeviceAggregator::merge(burst);

    // Assert exactly ONE entry exists (no duplicate tray / aggregator entries)
    assert_eq!(
        merged.len(),
        1,
        "Rapid connect/disconnect bursts must result in exactly 1 deduplicated entry"
    );
    assert_eq!(merged[0].device_id, mac);
    assert!(merged[0].is_connected);
    assert_eq!(merged[0].level, 69);
}

#[tokio::test]
async fn test_mixed_six_devices_debounce_convergence() {
    tokio::time::pause();

    let query_count = Arc::new(AtomicUsize::new(0));
    let coalescer = DebouncedEventCoalescer::new(Arc::clone(&query_count));
    let (tx, rx) = mpsc::channel(128);

    let handle = tokio::spawn(async move {
        coalescer.run(rx, Duration::from_millis(250)).await;
    });

    let devices = [
        ("00:11:22:33:44:01", "BT Headset"),
        ("00:11:22:33:44:02", "BLE Earbuds"),
        ("VID_046D&PID_C539", "Wireless Mouse"),
        ("VID_3434&PID_0200", "Wireless Keyboard"),
        ("XInput#0", "Xbox Gamepad P1"),
        ("XInput#1", "Xbox Gamepad P2"),
    ];

    // Emit 3 rounds of updates for all 6 devices in rapid bursts (18 events total)
    for round in 0..3 {
        for (id, name) in &devices {
            let dev = make_test_device(id, name, 60 + round * 5, true);
            tx.send(DeviceEvent::BatteryUpdated(dev)).await.unwrap();
        }
        tokio::time::advance(Duration::from_millis(20)).await;
        tokio::task::yield_now().await;
    }

    // Still in storm: 0 queries
    assert_eq!(query_count.load(Ordering::SeqCst), 0);

    // Let the debounce quiet window elapse
    tokio::time::advance(Duration::from_millis(300)).await;
    tokio::task::yield_now().await;

    // Exactly one unified query fired
    assert_eq!(query_count.load(Ordering::SeqCst), 1);

    drop(tx);
    let _ = handle.await;
}
