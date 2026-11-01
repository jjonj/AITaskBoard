//! Durable TaskBoard-owned notification outbox. No blind retries after uncertain send.
//! Entries live with the board, independent of temporary T3 browser identities.
use std::{fs,io::Write,path::{Path,PathBuf},process::{Command,Stdio},sync::{Mutex,OnceLock},time::{SystemTime,UNIX_EPOCH}};
use serde::{Deserialize,Serialize};
#[cfg(windows)] use std::os::windows::process::CommandExt;
use serde_json::{json,Value};
use task_core::{FsTaskStore,TaskStore,MutationActor};

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct AgentNotice {
    id:String, task_id:String, owner:String, revision:String, event:String,
    created_at_ms:i64, state:String,
    #[serde(default)] reason:Option<String>,
    #[serde(default)] t3_receipt:Option<Value>,
}
fn now_ms()->i64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64}
fn notice_dir(root:&Path)->PathBuf {root.join(".agent-notices")}
fn uuid_owner(raw:&str)->Option<&str> {
    let value=raw.strip_prefix("t3-browser:").unwrap_or(raw);
    (value.len()==36 && value.bytes().enumerate().all(|(i,b)|
      if [8,13,18,23].contains(&i){b==b'-'}else{b.is_ascii_hexdigit()})).then_some(value)
}
fn write_notice(root:&Path, notice:&AgentNotice)->Result<(),String>{
    let folder=notice_dir(root);fs::create_dir_all(&folder).map_err(|e|e.to_string())?;
    let path=folder.join(format!("{}.json",notice.id));
    let temp=folder.join(format!("{}.{}.tmp",notice.id,uuid::Uuid::new_v4()));
    let mut file=fs::File::create_new(&temp).map_err(|e|e.to_string())?;
    file.write_all(serde_json::to_vec_pretty(notice).map_err(|e|e.to_string())?.as_slice())
      .map_err(|e|e.to_string())?;
    file.sync_all().map_err(|e|e.to_string())?;
    drop(file);
    fs::rename(&temp,&path).map_err(|e|format!("Cannot persist agent notice: {e}"))
}
fn t3_script()->&'static str {r"D:\SSDProjects\Tools\t3code\scripts\chatgpt-sessions.mjs"}
fn t3_control(payload:Value)->Result<Value,String>{
    if !Path::new(t3_script()).is_file() {return Err("T3 CLI unavailable".into());}
    let mut child=Command::new("node").arg(t3_script())
      .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
      .creation_flags(0x08000000).spawn().map_err(|e|format!("T3 unavailable: {e}"))?;
    child.stdin.take().ok_or("T3 pipe unavailable")?
      .write_all(payload.to_string().as_bytes()).map_err(|e|e.to_string())?;
    let output=child.wait_with_output().map_err(|e|e.to_string())?;
    if !output.status.success(){return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());}
    serde_json::from_slice(&output.stdout).map_err(|e|format!("Ambiguous T3 response: {e}"))
}
/// Source commit and exact owner are checked before recording the outbox item.
/// The board mutation itself is already durable when this command executes.
#[tauri::command]
pub(crate) fn task_enqueue_agent_notice(path:String,id:String,owner:String,revision:String,event:String)
    ->Result<AgentNotice,String>{
    if !["requirements-change","human-response","reopen"].contains(&event.as_str())
      {return Err("Unsupported agent notice event".into());}
    let root=PathBuf::from(&path);
    let board=FsTaskStore::default().load(&root).map_err(|e|e.to_string())?;
    let task=board.tasks.iter().find(|t|t.id==id && !t.archived)
      .ok_or("Task no longer exists")?;
    if task.claimed_by.as_deref()!=Some(owner.as_str()) || task.column!="in-progress" {
       return Err("The claimed worker changed or task is no longer In Progress".into());
    }
    if task.revision!=revision {return Err("Task changed before notification was recorded".into());}
    let notice=AgentNotice{id:uuid::Uuid::new_v4().to_string(),task_id:id,
      owner,revision,event,created_at_ms:now_ms(),state:"pending".into(),reason:None,t3_receipt:None};
    write_notice(&root,&notice)?;
    Ok(notice)
}
fn reconciler_lock()->&'static Mutex<()> { static LOCK:OnceLock<Mutex<()>>=OnceLock::new();LOCK.get_or_init(||Mutex::new(())) }
fn resolve_notice(root:&Path,notice:&mut AgentNotice, t3:&Option<Value>)->Result<(),String>{
    if notice.state!="pending" {return Ok(());}
    let board=FsTaskStore::default().load(root).map_err(|e|e.to_string())?;
    let Some(task)=board.tasks.into_iter().find(|t|t.id==notice.task_id && !t.archived) else {
        notice.state="superseded".into();notice.reason=Some("Task no longer exists".into());return Ok(());
    };
    if task.claimed_by.as_deref()!=Some(notice.owner.as_str()) || task.column!="in-progress" {
        notice.state="superseded".into();notice.reason=Some("Worker ownership changed; preserved on disk".into());return Ok(());
    }
    let Some(session_id)=uuid_owner(&notice.owner) else {
        notice.state="external".into();notice.reason=Some("External worker: notice retained for TaskBoard polling".into());return Ok(());
    };
    let Some(sessions)=t3.as_ref().and_then(|json|json.get("sessions")).and_then(|v|v.as_array()) else {
        notice.reason=Some("T3 unavailable; preserving claim and pending notice".into());return Ok(());
    };
    let Some(session)=sessions.iter().find(|s|s.get("id").and_then(|v|v.as_str())==Some(session_id)) else {
        notice.reason=Some("No authoritative gone receipt: T3 session missing from inventory".into());return Ok(());
    };
    let activity=session.get("activity").and_then(|v|v.as_str()).unwrap_or("");
    let status=session.get("status").and_then(|v|v.as_str()).unwrap_or("");
    let working=activity=="working" && matches!(status,"generating" | "working" | "streaming");
    let idle=status=="done_or_idle" && activity=="idle";
    if !working && !idle {
        notice.reason=Some("T3 state unknown/recovering/errored: no dispatch or release".into());return Ok(());
    }
    let policy=crate::agent_policy::read(root)?.values;
    let policy_hours=policy.resume_window_hours;
    let wake_idle=policy.wake_recent_idle;
    let last_work=session.get("lastWorkingAt").and_then(|v|v.as_i64()).filter(|v|*v>0);
    let recent=last_work.is_some_and(|time|now_ms()-time<policy_hours*3_600_000);
    if idle && (!recent || !wake_idle) {
        // A positively observed idle but out-of-window session is not resumed.
        // Release only this exact claim through TaskBoard's revision-checked writer.
        let mut revised=task.clone();revised.claimed_by=None;
        revised.column="ai-ready".into();
        FsTaskStore::default().update_as(root,revised,MutationActor::Human)
          .map_err(|e|format!("Cannot release out-of-window claim safely: {e}"))?;
        notice.state="handoff-ready".into();
        notice.reason=Some("Resume window expired or wake-idle disabled; task returned to AI Ready".into());
        return Ok(());
    }
    let message=format!("TaskBoard {} for {} in {} (notice {}). Re-read the current task, human decisions and acceptance criteria before continuing. You remain the exact claimed worker; no additional agent was spawned.",
        notice.event,notice.task_id,root.display(),notice.id);
    // Persist the intent BEFORE attempting delivery. If the CLI response is
    // ambiguous, never send the same user message a second time blindly.
    notice.state="attempting".into();write_notice(root,notice)?;
    match t3_control(json!({"action":"send","sessionId":session_id,
        "messageId":notice.id,"message":message})) {
      Ok(receipt) if receipt.get("messageId").and_then(|v|v.as_str())==Some(&notice.id) &&
           receipt.get("status").and_then(|v|v.as_str()).is_some() => {
        notice.t3_receipt=Some(receipt);notice.state="accepted".into();notice.reason=None;
      }
      Ok(receipt)=>{
        notice.t3_receipt=Some(receipt);notice.state="unknown".into();
        notice.reason=Some("T3 acknowledgment did not match exact message ID; reconcile before retry".into());
      }
      Err(err)=>{
        notice.state="unknown".into();notice.reason=Some(format!("T3 send receipt unknown: {err}"));
      }
    }
    Ok(())
}
#[tauri::command]
pub(crate) async fn task_reconcile_agent_notices(path:String)->Result<Vec<AgentNotice>,String>{
    tauri::async_runtime::spawn_blocking(move || {
        let _guard=reconciler_lock().lock().map_err(|e|e.to_string())?;
        let root=PathBuf::from(&path);
        let directory=notice_dir(&root);
        let mut notices=if directory.is_dir() {
            fs::read_dir(&directory).map_err(|e|e.to_string())?
              .filter_map(|entry|entry.ok()).filter(|entry|entry.path().extension().is_some_and(|x|x=="json"))
              .filter_map(|entry|fs::read(entry.path()).ok())
              .filter_map(|data|serde_json::from_slice::<AgentNotice>(&data).ok()).collect::<Vec<_>>()
        }else{Vec::new()};
        notices.sort_by_key(|n|n.created_at_ms);
        let has_t3_claims=FsTaskStore::default().load(&root).map_err(|e|e.to_string())?
          .tasks.iter().any(|task|task.claimed_by.as_deref().and_then(uuid_owner).is_some());
        let t3=if has_t3_claims || notices.iter().any(|n|uuid_owner(&n.owner).is_some() &&
             ["pending","unknown","attempting"].contains(&n.state.as_str())) {
            t3_control(json!({"action":"list","which":"all","includeHidden":true})).ok()
        }else{None};
        // Stable claims stay put unless there is explicit permanent-gone proof
        // AND the configured grace elapsed. Unknown states never cause release.
        crate::orphan_recovery::reconcile(&root,t3.as_ref())?;
        // An uncertain send can only be reconciled by the *same* message ID;
        // never blindly submit the instruction a second time.
        if t3.is_some() {
            for n in notices.iter_mut().filter(|n|n.state=="unknown" || n.state=="attempting") {
                if let Ok(receipt)=t3_control(json!({"action":"delivery_status","messageId":n.id})) {
                    if receipt.get("messageId").and_then(|v|v.as_str())==Some(n.id.as_str()) &&
                       receipt.get("status").and_then(|v|v.as_str()).is_some() {
                        n.state="accepted".into();n.reason=None;n.t3_receipt=Some(receipt);
                        write_notice(&root,n)?;
                    }
                }
            }
        }
        for n in notices.iter_mut().filter(|n|n.state=="pending") {
            resolve_notice(&root,n,&t3)?;
            write_notice(&root,n)?;
        }
        Ok(notices)
    }).await.map_err(|e|e.to_string())?
}
