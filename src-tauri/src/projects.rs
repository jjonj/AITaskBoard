use std::{
    fs,
    path::{Path, PathBuf},
};
use serde::{Deserialize, Serialize};
use task_core::{FsTaskStore, TaskStore};

use crate::WARTRIBES_BOARD;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCatalog {
    pub schema: u32,
    pub active_id: String,
    pub projects: Vec<Project>,
}

fn default_catalog() -> ProjectCatalog {
    ProjectCatalog {
        schema: 1,
        active_id: "wartribes".into(),
        projects: vec![Project {
            id: "wartribes".into(),
            name: "Wartribes".into(),
            path: WARTRIBES_BOARD.into(),
        }],
    }
}

fn catalog_path() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| std::env::var_os("HOME").map(|home| {
            let mut path = PathBuf::from(home);
            path.push(".config");
            path.into_os_string()
        }))
        .ok_or("Cannot determine a user configuration directory")?;
    Ok(PathBuf::from(base).join("TaskBoard").join("projects.json"))
}

fn load() -> Result<ProjectCatalog, String> { load_at(&catalog_path()?) }

fn load_at(path: &Path) -> Result<ProjectCatalog, String> {
    if !path.exists() { return Ok(default_catalog()); }
    let bytes = fs::read(&path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    let catalog: ProjectCatalog = serde_json::from_slice(&bytes)
        .map_err(|e| format!("Invalid project catalog at {} (not overwritten): {e}", path.display()))?;
    if catalog.schema != 1 || catalog.projects.is_empty() ||
        !catalog.projects.iter().any(|project| project.id == catalog.active_id) {
        return Err(format!("Invalid project catalog structure at {}", path.display()));
    }
    Ok(catalog)
}

fn save(catalog: &ProjectCatalog) -> Result<(), String> { save_at(&catalog_path()?, catalog) }

fn save_at(path: &Path, catalog: &ProjectCatalog) -> Result<(), String> {
    let dir = path.parent().ok_or("Invalid project catalog path")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    // Keep a readable previous copy if the catalog write is interrupted.
    if path.exists() {
        fs::copy(&path, path.with_extension("json.bak"))
            .map_err(|e| format!("Unable to back up project catalog: {e}"))?;
    }
    let payload = serde_json::to_vec_pretty(catalog).map_err(|e| e.to_string())?;
    fs::write(path, payload).map_err(|e| e.to_string())
}

fn same_path(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.replace('/', "\\").trim_end_matches('\\').eq_ignore_ascii_case(
            b.replace('/', "\\").trim_end_matches('\\'))
    } else { a == b }
}

#[tauri::command]
pub fn list_projects() -> Result<ProjectCatalog, String> { load() }

#[tauri::command]
pub fn add_project(name: String, path: String, create_board: bool) -> Result<ProjectCatalog, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err("Project name must contain 1–80 characters".into());
    }
    let location = path.trim().trim_matches('"');
    if location.is_empty() { return Err("Choose an existing project folder".into()); }
    let root = fs::canonicalize(Path::new(location))
        .map_err(|e| format!("Project folder must already exist: {e}"))?;
    if !root.is_dir() { return Err("Project path must be a folder".into()); }
    let root_string = root.to_string_lossy().to_string();
    let mut catalog = load()?;
    if catalog.projects.iter().any(|p| same_path(&p.path, &root_string)) {
        return Err("That folder is already open as a project tab".into());
    }
    if !root.join("_board.md").is_file() {
        if !create_board {
            return Err("No _board.md found. Check 'Create a new board here' to initialize this folder.".into());
        }
        FsTaskStore::default().ensure_board(&root, name).map_err(|e| e.to_string())?;
    }
    // Validate existing board before adding it. Invalid boards are never reset.
    FsTaskStore::default().load(&root).map_err(|e| e.to_string())?;
    // IDs identify UI tabs and must not be inferred from task IDs (WT-xxxx can
    // legitimately repeat in different projects).
    let id = format!("project-{}", uuidless_unique_id());
    catalog.projects.push(Project { id: id.clone(), name: name.to_string(), path: root_string });
    catalog.active_id = id;
    save(&catalog)?;
    Ok(catalog)
}

fn uuidless_unique_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{}-{stamp}", std::process::id())
}

#[tauri::command]
pub fn activate_project(id: String) -> Result<ProjectCatalog, String> {
    let mut catalog = load()?;
    if !catalog.projects.iter().any(|p| p.id == id) {
        return Err("Unknown project tab".into());
    }
    catalog.active_id = id;
    save(&catalog)?;
    Ok(catalog)
}

#[tauri::command]
pub fn remove_project(id: String) -> Result<ProjectCatalog, String> {
    let mut catalog = load()?;
    if catalog.projects.len() == 1 {
        return Err("Keep at least one project tab".into());
    }
    if !catalog.projects.iter().any(|p| p.id == id) {
        return Err("Unknown project tab".into());
    }
    catalog.projects.retain(|p| p.id != id);
    if catalog.active_id == id { catalog.active_id = catalog.projects[0].id.clone(); }
    save(&catalog)?;
    // Intentionally never touch the on-disk task board or its directory.
    Ok(catalog)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_defaults_to_existing_wartribes_and_preserves_active_tab() {
        let path = std::env::temp_dir().join(format!(
            "taskboard-project-catalog-{}-{}.json",
            std::process::id(),
            uuidless_unique_id(),
        ));
        let defaults = load_at(&path).expect("read absent catalog");
        assert_eq!(defaults.active_id, "wartribes");
        assert_eq!(defaults.projects[0].path, WARTRIBES_BOARD);
        let mut two = defaults.clone();
        two.projects.push(Project { id: "other".into(), name: "Another board".into(),
            path: r"D:\Projects\Another\Tasks".into() });
        two.active_id = "other".into();
        save_at(&path, &two).expect("persist catalog");
        assert_eq!(load_at(&path).unwrap().active_id, "other");
        let corrupted = fs::write(&path, "{garbled");
        assert!(corrupted.is_ok());
        assert!(load_at(&path).is_err(), "corrupted state must never be reset silently");
        fs::remove_file(&path).ok();
        fs::remove_file(path.with_extension("json.bak")).ok();
    }

    #[test]
    fn duplicate_root_paths_are_compared_as_windows_paths() {
        assert!(same_path(WARTRIBES_BOARD, WARTRIBES_BOARD));
        if cfg!(windows) {
            assert!(same_path(r"D:\Projects\Tasks", "d:/projects/tasks/"));
        }
    }
}
