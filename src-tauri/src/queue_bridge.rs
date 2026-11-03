//! OW-0014: explicit TaskBoard Queue -> the ONE OpenRig queue. No scheduler or
//! background polling. Durable pre-send CAS precedes network; unknown writes are
//! reconciled by exact deterministic qitem ID and NEVER resent.
use std::{path::{Path, PathBuf}, time::Duration};
use serde::Serialize;
use serde_json::{json, Value};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use task_core::{FsTaskStore, TaskStore, ExecutionMode, TaskKind, Task, queue_task_ready};
use reqwest::{Client, StatusCode};

const DAEMON: &str = "http://127.0.0.1:7433";
const SOURCE: &str = "taskboard@omni-command";
const DESTINATION: &str = "queue@omni-command";

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct QueueSyncStatus {
    pub status: &'static str,
    pub qitem_id: Option<String>,
    pub reason: Option<String>,
}
fn status(status: &'static str, qid: Option<String>, reason: Option<String>) -> QueueSyncStatus {
    QueueSyncStatus { status, qitem_id: qid, reason }
}
fn token() -> Result<String, String> {
    let root = std::env::var_os("OPENRIG_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".openrig")))
        .ok_or("OpenRig credential directory unavailable")?;
    let token = std::fs::read_to_string(root.join("terminal-token"))
        .map_err(|_| "OpenRig terminal credential unavailable".to_string())?;
    if token.trim().is_empty() { return Err("OpenRig credential empty".into()); }
    Ok(token.trim().into())
}
fn client() -> Result<Client, String> {
    Client::builder().timeout(Duration::from_secs(8)).build().map_err(|e| e.to_string())
}
fn ref_json(path: &Path, id: &str) -> Value {
    json!({
      "environmentId": "pc",
      "boardRoot": path.to_string_lossy().trim_end_matches(['\\','/']).replace('/', "\\"),
      "taskId": id,
    })
}
fn ref_tag(reference: &Value) -> String {
    let fields = [
        reference["environmentId"].as_str().unwrap_or(""),
        reference["boardRoot"].as_str().unwrap_or(""),
        reference["taskId"].as_str().unwrap_or(""),
    ];
    format!("taskboard-ref:v1:{}", URL_SAFE_NO_PAD.encode(serde_json::to_vec(&fields).unwrap_or_default()))
}
fn registration_tag(request: &str, source_revision: &str) -> String {
    format!("taskboard-queue:v1:{}", URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&[request, source_revision]).unwrap_or_default()))
}
fn body(task: &Task, reference: &Value) -> String {
    let mut message = format!("Work on {} — {}. Read the exact TaskBoard card at {}/{} ({}), its latest acceptance criteria and project instructions. This queue receipt is NOT a completion claim.",
        task.id, task.title, reference["boardRoot"].as_str().unwrap_or(""),
        task.id, reference["environmentId"].as_str().unwrap_or("pc"));
    if task.live.as_ref().is_some_and(|live| live.with_me) {
        message.push_str("\n\nWith me: prepare everything that does not need the human first. When live back-and-forth is genuinely needed, write a three-line brief (Done / Need from you / Minutes), run task live ready <ID> --brief \"Done: ... / Need from you: ... / Minutes: ...\" --est <minutes> on this owning board, then end your turn. Do NOT send an OpenRig Inbox, aispeak or feedback request merely to announce readiness. Quick standalone questions still use the existing TaskBoard question mechanism.");
    }
    message
}
async fn get(client: &Client, bearer: &str, route: &str) -> Result<(StatusCode, Value), String> {
    let response = client.get(format!("{DAEMON}{route}")).bearer_auth(bearer).send().await
        .map_err(|_| "OpenRig unavailable (read; no write attempted)".to_owned())?;
    let code = response.status();
    let value = response.json::<Value>().await
        .map_err(|_| "OpenRig returned invalid read JSON".to_owned())?;
    Ok((code, value))
}
async fn post(client: &Client, bearer: &str, route: &str, payload: Value) -> Result<(StatusCode, Value), String> {
    let response = client.post(format!("{DAEMON}{route}")).bearer_auth(bearer)
        .json(&payload).send().await
        .map_err(|_| "OpenRig write outcome UNKNOWN; reconcile exact qitem, do not resend".to_owned())?;
    let code = response.status();
    let value = response.json::<Value>().await
        .map_err(|_| "OpenRig write response unreadable: outcome UNKNOWN".to_owned())?;
    Ok((code, value))
}
fn row_matches(row: &Value, reference: &Value, request: &str, source_revision: Option<&str>, task: &Task) -> bool {
    let tags = row["tags"].as_array();
    let linked = tags.is_some_and(|tags| tags.iter().any(|v| v.as_str() == Some(ref_tag(reference).as_str())));
    let request_marker = tags.is_some_and(|tags| tags.iter().filter_map(Value::as_str).any(|t|
        if let Some(revision) = source_revision { t == registration_tag(request, revision) }
        else { t.starts_with("taskboard-queue:v1:") }
    ));
    linked && request_marker &&
        row["qitemId"].as_str() == Some(format!("qitem-taskboard-{request}").as_str()) &&
        row["sourceSession"].as_str() == Some(SOURCE) &&
        row["destinationSession"].as_str() == Some(DESTINATION) &&
        row["body"].as_str().is_some_and(|s| s.contains(&format!("{}/{}",reference["boardRoot"].as_str().unwrap_or(""),task.id)))
}
fn terminal_state(row: &Value) -> bool {
    matches!(row["state"].as_str(), Some("done"|"canceled"|"failed"|"denied"|"handed-off"))
}
pub(crate) async fn sync(path: PathBuf, id: String) -> Result<QueueSyncStatus, String> {
    let store = FsTaskStore::default();
    let snapshot = store.load(&path).map_err(|e| e.to_string())?;
    let task = snapshot.tasks.iter().find(|task| task.id == id).cloned()
        .ok_or_else(|| format!("{id} no longer exists in this board"))?;
    let execution = store.execution_intent(&path, &id).map_err(|e| e.to_string())?;
    // Archived tasks are no longer dispatchable even when archived directly
    // from a nonterminal column. A queued archived task must cancel its row.
    let finished = task.archived || task.completed.is_some() || snapshot.config.columns.iter().any(|col| col.terminal && col.id == task.column);
    let dispatchable = queue_task_ready(&snapshot, &task);
    let reference = ref_json(&path, &id);
    let request = execution.queue_request.as_deref();
    let qid = request.map(|request| format!("qitem-taskboard-{request}"));
    // T3 now owns NEW queued work. Never register an unattempted intent with
    // OpenRig: that would allow two independent schedulers to start the task.
    // Existing OpenRig attempts/receipts retain their original owner until
    // terminal reconciliation, so an UNKNOWN registration is never duplicated.
    if execution.mode == ExecutionMode::Queue && dispatchable &&
        execution.queue_attempt.is_none() && execution.queue_receipt.is_none() {
        return Ok(status("queued", qid,
            Some("T3 local queue; starts only while Keep T3 Busy is enabled and capacity permits".into())));
    }
    if !matches!(task.kind, TaskKind::Task|TaskKind::Recurring) || finished || execution.mode == ExecutionMode::Backlog || (!dispatchable && execution.queue_attempt.is_some()) {
        if let (Some(qitem), Some(request), Some(attempt)) = (qid.as_deref(),request,execution.queue_attempt.as_deref()) {
            if attempt != request || execution.queue_receipt.as_deref().is_some_and(|receipt| receipt != qitem) {
                return Ok(status("conflict",qid,Some("Queue attempt or receipt identity mismatch".into())));
            }
            if execution.queue_retired_receipt.as_deref() == Some(qitem) {
                return Ok(status("canceled",qid,None));
            }
            let bearer = token()?;
            let client = client()?;
            let (code,row) = get(&client,&bearer,&format!("/api/queue/{qitem}")).await?;
            if code == StatusCode::NOT_FOUND {
                return Ok(status("unknown",qid,Some("Previous Queue attempt is unverified; do not requeue until reconciled".into())));
            }
            if !code.is_success() || !row_matches(&row,&reference,request,None,&task) {
                return Ok(status("conflict",qid,Some("Previous Queue record identity cannot be verified".into())));
            }
            if terminal_state(&row) {
                if let Err(error)=store.retire_queue(&path,&id,request,&execution.revision,qitem) {
                    return Ok(status("conflict",qid,Some(format!("Queue is terminal, but retirement CAS failed: {error}"))));
                }
                return Ok(status("canceled",qid,None));
            }
            if row["state"] != "pending" || row["claimedAt"].as_str().is_some_and(|value| !value.is_empty()) {
                return Ok(status("running",qid,Some("Already assigned/running: separate explicit stop required".into())));
            }
            let (response,result)=match post(&client,&bearer,&format!("/api/queue/taskboard/{qitem}/sync"),json!({
                "taskRef":reference,"action":"cancel","requestId":request,"actorSession":SOURCE,
            })).await {
                Ok(result) => result,
                Err(reason) => return Ok(status("unknown",qid,Some(format!("Queue cancellation UNKNOWN: {reason}")))),
            };
            if response.is_success() && result["state"]=="canceled" {
                if let Err(error)=store.retire_queue(&path,&id,request,&execution.revision,qitem) {
                    return Ok(status("conflict",qid,Some(format!("Cancellation committed but retirement CAS failed: {error}"))));
                }
                return Ok(status("canceled",qid,None));
            }
            return Ok(status("conflict",qid,Some(format!("Cancel refused: {}",result["error"].as_str().unwrap_or("unknown reason")))));
        }
        if execution.queue_receipt.is_some() {
            return Ok(status("conflict",qid,Some("Queue receipt exists without its registration attempt".into())));
        }
        return Ok(status("not_queued",qid,None));
    }
    if !dispatchable {
        return Ok(status("unavailable",qid,Some("Task not executable: dependencies, claim, blocker or column must be cleared before dispatch".into())));
    }
    let request = match request {
        Some(request) => request,
        None => return Ok(status("conflict",None,Some("Queue mode lacks durable request identity".into())))
    };
    let qitem = qid.clone().unwrap_or_default();
    let bearer = token()?;
    let client = client()?;
    if let Some(receipt) = execution.queue_receipt.as_deref() {
        if receipt != qitem {return Ok(status("conflict",qid,Some("TaskBoard receipt identity mismatch".into())));}
        let (code,row)=get(&client,&bearer,&format!("/api/queue/{qitem}")).await?;
        if !code.is_success() || !row_matches(&row,&reference,request,None,&task) {
            return Ok(status("conflict",qid,Some("OpenRig receipt could not be authenticated to task".into())));
        }
        if row["summary"].as_str() != Some(task.title.as_str()) && !terminal_state(&row) {
            if row["state"] != "pending" || row["claimedAt"].as_str().is_some_and(|s| !s.is_empty()) {
                return Ok(status("running",qid,Some("Queued task title changed after pickup; worker must re-read its authoritative TaskBoard card".into())));
            }
            let (code, updated) = match post(&client,&bearer,&format!("/api/queue/taskboard/{qitem}/sync"), json!({
                "taskRef":reference,"action":"refresh","requestId":request,
                "actorSession":SOURCE,"summary":task.title,
            })).await {
                Ok(result) => result,
                Err(reason) => return Ok(status("unknown",qid,Some(reason))),
            };
            if !code.is_success() || updated["summary"].as_str() != Some(task.title.as_str()) {
                return Ok(status("conflict",qid,Some(format!("Queue title refresh refused: {}",updated["error"].as_str().unwrap_or("unknown reason")))));
            }
        }
        return Ok(status(if terminal_state(&row) {"terminal"} else {"registered"},qid,None));
    }
    if execution.queue_attempt.is_some() {
        // A persisted pre-send marker makes a lost response UNKNOWN, not a retry.
        let (code,row)=get(&client,&bearer,&format!("/api/queue/{qitem}")).await?;
        if code == StatusCode::NOT_FOUND {
            return Ok(status("unknown",qid,Some("Pre-send attempt exists but exact qitem absent; manual reconciliation required".into())));
        }
        if !code.is_success() || !row_matches(&row,&reference,request,None,&task) {
            return Ok(status("conflict",qid,Some("Attempted qitem differs from recorded TaskBoard intent".into())));
        }
        let accepted = store.receipt_queue(&path,&id,request,&execution.revision,&qitem);
        if accepted.is_err() {return Ok(status("conflict",qid,Some("OpenRig registered but TaskBoard card was edited; verify before receipt".into())));}
        return Ok(status("registered",qid,None));
    }
    let (ready_code,ready)=get(&client,&bearer,"/api/queue/taskboard/registration-readiness").await?;
    if !ready_code.is_success() || ready["ready"]!=true {
        return Ok(status("unavailable",qid,Some(ready["reason"].as_str().unwrap_or("Queue registration not available").into())));
    }
    // CAS before write. This exact request is never resent after UNKNOWN.
    let attempted = store.begin_queue_registration(&path,&id,request,&execution.revision)
        .map_err(|e| format!("Queue intent changed before registration: {e}"))?;
    let payload = json!({
        "qitemId":qitem,"sourceSession":SOURCE,"destinationSession":DESTINATION,
        "body":body(&task,&reference),"summary":task.title,
        "taskRef":reference,"nudge":false,
        "taskboardQueueIntent":{"executionMode":"queue","requestId":request,"sourceRevision":attempted.revision},
    });
    let (code,reply) = match post(&client,&bearer,"/api/queue/create",payload).await {
        Ok(result) => result,
        Err(message) => return Ok(status("unknown",qid,Some(message))),
    };
    if !code.is_success() {
        return Ok(status("rejected",qid,Some(format!("OpenRig rejected registration (HTTP {}): {}",code,reply["error"].as_str().unwrap_or("unknown")))));
    }
    let receipt=&reply["registrationReceipt"];
    if receipt["accepted"]!=true ||
        receipt["qitemId"]!=qitem || receipt["requestId"]!=request ||
        receipt["sourceRevision"]!=attempted.revision || receipt["taskRef"]!=reference {
        return Ok(status("unknown",qid,Some("OpenRig response does not certify exact registered task".into())));
    }
    if let Err(error)=store.receipt_queue(&path,&id,request,&attempted.revision,&qitem) {
        return Ok(status("conflict",qid,Some(format!("Registered in OpenRig but TaskBoard CAS conflict: {error}"))));
    }
    Ok(status("registered",qid,None))
}

