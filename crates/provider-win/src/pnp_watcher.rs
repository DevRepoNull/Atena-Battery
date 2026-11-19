//! Windows Plug-and-Play (PnP) Event Watcher.
//!
//! Spawns a dedicated OS thread hosting a hidden Win32 Message-Only Window (`HWND_MESSAGE`)
//! registered for Bluetooth device notifications (`RegisterDeviceNotificationW`).
//! Emits events over `tokio::sync::mpsc::Sender<DeviceEvent>` with a 1.5-second debounce.

use std::collections::HashMap;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::Arc;

use atena_core::{DeviceEvent, ProviderError};
use tokio::sync::mpsc::Sender;

use windows::core::{w, PCWSTR};
use windows::Win32::Devices::Bluetooth::{
    GUID_BLUETOOTHLE_DEVICE_INTERFACE, GUID_BTHPORT_DEVICE_INTERFACE,
};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetWindowLongPtrW, KillTimer, PostMessageW, PostQuitMessage, RegisterClassExW,
    RegisterDeviceNotificationW, SetTimer, SetWindowLongPtrW, TranslateMessage, UnregisterClassW,
    UnregisterDeviceNotification, CS_OWNDC, DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE,
    DBT_DEVNODES_CHANGED, DBT_DEVTYP_DEVICEINTERFACE, DEVICE_NOTIFY_WINDOW_HANDLE,
    DEV_BROADCAST_DEVICEINTERFACE_W, GWLP_USERDATA, HDEVNOTIFY, HWND_MESSAGE, MSG, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DESTROY, WM_DEVICECHANGE, WM_TIMER, WM_USER, WNDCLASSEXW,
};

use crate::Win32PnpProvider;

/// Debounce delay for rapid Windows PnP burst events (1.5 seconds).
pub const DEBOUNCE_DELAY_MS: u32 = 1500;
pub const DEBOUNCE_TIMER_ID: usize = 1001;

/// Custom message to trigger orderly window destruction and thread exit.
pub const WM_SHUTDOWN_WATCHER: u32 = WM_USER + 42;

/// Standard Bluetooth radio device interface GUID: `{00f40965-e89d-4487-9890-87c3abb211f4}`.
pub const GUID_DEVINTERFACE_BLUETOOTH: windows::core::GUID =
    windows::core::GUID::from_u128(0x00f40965_e89d_4487_9890_87c3abb211f4);

/// Standard HID device interface GUID: `{4d1e55b2-f16f-11cf-88cb-001111000030}`.
pub const GUID_DEVINTERFACE_HID: windows::core::GUID =
    windows::core::GUID::from_u128(0x4d1e55b2_f16f_11cf_88cb_001111000030);

/// Safe RAII wrapper around `HDEVNOTIFY` ensuring `UnregisterDeviceNotification`
/// is automatically invoked when dropped, preventing notification handle leaks.
#[derive(Debug)]
pub struct SafeHdevnotify(pub HDEVNOTIFY);

impl Drop for SafeHdevnotify {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: `self.0` is a valid HDEVNOTIFY returned by `RegisterDeviceNotificationW`
            // and is uniquely owned by this SafeHdevnotify instance.
            unsafe {
                let _ = UnregisterDeviceNotification(self.0);
            }
        }
    }
}

/// Internal state passed to the Win32 window procedure via `GWLP_USERDATA`.
struct WatcherContext {
    tx: Sender<DeviceEvent>,
    provider: Win32PnpProvider,
    _notifications: Vec<SafeHdevnotify>,
    known_devices: HashMap<String, u8>,
}

