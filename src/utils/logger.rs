use tracing::{debug, error, info, warn};
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub fn init_logger(log_file: &str, log_level: &str) {
    // Parse the directory and file name from the log_file path
    let log_path = std::path::Path::new(log_file);
    let log_dir = log_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let file_name = log_path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("jellycli.log"));

    // Create a rolling file appender that rotates daily
    // This will create files like: jellycli.log, jellycli.log.2025-09-01, jellycli.log.2025-09-02, etc.
    let file_appender = RollingFileAppender::new(Rotation::DAILY, log_dir, file_name);

    // Create a fmt layer for file output
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false) // Disable ANSI colors in file output
        .with_writer(file_appender);

    // Create a fmt layer for console output
    let console_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    // Create an EnvFilter for log level
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| log_level.into());

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .init();
}

pub struct Logger;

impl Logger {
    pub fn info(msg: &str) {
        info!("{}", msg);
    }

    pub fn warn(msg: &str) {
        warn!("{}", msg);
    }

    pub fn error(msg: &str) {
        error!("{}", msg);
    }

    pub fn debug(msg: &str) {
        debug!("{}", msg);
    }
}