/** T3 is the dispatcher; show the authoritative TaskBoard queued intents
 * rather than an unrelated OpenRig outbox. Legacy registrations remain labeled. */
pub(crate) async fn overview(path: PathBuf) -> Result<Value, String> {
    let store = FsTaskStore::default();
    let board = store.load(&path).map_err(|e| e.to_string())?;
    let root = path.to_string_lossy().trim_end_matches(['\\', '/']).replace('/', "\\");
    let mut items = Vec::new();
    for task in board.tasks.iter().filter(|task| !task.archived &&
        task.completed.is_none() && task.column == task_core::AI_READY_COLUMN_ID &&
        matches!(task.kind, TaskKind::Task | TaskKind::Recurring)) {
        let intent = store.execution_intent(&path, &task.id).map_err(|e| e.to_string())?;
        if intent.mode != ExecutionMode::Queue {continue;}
        let Some(request) = intent.queue_request else {continue;};
        let legacy = intent.queue_attempt.is_some() || intent.queue_receipt.is_some();
        let started = task.claimed_by.as_ref().map(|claim| claim.starts_with("t3:") ||
            claim.starts_with("t3-browser:")).unwrap_or(false);
        let session = task.claimed_by.as_ref().and_then(|claim| {
            claim.strip_prefix("t3-browser:").or_else(|| claim.strip_prefix("t3:"))
        });
        items.push(json!({
            "qitemId": format!("qitem-taskboard-{request}"),
            "taskId":task.id,"summary":task.title,
            "state":if started {"running"}else{"pending"},
            "priority":if task.attention.contains(&task_core::TaskAttention::Priority){"urgent"}else{"routine"},
            "createdAt":task.created,"updatedAt":chrono::Utc::now().to_rfc3339(),
            "claimedAt":null,"pickup":null,"waiting":null,
            "startupState":if legacy {Some("legacy-openrig")} else {None},
            "t3SessionId":session
        }));
    }
    items.sort_by(|a,b| {
        let pa = a["priority"] == "urgent";
        let pb = b["priority"] == "urgent";
        pb.cmp(&pa).then_with(|| a["createdAt"].as_str().cmp(&b["createdAt"].as_str()))
    });
    let active_count = items.len();
    Ok(json!({"schema":"t3-taskboard-queue-overview/v1",
        "observedAt":chrono::Utc::now().to_rfc3339(),
        "environmentId":"pc","boardRoot":root,"activeCount":active_count,
        "limit":active_count,"hasMore":false,"items":items}))
}

