//! Coordinates a pending desktop restart with the canonical PowerShell rebuild script.
//! The script owns the countdown; the renderer can only observe or postpone its request.
use std::{env, fs, path::PathBuf};

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestartWarning {
    id: String,
    deadline_unix_ms: i64,
}

fn warning_dir() -> Option<PathBuf> {
    Some(PathBuf::from(env::var_os("LOCALAPPDATA")?).join("TaskBoard"))
}
fn request_path() -> Option<PathBuf> {
    Some(warning_dir()?.join("restart-warning.json"))
}

fn current_request() -> Option<RestartWarning> {
    let request: RestartWarning =
        serde_json::from_slice(&fs::read(request_path()?).ok()?).ok()?;
    // An interrupted script must not leave an eternal stale banner.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    if request.deadline_unix_ms < now - 60_000 { return None; }
    Some(request)
}

#[tauri::command]
pub fn restart_warning_status() -> Option<RestartWarning> {
    current_request()
}

#[tauri::command]
pub fn postpone_restart_warning(id: String) -> Result<(), String> {
    let request = current_request().ok_or("Restart is no longer pending")?;
    if request.id != id { return Err("Restart request has changed".into()); }
    let directory = warning_dir().ok_or("LOCALAPPDATA is unavailable")?;
    // A separate receipt avoids concurrent writes to the request the script reads.
    fs::write(directory.join("restart-postponed.json"),
        serde_json::json!({ "id": id }).to_string())
        .map_err(|err| err.to_string())?;
    Ok(())
}
