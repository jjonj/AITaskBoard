use serde::Serialize;
use std::{env, fs, path::PathBuf};

#[derive(Serialize)]
pub(crate) struct FavoriteModel {
    provider: String,
    model: String,
}

/// T3 owns the favorite list. Read each time so model favorites are live.
#[tauri::command]
pub(crate) fn task_model_favorites() -> Result<Vec<FavoriteModel>, String> {
    let home = env::var_os("T3CODE_HOME").map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(|v| PathBuf::from(v).join(".t3")))
        .ok_or("No T3 home directory")?;
    // T3's server settings own migrated favorites; older installs keep their
    // preferences in the client file. Prefer the same primary source as T3.
    let server = home.join("userdata").join("settings.json");
    let client = home.join("userdata").join("client-settings.json");
    let preferred = fs::read_to_string(&server).ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|settings| settings.get("favorites").and_then(|v| v.as_array()).cloned());
    let favorites = if let Some(rows) = preferred { rows } else {
        let text = fs::read_to_string(&client)
            .map_err(|e| format!("T3 model favorites unavailable: {e}"))?;
        let settings: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
        settings.get("favorites").and_then(|v| v.as_array()).cloned().unwrap_or_default()
    };
    Ok(favorites.iter()
        .filter_map(|row| {
            let provider = row.get("provider")?.as_str()?.trim();
            let model = row.get("model")?.as_str()?.trim();
            if provider.is_empty() || model.is_empty() { return None; }
            Some(FavoriteModel { provider: provider.into(), model: model.into() })
        }).collect())
}