/** T3's one authority counts all manually and automatically working agents.
 * No OpenRig credentials, daemon readiness or independent capacity policy. */
pub(crate) async fn capacity() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(|| {
        use std::process::{Command, Stdio};
        use std::io::Write;
        let script = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
        if !std::path::Path::new(script).is_file() {
            return Err("T3 capacity CLI unavailable".to_owned());
        }
        let mut cmd = Command::new("node.exe");
        cmd.arg(script).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        #[cfg(windows)] {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000);
        }
        let mut child=cmd.spawn().map_err(|e|format!("Cannot query T3 capacity: {e}"))?;
        if let Some(mut input)=child.stdin.take() {
            input.write_all(br#"{"action":"caffy","op":"capacity"}"#)
                .map_err(|e|format!("T3 capacity request failed: {e}"))?;
        }
        let output=child.wait_with_output()
            .map_err(|e|format!("T3 capacity receipt unavailable: {e}"))?;
        if !output.status.success() {
            return Err(format!("T3 capacity unavailable: {}",
                String::from_utf8_lossy(&output.stderr).chars().take(240).collect::<String>()));
        }
        let value:Value=serde_json::from_slice(&output.stdout)
            .map_err(|e|format!("T3 capacity response invalid: {e}"))?;
        if !value["working"].is_number() || !value["limit"].is_number() ||
            !value["admissible"].is_boolean() {
            return Err("T3 capacity response incomplete".into());
        }
        Ok(value)
    }).await.map_err(|e|format!("T3 capacity query failed: {e}"))?
}

