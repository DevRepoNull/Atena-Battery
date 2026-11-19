#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use atena_core::{
    BatteryInfo, BatteryProvider, BatterySource, ConnectionType, DeviceCategory, DeviceEvent,
    ProviderError,
};
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex;

/// Highly configurable deterministic mock provider for testing.
pub struct MockBatteryProvider {
    pub name: &'static str,
    pub source: BatterySource,
    pub devices: Arc<Mutex<Vec<BatteryInfo>>>,
    pub fail: Arc<Mutex<Option<ProviderError>>>,
    pub call_count: Arc<AtomicUsize>,
    pub event_tx: Arc<Mutex<Option<Sender<DeviceEvent>>>>,
}

impl MockBatteryProvider {
    pub fn new(name: &'static str, source: BatterySource, devices: Vec<BatteryInfo>) -> Self {
        Self {
            name,
            source,
            devices: Arc::new(Mutex::new(devices)),
            fail: Arc::new(Mutex::new(None)),
            call_count: Arc::new(AtomicUsize::new(0)),
            event_tx: Arc::new(Mutex::new(None)),
        }
    }

    pub fn failing(name: &'static str, source: BatterySource, error: ProviderError) -> Self {
        Self {
            name,
            source,
            devices: Arc::new(Mutex::new(Vec::new())),
            fail: Arc::new(Mutex::new(Some(error))),
            call_count: Arc::new(AtomicUsize::new(0)),
            event_tx: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn set_devices(&self, devices: Vec<BatteryInfo>) {
        let mut d = self.devices.lock().await;
        *d = devices;
    }

    pub async fn set_error(&self, error: ProviderError) {
        let mut f = self.fail.lock().await;
        *f = Some(error);
    }

    pub async fn clear_error(&self) {
        let mut f = self.fail.lock().await;
        *f = None;
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    pub async fn emit_event(&self, event: DeviceEvent) -> bool {
        let tx_guard = self.event_tx.lock().await;
        if let Some(tx) = &*tx_guard {
            tx.send(event).await.is_ok()
        } else {
            false
        }
    }
}

#[async_trait]
impl BatteryProvider for MockBatteryProvider {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn fetch_snapshot(&self) -> Result<Vec<BatteryInfo>, ProviderError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let fail_guard = self.fail.lock().await;
        if let Some(err) = &*fail_guard {
            return Err(err.clone());
        }

        let dev_guard = self.devices.lock().await;
        Ok(dev_guard.clone())
    }

    async fn subscribe_events(&self, tx: Sender<DeviceEvent>) -> Result<(), ProviderError> {
        let mut tx_guard = self.event_tx.lock().await;
        *tx_guard = Some(tx);
        Ok(())
    }
}

pub fn make_bt_classic(
    mac: &str,
    name: &str,
    level: u8,
    source: BatterySource,
    t: SystemTime,
) -> BatteryInfo {
    BatteryInfo::new(
        mac,
        name,
        level,
        true,
        source,
        ConnectionType::BluetoothClassic,
        DeviceCategory::Audio,
        t,
    )
}

pub fn make_ble(mac: &str, name: &str, level: u8, t: SystemTime) -> BatteryInfo {
    BatteryInfo::new(
        mac,
        name,
        level,
        true,
        BatterySource::BleGatt,
        ConnectionType::BluetoothLE,
        DeviceCategory::Audio,
        t,
    )
}

pub fn make_hid(
    id: &str,
    name: &str,
    level: u8,
    cat: DeviceCategory,
    container_id: Option<&str>,
    t: SystemTime,
) -> BatteryInfo {
    let mut dev = BatteryInfo::new(
        id,
        name,
        level,
        true,
        BatterySource::HidStandard,
        ConnectionType::Wireless24Ghz,
        cat,
        t,
    );
    if let Some(cid) = container_id {
        dev = dev.with_container_id(cid);
    }
    dev
}

pub fn make_gamepad(id: &str, name: &str, level: u8, t: SystemTime) -> BatteryInfo {
    BatteryInfo::new(
        id,
        name,
        level,
        true,
        BatterySource::XInput,
        ConnectionType::Gamepad,
        DeviceCategory::Gamepad,
        t,
    )
}
