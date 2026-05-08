use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::Mutex;

fn open_or_memory() -> Connection {
    let db_path = get_db_path();
    if db_path.exists() {
        if let Ok(meta) = std::fs::metadata(&db_path) {
            if meta.len() == 0 {
                let _ = std::fs::remove_file(&db_path);
            }
        }
    }
    if let Ok(conn) = Connection::open(&db_path) {
        conn.busy_timeout(std::time::Duration::from_secs(5)).ok();
        conn.pragma_update(None, "journal_mode", "WAL").ok();
        let ok = conn.execute(
            "CREATE TABLE IF NOT EXISTS recognition_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                text TEXT NOT NULL,
                source TEXT NOT NULL DEFAULT 'asr'
            )",
            [],
        ).is_ok();
        if ok {
            conn.execute(
                "CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                )",
                [],
            ).ok();
            return conn;
        }
    }
    eprintln!("[history] file-based SQLite unavailable, falling back to in-memory");
    let conn = Connection::open_in_memory().expect("Failed to open in-memory database");
    conn.execute(
        "CREATE TABLE IF NOT EXISTS recognition_history (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            text TEXT NOT NULL,
            source TEXT NOT NULL DEFAULT 'asr'
        )",
        [],
    ).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    ).unwrap();
    conn
}

static DB: once_cell::sync::Lazy<Mutex<Connection>> = once_cell::sync::Lazy::new(|| {
    Mutex::new(open_or_memory())
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
    let conn = match DB.lock() {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut stmt = match conn.prepare(
        "SELECT id, timestamp, text, source FROM recognition_history ORDER BY id DESC LIMIT ?1",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let rows: Vec<HistoryEntry> = match stmt.query_map(params![limit as i64], |row| {
        Ok(HistoryEntry {
            id: row.get(0)?,
            timestamp: row.get(1)?,
            text: row.get(2)?,
            source: row.get(3)?,
        })
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => return Vec::new(),
    };
    drop(stmt);
    rows
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