/** OW-0019: change the ordering priority only for the currently registered
 * pending queue row; on a transport timeout, return UNKNOWN without retry. */
pub(crate) async fn reprioritize(
    path: PathBuf, id: String, expected_qitem: String, priority: String
) -> Result<QueueSyncStatus, String> {
    if !["routine","urgent","critical"].contains(&priority.as_str()) {
        return Err("Invalid Queue priority".into());
    }
    let store = FsTaskStore::default();
    let snapshot = store.load(&path).map_err(|e| e.to_string())?;
    let task = snapshot.tasks.into_iter().find(|row| row.id == id && !row.archived)
        .ok_or("TaskBoard task no longer exists or was archived")?;
    let intent = store.execution_intent(&path,&id).map_err(|e|e.to_string())?;
    let request = intent.queue_request.as_deref().ok_or("Queue request missing")?;
    let qid = format!("qitem-taskboard-{request}");
    if intent.mode != ExecutionMode::Queue || intent.queue_receipt.as_deref() != Some(qid.as_str()) ||
        qid != expected_qitem || intent.queue_retired_receipt.is_some() {
        return Ok(status("conflict",Some(expected_qitem),Some("Queue receipt changed; no priority write attempted".into())));
    }
    let bearer=token()?;
    let client=client()?;
    let reference=ref_json(&path,&id);
    let (code,row)=get(&client,&bearer,&format!("/api/queue/{qid}")).await?;
    if !code.is_success() || !row_matches(&row,&reference,request,None,&task) {
        return Ok(status("conflict",Some(qid),Some("OpenRig Queue identity cannot be verified".into())));
    }
    if row["state"] != "pending" || row["claimedAt"].as_str().is_some_and(|s|!s.is_empty()) {
        return Ok(status("running",Some(qid),Some("Task was claimed; priority cannot change".into())));
    }
    let (code,result)=match post(&client,&bearer,&format!("/api/queue/taskboard/{qid}/sync"),json!({
        "action":"priority","priority":priority,"actorSession":SOURCE,
        "requestId":request,"taskRef":reference
    })).await {
        Ok(x)=>x,
        Err(err)=>return Ok(status("unknown",Some(qid),Some(err)))
    };
    if code.is_success() && result["priority"].as_str()==Some(priority.as_str()) {
        return Ok(status("registered",Some(qid),None));
    }
    Ok(status("conflict",Some(qid),Some(format!("OpenRig priority update refused: {}",
        result["error"].as_str().unwrap_or("unknown error")))))
}

