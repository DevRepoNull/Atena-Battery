use atena_core::{BatteryProvider, CircuitBreaker};
use atena_provider_win::{
    BleGattProvider, LegacyRfcommProvider, Win32HidProvider, Win32PnpProvider, Win32XInputProvider,
};
use std::sync::Arc;

#[tokio::test]
async fn test_all_five_providers_initialization_and_snapshot_execution() {
    let cb = Arc::new(CircuitBreaker::new());

    let pnp = Arc::new(Win32PnpProvider::new());
    let hid = Arc::new(Win32HidProvider::new());
    let xinput = Arc::new(Win32XInputProvider::new());
    let gatt = Arc::new(BleGattProvider::with_circuit_breaker(Arc::clone(&cb)));
    let rfcomm = Arc::new(LegacyRfcommProvider::with_circuit_breaker(Arc::clone(&cb)));

    // Verify provider friendly diagnostic names
    assert_eq!(pnp.name(), "Win32-PnP");
    assert_eq!(hid.name(), "Win32-HID");
    assert_eq!(xinput.name(), "Win32-XInput");
    assert_eq!(gatt.name(), "WinRT-BleGatt");
    assert_eq!(rfcomm.name(), "Win32-Rfcomm");

    // Execute snapshot queries on all 5 providers: none must panic, all must return Result
    let res_pnp = pnp.fetch_snapshot().await;
    assert!(res_pnp.is_ok(), "PnP snapshot should return Ok");

    let res_hid = hid.fetch_snapshot().await;
    assert!(res_hid.is_ok(), "HID snapshot should return Ok");

    let res_xinput = xinput.fetch_snapshot().await;
    assert!(res_xinput.is_ok(), "XInput snapshot should return Ok");

    let res_gatt = gatt.fetch_snapshot().await;
    assert!(res_gatt.is_ok(), "BleGatt snapshot should return Ok");

    let res_rfcomm = rfcomm.fetch_snapshot().await;
    assert!(res_rfcomm.is_ok(), "Rfcomm snapshot should return Ok");
}

#[tokio::test]
async fn test_circuit_breaker_gates_gatt_and_rfcomm_providers() {
    let cb = Arc::new(CircuitBreaker::new());
    let mac = "AA:BB:CC:11:22:99";

    // Trip circuit breaker for this MAC
    cb.record_failure(mac);
    cb.record_failure(mac);
    cb.record_failure(mac);

    let gatt = BleGattProvider::with_circuit_breaker(Arc::clone(&cb));
    let rfcomm = LegacyRfcommProvider::with_circuit_breaker(Arc::clone(&cb));

    // Calling query_device on tripped MAC must return None immediately
    let gatt_res = gatt.query_device(mac, "Tripped Device");
    assert!(
        gatt_res.is_none(),
        "GATT must block query when breaker is Open"
    );

    let now = std::time::Instant::now();
    let rfcomm_res = rfcomm.probe_device(mac, "Tripped Device", now);
    assert!(
        rfcomm_res.is_none(),
        "RFCOMM must block query when breaker is Open"
    );
}
