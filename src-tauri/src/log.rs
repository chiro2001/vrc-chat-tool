/// Structured file logger for diagnostics.
/// Writes to both stderr (PTY visibility) and tmp/app.log (persistent).
/// Also pushes to the frontend LOG_BUFFER for UI display.
use std::fs::File;
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOGGER: Mutex<Option<LogWriter>> = Mutex::new(None);

struct LogWriter {
    file: File,
}

/// Initialize the file logger. Creates tmp/ directory if needed.
pub fn init(log_path: &str) {
    let path = std::path::Path::new(log_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    match File::create(log_path) {
        Ok(file) => {
            *LOGGER.lock().unwrap() = Some(LogWriter { file });
            let _ = log_internal("INFO", "log", "Logger initialized");
        }
        Err(e) => {
            eprintln!("[LOG] Failed to create log file {}: {}", log_path, e);
        }
    }
}

/// Core log function. Writes to both stderr and file.
fn log_internal(level: &str, module: &str, message: &str) {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // Format: HH:MM:SS.mmm [LEVEL] [module] message
    let secs = ts / 1000;
    let millis = ts % 1000;
    let hours = (secs / 3600) % 24;
    let minutes = (secs / 60) % 60;
    let seconds = secs % 60;
    let line = format!(
        "{:02}:{:02}:{:02}.{:03} [{}] [{}] {}\n",
        hours, minutes, seconds, millis, level, module, message
    );

    // Always write to stderr for PTY visibility
    eprint!("{}", line);

    // Write to file
    if let Ok(mut guard) = LOGGER.lock() {
        if let Some(ref mut writer) = *guard {
            let _ = writer.file.write_all(line.as_bytes());
            let _ = writer.file.flush();
        }
    }

    // Push to frontend log buffer for UI display
    if let Ok(mut buf) = crate::state::LOG_BUFFER.lock() {
        buf.push(crate::state::LogEntry {
            timestamp: ts,
            level: level.to_string(),
            message: message.to_string(),
            module: module.to_string(),
        });
        if buf.len() > crate::state::MAX_LOG_ENTRIES {
            buf.remove(0);
        }
    }
}

/// Log at INFO level.
pub fn info(module: &str, message: &str) {
    log_internal("INFO", module, message);
}

/// Log at WARN level.
pub fn warn(module: &str, message: &str) {
    log_internal("WARN", module, message);
}

/// Log at ERROR level.
pub fn error(module: &str, message: &str) {
    log_internal("ERROR", module, message);
}

/// Log at DEBUG level (detailed, high-frequency).
pub fn debug(module: &str, message: &str) {
    log_internal("DEBUG", module, message);
}
