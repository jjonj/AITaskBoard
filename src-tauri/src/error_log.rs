use serde::Deserialize;
use std::{fs::{self, OpenOptions}, io::Write, path::PathBuf, sync::Mutex};

static LOG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UiErrorEntry {
    timestamp: String,
    source: String,
    category: String,
    task_id: Option<String>,
    board_path: String,
    message: String,
}

/// Append-only JSONL, rotated at 5 MiB. Location: %APPDATA%\TaskBoard\logs\errors.jsonl.
#[tauri::command]
pub fn record_ui_error(entry: UiErrorEntry) -> Result<(), String> {
    let _guard = LOG_LOCK.lock().map_err(|e| e.to_string())?;
    let appdata = std::env::var_os("APPDATA").ok_or("APPDATA not set")?;
    let dir = PathBuf::from(appdata).join("TaskBoard").join("logs");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("errors.jsonl");
    if fs::metadata(&path).is_ok_and(|meta| meta.len() > 5 * 1024 * 1024) {
        let previous = dir.join("errors.previous.jsonl");
        if previous.exists() { fs::remove_file(&previous).map_err(|e| e.to_string())?; }
        fs::rename(&path, previous).map_err(|e| e.to_string())?;
    }
    let mut file = OpenOptions::new().append(true).create(true).open(path).map_err(|e| e.to_string())?;
    serde_json::to_writer(&mut file, &entry).map_err(|e| e.to_string())?;
    file.write_all(b"\n").map_err(|e| e.to_string())
}
