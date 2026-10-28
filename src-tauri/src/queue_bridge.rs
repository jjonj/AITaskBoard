//! OW-0014: explicit TaskBoard Queue -> the ONE OpenRig queue. No scheduler or
//! background polling. Durable pre-send CAS precedes network; unknown writes are
//! reconciled by exact deterministic qitem ID and NEVER resent.
use std::{path::{Path, PathBuf}, time::Duration};
use serde::Serialize;
use serde_json::{json, Value};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use task_core::{FsTaskStore, TaskStore, ExecutionMode, TaskKind, Task};
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
    format!("Execute the exact TaskBoard task {}/{} ({}): {}. Read the TaskBoard card and latest acceptance criteria before work; this queue receipt is NOT a completion claim.",
        reference["boardRoot"].as_str().unwrap_or(""),
        task.id, reference["environmentId"].as_str().unwrap_or("pc"), task.title)
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
    let task = snapshot.tasks.into_iter().find(|task| task.id == id)
        .ok_or_else(|| format!("{id} no longer exists in this board"))?;
    let execution = store.execution_intent(&path, &id).map_err(|e| e.to_string())?;
    // Archived tasks are no longer dispatchable even when archived directly
    // from a nonterminal column. A queued archived task must cancel its row.
    let finished = task.archived || task.completed.is_some() || snapshot.config.columns.iter().any(|col| col.terminal && col.id == task.column);
    let reference = ref_json(&path, &id);
    let request = execution.queue_request.as_deref();
    let qid = request.map(|request| format!("qitem-taskboard-{request}"));
    if !matches!(task.kind, TaskKind::Task|TaskKind::Recurring) || finished || execution.mode == ExecutionMode::Backlog {
        if let (Some(qitem), Some(receipt), Some(request)) = (qid.as_deref(),execution.queue_receipt.as_deref(),request) {
            if qitem != receipt { return Ok(status("conflict",qid,Some("Queue receipt does not match current request".into()))); }
            let bearer = token()?;
            let client = client()?;
            let (code,row) = get(&client,&bearer,&format!("/api/queue/{qitem}")).await?;
            if code == StatusCode::NOT_FOUND { return Ok(status("unknown",qid,Some("Registered qitem not found".into()))); }
            if !code.is_success() || !row_matches(&row,&reference,request,None,&task) {
                return Ok(status("conflict",qid,Some("Queue record identity cannot be verified".into())));
            }
            if terminal_state(&row) { return Ok(status("canceled",qid,None)); }
            if row["state"] != "pending" || row["claimedAt"].as_str().is_some_and(|value| !value.is_empty()) {
                return Ok(status("running",qid,Some("Already assigned/running: separate explicit stop required".into())));
            }
            let (response,result)=post(&client,&bearer,&format!("/api/queue/taskboard/{qitem}/sync"),json!({
                "taskRef":reference,"action":"cancel","requestId":request,"actorSession":SOURCE,
            })).await.map_err(|_| "Queue cancellation UNKNOWN: check qitem before retry".to_owned())?;
            if response.is_success() && result["state"]=="canceled" {return Ok(status("canceled",qid,None));}
            return Ok(status("conflict",qid,Some(format!("Cancel refused: {}",result["error"].as_str().unwrap_or("unknown reason")))));
        }
        return Ok(status("not_queued",qid,None));
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
