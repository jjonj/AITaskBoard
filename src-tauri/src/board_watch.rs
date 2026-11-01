//! Lightweight change detection for all registered boards, including inactive tabs.
//! Only task Markdown, board config and review manifests participate; attachments
//! and transient reservation files must not cause constant UI refreshes.
use std::{collections::{HashMap, hash_map::DefaultHasher}, fs, hash::{Hash, Hasher},
    io::Write, path::Path, process::{Command, Stdio}, sync::{Mutex, OnceLock},
    time::{Duration, Instant, UNIX_EPOCH}};
use task_core::{FsTaskStore, TaskStore};

#[derive(Clone)]
struct WatchedTask {
    revision: String,
    claimant: Option<String>,
    content: String,
    column: String,
}

/// Detect externally written updates, including changes made through the CLI.
/// Only deliver to an exact claimed T3 UUID, never infer a session from a title.
fn notify_task_changes(root: &Path) {
    static SEEN: OnceLock<Mutex<HashMap<String, HashMap<String, WatchedTask>>>> = OnceLock::new();
    let Ok(board) = FsTaskStore::default().load(root) else { return; };
    let current = board.tasks.into_iter().filter(|task| !task.archived).map(|task| {
        let content = serde_json::json!({
            // Only substantive instructions should interrupt the worker.
            // Agent-written progress notes must not generate self-notification loops.
            "title":task.title, "description":task.description,
            "acceptance":task.acceptance_criteria, "user_notes":task.user_notes,
        }).to_string();
        (task.id, WatchedTask { revision:task.revision, claimant:task.claimed_by,
            content, column:task.column })
    }).collect::<HashMap<_,_>>();
    let key = root.to_string_lossy().to_string();
    let mut seen = SEEN.get_or_init(|| Mutex::new(HashMap::new())).lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(previous) = seen.get(&key) {
        for (id, after) in &current {
            let Some(before) = previous.get(id) else { continue; };
            if after.revision == before.revision || after.content == before.content ||
               after.column != "in-progress" { continue; }
            let Some(raw) = after.claimant.as_deref() else { continue; };
            let session = raw.strip_prefix("t3-browser:").unwrap_or(raw);
            if !is_t3_uuid(session) { continue; }
            dispatch_t3_task_update(key.clone(), id.clone(), session.to_string());
        }
    }
    seen.insert(key, current);
}

fn is_t3_uuid(value: &str) -> bool {
    value.len() == 36 && value.bytes().enumerate().all(|(i, c)|
        if [8, 13, 18, 23].contains(&i) { c == b'-' } else { c.is_ascii_hexdigit() })
}

fn dispatch_t3_task_update(board: String, id: String, session: String) {
    std::thread::spawn(move || {
        let script = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
        if !Path::new(script).is_file() { return; }
        let message = format!("TaskBoard update: {id} has changed. Re-read task {id} in {board} before continuing. Do not assume the previous task revision is current.");
        let request = serde_json::json!({"action":"send", "sessionId":session, "message":message});
        let Ok(mut child) = Command::new("node").arg(script)
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else { return; };
        if let Some(mut input) = child.stdin.take() {
            let _ = input.write_all(request.to_string().as_bytes());
        }
        let _ = child.wait();
    });
}


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
        let token = format!("{:016x}", hasher.finish());
        static LAST_TOKEN: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
        let changed = {
            let mut cache = LAST_TOKEN.get_or_init(|| Mutex::new(HashMap::new())).lock()
                .unwrap_or_else(|poison| poison.into_inner());
            cache.insert(path.clone(), token.clone()).as_deref() != Some(token.as_str())
        };
        if changed { notify_task_changes(root); }
        BoardChangeToken { path, token: Some(token) }
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