/// Spawns a dedicated OS thread hosting a message-only window and registers for Bluetooth PnP events.
pub fn spawn_pnp_watcher(
    provider: Win32PnpProvider,
    tx: Sender<DeviceEvent>,
) -> Result<(), ProviderError> {
    let hwnd_atomic = Arc::new(AtomicIsize::new(0));
    let hwnd_store = Arc::clone(&hwnd_atomic);
    let tx_watchdog = tx.clone();

    // Spawn dedicated OS thread for the Win32 message pump
    std::thread::Builder::new()
        .name("atena-pnp-watcher".to_string())
        .spawn(move || {
            run_pnp_event_loop(provider, tx, hwnd_store);
        })
        .map_err(|e| {
            ProviderError::Unavailable(format!("Failed to spawn PnP watcher thread: {e}"))
        })?;

    // Spawn async watchdog task: shuts down the Win32 window thread when receiver drops
    tokio::spawn(async move {
        tx_watchdog.closed().await;
        let raw_hwnd = hwnd_atomic.load(Ordering::SeqCst);
        if raw_hwnd != 0 {
            let hwnd = HWND(raw_hwnd as *mut std::ffi::c_void);
            // SAFETY: `hwnd` is a valid window handle created by this module; sending WM_SHUTDOWN_WATCHER
            // prompts it to destroy itself cleanly.
            unsafe {
                let _ = PostMessageW(hwnd, WM_SHUTDOWN_WATCHER, WPARAM(0), LPARAM(0));
            }
        }
    });

    Ok(())
}

