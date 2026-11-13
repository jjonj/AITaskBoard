//! Pre-save TaskBoard preparation sessions, routed through T3's existing durable browser transport.
use std::{fs, io::Write, path::{Path, PathBuf}, process::{Command, Stdio}};
#[cfg(windows)] use std::os::windows::process::CommandExt;

use serde_json::{json, Value};
use task_core::{FsTaskStore, TaskStore};
use uuid::Uuid;

const T3_SPAWN: &str = r"D:\SSDProjects\Tools\t3code\scripts\spawn-chatgpt-session.mjs";
const T3_SESSIONS: &str = r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs";

fn t3_home() -> Result<PathBuf, String> {
    std::env::var_os("T3CODE_HOME").map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join(".t3")))
        .ok_or("The active T3 home cannot be resolved".to_string())
}

fn run_node(script: &str, args: &[String], input: Option<&Value>) -> Result<Value, String> {
    if !Path::new(script).is_file() { return Err(format!("T3 CLI unavailable: {script}")); }
    let home = t3_home()?;
    let mut cmd = Command::new("node");
    cmd.arg(script).args(args).env("T3CODE_HOME", &home)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)] cmd.creation_flags(0x08000000);
    let mut process = cmd.spawn().map_err(|e| format!("Cannot start T3 CLI: {e}"))?;
    if let Some(mut stdin) = process.stdin.take() {
        if let Some(value) = input { stdin.write_all(value.to_string().as_bytes()).map_err(|e| e.to_string())?; }
    }
    let output = process.wait_with_output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("T3 CLI did not confirm the operation: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("T3 returned an ambiguous receipt; do not repeat the spawn: {e}"))
}

#[tauri::command]
pub async fn start_task_preparation(
    path: String, id: String, request_id: String, title: String, description: String,
    prepfile: bool,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Uuid::parse_str(&request_id).map_err(|_| "Invalid preparation request ID")?;
        if !Path::new(&path).join("_board.md").is_file() {
            return Err("TaskBoard root is unavailable".to_string());
        }
        let prompt = format!(
            "You are a PREPARATION-ONLY agent for a new TaskBoard task that the human has not saved yet.\n\
             Board: {path}\nReserved task ID: {id}\nProposed title: {title}\n\
             Current description (the human may still edit it):\n{description}\n\n\
             Start researching requirements, relevant source/design files and existing tasks NOW. \
             Ask the user precise follow-up questions in this T3 session when requirements or design choices are unclear. \
             Do not implement code, claim tasks, spawn another agent, create a task, or change human Draft cards before the save.\n\
             The TaskBoard app will send an explicit TASK SAVED message with the canonical ID ONLY after the human clicks Save. \
             Closing may auto-save an unfinished Draft, but that is NOT authorization to release it. \
             Continue your investigation while the human composes the task.\n\
             When TASK SAVED arrives, reread the task from disk and all human decisions. \
             Record technical findings, relevant file/function references, decisions, unresolved risks, \
             and implementation-ready acceptance criteria in AI Notes on that SAME task. \
             Keep asking follow-ups until clarified. Only then remove the 'prepare' attention flag \
             and move the saved task out of Draft to the intended column indicated in its \
             taskboard:preparation-target marker. If it remains unsaved or the user removes \
             preparation, do not release it for autonomous implementation. \
             Do not overwrite human description/notes or invent answers. \
             Never mark the task Done: your role ends at implementation readiness."
        );
        // The same T3 preparation session handles research, a skill-authored prepfile
        // and the existing durable TaskBoard notes/queue release. No second agent is
        // spawned here; TaskBoard dispatches implementation after preparation.
        let prompt = if prepfile {
            let output = format!(r"E:\_AgentScratch\Unassigned\Prepfile\Deliverables\prepfile-{request_id}.md");
            format!("{prompt}\n\nPREPFILE WORKFLOW (user opted in at task creation):\n\
                The task has already been saved. Wait for the idempotent TASK SAVED notification, then read and follow\n\
                C:\\Users\\crovea\\.agents\\skills\\extra-skill-finder\\on-request\\prepfile\\SKILL.md.\n\
                Depth: heavy. Use the skill's bundled build_context.py writer with --output set to exactly {output}.\n\
                Prepare source-labelled evidence and requirements for the saved task {id}, including current task description\n\
                and any attached images. Do not implement. Record the prepfile path, findings and user's answered design\n\
                decisions in that task's AI Notes before releasing the TaskBoard Draft to its intended column.\n\
                The TaskBoard queue will choose the implementation worker; do not spawn a separate implementer yourself.\n\
                Do not create a duplicate TaskBoard task.")
        } else { prompt };
        // The initial prompt may contain user notes: keep it out of process arguments.
        let scratch = Path::new(r"E:\_AgentScratch\Tasks\Misc\MISC-0137\Temporary");
        fs::create_dir_all(scratch).map_err(|e| format!("Cannot prepare T3 brief: {e}"))?;
        let message_path = scratch.join(format!("prep-{request_id}.txt"));
        fs::write(&message_path, prompt).map_err(|e| format!("Cannot write T3 brief: {e}"))?;
        let name = format!("Prepare {id}: {}", title.trim().chars().take(48).collect::<String>());
        let args = vec![
            "--home".to_string(), t3_home()?.to_string_lossy().into_owned(),
            "--title".to_string(), name,
            "--message-file".to_string(), message_path.to_string_lossy().into_owned(),
            "--role".to_string(), "worker".to_string(),
            "--request-id".to_string(), request_id,
        ];
        let result = run_node(T3_SPAWN, &args, None);
        let _ = fs::remove_file(&message_path);
        result
    }).await.map_err(|e| format!("T3 preparation launch failed: {e}"))?
}

