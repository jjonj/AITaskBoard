mod projects;
mod board_watch;
mod queue_bridge;

use std::{fs, path::PathBuf, process::{Command, Stdio}, io::Write};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use task_core::{
    BoardConfig, BoardSnapshot, FsTaskStore, MutationActor, Task, TaskDraft, TaskIdReservation, TaskKind, TaskStore,
    ReviewManifest, ReviewRequest, ReviewResolution, ReviewReconsider,
    ReleaseSummary, ReleaseRecord, SourceRef, ExecutionIntent, ExecutionMode,
};

pub(crate) const WARTRIBES_BOARD: &str =
    r"B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks";
const FOMNI_EXE: &str =
    r"D:\SSDProjects\Tools\Omnish\main\target\release\fomni.exe";
const OBSIDIAN_EXE: &str =
    r"E:\Program Files\Obsidian\Obsidian.exe";

fn root(path: Option<String>) -> PathBuf {
    PathBuf::from(path.unwrap_or_else(|| WARTRIBES_BOARD.to_string()))
}

#[tauri::command]
fn default_board_path() -> String {
    WARTRIBES_BOARD.to_string()
}

#[tauri::command]
fn load_board(path: Option<String>) -> Result<BoardSnapshot, String> {
    FsTaskStore::default()
        .load(&root(path))
        .map_err(|e| e.to_string())
}

/// Lightweight project-tab inbox source: return exact eligible IDs, not a
/// misleading count that cannot reflect renderer-local snoozes.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UserQueueProjectIds {
    path: String,
    eligible_ids: Vec<String>,
    error: Option<String>,
}

#[tauri::command]
async fn user_queue_project_ids(paths: Vec<String>) -> Vec<UserQueueProjectIds> {
    tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(|path| {
        match FsTaskStore::default().load(&PathBuf::from(&path)) {
            Ok(board) => {
                let terminal: std::collections::HashSet<_> = board.config.columns.iter()
                    .filter(|column| column.terminal).map(|column| column.id.as_str()).collect();
                let ids = board.tasks.iter().filter(|task| {
                    let review = task.review_status.as_deref();
                    if matches!(review, Some("open" | "awaiting_revision")) { return false; }
                    let independent_question = task.question.as_ref()
                        .is_some_and(|q| !q.prompt.trim().is_empty() && q.answer.is_none()) &&
                        task.attention.contains(&task_core::TaskAttention::Question);
                    if review == Some("resolved") && !independent_question { return false; }
                    ((task.kind == TaskKind::Task &&
                      task.attention.contains(&task_core::TaskAttention::Question) &&
                      task.question.as_ref().is_some_and(|q| !q.prompt.trim().is_empty() && q.answer.is_none())) ||
                     ((task.kind == TaskKind::ForUser ||
                       (task.kind == TaskKind::Task && task.attention.contains(&task_core::TaskAttention::NeedsUser))) &&
                      task.completed.is_none() && !terminal.contains(task.column.as_str()))) &&
                    !task.archived &&
                    task.depends_on.iter().all(|id| board.tasks.iter().any(|dep| {
                        dep.id == *id &&
                        (dep.archived || dep.completed.is_some() ||
                         terminal.contains(dep.column.as_str()))
                    }))
                }).map(|task| task.id.clone()).collect();
                UserQueueProjectIds { path, eligible_ids: ids, error: None }
            }
            Err(err) => UserQueueProjectIds { path, eligible_ids: vec![],
                error: Some(err.to_string()) },
        }
    }).collect()).await.unwrap_or_default()
}

#[tauri::command]
fn create_task(path: Option<String>, draft: TaskDraft) -> Result<Task, String> {
    // Desktop edits are human: they may block/unblock but never author blocked_reason.
    FsTaskStore::default()
        .create_as(&root(path), draft, MutationActor::Human)
        .map_err(|e| e.to_string())
}

/// Execution selection is stored in TaskBoard's revision-checked core, never a UI-only flag.
#[tauri::command]
async fn task_sync_queue(path: Option<String>, id: String) -> Result<queue_bridge::QueueSyncStatus, String> {
    queue_bridge::sync(root(path), id).await
}

#[tauri::command]
async fn task_queue_overview(path: Option<String>) -> Result<serde_json::Value, String> {
    queue_bridge::overview(root(path)).await
}

#[tauri::command]
async fn task_queue_capacity() -> Result<serde_json::Value, String> {
    queue_bridge::capacity().await
}