/** OW-0014: bounded startup/focus reconciliation of explicit Queue intents.
 * This is NOT a second dispatch timer; it only drives the same idempotent
 * native producer when the TaskBoard UI activates a board. */
pub(crate) async fn reconcile(path: PathBuf) -> Result<Value, String> {
    let store = FsTaskStore::default();
    let snapshot = store.load(&path).map_err(|e| e.to_string())?;
    let mut checked = 0u32;
    let mut incomplete = false;
    let mut outcomes = Vec::<Value>::new();
    for task in snapshot.tasks {
        if !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) { continue; }
        let intent = match store.execution_intent(&path, &task.id) {
            Ok(intent) => intent,
            Err(error) => {
                outcomes.push(json!({"id":task.id,"status":"conflict","reason":error.to_string()}));
                continue;
            }
        };
        let needs_reconcile = intent.mode == ExecutionMode::Queue ||
            (intent.queue_attempt.is_some() && intent.queue_retired_receipt.is_none());
        if !needs_reconcile { continue; }
        if checked >= 16 { incomplete = true; break; }
        checked += 1;
        let result = match sync(path.clone(), task.id.clone()).await {
            Ok(value) => json!({"id":task.id,"status":value.status,"qitemId":value.qitem_id,"reason":value.reason}),
            Err(error) => json!({"id":task.id,"status":"unavailable","reason":error}),
        };
        outcomes.push(result);
    }
    Ok(json!({"checked":checked,"truncated":incomplete,"items":outcomes}))
}

