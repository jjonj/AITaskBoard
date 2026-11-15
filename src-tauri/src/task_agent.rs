//! Task-linked T3 controls. All operations resolve the existing exact T3 session;
//! TaskBoard never creates a guest or runs a second scheduler.
use serde_json::{json, Value};
use std::{io::Write, path::{Path, PathBuf}, process::{Command, Stdio}};
use task_core::{FsTaskStore, TaskStore};
use uuid::Uuid;
#[cfg(windows)] use std::os::windows::process::CommandExt;

const T3_CLI: &str = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";
fn cli(request: Value) -> Result<Value, String> {
    if !Path::new(T3_CLI).is_file() { return Err("T3 desktop control is not installed.".into()); }
    let home = std::env::var_os("T3CODE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join(".t3")))
        .ok_or("T3 home is not configured")?;
    let mut cmd = Command::new("node");
    cmd.arg(T3_CLI).env("T3CODE_HOME", home)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)] cmd.creation_flags(0x08000000);
    let mut child = cmd.spawn().map_err(|e| format!("Could not reach T3: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(request.to_string().as_bytes()).map_err(|e| e.to_string())?;
    }
    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("T3 did not confirm the operation: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("T3 receipt ambiguous: {e}"))
}
fn uuid_from_claim(claim: &str) -> Option<&str> {
    let raw = claim.strip_prefix("t3-browser:").or_else(|| claim.strip_prefix("t3:")).unwrap_or(claim);
    Uuid::parse_str(raw).ok().map(|_| raw)
}
fn resolve(path: &str, id: &str) -> Result<(Value, String), String> {
    if !Path::new(path).join("_board.md").is_file() { return Err("TaskBoard root unavailable".into()); }
    let task = FsTaskStore::default().load(Path::new(path)).map_err(|e| e.to_string())?
        .tasks.into_iter().find(|task| task.id == id && !task.archived)
        .ok_or("Task does not exist on this board")?;
    let owner = task.claimed_by.as_deref().and_then(uuid_from_claim)
        .ok_or("No currently linked T3 session. Open a previous session separately; controls never start a new agent.")?;
    let inventory = cli(json!({"action":"list","which":"all","includeHidden":true}))?;
    let candidates: Vec<_> = inventory.get("sessions").and_then(Value::as_array)
        .ok_or("T3 inventory unavailable")?.iter().filter(|row| {
            row.get("id").and_then(Value::as_str) == Some(owner)
                || row.get("publicSessionId").and_then(Value::as_str) == Some(owner)
                || row.get("conversationId").and_then(Value::as_str) == Some(owner)
        }).collect();
    if candidates.len() != 1 {
        return Err("Linked session could not be uniquely verified in live T3; no action was sent.".into());
    }
    let entry = candidates[0];
    let bound = entry.pointer("/binding/taskRef");
    if let Some(bound) = bound {
        let exact_id = bound.get("id").and_then(Value::as_str);
        let exact_board = bound.get("board").and_then(Value::as_str);
        if exact_id.is_some() && (exact_id != Some(id) ||
            !exact_board.is_some_and(|p| p.replace('/', "\\").eq_ignore_ascii_case(&path.replace('/', "\\")))) {
            return Err("T3 session is bound to a different task; refusing cross-task control.".into());
        }
    }
    let sid = entry.get("id").and_then(Value::as_str)
        .ok_or("T3 returned a session without an ID")?.to_string();
    Ok((entry.clone(), sid))
}
fn status(path: String, id: String) -> Result<Value,String> {
    let (session, sid) = resolve(&path, &id)?;
    let bridge = cli(json!({"action":"task_agent_status","sessionId":sid,
        "taskBoardPath":path,"taskId":id}));
    let caffy = bridge.as_ref().ok().and_then(|s|s.get("caffy")).cloned()
        .unwrap_or_else(|| cli(json!({"action":"caffy","op":"status","sessionId":sid}))
            .unwrap_or(Value::Null));
    let attentions = bridge.as_ref().ok().and_then(|s|s.get("attentions")).cloned()
        .unwrap_or_else(|| json!([]));
    Ok(json!({"sessionId":sid, "title":session.get("title"), "activity":session.get("activity"),
        "status":session.get("status"), "statusObservedAt":session.get("statusObservedAt"),
        "lastWorkingAt":session.get("lastWorkingAt"), "url":session.get("url"),
        "caffy":caffy, "attentions":attentions,
        "gmaUnavailable":bridge.err().map(|e|format!("GMA and canonical Prepfile require the newer T3 control API: {e}"))}))
}
fn action(path: String, id: String, command: String, options: Value) -> Result<Value,String> {
    let (_, sid) = resolve(&path, &id)?;
    if !options.is_object() { return Err("Options must be an object".into()); }
    if command == "open" {
        let value = cli(json!({"action":"focus","sessionId":sid}))?;
        if value.get("acknowledged").and_then(Value::as_bool) != Some(true) {
            return Err("T3 did not confirm opening that exact session".into());
        }
        return Ok(value);
    }
    if command == "prepfile" {
        let request_id = options.get("requestId").and_then(Value::as_str)
            .ok_or("Stable requestId required")?;
        Uuid::parse_str(request_id).map_err(|_| "Invalid prepfile request ID")?;
        let user_request = options.get("request").and_then(Value::as_str)
            .ok_or("Prepfile request required")?.trim();
        if user_request.is_empty() || user_request.len() > 6_000 {
            return Err("Prepfile request must be between 1 and 6000 characters".into());
        }
        let depth = options.get("depth").and_then(Value::as_str).unwrap_or("medium");
        if !["light","medium","heavy"].contains(&depth) { return Err("Unknown preparation depth".into()); }
        let scoped_request = format!("Existing TaskBoard task: {path} / {id}\nFocus: {user_request}\nRead the canonical task criteria, provide evidence and record the Prepfile output path in AI Notes on THIS existing task; never create a second TaskBoard task.");
        return cli(json!({"action":"task_agent_action","sessionId":sid,
            "taskBoardPath":path,"taskId":id,"taskCommand":"prepfile_create",
            "taskOptionsJson":json!({"id":request_id,"request":scoped_request,"depth":depth,
                "generator":"chatgpt"}).to_string()}));
    }
    if ["attention_open","attention_snooze","attention_dismiss"].contains(&command.as_str()) {
        let attention_id=options.get("requestId").and_then(Value::as_str)
            .ok_or("Exact attention request ID required")?;
        Uuid::parse_str(attention_id).map_err(|_|"Invalid attention request ID")?;
        let mut args=json!({"requestId":attention_id});
        if command == "attention_snooze" {
            let minutes=options.get("minutes").and_then(Value::as_u64).unwrap_or(15);
            if ![15,60].contains(&minutes) {return Err("Snooze accepts 15 or 60 minutes".into());}
            args["minutes"]=json!(minutes);
        }
        return cli(json!({"action":"task_agent_action","sessionId":sid,
            "taskBoardPath":path,"taskId":id,"taskCommand":command,
            "taskOptionsJson":args.to_string()}));
    }
    let op = command.strip_prefix("caffy_").ok_or("Unsupported TaskBoard agent command")?;
    if !["set","configure","stop","pause","resume","wake","startup","begin_in","cancel_begin","auto_reboot"].contains(&op) {
        return Err("Unsupported Caffy action; no command was sent".into());
    }
    let mut request = json!({"action":"caffy","op":op,"sessionId":sid});
    if op == "set" {
        let mode = options.get("mode").and_then(Value::as_str).ok_or("Caffy mode required")?;
        if !["idle","periodic","startup"].contains(&mode) { return Err("Unsupported Caffy mode".into()); }
        request["mode"] = json!(mode);
    }
    let fields: &[&str] = match op {
        "set" => &["durationMinutes","intervalMinutes","onceOnly","startup","keepAwake","respectSlots","repeatPrompt","beginAt"],
        "configure" => &["respectSlots","repeatPrompt"],
        "startup" => &["startup"],
        "begin_in" => &["beginAt","prompt","respectSlots","requestId"],
        "auto_reboot" => &["rebootMinutes","rebootEveryRuns"],
        _ => &[]
    };
    for &field in fields {
        if let Some(value) = options.get(field) { request[field] = value.clone(); }
    }
    if op == "set" && options.get("mode").and_then(Value::as_str) == Some("startup") {
        request["startup"] = json!(true);
    }
    if let Some(n) = request.get("durationMinutes").and_then(Value::as_u64) {
        if !(1..=10_080).contains(&n) { return Err("Duration must be 1–10080 minutes".into()); }
    }
    if let Some(n) = request.get("intervalMinutes").and_then(Value::as_u64) {
        if !(1..=1440).contains(&n) { return Err("Interval must be 1–1440 minutes".into()); }
    }
    if let Some(n) = request.get("rebootEveryRuns").and_then(Value::as_u64) {
        if !(1..=10_000).contains(&n) { return Err("Reboot every-runs must be 1–10000".into()); }
    }
    if let Some(n) = request.get("rebootMinutes").and_then(Value::as_u64) {
        if !(1..=525_600).contains(&n) { return Err("Reboot minutes must be 1–525600".into()); }
    }
    cli(request)
}
#[tauri::command]
pub async fn task_agent_status(path:String,id:String) -> Result<Value,String> {
    tauri::async_runtime::spawn_blocking(move || status(path,id))
        .await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub async fn task_agent_action(path:String,id:String,command:String,options:Value) -> Result<Value,String> {
    tauri::async_runtime::spawn_blocking(move || action(path,id,command,options))
        .await.map_err(|e|e.to_string())?
}