/// Runs the Win32 message loop on the dedicated watcher thread.
fn run_pnp_event_loop(
    provider: Win32PnpProvider,
    tx: Sender<DeviceEvent>,
    hwnd_store: Arc<AtomicIsize>,
) {
    // SAFETY: GetModuleHandleW(None) retrieves the module handle for the current process.
    let hinstance = match unsafe { GetModuleHandleW(None) } {
        Ok(h) => h,
        Err(_) => return,
    };

    let class_name = w!("AtenaBatteryPnpWatcherClass");

    let wnd_class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_OWNDC,
        lpfnWndProc: Some(pnp_wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance.into(),
        hIcon: Default::default(),
        hCursor: Default::default(),
        hbrBackground: Default::default(),
        lpszMenuName: PCWSTR::null(),
        lpszClassName: class_name,
        hIconSm: Default::default(),
    };

    // SAFETY: RegisterClassExW registers the window class for the message-only window.
    let atom = unsafe { RegisterClassExW(&wnd_class) };
    if atom == 0 {
        return;
    }

    // SAFETY: Creates a hidden Message-Only Window parented to HWND_MESSAGE.
    let hwnd = match unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("AtenaPnpWatcherWindow"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        )
    } {
        Ok(h) => h,
        Err(_) => {
            // SAFETY: Clean up registered window class on creation failure
            unsafe {
                let _ = UnregisterClassW(class_name, hinstance);
            }
            return;
        }
    };

    // Store HWND for clean async shutdown
    hwnd_store.store(hwnd.0 as isize, Ordering::SeqCst);

    // Register for Bluetooth and HID device notifications
    let mut notifications = Vec::new();
    let target_guids = [
        GUID_BLUETOOTHLE_DEVICE_INTERFACE,
        GUID_BTHPORT_DEVICE_INTERFACE,
        GUID_DEVINTERFACE_BLUETOOTH,
        GUID_DEVINTERFACE_HID,
    ];

    for guid in &target_guids {
        let mut filter = DEV_BROADCAST_DEVICEINTERFACE_W {
            dbcc_size: std::mem::size_of::<DEV_BROADCAST_DEVICEINTERFACE_W>() as u32,
            dbcc_devicetype: DBT_DEVTYP_DEVICEINTERFACE.0,
            dbcc_reserved: 0,
            dbcc_classguid: *guid,
            dbcc_name: [0u16; 1],
        };

        // SAFETY: RegisterDeviceNotificationW called with valid HWND and filter struct pointer.
        let hnotify = unsafe {
            RegisterDeviceNotificationW(
                hwnd,
                &mut filter as *mut _ as *mut std::ffi::c_void,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        };

        if let Ok(h) = hnotify {
            notifications.push(SafeHdevnotify(h));
        }
    }

    let context = Box::new(WatcherContext {
        tx,
        provider,
        _notifications: notifications,
        known_devices: HashMap::new(),
    });

    let context_ptr = Box::into_raw(context);

    // SAFETY: Attach boxed context pointer to window's GWLP_USERDATA.
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, context_ptr as isize);
    }

    // Trigger initial snapshot refresh immediately via a 0ms timer
    // SAFETY: SetTimer initializes the first refresh pass.
    unsafe {
        let _ = SetTimer(hwnd, DEBOUNCE_TIMER_ID, 10, None);
    }

    // Standard Win32 Message Loop
    let mut msg = MSG::default();
    // SAFETY: GetMessageW retrieves messages for the calling thread's message queue.
    while unsafe { GetMessageW(&mut msg, None, 0, 0).as_bool() } {
        // SAFETY: Translate and dispatch messages to pnp_wnd_proc.
        unsafe {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    // Clean up window class
    // SAFETY: UnregisterClassW called after message loop terminates.
    unsafe {
        let _ = UnregisterClassW(class_name, hinstance);
    }
}

/// Window procedure processing `WM_DEVICECHANGE`, `WM_TIMER`, and clean shutdown messages.
unsafe extern "system" fn pnp_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DEVICECHANGE => {
            let event_type = wparam.0 as u32;
            if event_type == DBT_DEVICEARRIVAL
                || event_type == DBT_DEVICEREMOVECOMPLETE
                || event_type == DBT_DEVNODES_CHANGED
            {
                // Reset / restart the 1.5-second debounce timer on every PnP burst event
                // SAFETY: SetTimer called with valid window handle and constant timer ID.
                unsafe {
                    let _ = SetTimer(hwnd, DEBOUNCE_TIMER_ID, DEBOUNCE_DELAY_MS, None);
                }
            }
            LRESULT(1)
        }

        WM_TIMER => {
            if wparam.0 == DEBOUNCE_TIMER_ID {
                // Kill the debounce timer
                // SAFETY: KillTimer disarms the debounce timer.
                unsafe {
                    let _ = KillTimer(hwnd, DEBOUNCE_TIMER_ID);
                }

                // SAFETY: Retrieve context pointer from GWLP_USERDATA.
                let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WatcherContext;
                if !ptr.is_null() {
                    // SAFETY: `ptr` points to our heap-allocated WatcherContext.
                    let ctx = unsafe { &mut *ptr };

                    // If receiver is dropped, trigger clean shutdown
                    if ctx.tx.is_closed() {
                        // SAFETY: Destroy window cleanly
                        unsafe {
                            let _ = DestroyWindow(hwnd);
                        }
                        return LRESULT(0);
                    }

                    // Perform snapshot scan
                    if let Ok(devices) = ctx.provider.scan_devices() {
                        let mut current_ids = HashMap::new();

                        for dev in devices {
                            current_ids.insert(dev.device_id.clone(), dev.level);

                            // Push update to channel
                            let _ = ctx.tx.try_send(DeviceEvent::BatteryUpdated(dev));
                        }

                        // Check for disconnected devices
                        for old_id in ctx.known_devices.keys() {
                            if !current_ids.contains_key(old_id) {
                                let _ = ctx.tx.try_send(DeviceEvent::DeviceDisconnected {
                                    device_id: old_id.clone(),
                                });
                            }
                        }

                        ctx.known_devices = current_ids;
                    }
                }
            }
            LRESULT(0)
        }

        WM_SHUTDOWN_WATCHER => {
            // SAFETY: Destroy the message window on shutdown request.
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }

        WM_DESTROY => {
            // SAFETY: Retrieve and take ownership of context to drop it cleanly.
            let ptr = unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0) } as *mut WatcherContext;
            if !ptr.is_null() {
                // SAFETY: Reconstitute Box to free WatcherContext heap memory and drop notifications.
                drop(unsafe { Box::from_raw(ptr) });
            }

            // SAFETY: PostQuitMessage causes GetMessageW to return false, exiting the thread.
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }

        _ => {
            // SAFETY: Forward unhandled messages to DefWindowProcW.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn test_spawn_pnp_watcher_and_clean_shutdown() {
        let (tx, mut rx) = mpsc::channel(16);
        let provider = Win32PnpProvider::new();

        // Spawn watcher
        let res = spawn_pnp_watcher(provider, tx);
        assert!(res.is_ok());

        // Wait briefly for initial snapshot event or timer trigger
        let event = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        // Whether devices exist or channel is open, no panic
        if let Ok(Some(DeviceEvent::BatteryUpdated(info))) = event {
            assert!(!info.device_id.is_empty());
        }

        // Dropping rx triggers tx.closed(), which posts WM_SHUTDOWN_WATCHER and cleans up
        drop(rx);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
