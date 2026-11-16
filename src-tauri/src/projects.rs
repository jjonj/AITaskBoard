use std::{
    fs,
    path::{Path, PathBuf},
};
use serde::{Deserialize, Serialize};
use task_core::{FsTaskStore, TaskStore};

use crate::WARTRIBES_BOARD;

const OBSIDIAN_PROJECTS: &str = r"B:\GDrive\SharedWithPhone\Obsidian\jjj";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProjectPriority { High, #[default] Normal, Low }

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
    // TaskBoard and T3 share the same local catalog to rank new work.
    #[serde(default)]
    pub priority: ProjectPriority,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCatalog {
    pub schema: u32,
    pub active_id: String,
    pub projects: Vec<Project>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_paths: Vec<String>,
}

fn default_catalog() -> ProjectCatalog {
    ProjectCatalog {
        schema: 1,
        active_id: "wartribes".into(),
        hidden_paths: Vec::new(),
        projects: vec![Project {
            id: "wartribes".into(),
            name: "Wartribes".into(),
            path: WARTRIBES_BOARD.into(),
            folders: Vec::new(),
            priority: ProjectPriority::Normal,
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

fn load() -> Result<ProjectCatalog, String> {
    discover_boards(load_at(&catalog_path()?)?, Path::new(OBSIDIAN_PROJECTS))
}

// Discovery reads only board metadata: loading a whole task store can perform
// maintenance writes, and must not happen just because its tab is visible.
fn discover_boards(mut catalog: ProjectCatalog, vault: &Path) -> Result<ProjectCatalog, String> {
    #[derive(Deserialize)]
    struct BoardName { name: String }

    let entries = match fs::read_dir(vault) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(catalog),
        Err(error) => return Err(format!("Cannot discover TaskBoards in {}: {error}", vault.display())),
    };
    let mut roots = entries.filter_map(Result::ok)
        .map(|entry| entry.path().join("Tasks"))
        .filter(|root| root.join("_board.md").is_file())
        .collect::<Vec<_>>();
    roots.sort();
    // Registered roots outside the vault still get live board-name updates.
    roots.extend(catalog.projects.iter().map(|project| PathBuf::from(&project.path)));
    for root in roots {
        let path = root.to_string_lossy().into_owned();
        if catalog.hidden_paths.iter().any(|hidden| same_path(hidden, &path)) { continue; }
        let Ok(markdown) = fs::read_to_string(root.join("_board.md")) else { continue; };
        let mut lines = markdown.lines();
        if lines.next().map(str::trim) != Some("---") { continue; }
        let yaml = lines.take_while(|line| line.trim() != "---").collect::<Vec<_>>().join("\n");
        // A board may be midway through an external save. Retry next refresh.
        let Ok(board) = serde_yaml::from_str::<BoardName>(&yaml) else { continue; };
        if board.name.trim().is_empty() { continue; }
        // CLI-created placeholder boards use this generic name. Never promote
        // them into tabs unless the user explicitly registered the project.
        if board.name.trim().eq_ignore_ascii_case("TaskBoard")
            && !catalog.projects.iter().any(|project| same_path(&project.path, &path)) {
            continue;
        }
        if let Some(project) = catalog.projects.iter_mut().find(|project| same_path(&project.path, &path)) {
            project.name = board.name;
        } else {
            let identity = normalized_path(&path);
            catalog.projects.push(Project {
                id: format!("discovered:{identity}"), name: board.name,
                path, folders: Vec::new(), priority: ProjectPriority::Normal,
            });
        }
    }
    // Listing never writes projects.json, so polling cannot overwrite another
    // window's folder associations or active tab. Mutations persist as before.
    Ok(catalog)
}

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

fn normalized_path(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\").trim_start_matches(r"\\?\").trim_end_matches('\\').to_lowercase()
    } else { path.trim_end_matches('/').to_string() }
}

fn same_path(a: &str, b: &str) -> bool {
    normalized_path(a) == normalized_path(b)
}

// The inactive-board sweep calls this synchronously from its own worker.
pub(crate) fn list_projects_sync() -> Result<ProjectCatalog, String> { load() }

#[tauri::command]
pub async fn list_projects() -> Result<ProjectCatalog, String> {
    tauri::async_runtime::spawn_blocking(load).await.map_err(|e| e.to_string())?
}

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
pub fn add_project(name: String, folders: Vec<String>, task_prefix: String, epic_prefix: String) -> Result<ProjectCatalog, String> {
    let root = board_directory(&name)?;
    let normalized_folders = normalize_folders(folders)?;
    let mut catalog = load()?;
    if catalog.projects.iter().any(|p| same_path(&p.path, &root.to_string_lossy())) {
        return Err("This project's TaskBoard already has a tab".into());
    }
    // Creating a project never touches its source directories. An existing
    // Obsidian board is reused, and the store only initializes missing files.
    fs::create_dir_all(&root).map_err(|e| format!("Cannot create Obsidian Tasks folder: {e}"))?;
    let store = FsTaskStore::default();
    store.ensure_board(&root, name.trim()).map_err(|e| e.to_string())?;
    let mut board = store.load(&root).map_err(|e| e.to_string())?.config;
    board.task_prefix = Some(task_prefix.trim().to_string());
    board.epic_prefix = Some(epic_prefix.trim().to_string());
    // Namespaces belong to _board.md, NOT the local project catalog, so --root
    // CLI calls and desktop creation always agree about the next IDs.
    store.save_config(&root, board).map_err(|e| e.to_string())?;
    let id = format!("project-{}", uuidless_unique_id());
    catalog.projects.push(Project {
        id: id.clone(), name: name.trim().to_string(),
        path: root.to_string_lossy().to_string(), folders: normalized_folders,
        priority: ProjectPriority::Normal,
    });
    catalog.hidden_paths.retain(|path| !same_path(path, &root.to_string_lossy()));
    catalog.active_id = id;
    save(&catalog)?;
    Ok(catalog)
}

#[tauri::command]
pub fn set_project_folders(id: String, folders: Vec<String>, priority: ProjectPriority) -> Result<ProjectCatalog, String> {
    let normalized = normalize_folders(folders)?;
    let mut catalog = load()?;
    let project = catalog.projects.iter_mut().find(|p| p.id == id)
        .ok_or("Unknown project tab")?;
    project.folders = normalized;
    project.priority = priority;
    save(&catalog)?;
    Ok(catalog)
}

fn uuidless_unique_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    format!("{}-{stamp}", std::process::id())
}

#[tauri::command]
pub fn reorder_projects(ids: Vec<String>) -> Result<ProjectCatalog, String> {
    let mut catalog = load()?;
    let expected: std::collections::HashSet<_> = catalog.projects.iter().map(|p| p.id.as_str()).collect();
    let requested: std::collections::HashSet<_> = ids.iter().map(String::as_str).collect();
    if ids.len() != catalog.projects.len() || requested != expected {
        return Err("Project tabs changed: refresh before reordering (no change saved)".into());
    }
    let mut order = std::collections::HashMap::new();
    for (position, id) in ids.iter().enumerate() {
        order.insert(id.as_str(), position);
    }
    catalog.projects.sort_by_key(|p| order[&p.id.as_str()]);
    save(&catalog)?;
    Ok(catalog)
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
pub fn move_task_between_projects(
    source_project_id: String,
    destination_project_id: String,
    task_id: String,
    expected_revision: String,
) -> Result<task_core::Task, String> {
    use task_core::{MutationActor, TaskDraft, TaskKind};
    if source_project_id == destination_project_id {
        return Err("Choose a different project tab".into());
    }
    let catalog = load()?;
    let source_project = catalog.projects.iter()
        .find(|project| project.id == source_project_id)
        .ok_or("Source project is no longer registered")?;
    let destination_project = catalog.projects.iter()
        .find(|project| project.id == destination_project_id)
        .ok_or("Target project is no longer registered")?;
    let source_root = PathBuf::from(&source_project.path);
    let destination_root = PathBuf::from(&destination_project.path);
    if same_path(&source_project.path, &destination_project.path) {
        return Err("The source and target use the same TaskBoard".into());
    }
    let store = FsTaskStore::default();
    let source = store.load(&source_root).map_err(|e| e.to_string())?;
    let old = source.tasks.iter().find(|task| task.id == task_id && !task.archived)
        .ok_or("Source task no longer exists")?;
    if old.revision != expected_revision {
        return Err("The task changed on disk. Refresh and retry.".into());
    }
    if old.kind == TaskKind::Epic || old.claimed_by.is_some() || old.parent.is_some()
        || old.position_after.is_some() || !old.depends_on.is_empty() ||
        !old.related_to.is_empty() || old.completed.is_some()
        || old.review_status.is_some() {
        return Err("Cannot transfer an epic, claimed, linked, completed or reviewed task. Remove its relationships first.".into());
    }
    if source.tasks.iter().any(|other| other.id != old.id && (
        other.parent.as_deref() == Some(&old.id)
        || other.position_after.as_deref() == Some(&old.id)
        || other.depends_on.contains(&old.id)
        || other.related_to.contains(&old.id)
    )) {
        return Err("Another source task refers to this task. Remove those links before transferring it.".into());
    }
    let source_file = source_root.join("Active").join(format!("{}.md", old.id));
    if old.path.to_lowercase() != source_file.to_string_lossy().to_lowercase() {
        return Err("Only normal active Markdown task cards can be transferred".into());
    }
    if source_root.join("Reviews").join(&old.id).exists() {
        return Err("This task has review media. Move the review and attachments separately before transferring.".into());
    }
    let original = fs::read(&source_file).map_err(|e| e.to_string())?;
    let markdown = String::from_utf8(original.clone()).map_err(|e| e.to_string())?;
    let source_attachments = source_root.join("Attachments").join(&old.id);
    // Image references are task-local; the attachment directory moves with the
    // Markdown card, and links are rewritten to the destination ID below.
    let own_attachment_prefix = format!("Attachments/{}/", old.id);
    if markdown.contains("Attachments/") && !markdown.contains(&own_attachment_prefix) {
        return Err("This card links attachments belonging to a different task. Move those references manually.".into());
    }
    if markdown.contains(&own_attachment_prefix) && !source_attachments.is_dir() {
        return Err("This task references attachments that are missing on disk.".into());
    }
    let destination = store.load(&destination_root).map_err(|e| e.to_string())?;
    let column = if destination.config.columns.iter()
        .any(|col| col.id == old.column && !col.hidden && !col.terminal) {
        old.column.clone()
    } else {
        destination.config.columns.iter()
            .find(|col| col.ai_ready && !col.hidden && !col.terminal)
            .or_else(|| destination.config.columns.iter()
                .find(|col| col.id == "draft" && !col.hidden))
            .ok_or("Target board has no usable active column")?
            .id.clone()
    };
    let draft = TaskDraft {
        reserved_id: None,
        reservation_token: None,
        kind: old.kind,
        review_mode: old.review_mode,
        deployable: old.deployable,
        repeat_every_days: old.repeat_every_days,
        next_due: old.next_due.clone(),
        depends_on: vec![],
        related_to: vec![],
        title: old.title.clone(),
        column: column.clone(),
        parent: None,
        category: old.category.clone(),
        icon: old.icon.clone(),
        attention: old.attention.clone(),
        question: old.question.clone(),
        blocked_reason: old.blocked_reason.clone(),
        description: old.description.clone(),
        acceptance_criteria: old.acceptance_criteria.clone(),
        ai_notes: old.ai_notes.clone(),
        user_notes: old.user_notes.clone(),
        notes: old.notes.clone(),
    };
    // The target's native ID reservation and store lock prevent duplicate IDs
    // even when someone creates a task concurrently. Never make up an ID.
    let created = store.create_as(&destination_root, draft, MutationActor::Human)
        .map_err(|e| e.to_string())?;
    let created_file = destination_root.join("Active").join(format!("{}.md", created.id));
    let destination_attachments = destination_root.join("Attachments").join(&created.id);
    let mut frontmatter = false;
    let mut field_count = 0usize;
    let migrated = markdown.lines().enumerate().map(|(n, line)| {
        if n == 0 && line.trim() == "---" {
            frontmatter = true;
            return line.to_string();
        }
        if frontmatter && line.trim() == "---" {
            frontmatter = false;
            return line.to_string();
        }
        if frontmatter && line.starts_with("id: ") {
            field_count += 1;
            return format!("id: {}", created.id);
        }
        if frontmatter && line.starts_with("column: ") {
            field_count += 1;
            return format!("column: {column}");
        }
        line.to_string()
    }).collect::<Vec<_>>().join("\n").replace(
        &own_attachment_prefix, &format!("Attachments/{}/", created.id)) + "\n";
    if field_count != 2 {
        let _ = fs::remove_file(&created_file);
        return Err("Source frontmatter has no unique id/column; transfer aborted".into());
    }
    if let Err(error) = fs::write(&created_file, migrated) {
        let _ = fs::remove_file(&created_file);
        return Err(format!("Could not persist moved task: {error}"));
    }
    let verified = store.load(&destination_root).map_err(|e| e.to_string())
        .and_then(|validated| validated.tasks.into_iter()
            .find(|task| task.id == created.id && task.title == old.title
                && task.description == old.description.replace(
                    &own_attachment_prefix, &format!("Attachments/{}/", created.id)))
            .ok_or_else(|| "Target task could not be verified; original kept".to_string()));
    let new_task = match verified {
        Ok(task) => task,
        Err(error) => {
            let _ = fs::remove_file(&created_file);
            return Err(error);
        }
    };
    if fs::read(&source_file).map_err(|e| e.to_string())? != original {
        let _ = fs::remove_file(&created_file);
        return Err("Source was edited during transfer; target copy rolled back".into());
    }
    // Same vault drive: rename preserves exact media bytes and is reversible if
    // the Markdown removal fails. Do not leave broken links on either board.
    if source_attachments.exists() {
        if destination_attachments.exists() {
            let _ = fs::remove_file(&created_file);
            return Err("Destination attachment directory already exists; source unchanged".into());
        }
        if let Err(error) = fs::rename(&source_attachments, &destination_attachments) {
            let _ = fs::remove_file(&created_file);
            return Err(format!("Could not move attachments; source unchanged: {error}"));
        }
    }
    if let Err(error) = fs::remove_file(&source_file) {
        if destination_attachments.exists() {
            let _ = fs::rename(&destination_attachments, &source_attachments);
        }
        let _ = fs::remove_file(&created_file);
        return Err(format!("Could not remove source; target copy rolled back: {error}"));
    }
    Ok(new_task)
}

#[tauri::command]
pub fn restore_project(project: Project, index: usize) -> Result<ProjectCatalog, String> {
    let mut catalog = load()?;
    if catalog.projects.iter().any(|item| item.id == project.id || same_path(&item.path, &project.path)) {
        return Err("The project is already open".into());
    }
    if !catalog.hidden_paths.iter().any(|path| same_path(path, &project.path)) {
        return Err("This project was not closed or has already been restored".into());
    }
    if !Path::new(&project.path).join("_board.md").is_file() {
        return Err("The original project TaskBoard is not accessible".into());
    }
    catalog.hidden_paths.retain(|path| !same_path(path, &project.path));
    catalog.projects.insert(index.min(catalog.projects.len()), project.clone());
    catalog.active_id = project.id;
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
    let removed_path = catalog.projects.iter().find(|p| p.id == id).unwrap().path.clone();
    if !catalog.hidden_paths.iter().any(|path| same_path(path, &removed_path)) {
        catalog.hidden_paths.push(removed_path);
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
    fn external_boards_are_discovered_live_without_changing_identity_or_selection() {
        let base = std::env::temp_dir().join(format!("taskboard-discovery-{}", uuidless_unique_id()));
        let vault = base.join("vault");
        let root = vault.join("CitizenImperium").join("Tasks");
        fs::create_dir_all(&root).unwrap();
        let metadata = root.join("_board.md");
        let initial = default_catalog();
        assert_eq!(discover_boards(initial.clone(), &vault).unwrap().projects.len(), 1);
        fs::write(&metadata, "---\nname: TaskBoard\n---\n").unwrap();
        assert_eq!(discover_boards(initial.clone(), &vault).unwrap().projects.len(), 1,
            "generic placeholder boards must not be discovered");
        fs::write(&metadata, "---\nschema: 1\nname: Citizen\n---\n").unwrap();
        let first = discover_boards(initial, &vault).unwrap();
        assert_eq!(first.active_id, "wartribes");
        assert_eq!(first.projects.len(), 2);
        let discovered = first.projects[1].clone();
        assert_eq!(discovered.name, "Citizen");
        assert_eq!(discovered.path, root.to_string_lossy());
        assert!(!root.join("Active").exists(), "discovery must not initialize or maintain task stores");
        assert_eq!(discover_boards(default_catalog(), &vault).unwrap().projects[1].id, discovered.id);

        let mut registered = first;
        registered.projects[1].id = "existing-citizen".into();
        registered.projects[1].folders = vec!["D:/Unreal/CitizenImperium".into()];
        let catalog_file = base.join("projects.json");
        save_at(&catalog_file, &registered).unwrap();
        fs::write(&metadata, "---\nname: Citizen renamed\n---\n").unwrap();
        let renamed = discover_boards(load_at(&catalog_file).unwrap(), &vault).unwrap();
        assert_eq!(renamed.projects.len(), 2);
        assert_eq!(renamed.projects[1].id, "existing-citizen");
        assert_eq!(renamed.projects[1].folders, registered.projects[1].folders);
        assert_eq!(renamed.projects[1].name, "Citizen renamed");
        assert_eq!(load_at(&catalog_file).unwrap().projects[1].name, "Citizen", "discovery is read-only");

        let mut hidden = default_catalog();
        hidden.hidden_paths.push(discovered.path);
        save_at(&catalog_file, &hidden).unwrap();
        assert_eq!(discover_boards(load_at(&catalog_file).unwrap(), &vault).unwrap().projects.len(), 1);
        fs::write(&metadata, "---\nname: [incomplete").unwrap();
        assert_eq!(discover_boards(default_catalog(), &vault).unwrap().projects.len(), 1);
        assert_eq!(discover_boards(registered, &base.join("offline")).unwrap().projects.len(), 2);
        let resolved = fs::canonicalize(&base).unwrap();
        assert!(resolved.starts_with(fs::canonicalize(std::env::temp_dir()).unwrap()));
        fs::remove_dir_all(resolved).unwrap();
    }

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
            path: r"D:\Projects\Another\Tasks".into(), folders: Vec::new(),
            priority: ProjectPriority::Normal });
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
        assert_eq!(loaded.projects[0].priority, ProjectPriority::Normal);
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
