use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

static DB: once_cell::sync::Lazy<Mutex<Connection>> = once_cell::sync::Lazy::new(|| {
    let db_path = get_db_path();
    let conn = Connection::open(&db_path).expect("Failed to open history database");
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS recognition_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            text TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'asr'
        );
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .expect("Failed to create tables");
    Mutex::new(conn)
});

fn get_db_path() -> PathBuf {
    let mut dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    dir.push("data");
    std::fs::create_dir_all(&dir).ok();
    dir.push("history.db");
    dir
}

#[derive(Serialize, Clone)]
pub struct HistoryEntry {
    pub id: i64,
    pub timestamp: String,
    pub text: String,
    pub source: String,
}

pub fn add_entry(text: &str, source: &str) {
    let conn = DB.lock().unwrap();
    let now = chrono::Local::now()
        .format("%Y-%m-%d %H:%M:%S")
        .to_string();
    conn.execute(
        "INSERT INTO recognition_history (timestamp, text, source) VALUES (?1, ?2, ?3)",
        params![now, text, source],
    )
    .ok();
}

pub fn get_recent(limit: usize) -> Vec<HistoryEntry> {
    let conn = DB.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT id, timestamp, text, source FROM recognition_history ORDER BY id DESC LIMIT ?1",
        )
        .unwrap();
    stmt.query_map(params![limit as i64], |row| {
        Ok(HistoryEntry {
            id: row.get(0)?,
            timestamp: row.get(1)?,
            text: row.get(2)?,
            source: row.get(3)?,
        })
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

pub fn clear_all() {
    let conn = DB.lock().unwrap();
    conn.execute("DELETE FROM recognition_history", []).ok();
}

// ---- Settings (key-value) ----

pub fn get_setting(key: &str) -> Option<String> {
    let conn = DB.lock().unwrap();
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |row| row.get(0),
    )
    .ok()
}

pub fn set_setting(key: &str, value: &str) {
    let conn = DB.lock().unwrap();
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .ok();
}

pub fn get_audio_device_index() -> usize {
    get_setting("audio_device_index")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub fn set_audio_device_index(index: usize) {
    set_setting("audio_device_index", &index.to_string());
}