#[tauri::command]
async fn task_queue_reprioritize(path: Option<String>, id: String, expected_qitem: String, priority: String)
    -> Result<queue_bridge::QueueSyncStatus, String> {
    queue_bridge::reprioritize(root(path), id, expected_qitem, priority).await
}

#[tauri::command]
async fn task_queue_move(path: Option<String>, id: String, expected_qitem: String, direction: String)
    -> Result<queue_bridge::QueueSyncStatus, String> {
    queue_bridge::move_pending(root(path), id, expected_qitem, direction).await
}

#[tauri::command]
async fn task_queue_reconcile(path: Option<String>) -> Result<serde_json::Value, String> {
    queue_bridge::reconcile(root(path)).await
}

#[tauri::command]
async fn task_openrig_trace(path: Option<String>, id: String) -> Result<serde_json::Value, String> {
    queue_bridge::trace(root(path), id).await
}

#[tauri::command]
async fn task_openrig_worker_proof(qitem_id: String) -> Result<serde_json::Value, String> {
    queue_bridge::worker_proof(qitem_id).await
}

#[tauri::command]
fn task_execution_intent(path: Option<String>, id: String) -> Result<ExecutionIntent, String> {
    FsTaskStore::default().execution_intent(&root(path), &id).map_err(|e| e.to_string())
}

#[tauri::command]
fn task_set_execution_mode(path: Option<String>, id: String, expected_revision: String, mode: ExecutionMode) -> Result<ExecutionIntent, String> {
    FsTaskStore::default().set_execution_mode(&root(path), &id, &expected_revision, mode).map_err(|e| e.to_string())
}

#[tauri::command]
fn task_request_start_now(path: Option<String>, id: String, expected_revision: String) -> Result<ExecutionIntent, String> {
    FsTaskStore::default().request_start_now(&root(path), &id, &expected_revision).map_err(|e| e.to_string())
}


/// A human editor toggle for the same optional live-task state used by the CLI.
/// All changes pass through the shared revision-checked Markdown store.
#[tauri::command]
fn task_live_with_me(path: Option<String>, id: String, expected_revision: String, with_me: bool) -> Result<Task, String> {
    let path = root(path);
    let store = FsTaskStore::default();
    let mut task = store.load(&path).map_err(|e| e.to_string())?.tasks.into_iter()
        .find(|task| task.id == id && !task.archived)
        .ok_or_else(|| format!("Task not found: {id}"))?;
    if task.revision != expected_revision {
        return Err("Task revision changed. Refresh and retry the With me toggle.".into());
    }
    if !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) || task.completed.is_some() {
        return Err("With me requires an active task".into());
    }
    if with_me {
        if task.live.is_none() {
            task.live = Some(task_core::TaskLive {
                with_me: true, state: None, brief: None, est_minutes: None,
                ready_at: None, later_until: None,
            });
        }
    } else {
        if task.live.as_ref().is_some_and(|live| matches!(live.state, Some(task_core::LiveTaskState::Ready))) {
            task.attention.retain(|flag| *flag != task_core::TaskAttention::NeedsUser);
        }
        task.live = None;
    }
    store.update_as(&path, task, MutationActor::Human).map_err(|e| e.to_string())
}

#[tauri::command]
fn reserve_task_id(path: Option<String>, kind: TaskKind) -> Result<TaskIdReservation, String> {
    FsTaskStore::default().reserve_task_id(&root(path), kind).map_err(|e| e.to_string())
}

