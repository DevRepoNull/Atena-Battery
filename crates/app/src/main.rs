#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod autostart;
mod config_store;
mod icon;
mod logging;
mod notification;
mod tray;

use std::sync::Arc;
use std::time::Duration;

use atena_core::{BatteryInfo, BatteryProvider, CircuitBreaker, DeviceAggregator, DeviceEvent};
use atena_provider_win::{
    BleGattProvider, LegacyRfcommProvider, Win32HidProvider, Win32PnpProvider, Win32XInputProvider,
};
use muda::MenuEvent;
use notification::NotificationManager;
use tokio::sync::mpsc;
use tray::{
    TrayManager, ID_AUTOSTART, ID_OPEN_LOGS, ID_QUIT, ID_REFRESH, ID_THRESH_10, ID_THRESH_20,
    ID_THRESH_30,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 0. Initialize non-blocking rolling file logging in %APPDATA%\AtenaBattery\logs\
    let _log_guard = logging::init_logging(config_store::get_logs_dir());
    tracing::info!("Starting Atena Battery System Tray utility (Universal Wireless)...");

    // Single-threaded Tokio runtime (< 5MB RAM, zero thread-pool overhead)
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let res = rt.block_on(async_main());
    tracing::info!("Atena Battery shutdown complete.");
    res
}

async fn async_main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. Load application configuration
    let mut config = config_store::load_config();
    tracing::info!(
        "Loaded configuration: warning_threshold={}%, autostart={}",
        config.notifications.warning_threshold,
        autostart::is_autostart_enabled()
    );

    // 2. Initialize 5-tier providers and aggregator with CircuitBreaker
    let cb = Arc::new(CircuitBreaker::new());
    let pnp_provider = Arc::new(Win32PnpProvider::new());
    let hid_provider = Arc::new(Win32HidProvider::new());
    let xinput_provider = Arc::new(Win32XInputProvider::new());
    let gatt_provider = Arc::new(BleGattProvider::with_circuit_breaker(Arc::clone(&cb)));
    let rfcomm_provider = Arc::new(LegacyRfcommProvider::with_circuit_breaker(Arc::clone(&cb)));

    let mut aggregator = DeviceAggregator::with_circuit_breaker(Arc::clone(&cb));
    aggregator.add_provider(Arc::clone(&pnp_provider) as Arc<dyn BatteryProvider>);
    aggregator.add_provider(Arc::clone(&hid_provider) as Arc<dyn BatteryProvider>);
    aggregator.add_provider(Arc::clone(&xinput_provider) as Arc<dyn BatteryProvider>);
    aggregator.add_provider(Arc::clone(&gatt_provider) as Arc<dyn BatteryProvider>);
    aggregator.add_provider(Arc::clone(&rfcomm_provider) as Arc<dyn BatteryProvider>);

    // 3. Query initial snapshot across all providers
    let mut devices = aggregator.aggregate().await;
    tracing::info!("Initial aggregated scan found {} device(s)", devices.len());
    for d in &devices {
        tracing::info!(
            "Device detected: '{}' [{}] | {}% | Source: {} | Type: {} | Connected: {}",
            d.name,
            d.category.tag(),
            d.level,
            d.source,
            d.connection_type,
            d.is_connected
        );
    }

    // 4. Initialize Notification Manager & Discharge Tracker
    let mut notif_manager = NotificationManager::new();
    evaluate_notifications(
        &mut notif_manager,
        &devices,
        config.notifications.warning_threshold,
    );

    // 5. Initialize System Tray Manager
    let mut tray_manager = TrayManager::new(&devices, &config)?;

    // 6. Set up debounced event subscription from Win32 PnP Watcher
    let (tx, mut rx) = mpsc::channel::<DeviceEvent>(64);
    pnp_provider.subscribe_events(tx).await?;

    // 7. Post-initialization Working Set Trim: Release cached startup pages back to Windows Memory Manager
    trim_working_set();

    let menu_channel = MenuEvent::receiver();

    // 8. Event & Message Pump Loop (16ms tick for UI responsiveness, ~0% idle CPU)
    let mut ticker = tokio::time::interval(Duration::from_millis(16));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Pump Win32 message queue for Tray Icon and Muda context menus
                pump_win32_messages();

                // Process menu click events
                while let Ok(event) = menu_channel.try_recv() {
                    let id_str = event.id().0.as_str();
                    tracing::debug!("Context menu item clicked: {}", id_str);
                    match id_str {
                        ID_REFRESH => {
                            tracing::info!("User triggered 'Refresh Now'");
                            devices = aggregator.aggregate().await;
                            evaluate_notifications(&mut notif_manager, &devices, tray_manager.threshold());
                            let _ = tray_manager.update_devices(&devices);
                        }
                        ID_THRESH_10 => {
                            tracing::info!("Threshold changed to 10%");
                            config.notifications.warning_threshold = 10;
                            let _ = config_store::save_config(&config);
                            let _ = tray_manager.set_threshold(&devices, 10);
                            evaluate_notifications(&mut notif_manager, &devices, 10);
                        }
                        ID_THRESH_20 => {
                            tracing::info!("Threshold changed to 20%");
                            config.notifications.warning_threshold = 20;
                            let _ = config_store::save_config(&config);
                            let _ = tray_manager.set_threshold(&devices, 20);
                            evaluate_notifications(&mut notif_manager, &devices, 20);
                        }
                        ID_THRESH_30 => {
                            tracing::info!("Threshold changed to 30%");
                            config.notifications.warning_threshold = 30;
                            let _ = config_store::save_config(&config);
                            let _ = tray_manager.set_threshold(&devices, 30);
                            evaluate_notifications(&mut notif_manager, &devices, 30);
                        }
                        ID_AUTOSTART => {
                            let current = autostart::is_autostart_enabled();
                            let new_state = !current;
                            tracing::info!("Toggling autostart: {} -> {}", current, new_state);
                            let _ = autostart::set_autostart_enabled(new_state);
                            let _ = tray_manager.update_devices(&devices);
                        }
                        ID_OPEN_LOGS => {
                            tracing::info!("Opening logs directory in Explorer");
                            config_store::open_logs_folder();
                        }
                        ID_QUIT => {
                            tracing::info!("Quit requested via context menu");
                            return Ok(());
                        }
                        _ => {}
                    }
                }
            }

            Some(event) = rx.recv() => {
                match event {
                    DeviceEvent::BatteryUpdated(dev) => {
                        tracing::info!("PnP Event: Battery updated for '{}' ({}%)", dev.name, dev.level);
                        devices = aggregator.aggregate().await;
                        evaluate_notifications(&mut notif_manager, &devices, tray_manager.threshold());
                        let _ = tray_manager.update_devices(&devices);
                    }
                    DeviceEvent::DeviceConnected { device_id, name } => {
                        tracing::info!("PnP Event: Device connected: '{}' ({})", name, device_id);
                        devices = aggregator.aggregate().await;
                        evaluate_notifications(&mut notif_manager, &devices, tray_manager.threshold());
                        let _ = tray_manager.update_devices(&devices);
                    }
                    DeviceEvent::DeviceDisconnected { device_id } => {
                        tracing::info!("PnP Event: Device disconnected: {}", device_id);
                        devices = aggregator.aggregate().await;
                        evaluate_notifications(&mut notif_manager, &devices, tray_manager.threshold());
                        let _ = tray_manager.update_devices(&devices);
                    }
                }
            }

            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Ctrl+C received, initiating shutdown");
                break;
            }
        }
    }

    Ok(())
}

