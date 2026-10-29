//! Lightweight change detection for all registered boards, including inactive tabs.
//! Only task Markdown, board config and review manifests participate; attachments
//! and transient reservation files must not cause constant UI refreshes.
use std::{collections::{HashMap, hash_map::DefaultHasher}, fs, hash::{Hash, Hasher},
    path::Path, sync::{Mutex, OnceLock}, time::{Duration, Instant, UNIX_EPOCH}};
use task_core::{FsTaskStore, TaskStore};

/// Board reads reconcile stale work; limit them to once per five minutes per board.
/// This hooks into the existing UI change polling, without another background agent.
fn maintain_if_due(root: &Path) {
    static LAST: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let now = Instant::now();
    let key = root.to_string_lossy().to_string();
    let should_run = {
        let mut guard = LAST.get_or_init(|| Mutex::new(HashMap::new())).lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if guard.get(&key).is_some_and(|last| now.duration_since(*last) < Duration::from_secs(300)) {
            false
        } else {
            guard.insert(key, now);
            true
        }
    };
    if should_run {
        let _ = FsTaskStore::default().load(root);
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BoardChangeToken {
    path: String,
    token: Option<String>,
}

fn scan_files(dir: &Path, hasher: &mut DefaultHasher, depth: u8) {
    if depth > 4 { return; }
    let Ok(entries) = fs::read_dir(dir) else { return; };
    let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            scan_files(&path, hasher, depth + 1);
        } else if path.extension().is_some_and(|ext| {
            let ext = ext.to_string_lossy();
            ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("json")
        }) {
            record_file(&path, hasher);
        }
    }
}

fn record_file(path: &Path, hasher: &mut DefaultHasher) {
    path.hash(hasher);
    match fs::metadata(path) {
        Ok(meta) => {
            meta.len().hash(hasher);
            meta.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos()).hash(hasher);
        }
        Err(_) => "missing".hash(hasher),
    }
}

#[tauri::command]
pub(crate) async fn board_change_tokens(paths: Vec<String>) -> Vec<BoardChangeToken> {
    // The Obsidian vault may be cloud-backed; never stat its files on the UI thread.
    tauri::async_runtime::spawn_blocking(move || scan_tokens(paths))
        .await.unwrap_or_default()
}

fn scan_tokens(paths: Vec<String>) -> Vec<BoardChangeToken> {
    paths.into_iter().map(|path| {
        let root = Path::new(&path);
        let board = root.join("_board.md");
        if !board.is_file() {
            return BoardChangeToken { path, token: None };
        }
        maintain_if_due(root);
        let mut hasher = DefaultHasher::new();
        record_file(&board, &mut hasher);
        for folder in ["Active", "Archive", "Reviews"] {
            scan_files(&root.join(folder), &mut hasher, 0);
        }
        BoardChangeToken { path, token: Some(format!("{:016x}", hasher.finish())) }
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::scan_tokens;
    use std::{fs, time::{SystemTime, UNIX_EPOCH}};

    #[test]
    fn detects_external_task_edits_and_new_tasks() {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("taskboard-watch-{}-{suffix}", std::process::id()));
        fs::create_dir_all(root.join("Active")).unwrap();
        fs::write(root.join("_board.md"), "name: Test\n").unwrap();
        let path = root.to_string_lossy().to_string();
        let token = || scan_tokens(vec![path.clone()])[0].token.clone().unwrap();
        let before = token();
        fs::write(root.join("Active").join("TST-0001.md"), "first").unwrap();
        let created = token();
        assert_ne!(before, created);
        fs::write(root.join("Active").join("TST-0001.md"), "changed by another agent").unwrap();
        let edited = token();
        assert_ne!(created, edited);
        assert_eq!(edited, token());
        fs::remove_dir_all(root).unwrap();
    }
}