/** OW-0006: source-qualified trace comes solely from the canonical OpenRig
 * correlation endpoint. No task title/claimed_by inference or second ledger.
 */
pub(crate) async fn trace(path: PathBuf, id: String) -> Result<Value, String> {
    if id.len() > 128 || !id.chars().all(|ch| ch.is_ascii_alphanumeric() || "_.-".contains(ch)) {
        return Err("Invalid TaskBoard trace ID".into());
    }
    let bearer = token()?;
    let client = client()?;
    let board_root = path.to_string_lossy().trim_end_matches(['\\','/']).replace('/', "\\");
    let response = client.get(format!("{DAEMON}/api/integrations/t3/taskboard/trace"))
        .bearer_auth(&bearer)
        .query(&[("environmentId", "pc"), ("boardRoot", board_root.as_str()), ("taskId", id.as_str())])
        .send().await.map_err(|_| "OpenRig trace unavailable; task status not inferred".to_owned())?;
    let code = response.status();
    let result = response.json::<Value>().await
        .map_err(|_| "OpenRig trace returned invalid JSON".to_owned())?;
    if !code.is_success() {
        return Err(format!("OpenRig trace refused ({code}): {}",
            result["error"].as_str().unwrap_or("unavailable")));
    }
    if result["schema"] != "openrig-taskboard-trace/v1" ||
        result["available"] != true ||
        result["taskRef"]["environmentId"] != "pc" ||
        result["taskRef"]["boardRoot"] != board_root ||
        result["taskRef"]["taskId"] != id {
        return Err("OpenRig trace identity or schema mismatch".into());
    }
    Ok(result)
}

