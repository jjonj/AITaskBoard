#[cfg(windows)] use std::os::windows::process::CommandExt;
mod projects;
mod board_watch;
mod claim_health;
mod agent_notices;
mod stale_blockers;
mod agent_policy;
mod preparation;
mod task_agent;
mod queue_bridge;
mod queue_workload;
mod busy_recovery;
mod error_log;
mod restart_warning;
mod window_placement;
mod quick_create;
mod model_favorites;

use std::{fs, path::PathBuf, process::{Command, Stdio}, io::Write};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use task_core::{
    BoardConfig, BoardSnapshot, FsTaskStore, MutationActor, Task, TaskDraft, TaskIdReservation, TaskKind, TaskStore,
    ReviewManifest, ReviewRequest, ReviewResolution, ReviewReconsider, ReviewClose,
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
async fn load_board(path: Option<String>) -> Result<BoardSnapshot, String> {
    // Scanning/parsing cloud-backed Markdown and reconciling a board can be slow.
    // Never block the Tauri command/UI thread while a board opens or refreshes.
    tauri::async_runtime::spawn_blocking(move || {
        FsTaskStore::default().load(&root(path)).map_err(|e| e.to_string())
    }).await.map_err(|e| e.to_string())?
}

/// A new-task overlay needs the board's columns immediately, not a full scan
/// of every task in the cloud-backed directory. The real snapshot is fetched
/// in the background after the editor opens; mutations still use FsTaskStore.
#[tauri::command]
async fn load_quick_board(path: String) -> Result<BoardSnapshot, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let board = PathBuf::from(path).join("_board.md");
        let yaml = fs::read_to_string(&board).map_err(|e| e.to_string())?;
        let config: BoardConfig = serde_yaml::from_str(&yaml).map_err(|e| e.to_string())?;
        Ok(BoardSnapshot { config, tasks: Vec::new(), warnings: Vec::new() })
    }).await.map_err(|e| e.to_string())?
}

/// Lightweight project-tab inbox source: return both human-response and open
/// review IDs, so renderer-local snoozes can be applied before showing a badge.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UserQueueProjectIds {
    path: String,
    eligible_ids: Vec<String>,
    review_ids: Vec<String>,
    request_keys: std::collections::HashMap<String, String>,
    error: Option<String>,
}

