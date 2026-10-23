use std::{
    fs,
    path::{Path, PathBuf},
};
use serde::{Deserialize, Serialize};
use task_core::{FsTaskStore, TaskStore};

use crate::WARTRIBES_BOARD;

const OBSIDIAN_PROJECTS: &str = r"B:\GDrive\SharedWithPhone\Obsidian\jjj";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
    // Additional working directories belonging to this project. TaskBoard
    // Markdown always lives in `path`; never scan these directories for tasks.
    #[serde(default)]
    pub folders: Vec<String>,
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
            folders: Vec::new(),
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

// The user's Obsidian vault is the one canonical TaskBoard storage location.
// Source folders are metadata and can live anywhere on disk.
fn board_directory(name: &str) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 80 ||
       name == "." || name == ".." || name.ends_with(['.', ' ']) ||
       name.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')) ||
       ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5",
        "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4",
        "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"].iter()
           .any(|reserved| name.split('.').next().unwrap_or("").eq_ignore_ascii_case(reserved)) {
        return Err("Project name contains characters invalid for a Windows folder".into());
    }
    Ok(Path::new(OBSIDIAN_PROJECTS).join(name).join("Tasks"))
}

fn normalize_folders(folders: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = Vec::<String>::new();
    for folder in folders {
        let input = folder.trim().trim_matches('"');
        if input.is_empty() { continue; }
        let full = fs::canonicalize(input)
            .map_err(|e| format!("Project folder {input:?} does not exist: {e}"))?;
        if !full.is_dir() {
            return Err(format!("Project folder {} is not a directory", full.display()));
        }
        let full = full.to_string_lossy().to_string();
        if !normalized.iter().any(|existing| same_path(existing, &full)) {
            normalized.push(full);
        }
    }
    Ok(normalized)
}

#[tauri::command]
pub fn add_project(name: String, folders: Vec<String>) -> Result<ProjectCatalog, String> {
    let root = board_directory(&name)?;
    let normalized_folders = normalize_folders(folders)?;
    let mut catalog = load()?;
    if catalog.projects.iter().any(|p| same_path(&p.path, &root.to_string_lossy())) {
        return Err("This project's TaskBoard already has a tab".into());
    }
    // Creating a project never touches its source directories. An existing
    // Obsidian board is reused, and the store only initializes missing files.
    fs::create_dir_all(&root).map_err(|e| format!("Cannot create Obsidian Tasks folder: {e}"))?;
    FsTaskStore::default().ensure_board(&root, name.trim()).map_err(|e| e.to_string())?;
    FsTaskStore::default().load(&root).map_err(|e| e.to_string())?;
    let id = format!("project-{}", uuidless_unique_id());
    catalog.projects.push(Project {
        id: id.clone(), name: name.trim().to_string(),
        path: root.to_string_lossy().to_string(), folders: normalized_folders,
    });
    catalog.active_id = id;
    save(&catalog)?;
    Ok(catalog)
}

#[tauri::command]
pub fn set_project_folders(id: String, folders: Vec<String>) -> Result<ProjectCatalog, String> {
    let normalized = normalize_folders(folders)?;
    let mut catalog = load()?;
    let project = catalog.projects.iter_mut().find(|p| p.id == id)
        .ok_or("Unknown project tab")?;
    project.folders = normalized;
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
            path: r"D:\Projects\Another\Tasks".into(), folders: Vec::new() });
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
    fn old_project_catalog_entries_load_without_folder_metadata() {
        let path = std::env::temp_dir().join(format!(
            "taskboard-old-catalog-{}-{}.json",
            std::process::id(), uuidless_unique_id(),
        ));
        fs::write(&path, r#"{
            "schema":1,"activeId":"wartribes",
            "projects":[{"id":"wartribes","name":"Wartribes","path":"B:/Old/Tasks"}]
        }"#).unwrap();
        let loaded = load_at(&path).unwrap();
        assert!(loaded.projects[0].folders.is_empty());
        assert_eq!(loaded.projects[0].path, "B:/Old/Tasks");
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn project_names_have_safe_fixed_task_paths_and_multiple_source_folders() {
        assert_eq!(
            board_directory("Altraea").unwrap(),
            Path::new(OBSIDIAN_PROJECTS).join("Altraea").join("Tasks"),
        );
        for name in ["../escaped", "name\\child", "CON", "LPT9.txt", "with:", "x.", ""] {
            assert!(board_directory(name).is_err(), "{name} should be rejected");
        }
        let base = std::env::temp_dir().join(format!(
            "taskboard-multi-folders-{}-{}", std::process::id(), uuidless_unique_id(),
        ));
        let first = base.join("code");
        let second = base.join("assets");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        let paths = normalize_folders(vec![
            first.to_string_lossy().into_owned(),
            second.to_string_lossy().into_owned(),
            first.to_string_lossy().into_owned(),
        ]).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(normalize_folders(vec![base.join("missing").to_string_lossy().into_owned()]).is_err());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn duplicate_root_paths_are_compared_as_windows_paths() {
        assert!(same_path(WARTRIBES_BOARD, WARTRIBES_BOARD));
        if cfg!(windows) {
            assert!(same_path(r"D:\Projects\Tasks", "d:/projects/tasks/"));
        }
    }
}