#[tauri::command]
fn release_task_id(path: Option<String>, reservation: TaskIdReservation) -> Result<(), String> {
    FsTaskStore::default().release_task_id(&root(path), &reservation).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_task_image(path: Option<String>, task_id: String, data: String, mime: String) -> Result<String, String> {
    let extension = match mime.as_str() {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => return Err("clipboard image must be PNG, JPEG, WebP, or GIF".into()),
    };
    if data.len() > 28 * 1024 * 1024 {
        return Err("clipboard image is over 20 MB".into());
    }
    let bytes = BASE64.decode(data).map_err(|e| e.to_string())?;
    FsTaskStore::default().attach_task_image(&root(path), &task_id, &bytes, extension)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn read_task_image(path: Option<String>, relative_path: String) -> Result<String, String> {
    let components: Vec<_> = relative_path.split('/').collect();
    if components.len() != 3 || components[0] != "Attachments"
        || !components[1].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        || components[2].split('.').count() != 2
        || !components[2].split('.').next().is_some_and(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err("invalid task image path".into());
    }
    let ext = components[2].rsplit('.').next().unwrap_or_default();
    let mime = match ext {
        "png" => "image/png", "jpg" => "image/jpeg",
        "webp" => "image/webp", "gif" => "image/gif",
        _ => return Err("unsupported task image type".into()),
    };
    let bytes = fs::read(root(path).join("Attachments").join(components[1]).join(components[2]))
        .map_err(|e| e.to_string())?;
    if bytes.len() > 20 * 1024 * 1024 { return Err("image over 20 MB".into()); }
    Ok(format!("data:{mime};base64,{}", BASE64.encode(bytes)))
}

#[tauri::command]
fn review_show(path: Option<String>, task_id: String) -> Result<ReviewManifest, String> {
    FsTaskStore::default().review_show(&root(path), &task_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn review_request(path: Option<String>, request: ReviewRequest) -> Result<ReviewManifest, String> {
    FsTaskStore::default().review_request(&root(path), request).map_err(|e| e.to_string())
}

#[tauri::command]
fn review_reconsider(path: Option<String>, request: ReviewReconsider) -> Result<ReviewManifest, String> {
    FsTaskStore::default().review_reconsider(&root(path), request).map_err(|e| e.to_string())
}

#[tauri::command]
fn review_resolve(path: Option<String>, resolution: ReviewResolution) -> Result<ReviewManifest, String> {
    FsTaskStore::default().review_resolve(&root(path), resolution).map_err(|e| e.to_string())
}

#[tauri::command]
fn review_asset(path: Option<String>, asset_path: String) -> Result<String, String> {
    let root = root(path);
    let candidate = PathBuf::from(&asset_path);
    let real = candidate.canonicalize().map_err(|e| e.to_string())?;
    let allowed_board = root.join("Reviews").canonicalize().ok()
        .is_some_and(|folder| real.starts_with(folder));
    let allowed_local = std::env::var_os("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join("TaskBoard").join("review-media"))
        .and_then(|path| path.canonicalize().ok())
        .is_some_and(|folder| real.starts_with(folder));
    if !allowed_board && !allowed_local { return Err("review media path is outside permitted review roots".into()); }
    let ext = real.extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp", "gif" => "image/gif",
        "mp4" => "video/mp4", "webm" => "video/webm",
        "html" | "htm" => "text/html", "css" => "text/css",
        "js" | "mjs" => "application/javascript", "svg" => "image/svg+xml",
        "woff" => "font/woff", "woff2" => "font/woff2",
        "ttf" => "font/ttf", "ico" => "image/x-icon", "json" => "application/json",
        _ => return Err("unsupported review media".into()),
    };
    let bytes = fs::read(real).map_err(|e| e.to_string())?;
    if bytes.len() > 30 * 1024 * 1024 { return Err("review asset is larger than 30 MB".into()); }
    Ok(format!("data:{mime};base64,{}", BASE64.encode(bytes)))
}

#[tauri::command]
fn update_task(path: Option<String>, task: Task) -> Result<Task, String> {
    FsTaskStore::default()
        .update_as(&root(path), task, MutationActor::Human)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn archive_task(path: Option<String>, id: String) -> Result<Task, String> {
    FsTaskStore::default()
        .archive(&root(path), &id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn archive_column(path: Option<String>, column_id: String, expected: Vec<(String, String)>) -> Result<BoardSnapshot, String> {
    FsTaskStore::default()
        .archive_column(&root(path), &column_id, &expected)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn archive_column_older_than(
    path: Option<String>,
    column_id: String,
    cutoff_iso: String,
    expected: Vec<(String, String)>,
) -> Result<BoardSnapshot, String> {
    FsTaskStore::default()
        .archive_column_older_than(&root(path), &column_id, &cutoff_iso, &expected)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn save_board_config(path: Option<String>, config: BoardConfig) -> Result<BoardConfig, String> {
    FsTaskStore::default()
        .save_config(&root(path), config)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn restore_snapshot(
    path: Option<String>,
    before: BoardSnapshot,
    after: BoardSnapshot,
) -> Result<BoardSnapshot, String> {
    FsTaskStore::default()
        .restore_snapshot(&root(path), before, after)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn new_release(path: Option<String>) -> Result<ReleaseRecord, String> {
    FsTaskStore::default().release_new(&root(path)).map_err(|e| e.to_string())
}
#[tauri::command]
fn list_releases(path: Option<String>) -> Result<ReleaseSummary, String> {
    FsTaskStore::default().releases(&root(path)).map_err(|e| e.to_string())
}
#[tauri::command]
fn enable_releases(path: Option<String>, enabled: bool) -> Result<ReleaseSummary, String> {
    FsTaskStore::default().release_enable(&root(path), enabled).map_err(|e| e.to_string())
}
#[tauri::command]
fn reconcile_releases(path: Option<String>, historical: bool) -> Result<ReleaseSummary, String> {
    FsTaskStore::default().reconcile_releases(&root(path), historical).map_err(|e| e.to_string())
}
#[tauri::command]
fn edit_release(path: Option<String>, action: String, release: String, task: Option<String>, source: Option<SourceRef>, evidence: Option<String>) -> Result<ReleaseRecord, String> {
    FsTaskStore::default().release_edit(&root(path), &action, &release, task.as_deref(), source, evidence.as_deref()).map_err(|e| e.to_string())
}
#[tauri::command]
fn prepare_release(path: Option<String>, release: String, evidence: String) -> Result<ReleaseRecord, String> {
    FsTaskStore::default().release_prepare(&root(path), &release, &evidence).map_err(|e| e.to_string())
}
#[tauri::command]
fn outcome_release(path: Option<String>, release: String, attempt: u32, status: String, evidence: String) -> Result<ReleaseRecord, String> {
    FsTaskStore::default().release_outcome(&root(path), &release, attempt, &status, &evidence).map_err(|e| e.to_string())
}
#[tauri::command]
fn verify_release_member(path: Option<String>, release: String, attempt: u32, task: String,
    passed: bool, evidence: String) -> Result<ReleaseRecord, String> {
    FsTaskStore::default().release_verify_member(&root(path), &release, attempt, &task, passed, &evidence).map_err(|e| e.to_string())
}

#[tauri::command]
fn highlight_in_fomni(file: String) -> Result<(), String> {
    Command::new(FOMNI_EXE)
        .arg("--select")
        .arg(file)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to launch Fomni: {e}"))
}

fn percent_encode_query(value: &str) -> String {
    let mut out = String::new();
    for byte in value.as_bytes() {
        match *byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'~' => out.push(*byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[tauri::command]
fn open_in_obsidian(file: String) -> Result<(), String> {
    let uri = format!("obsidian://open?path={}", percent_encode_query(&file));
    Command::new(OBSIDIAN_EXE)
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("failed to launch Obsidian: {e}"))
}

/// Human-invoked navigation only. No prompts or ChatGPT messages are sent.
#[tauri::command]
fn open_claimed_t3_session(claimed_by: String) -> Result<serde_json::Value, String> {
    let claimant = claimed_by.strip_prefix("t3-browser:").unwrap_or(&claimed_by);
    let is_uuid = claimant.len() == 36 && claimant.chars().enumerate().all(|(i, c)|
        if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    if !is_uuid {
        return Err("Task claimant has no exact T3 browser UUID; cannot infer its conversation.".into());
    }
    let script = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
    if !std::path::Path::new(script).is_file() {
        return Err("T3 Code session CLI is unavailable.".into());
    }
    let mut process = Command::new("node").arg(script)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Cannot connect to T3 CLI: {e}"))?;
    if let Some(mut stdin) = process.stdin.take() {
        let request = serde_json::json!({ "action": "open", "sessionId": claimant });
        stdin.write_all(request.to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = process.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("T3 returned an invalid open receipt: {e}"))?;
    if result.get("action").and_then(|v| v.as_str()) != Some("open") ||
       result.get("acknowledged").and_then(|v| v.as_bool()) != Some(true) {
        return Err("T3 did not acknowledge opening the claimed session.".into());
    }
    Ok(result)
}


/// Open the canonical task's existing session, or ask T3's open-task action to
/// create one. The new-session path requires T3-0142; never synthesize a
/// duplicate session or silently treat a transport error as a failed start.
#[tauri::command]
fn open_task_session(path: Option<String>, id: String) -> Result<serde_json::Value, String> {
    let path = root(path);
    let task = FsTaskStore::default().load(&path).map_err(|e| e.to_string())?
        .tasks.into_iter().find(|task| task.id == id && !task.archived)
        .ok_or_else(|| format!("Task not found: {id}"))?;
    if !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) {
        return Err("Only executable tasks can start a session".into());
    }
    let claimant = task.claimed_by.as_deref().unwrap_or("");
    let raw_claimant = claimant.strip_prefix("t3-browser:").unwrap_or(claimant);
    let is_browser = raw_claimant.len() == 36 && raw_claimant.chars().enumerate().all(|(i, c)|
        if [8, 13, 18, 23].contains(&i) { c == '-' } else { c.is_ascii_hexdigit() });
    if is_browser {
        return open_claimed_t3_session(claimant.to_string());
    }
    let script = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
    if !std::path::Path::new(script).is_file() {
        return Err("T3 open-task CLI is unavailable".into());
    }
    let request = serde_json::json!({
        "action": "open-task",
        "taskRef": format!("{}#{}", path.display(), id),
    });
    let mut process = Command::new("node").arg(script)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("Cannot launch T3 CLI: {e}"))?;
    if let Some(mut stdin) = process.stdin.take() {
        stdin.write_all(request.to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = process.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("T3 open-task not confirmed; do not retry until reconciled: {}",
            String::from_utf8_lossy(&output.stderr).trim()));
    }
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("T3 open-task receipt ambiguous; reconcile before retry: {e}"))?;
    if receipt.get("action").and_then(|v| v.as_str()) != Some("open-task")
        || receipt.get("acknowledged").and_then(|v| v.as_bool()) != Some(true) {
        return Err("T3 did not confirm open-task; reconcile before retrying".into());
    }
    Ok(receipt)
}

#[tauri::command]
fn show_main_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    window.set_focus().map_err(|e| e.to_string())?;
    Ok(())
}

fn sweep_registered_review_boards() {
    let Some(appdata) = std::env::var_os("APPDATA") else { return };
    let catalog = PathBuf::from(appdata).join("TaskBoard").join("projects.json");
    let Ok(json) = fs::read_to_string(catalog) else { return };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&json) else { return };
    let mut visited = std::collections::HashSet::new();
    let Some(boards) = value.get("projects").and_then(|value| value.as_array()) else { return };
    for project in boards {
        let Some(path) = project.get("path").and_then(|value| value.as_str()) else { continue };
        let root = PathBuf::from(path);
        if root.join("_board.md").is_file() && visited.insert(path.to_ascii_lowercase()) {
            // Own cross-process lock; never interrupt the UI.
            let _ = FsTaskStore::default().review_gc(&root, false);
        }
    }
}

#[tauri::command]
fn startup_review() -> Option<serde_json::Value> {
    let value: serde_json::Value = serde_json::from_str(&std::env::var("TASKBOARD_START_REVIEW").ok()?).ok()?;
    let path = value.get("path")?.as_str()?;
    let task_id = value.get("taskId")?.as_str()?;
    if !PathBuf::from(path).join("_board.md").is_file() ||
       task_id.is_empty() || task_id.len() > 90 || task_id.contains(['/', '\\']) {
        return None;
    }
    Some(value)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            // Forward deep links to the already-open UI instead of opening a second window.
            if args.len() == 4 && args[1] == "--open-review" {
                use tauri::Emitter;
                let _ = app.emit("taskboard-open-review", serde_json::json!({ "path": args[2], "taskId": args[3] }));
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            std::thread::spawn(|| loop {
                sweep_registered_review_boards();
                std::thread::sleep(std::time::Duration::from_secs(6 * 60 * 60));
            });
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
                window.set_icon(icon)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            startup_review,
            projects::list_projects,
            projects::add_project,
            projects::set_project_folders,
            projects::activate_project,
            projects::reorder_projects,
            projects::remove_project,
            projects::restore_project,
            projects::move_task_between_projects,
            default_board_path,
            load_board,
            board_watch::board_change_tokens,
            user_queue_project_ids,
            create_task,
            task_execution_intent,
            task_sync_queue,
            task_queue_overview,
            task_queue_capacity,
            task_queue_reprioritize,
            task_queue_move,
            task_queue_reconcile,
            task_openrig_trace,
            task_openrig_worker_proof,
            task_set_execution_mode,
            task_request_start_now,
            task_live_with_me,
            reserve_task_id,
            release_task_id,
            save_task_image,
            read_task_image,
            review_show,
            review_request,
            review_reconsider,
            review_resolve,
            review_asset,
            update_task,
            archive_task,
            archive_column,
            archive_column_older_than,
            save_board_config,
            restore_snapshot,
            list_releases,
            new_release,
            enable_releases,
            reconcile_releases,
            edit_release,
            prepare_release,
            outcome_release,
            verify_release_member,
            highlight_in_fomni,
            open_in_obsidian,
            open_claimed_t3_session,
            open_task_session,
            show_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running TaskBoard");
}