#[tauri::command]
async fn user_queue_project_ids(paths: Vec<String>) -> Vec<UserQueueProjectIds> {
    tauri::async_runtime::spawn_blocking(move || paths.into_iter().map(|path| {
        match FsTaskStore::default().load(&PathBuf::from(&path)) {
            Ok(board) => {
                let terminal: std::collections::HashSet<_> = board.config.columns.iter()
                    .filter(|column| column.terminal).map(|column| column.id.as_str()).collect();
                let ids: Vec<String> = board.tasks.iter().filter(|task| {
                    let review = task.review_status.as_deref();
                    if matches!(review, Some("open" | "awaiting_revision")) { return false; }
                    let independent_question = task.question.as_ref()
                        .is_some_and(|q| !q.prompt.trim().is_empty() && q.answer.is_none()) &&
                        task.attention.contains(&task_core::TaskAttention::Question);
                    if review == Some("resolved") && !independent_question { return false; }
                    let pending_question = task.kind == TaskKind::Task && independent_question;
                    let user_owned = task.kind == TaskKind::ForUser ||
                        (task.kind == TaskKind::Task &&
                         task.attention.contains(&task_core::TaskAttention::NeedsUser));
                    !task.archived &&
                    (pending_question || (user_owned && task.completed.is_none() &&
                        !terminal.contains(task.column.as_str()))) &&
                    (pending_question || task.depends_on.iter().all(|id| board.tasks.iter().any(|dep| {
                        dep.id == *id &&
                        (dep.archived || dep.completed.is_some() ||
                         terminal.contains(dep.column.as_str()))
                    })))
                }).map(|task| task.id.clone()).collect();
                let review_ids: Vec<String> = board.tasks.iter()
                    .filter(|task| !task.archived && task.review_status.as_deref() == Some("open"))
                    .map(|task| task.id.clone()).collect();
                let requested: std::collections::HashSet<&str> = ids.iter().chain(review_ids.iter())
                    .map(String::as_str).collect();
                let request_keys = board.tasks.iter().filter(|task| requested.contains(task.id.as_str()))
                    .map(|task| {
                        let key = if task.review_status.as_deref() == Some("open") {
                            format!("review:{}:{}", task.review_round.unwrap_or(0),
                                task.question.as_ref().map(|q| q.prompt.as_str()).unwrap_or_default())
                        } else if task.kind == TaskKind::Task &&
                            task.attention.contains(&task_core::TaskAttention::Question) &&
                            task.question.as_ref().is_some_and(|q| !q.prompt.trim().is_empty() && q.answer.is_none()) {
                            format!("question:{}", task.question.as_ref().unwrap().prompt)
                        } else { "user".to_string() };
                        (task.id.clone(), key)
                    }).collect();
                UserQueueProjectIds { path, eligible_ids: ids, review_ids, request_keys, error: None }
            }
            Err(err) => UserQueueProjectIds { path, eligible_ids: vec![], review_ids: vec![],
                request_keys: Default::default(), error: Some(err.to_string()) },
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
async fn task_queue_workload() -> Result<queue_workload::QueueWorkload, String> {
    queue_workload::read().await
}

#[tauri::command]
fn task_busy_recovery_status() -> busy_recovery::RecoveryStatus {
    busy_recovery::current_status()
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

/// One native read for card-level Queue state; no per-card renderer RPC storm.
#[tauri::command]
fn task_execution_modes(path: Option<String>) -> Result<std::collections::BTreeMap<String, ExecutionIntent>, String> {
    let root = root(path);
    let store = FsTaskStore::default();
    let board = store.load(&root).map_err(|e| e.to_string())?;
    let mut modes = std::collections::BTreeMap::new();
    for task in board.tasks.into_iter().filter(|t| !t.archived &&
        matches!(t.kind, TaskKind::Task | TaskKind::Recurring) && board.config.columns.iter()
            .any(|column| column.id == t.column && column.ai_ready && !column.advisory && !column.terminal)) {
        modes.insert(task.id.clone(), store.execution_intent(&root, &task.id).map_err(|e| e.to_string())?);
    }
    Ok(modes)
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

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalImagePreview {
    data_url: String,
    mime: String,
    name: String,
}

/// Native Explorer drops carry paths, not web File objects. Read only bounded
/// supported image formats, never copy non-image documents into TaskBoard.
#[tauri::command]
fn read_local_image(path: String) -> Result<LocalImagePreview, String> {
    let file = PathBuf::from(&path);
    if !file.is_file() { return Err("Image file is unavailable".into()); }
    let ext = file.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp", "gif" => "image/gif",
        _ => return Err("Not a supported image file".into()),
    };
    if fs::metadata(&file).map_err(|e|e.to_string())?.len() > 20*1024*1024 {
        return Err("Image exceeds 20 MB limit".into());
    }
    let bytes = fs::read(&file).map_err(|e|e.to_string())?;
    if bytes.is_empty() { return Err("Image file is empty".into()); }
    Ok(LocalImagePreview {
        data_url: format!("data:{mime};base64,{}", BASE64.encode(bytes)),
        mime: mime.into(),
        name: file.file_name().unwrap_or_default().to_string_lossy().into_owned(),
    })
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
fn review_close(path: Option<String>, request: ReviewClose) -> Result<ReviewManifest, String> {
    FsTaskStore::default().review_close(&root(path), request).map_err(|e| e.to_string())
}

#[tauri::command]
fn review_resolve(path: Option<String>, resolution: ReviewResolution) -> Result<ReviewManifest, String> {
    FsTaskStore::default().review_resolve(&root(path), resolution).map_err(|e| e.to_string())
}

/// Start the bounded, loopback-only byte-range media server on first playback.
/// The separate authenticated OmniHub Gateway can proxy the same service to phones.
#[tauri::command]
async fn ensure_review_media_server() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(|| {
        use std::io::{Read, Write};
        use std::net::{SocketAddr, TcpStream};
        use std::time::Duration;
        fn healthy() -> bool {
            let addr: SocketAddr = "127.0.0.1:8785".parse().unwrap();
            let Ok(mut connection) = TcpStream::connect_timeout(&addr, Duration::from_millis(200)) else {
                return false;
            };
            let _ = connection.set_read_timeout(Some(Duration::from_millis(300)));
            let _ = connection.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n");
            let mut response = Vec::new();
            let _ = connection.read_to_end(&mut response);
            String::from_utf8_lossy(&response).contains("taskboard-review-media-ok")
        }
        if !healthy() {
            let script = PathBuf::from(r"D:\SSDProjects\Tools\TaskBoard\scripts\review_media_server.py");
            if !script.is_file() {
                return Err("TaskBoard media server script is not installed".to_string());
            }
            let mut cmd = Command::new("py");
            cmd.args(["-B", script.to_str().unwrap_or_default(), "--host", "127.0.0.1", "--port", "8785"])
                .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(windows)]
            cmd.creation_flags(0x08000000);
            cmd.spawn().map_err(|error| format!("Cannot start TaskBoard media server: {error}"))?;
            for _ in 0..15 {
                if healthy() { break; }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        if healthy() { Ok("http://127.0.0.1:8785".to_string()) }
        else { Err("TaskBoard media server unavailable (port 8785 occupied or startup failed)".into()) }
    }).await.map_err(|error| format!("Media server startup interrupted: {error}"))?
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

/// Launch Fomni through the Windows shell. Command::spawn returned Win32
/// ERROR_NOT_SUPPORTED (50) in TaskBoard despite Fomni being executable.
/// ShellExecuteW succeeds from the desktop process and forwards CLI arguments
/// to Fomni's existing instance. Fomni's --select mode highlights
/// the file within its explorer rather than opening a file tab.
#[cfg(windows)]
fn launch_fomni(path: &std::path::Path, select: bool) -> Result<(), String> {
    use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: isize, operation: *const u16, file: *const u16,
            parameters: *const u16, directory: *const u16, show: i32,
        ) -> isize;
    }
    if !std::path::Path::new(FOMNI_EXE).is_file() {
        return Err(format!("Fomni executable is missing: {FOMNI_EXE}"));
    }
    let exe: Vec<u16> = OsStr::new(FOMNI_EXE).encode_wide().chain([0]).collect();
    let path = path.to_string_lossy();
    let parameters = if select {
        format!("--select \"{path}\"")
    } else {
        format!("\"{path}\"")
    };
    let args: Vec<u16> = OsStr::new(&parameters).encode_wide().chain([0]).collect();
    let code = unsafe { ShellExecuteW(
        0, std::ptr::null(), exe.as_ptr(), args.as_ptr(), std::ptr::null(), 1,
    ) };
    if code <= 32 {
        Err(format!("Windows shell could not launch Fomni (ShellExecuteW code {code})"))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn launch_fomni(_path: &std::path::Path, _select: bool) -> Result<(), String> {
    Err("Fomni's Windows explorer integration is unavailable on this platform".into())
}

/// Fomni expects a normal drive or UNC path. fs::canonicalize on Windows
/// returns extended-length \\?\ paths, which Fomni incorrectly treats as
/// separate "/ ? / B:" breadcrumbs instead of a drive letter.
fn fomni_target(file: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(file);
    if !path.is_absolute() {
        return Err(format!("Fomni needs an absolute file path: {file}"));
    }
    if !path.exists() {
        return Err(format!("Fomni file does not exist: {file}"));
    }
    Ok(path)
}

#[tauri::command]
fn highlight_in_fomni(file: String) -> Result<(), String> {
    launch_fomni(&fomni_target(&file)?, true)
}

/// Open a real local Inbox artifact as a Fomni tab, rather than using the OS
/// default application or revealing just its parent folder.
#[tauri::command]
fn open_file_in_fomni(file: String) -> Result<(), String> {
    launch_fomni(&fomni_target(&file)?, false)
}

/// Read selected published Dashboard captures directly from the durable
/// evidence index, without hosting/embedding the entire Wartribes Dashboard.
/// User-supplied IDs never become filesystem paths.
#[tauri::command]
async fn dashboard_review_items(ids: Vec<String>, task: Option<String>)
    -> Result<Vec<serde_json::Value>, String>
{
    tauri::async_runtime::spawn_blocking(move || {
        fn valid_capture_id(value: &str) -> bool {
            !value.is_empty() && value.len() <= 150 &&
                value.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        }
        if ids.len() > 8 || ids.iter().any(|id| !valid_capture_id(id)) {
            return Err("Choose at most eight valid Dashboard capture IDs".into());
        }
        if let Some(task_id) = task.as_deref() {
            if !task_id.starts_with("WT-") ||
                task_id.len() > 20 ||
                !task_id[3..].bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("Invalid Wartribes task ID".into());
            }
        }
        if ids.is_empty() && task.is_none() {
            return Err("A Dashboard review needs ?item=, ?items= or ?task=".into());
        }
        let dir = PathBuf::from(r"D:\Wartribes\Saved\DevelopmentDashboard");
        let raw = fs::read(dir.join("index.json")).map_err(|e|
            format!("Dashboard evidence manifest is unavailable: {e}"))?;
        let manifest: serde_json::Value = serde_json::from_slice(&raw)
            .map_err(|e| format!("Dashboard evidence manifest is invalid: {e}"))?;
        let all = manifest.get("items").and_then(|v| v.as_array())
            .ok_or_else(|| "Dashboard evidence manifest has no items".to_string())?;

        let mut selected: Vec<serde_json::Value> = Vec::new();
        if ids.is_empty() {
            // Latest eight visible captures, in chronological order.
            let task_id = task.as_deref().unwrap_or_default();
            selected = all.iter().filter(|item|
                item.get("task").and_then(|v| v.as_str()) == Some(task_id) &&
                item.get("hidden").and_then(|v| v.as_bool()) != Some(true))
                .rev().take(8).cloned().collect();
            selected.reverse();
        } else {
            let mut seen = std::collections::HashSet::new();
            for id in &ids {
                if !seen.insert(id.as_str()) {
                    return Err(format!("Duplicate Dashboard capture ID: {id}"));
                }
                let item = all.iter().find(|item|
                    item.get("id").and_then(|v| v.as_str()) == Some(id.as_str()))
                    .ok_or_else(|| format!("No published Dashboard capture has ID {id}"))?;
                if item.get("hidden").and_then(|v| v.as_bool()) == Some(true) {
                    return Err(format!("Dashboard capture {id} is hidden"));
                }
                selected.push(item.clone());
            }
        }
        if selected.is_empty() {
            return Err("This Dashboard task has no published visible captures".into());
        }
        let owner = selected[0].get("task").and_then(|v| v.as_str())
            .ok_or_else(|| "Dashboard capture has no task ID".to_string())?.to_owned();
        if selected.iter().any(|item| item.get("task").and_then(|v| v.as_str()) != Some(owner.as_str()))
            || task.as_deref().is_some_and(|id| id != owner) {
            return Err("A review gallery must contain captures from one exact Wartribes task".into());
        }

        let mut total_bytes = 0_u64;
        for item in &mut selected {
            let file = item.get("filename").and_then(|v| v.as_str())
                .ok_or_else(|| "Dashboard capture is missing its media filename".to_string())?.to_owned();
            let (hash, ext) = file.rsplit_once('.')
                .ok_or_else(|| "Invalid Dashboard media filename".to_string())?;
            if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Err("Dashboard media is not a content-addressed filename".into());
            }
            let mime = match ext {
                "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp", "gif" => "image/gif",
                "mp4" => "video/mp4", "webm" => "video/webm",
                _ => return Err("Unsupported Dashboard media format".into()),
            };
            let path = dir.join("media").join(&file);
            let size = fs::metadata(&path).map_err(|e|
                format!("Dashboard capture media is unavailable: {e}"))?.len();
            item["mediaPath"] = serde_json::Value::String(path.to_string_lossy().into_owned());
            // Videos stream through HTTP Range: never base64-encode full clips
            // into Tauri IPC. This also enables seeking past the old 12 MiB cap.
            if mime.starts_with("video/") {
                item["mediaStreamUrl"] = serde_json::Value::String(
                    format!("http://127.0.0.1:8785/dashboard-media/{file}"));
                if let Some(poster) = item.get("posterFilename").and_then(|v| v.as_str()) {
                    let safe_poster = poster.len() < 80 &&
                        poster.split_once('.').is_some_and(|(hash, ext)| hash.len() == 64 &&
                            hash.bytes().all(|c| c.is_ascii_hexdigit()) &&
                            matches!(ext, "png" | "webp" | "jpg"));
                    if safe_poster {
                        let file = dir.join("media").join(poster);
                        if file.metadata().is_ok_and(|meta| meta.len() <= 2 * 1024 * 1024) {
                            if let Ok(bytes) = fs::read(file) {
                                item["posterDataUrl"] = serde_json::Value::String(
                                    format!("data:image/webp;base64,{}", BASE64.encode(bytes)));
                            }
                        }
                    }
                }
                continue;
            }
            // Keep image IPC payload bounded; overlarge originals remain
            // accessible through the existing file affordance.
            if size > 12 * 1024 * 1024 || total_bytes.saturating_add(size) > 32 * 1024 * 1024 {
                item["mediaUnavailable"] = serde_json::Value::String(
                    "Capture exceeds the inline preview limit; open the original.".into());
                continue;
            }
            let bytes = fs::read(&path)
                .map_err(|e| format!("Could not read published Dashboard media: {e}"))?;
            item["mediaDataUrl"] = serde_json::Value::String(format!(
                "data:{mime};base64,{}", BASE64.encode(bytes)));
            total_bytes += size;
        }
        Ok(selected)
    }).await.map_err(|e| format!("Dashboard evidence read interrupted: {e}"))?
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
    // T3's "open" interprets a UUID as a *conversation ID* and may
    // create a new session. Resolve against existing sessions first; link clicks
    // must never silently create an extra ChatGPT session.
    let call = |request: serde_json::Value| -> Result<serde_json::Value, String> {
        let mut process = Command::new("node").arg(script)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .creation_flags(0x08000000).spawn()
            .map_err(|e| format!("Cannot connect to T3 CLI: {e}"))?;
        if let Some(mut stdin) = process.stdin.take() {
            stdin.write_all(request.to_string().as_bytes()).map_err(|e| e.to_string())?;
        }
        let output = process.wait_with_output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
        }
        serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("T3 returned invalid JSON: {e}"))
    };
    let inventory = call(serde_json::json!({
        "action": "list", "which": "all", "includeHidden": true
    }))?;
    let session = inventory.get("sessions").and_then(|v| v.as_array())
        .and_then(|sessions| sessions.iter().find(|entry|
            entry.get("id").and_then(|v| v.as_str()) == Some(claimant) ||
            entry.get("conversationId").and_then(|v| v.as_str()) == Some(claimant)))
        .and_then(|entry| entry.get("id").and_then(|v| v.as_str()))
        .ok_or("That T3 session is no longer present; no new session was created.")?;
    let result = call(serde_json::json!({ "action": "focus", "sessionId": session }))?;
    if result.get("action").and_then(|v| v.as_str()) != Some("focus") ||
        result.get("sessionId").and_then(|v| v.as_str()) != Some(session) ||
        result.get("acknowledged").and_then(|v| v.as_bool()) != Some(true) {
        return Err("T3 did not confirm opening the requested session.".into());
    }
    Ok(result)
}


/// Open the latest known originating T3 browser session without creating a worker.
/// Session lineage survives release/reclaim in the task Markdown, even if T3 is
/// currently hidden or unloaded. Unknown availability does not block new claims.
#[tauri::command]
fn task_open_origin_session(path:String,id:String)->Result<serde_json::Value,String>{
    let root=PathBuf::from(&path);
    let board=FsTaskStore::default().load(&root).map_err(|e|e.to_string())?;
    let task=board.tasks.into_iter().find(|t|t.id==id && !t.archived)
      .ok_or("Task no longer exists")?;
    let handoff=task_core::handoff_bundle(&task);
    let candidate=handoff.origin_sessions.last().or_else(||
       task.claimed_by.as_ref().filter(|owner|owner.len()==36));
    let Some(id)=candidate else{return Err("No originating T3 session is linked to this task".into())};
    open_claimed_t3_session(id.clone())
}

#[tauri::command]
fn task_handoff_bundle(path:String,id:String)->Result<task_core::HandoffPacket,String>{
    let root=PathBuf::from(&path);
    let board=FsTaskStore::default().load(&root).map_err(|e|e.to_string())?;
    let task=board.tasks.into_iter().find(|t|t.id==id && !t.archived)
      .ok_or("Task no longer exists")?;
    Ok(task_core::handoff_bundle(&task))
}

/// Open the canonical task's existing session, or ask T3's open-task action to
/// create one. The new-session path requires T3-0142; never synthesize a
/// duplicate session or silently treat a transport error as a failed start.
// T3's CLI may wait for browser/session startup and prompt delivery. Never do
// this blocking work on Tauri's command/UI thread.
#[tauri::command]
async fn open_task_session(path: Option<String>, id: String, implement: Option<bool>) -> Result<serde_json::Value, String> {
    tauri::async_runtime::spawn_blocking(move || open_task_session_blocking(path, id, implement))
        .await.map_err(|e| format!("T3 session operation failed: {e}"))?
}

fn open_task_session_blocking(path: Option<String>, id: String, implement: Option<bool>) -> Result<serde_json::Value, String> {
    let path = root(path);
    let task = FsTaskStore::default().load(&path).map_err(|e| e.to_string())?
        .tasks.into_iter().find(|task| task.id == id && !task.archived)
        .ok_or_else(|| format!("Task not found: {id}"))?;
    if !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) {
        return Err("Only executable tasks can start a session".into());
    }
    let mut implement = implement.unwrap_or(false);
    let claimant = task.claimed_by.as_deref().unwrap_or("");
    // An in-flight queue start already owns this task. Treat Start now as
    // "open its existing session", never as permission for a second spawn.
    let t3_claimed = claimant.starts_with("t3:") || claimant.starts_with("t3-browser:");
    if implement && t3_claimed { implement = false; }
    // Every task-bound open, including a legacy browser claim, must use the
    // same T3 deduplication, live-state transition, and exact-session focus.
    let script = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
    if !std::path::Path::new(script).is_file() {
        return Err("T3 open-task CLI is unavailable".into());
    }
    if implement && !claimant.is_empty() {
        return Err("Task is claimed by another agent; cannot start a duplicate T3 worker.".into());
    }
    let store = FsTaskStore::default();
    if implement {
        // The board lock decides the race with T3's queue dispatcher. If the
        // queue won, preserve its reservation and return a pending receipt;
        // the UI will observe/focus its eventual session instead of retrying a spawn.
        if let Err(error) = store.prepare_manual_start(&path, &id) {
            let execution = store.execution_intent(&path, &id).map_err(|e| e.to_string())?;
            if execution.mode != task_core::ExecutionMode::Queue ||
                !matches!(execution.queue_state, Some(task_core::QueueState::Starting | task_core::QueueState::Working)) {
                return Err(error.to_string());
            }
            let current = store.load(&path).map_err(|e| e.to_string())?.tasks.into_iter()
                .find(|task| task.id == id && !task.archived)
                .ok_or_else(|| format!("Task not found: {id}"))?;
            let owner = current.claimed_by.as_deref().unwrap_or("");
            if owner.starts_with("t3:") || owner.starts_with("t3-browser:") {
                implement = false;
            } else if owner.is_empty() {
                return Ok(serde_json::json!({
                    "status": "in_progress", "created": false,
                    "taskRef": { "board": path.display().to_string(), "id": id },
                    "queueState": "starting",
                    "message": "T3 queue already owns this start; waiting for its existing session."
                }));
            } else {
                return Err(format!("{id} is claimed by another agent; cannot start a duplicate."));
            }
        }
    }
    if !implement {
        // Opening a task before T3 has attached its claim must not create a
        // second manual guest while the queued first prompt is in flight.
        let execution = store.execution_intent(&path, &id).map_err(|e| e.to_string())?;
        if execution.mode == task_core::ExecutionMode::Queue &&
            matches!(execution.queue_state, Some(task_core::QueueState::Starting | task_core::QueueState::Working)) {
            let current = store.load(&path).map_err(|e| e.to_string())?.tasks.into_iter()
                .find(|task| task.id == id && !task.archived)
                .ok_or_else(|| format!("Task not found: {id}"))?;
            if current.claimed_by.is_none() {
                return Ok(serde_json::json!({
                    "status": "in_progress", "created": false,
                    "taskRef": { "board": path.display().to_string(), "id": id },
                    "queueState": "starting",
                    "message": "T3 queue is already starting this task."
                }));
            }
        }
    }
    let request = serde_json::json!({
        "action": "open-task",
        "taskRef": { "board": path.display().to_string(), "id": id },
        "intent": if implement { "implement" } else { "conversation" },
        "kind": if implement { Some("chatgpt") } else { None },
    });
    let t3_home = std::env::var_os("T3CODE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".t3")))
        .ok_or("T3 home is not configured; cannot resolve the active app-control pipe")?;
    let mut process = Command::new("node").arg(script)
        .env("T3CODE_HOME", t3_home)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .creation_flags(0x08000000).spawn().map_err(|e| format!("Cannot launch T3 CLI: {e}"))?;
    if let Some(mut stdin) = process.stdin.take() {
        stdin.write_all(request.to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = process.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        eprintln!("TaskBoard T3 open-task failed for {id}: {}", diagnostic.trim());
        if diagnostic.contains("error[invalid_board_task_data]") {
            if diagnostic.contains("unknown variant") {
                return Err("TaskBoard saved the task, but T3's task launcher does not recognize a task flag. The launcher needs an update to match TaskBoard. Check T3 Sessions before starting the task again.".into());
            }
            return Err("T3 cannot read this board because some task data is invalid. The task was saved. Check the affected board data and T3 Sessions before retrying.".into());
        }
        return Err("T3 did not confirm whether the task session started. Check T3 Sessions for an existing session before trying again.".into());
    }
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("T3 open-task receipt ambiguous; reconcile before retry: {e}"))?;
    match receipt.get("status").and_then(|v| v.as_str()) {
        // T3's task journal guarantees that these pending states can be
        // observed again without repeating the original first prompt.
        Some("opened" | "created" | "prompt_unknown" | "in_progress" | "unknown") => {},
        _ => return Err("T3 did not confirm open-task; reconcile before retrying".into()),
    }
    Ok(receipt)
}

#[tauri::command]
fn show_main_window(window: tauri::WebviewWindow) -> Result<(), String> {
    // A hotkey launch only shows the separate always-on-top editor. The hidden
    // main board can still be opened later through a normal TaskBoard launch.
    if window.label() != "main" || startup_create_task() { return Ok(()); }
    // Both the prompt shell and the populated board may request visibility.
    // Avoid pulling focus away from another app when the slower read finishes.
    if window.is_visible().map_err(|e| e.to_string())? { return Ok(()); }
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

/// Accept only a board directory containing a TaskBoard manifest and a short
/// opaque task identifier. The UI additionally matches the board to its
/// registered project catalog before navigating.
#[tauri::command]
fn startup_task() -> Option<serde_json::Value> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 || args[1] != "--open-task" {
        return None;
    }
    let path = &args[2];
    let id = &args[3];
    if !PathBuf::from(path).join("_board.md").is_file()
        || id.is_empty() || id.len() > 90 || id.contains(['/', '\\'])
    {
        return None;
    }
    Some(serde_json::json!({ "path": path, "taskId": id }))
}

/// A fresh --create-task process handles the request after React has mounted;
/// subsequent invocations are forwarded by the single-instance plugin.
#[tauri::command]
fn startup_create_task() -> bool {
    std::env::args().nth(1).as_deref() == Some("--create-task")
}

#[tauri::command]
fn show_quick_create(window: tauri::WebviewWindow) -> Result<(), String> {
    quick_create::show(&window).map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default()
            // Quick-create is a transient overlay, never a user-resizable saved window.
            // Restoring its old bounds races with the placement on each hotkey press.
            .with_denylist(&["quick-create"])
            .with_state_flags(window_placement::state_flags())
            .build())
        .on_window_event(window_placement::on_window_event)
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            use tauri::Manager;
            if args.len() == 2 && args[1] == "--create-task" {
                // The Windows single-instance callback runs in the main window
                // procedure; WebView creation here can deadlock that UI thread.
                quick_create::request_open(app);
                return;
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
            // Forward normal deep links to the existing main board.
            if args.len() == 4 && args[1] == "--open-review" {
                use tauri::Emitter;
                let _ = app.emit("taskboard-open-review", serde_json::json!({ "path": args[2], "taskId": args[3] }));
            }
            if args.len() == 4 && args[1] == "--open-task"
                && PathBuf::from(&args[2]).join("_board.md").is_file()
                && !args[3].is_empty() && args[3].len() <= 90
                && !args[3].contains(['/', '\\'])
            {
                use tauri::Emitter;
                let _ = app.emit("taskboard-open-task",
                    serde_json::json!({ "path": args[2], "taskId": args[3] }));
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            window_placement::start(app);
            if startup_create_task() {
                // Startup setup is also on the event-loop thread. Defer WebView
                // construction so both first-launch and repeat hotkeys are safe.
                quick_create::request_open(app.handle());
            }
            busy_recovery::start();
            std::thread::spawn(|| {
                // Defer cloud-drive maintenance until the first board is visible.
                std::thread::sleep(std::time::Duration::from_secs(90));
                loop {
                    sweep_registered_review_boards();
                    std::thread::sleep(std::time::Duration::from_secs(6 * 60 * 60));
                }
            });
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
                window.set_icon(icon)?;
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            error_log::record_ui_error,
            restart_warning::restart_warning_status,
            restart_warning::postpone_restart_warning,
            restart_warning::confirm_restart_ready,
            startup_review,
            startup_task,
            startup_create_task,
            show_quick_create,
            projects::list_projects,
            projects::add_project,
            projects::set_project_folders,
            projects::activate_project,
            projects::reorder_projects,
            projects::remove_project,
            projects::unarchive_project,
            projects::restore_project,
            projects::move_task_between_projects,
            default_board_path,
            load_board,
            load_quick_board,
            model_favorites::task_model_favorites,
            claim_health::task_claim_health,
            agent_notices::task_enqueue_agent_notice,
            agent_notices::task_reconcile_agent_notices,
            stale_blockers::task_stale_blockers,
            stale_blockers::task_acknowledge_stale_blocker,
            agent_policy::task_read_agent_policy,
            agent_policy::task_save_agent_policy,
            board_watch::board_change_tokens,
            user_queue_project_ids,
            create_task,
            task_execution_intent,
            task_execution_modes,
            task_sync_queue,
            task_queue_overview,
            task_queue_capacity,
            task_queue_workload,
            task_busy_recovery_status,
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
            read_local_image,
            read_task_image,
            review_show,
            review_request,
            review_reconsider,
            review_close,
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
            open_file_in_fomni,
            dashboard_review_items,
            ensure_review_media_server,
            open_in_obsidian,
            open_claimed_t3_session,
            task_open_origin_session,
            task_handoff_bundle,
            open_task_session,
            task_agent::task_agent_status,
            task_agent::task_agent_action,
            preparation::start_task_preparation,
            preparation::notify_task_preparation_saved,
            preparation::start_task_prepfile,
            show_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running TaskBoard");
}
