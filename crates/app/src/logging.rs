//! Non-blocking rolling file logging subsystem.
//!
//! Uses `tracing`, `tracing-subscriber`, and `tracing-appender` to write asynchronous,
//! non-blocking rolling logs inside `%APPDATA%\AtenaBattery\logs\`.
//! Daily rotation prevents runaway disk usage.

use std::path::PathBuf;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// Initializes the asynchronous rolling file logger.
///
/// Returns a `WorkerGuard` that must be kept in scope for the entire lifetime of `main()`.
/// When dropped, remaining log events are flushed to disk.
pub fn init_logging(logs_dir: PathBuf) -> Option<WorkerGuard> {
    if std::fs::create_dir_all(&logs_dir).is_err() {
        return None;
    }

    // Daily rolling file appender: atena-battery.log.YYYY-MM-DD
    let file_appender = tracing_appender::rolling::daily(logs_dir, "atena-battery.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,atena_app=debug,atena_provider_win=debug"));

    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true)
        .with_thread_names(false);

    let subscriber = tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer);

    if subscriber.try_init().is_err() {
        return None;
    }

    tracing::info!("Atena Battery logging initialized successfully.");
    Some(guard)
}
