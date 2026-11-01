//! Low-priority technical blocker review candidates. No worker is spawned,
//! no task is unblocked or moved, and dependency waits are excluded.
use std::{fs,path::{Path,PathBuf},io::Write};
use chrono::{DateTime,Utc};
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};
use task_core::{FsTaskStore,Task,TaskStore,TaskAttention};

#[derive(Clone,Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct StaleBlocker {
    id:String,title:String,reason:String,
    last_activity_at:String,age_days:i64,signature:String,
}
#[derive(Serialize,Deserialize)]
struct Acknowledgment {signature:String,acknowledged_at:String}
fn signature(task:&Task)->String {
    let payload=serde_json::json!({"id":task.id,"blocked_reason":task.blocked_reason,
        "description":task.description,"acceptance":task.acceptance_criteria,
        "prerequisites":task.depends_on,"user_notes":task.user_notes});
    let digest=Sha256::digest(payload.to_string().as_bytes());
    digest.iter().map(|b|format!("{b:02x}")).collect()
}
fn last_task_activity(task:&Task)->Option<DateTime<Utc>> {
    let raw=fs::read_to_string(&task.path).ok()?;
    let stamp=raw.lines().skip(1).take_while(|line|line.trim()!="---")
      .find_map(|line|line.trim().strip_prefix("last_activity_at:")
          .map(|value|value.trim().trim_matches('"').trim_matches('\'').to_string()));
    stamp.and_then(|v|DateTime::parse_from_rfc3339(&v).ok().map(|t|t.with_timezone(&Utc)))
      .or_else(||fs::metadata(&task.path).ok()?.modified().ok().map(DateTime::<Utc>::from))
}
fn acknowledgments(root:&Path,id:&str)->PathBuf {root.join(".blocker-rechecks").join(format!("{id}.json"))}
fn candidate(task:&Task,all:&[Task],root:&Path,minimum_days:i64)->Option<StaleBlocker>{
    if task.archived || task.completed.is_some() || task.column!="blocked" ||
       !matches!(task.kind,task_core::TaskKind::Task | task_core::TaskKind::Recurring) {
        return None;
    }
    // Pure dependency waits never consume technical review attention. Missing
    // links are NOT silently assumed complete.
    let incomplete=task.depends_on.iter().any(|id| all.iter().find(|t|t.id==*id)
      .is_none_or(|dep|!dep.archived && dep.completed.is_none() && dep.column!="done"));
    if incomplete {return None;}
    if !task.attention.contains(&TaskAttention::Blocked) &&
       task.blocked_reason.as_deref().unwrap_or("").trim().is_empty(){return None;}
    let reason=task.blocked_reason.as_deref().filter(|v|!v.trim().is_empty())
      .unwrap_or("Explicit technical blocked flag; inspect before changing state");
    let last=last_task_activity(task)?;
    let days=Utc::now().signed_duration_since(last).num_days();
    if days<minimum_days {return None;}
    let sig=signature(task);
    if let Ok(bytes)=fs::read(acknowledgments(root,&task.id)) {
        if serde_json::from_slice::<Acknowledgment>(&bytes).ok()
            .is_some_and(|ack|ack.signature==sig){return None;}
    }
    Some(StaleBlocker{id:task.id.clone(),title:task.title.clone(),reason:reason.into(),
      last_activity_at:last.to_rfc3339(),age_days:days,signature:sig})
}
fn eligible(root:&Path)->Result<Vec<StaleBlocker>,String>{
    let snapshot=FsTaskStore::default().load(root).map_err(|e|e.to_string())?;
    let days=crate::agent_policy::read(root)?.values.technical_recheck_days;
    Ok(snapshot.tasks.iter().filter_map(|task|candidate(task,&snapshot.tasks,root,days)).collect())
}
#[tauri::command]
pub(crate) async fn task_stale_blockers(path:String)->Result<Vec<StaleBlocker>,String>{
    tauri::async_runtime::spawn_blocking(move ||eligible(&PathBuf::from(path)))
      .await.map_err(|e|e.to_string())?
}
#[tauri::command]
pub(crate) fn task_acknowledge_stale_blocker(path:String,id:String,signature:String)->Result<(),String>{
    if !id.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-') || id.is_empty() {
        return Err("Invalid task ID".into());
    }
    let root=PathBuf::from(path);
    let matches=eligible(&root)?.into_iter().any(|task|task.id==id && task.signature==signature);
    if !matches {return Err("This blocker changed or is no longer stale; refresh before acknowledging".into());}
    let file=acknowledgments(&root,&id);
    fs::create_dir_all(file.parent().ok_or("Invalid blocker path")?).map_err(|e|e.to_string())?;
    let ack=Acknowledgment{signature,acknowledged_at:Utc::now().to_rfc3339()};
    let temp=file.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    let mut f=fs::File::create_new(&temp).map_err(|e|e.to_string())?;
    f.write_all(serde_json::to_string_pretty(&ack).map_err(|e|e.to_string())?.as_bytes())
      .map_err(|e|e.to_string())?;
    f.sync_all().map_err(|e|e.to_string())?;
    fs::rename(temp,file).map_err(|e|e.to_string())
}