#[tauri::command]
pub async fn notify_task_preparation_saved(
    path: String, id: String, request_id: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Uuid::parse_str(&request_id).map_err(|_| "Invalid preparation request ID")?;
        let board = FsTaskStore::default().load(Path::new(&path)).map_err(|e| e.to_string())?;
        let task = board.tasks.into_iter().find(|task| task.id == id && !task.archived)
            .ok_or("Saved task not found")?;
        if !task.notes.contains(&format!("taskboard:preparation-request:{request_id}")) ||
            !task.notes.contains("taskboard:preparation-saved") {
            return Err("The preparation task has not been explicitly saved".to_string());
        }
        let list = run_node(T3_SESSIONS, &[], Some(&json!({
            "action": "list", "which": "all", "includeHidden": true
        })))?;
        let sessions = list.get("sessions").and_then(Value::as_array).ok_or("T3 inventory unavailable")?;
        let matching: Vec<_> = sessions.iter().filter(|session|
            session.get("spawnRequestId").and_then(Value::as_str) == Some(&request_id)
        ).collect();
        if matching.len() != 1 {
            return Err("Preparation session not yet uniquely confirmed in T3. Do not spawn a replacement; reconcile this request ID.".to_string());
        }
        let session_id = matching[0].get("id").and_then(Value::as_str)
            .ok_or("T3 session identity unavailable")?;
        // The message ID is stable: repeated notifications resolve through T3's durable outbox.
        let sent = run_node(T3_SESSIONS, &[], Some(&json!({
            "action": "send",
            "messageId": request_id,
            "sessionId": session_id,
            "message": format!(
                "TASK SAVED. The canonical TaskBoard task now exists: {id}\nBoard root: {path}\n\
                 Reread its current title, description, acceptance criteria and notes. \
                 The task is deliberately kept in Draft while preparation is underway; \
                 its taskboard:preparation-target marker records the human's intended column. \
                 Complete your research/questions, append the findings and answered decisions to AI Notes, \
                 then remove prepare and move to the target column when genuinely ready. \
                 Do not implement or claim the task."
            )
        })))?;
        Ok(json!({"sessionId": session_id, "delivery": sent}))
    }).await.map_err(|e| format!("Cannot notify preparation agent: {e}"))?
}