/** User-scoped pending Queue reorder; no T3 start or retry. */
pub(crate) async fn move_pending(
    path: PathBuf, id: String, expected_qitem: String, direction: String
) -> Result<QueueSyncStatus, String> {
    if !["up","down"].contains(&direction.as_str()) { return Err("Invalid direction".into()); }
    let store = FsTaskStore::default();
    let snapshot = store.load(&path).map_err(|e| e.to_string())?;
    let task = snapshot.tasks.into_iter().find(|row| row.id == id && !row.archived)
        .ok_or("TaskBoard task is missing or archived")?;
    let intent = store.execution_intent(&path,&id).map_err(|e|e.to_string())?;
    let request = intent.queue_request.as_deref().ok_or("Queue request missing")?;
    let qid = format!("qitem-taskboard-{request}");
    if intent.mode != ExecutionMode::Queue || intent.queue_receipt.as_deref() != Some(qid.as_str()) ||
       qid != expected_qitem || intent.queue_retired_receipt.is_some() {
        return Ok(status("conflict",Some(expected_qitem),Some("Queue receipt changed; no move attempted".into())));
    }
    let bearer=token()?;
    let client=client()?;
    let reference=ref_json(&path,&id);
    let (code,row)=get(&client,&bearer,&format!("/api/queue/{qid}")).await?;
    if !code.is_success() || !row_matches(&row,&reference,request,None,&task) {
        return Ok(status("conflict",Some(qid),Some("OpenRig Queue identity mismatch".into())));
    }
    if row["state"]!="pending" || row["claimedAt"].as_str().is_some_and(|s|!s.is_empty()) {
        return Ok(status("running",Some(qid),Some("Queued task already claimed".into())));
    }
    let (code,result)=match post(&client,&bearer,&format!("/api/queue/taskboard/{qid}/sync"),json!({
        "action":"move","direction":direction,"actorSession":SOURCE,
        "requestId":request,"taskRef":reference
    })).await {
        Ok(x)=>x,
        Err(err)=>return Ok(status("unknown",Some(qid),Some(err)))
    };
    if code.is_success() && result["qitemId"].as_str()==Some(qid.as_str()) {
        return Ok(status("registered",Some(qid),None));
    }
    Ok(status("conflict",Some(qid),Some(format!("OpenRig move refused: {}",
        result["error"].as_str().unwrap_or("unknown error")))))
}

/** Bounded read-only provider evidence; not a pickup/claim or worker retry. */
pub(crate) async fn worker_proof(qitem_id: String) -> Result<Value, String> {
    if !qitem_id.starts_with("qitem-taskboard-") ||
        qitem_id.len() != "qitem-taskboard-".len() + 36 ||
        !qitem_id["qitem-taskboard-".len()..].chars()
            .all(|ch| ch.is_ascii_hexdigit() || ch == '-') {
        return Err("Invalid exact TaskBoard Queue ID".into());
    }
    let bearer=token()?;
    let client=client()?;
    let response=client.get(format!("{DAEMON}/api/integrations/t3/taskboard/worker-proof"))
        .bearer_auth(&bearer)
        .query(&[("qitemId",qitem_id.as_str())])
        .send().await.map_err(|e|format!("OpenRig worker evidence unavailable: {e}"))?;
    let code=response.status();
    let result=response.json::<Value>().await
        .map_err(|_|"OpenRig worker evidence response invalid".to_owned())?;
    if !code.is_success() || result["schema"]!="openrig-queued-worker-proof/v1" ||
        result["qitemId"]!=qitem_id {
        return Err(format!("OpenRig exact worker proof unavailable: {}",
            result["error"].as_str().unwrap_or("identity mismatch")));
    }
    Ok(result)
}