/// Evaluates connected devices and dispatches Windows Toast notifications for low battery.
fn evaluate_notifications(
    notif_manager: &mut NotificationManager,
    devices: &[BatteryInfo],
    threshold: u8,
) {
    for dev in devices.iter().filter(|d| d.is_connected) {
        if dev.level <= threshold {
            tracing::warn!(
                "Low battery detected: '{}' is at {}% (threshold: {}%)",
                dev.name,
                dev.level,
                threshold
            );
        }
        notif_manager.check_and_notify(&dev.device_id, &dev.name, dev.level, threshold);
    }
}

/// Pumps pending Win32 messages on the current thread's message queue without blocking.
#[inline]
fn pump_win32_messages() {
    let mut msg = MSG::default();
    // SAFETY: Querying thread message queue with valid pointer and PM_REMOVE.
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Trims the process working set to minimize background memory footprint (< 5 MB RAM).
fn trim_working_set() {
    // SAFETY: Calling SetProcessWorkingSetSize with usize::MAX on the current process is the standard Win32 API
    // mechanism to release cached startup pages back to the operating system.
    unsafe {
        let _ = windows::Win32::System::Threading::SetProcessWorkingSetSize(
            windows::Win32::System::Threading::GetCurrentProcess(),
            usize::MAX,
            usize::MAX,
        );
    }
}

#[cfg(test)]
mod live_app_tests {
    use super::*;

    #[tokio::test]
    async fn test_live_aggregation_on_current_pc() {
        let cb = Arc::new(CircuitBreaker::new());
        let pnp = Arc::new(Win32PnpProvider::new());
        let hid = Arc::new(Win32HidProvider::new());
        let xinput = Arc::new(Win32XInputProvider::new());

        let mut agg = DeviceAggregator::with_circuit_breaker(cb);
        agg.add_provider(pnp as Arc<dyn BatteryProvider>);
        agg.add_provider(hid as Arc<dyn BatteryProvider>);
        agg.add_provider(xinput as Arc<dyn BatteryProvider>);

        let devices = agg.aggregate().await;
        assert!(devices.iter().all(|d| d.level <= 100));
    }
}
