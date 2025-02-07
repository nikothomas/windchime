use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use chrono::Utc;
use crate::OUTPUT_DIR;

/// A global mutex-guarded log file handle.
static LOG_FILE: Lazy<Mutex<Option<std::fs::File>>> = Lazy::new(|| Mutex::new(None));

/// Initialize the log file in append mode inside OUTPUT_DIR/windchime.log
pub fn init_log() {
    let log_path = format!("{}/windchime.log", OUTPUT_DIR);
    if let Ok(file) = OpenOptions::new().create(true).append(true).open(log_path) {
        let mut guard = LOG_FILE.lock().unwrap();
        *guard = Some(file);
    } else {
        eprintln!("Warning: failed to open windchime.log for logging.");
    }
}

/// Append a line to the log file.
pub fn log_action(action: &str) {
    let mut guard = LOG_FILE.lock().unwrap();
    if let Some(ref mut file) = *guard {
        let timestamp = Utc::now();
        let _ = writeln!(file, "[{}] {}", timestamp.to_rfc3339(), action);
    }
}
