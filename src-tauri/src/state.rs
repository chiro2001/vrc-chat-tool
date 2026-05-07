/// Global application state and log buffer.
/// Extracted from main.rs to allow command modules to share state.

use std::sync::{atomic::AtomicBool, Mutex};
use std::path::PathBuf;
use std::fs;
use crate::config;

// --- Recording State ---
pub static CURRENT_CONFIG: Mutex<Option<config::AppConfig>> = Mutex::new(None);
pub static SHOULD_STOP: AtomicBool = AtomicBool::new(false);
pub static IS_RECORDING: AtomicBool = AtomicBool::new(false);

// --- Log System ---
pub static LOG_BUFFER: Mutex<Vec<LogEntry>> = Mutex::new(Vec::new());
pub const MAX_LOG_ENTRIES: usize = 200;

#[derive(Clone, serde::Serialize)]
pub struct LogEntry {
    pub timestamp: u64,
    pub level: String,
    pub message: String,
    pub module: String,
}

// --- Recording Test Helpers ---
pub fn recordings_dir() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    dir.push("tmp");
    dir.push("recordings");
    let _ = fs::create_dir_all(&dir);
    dir
}
