//! Conservative confirmed-gone recovery. Missing, unloaded, errored, throttled,
//! archived or hidden T3 sessions are UNKNOWN and cannot expire a claim.
//! Only explicit permanent-gone lifecycle evidence starts a grace clock.
use std::{fs,path::{Path,PathBuf},io::Write,time::{SystemTime,UNIX_EPOCH}};
use serde::{Deserialize,Serialize};
use serde_json::Value;
use task_core::{FsTaskStore,TaskStore,MutationActor};

#[derive(Debug,Clone,Serialize,Deserialize)]
#[serde(rename_all="camelCase")]
struct GoneProof { task_id:String, owner:String, first_confirmed_at_ms:i64, evidence:String }
fn now_ms()->i64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64}
fn proof_file(root:&Path,id:&str)->PathBuf {root.join(".claim-recovery").join(format!("{id}.json"))}
fn exact_t3(raw:&str)->Option<&str>{
    let id=raw.strip_prefix("t3-browser:").unwrap_or(raw);
    (id.len()==36 && id.bytes().enumerate().all(|(i,b)|
      if [8,13,18,23].contains(&i){b==b'-'}else{b.is_ascii_hexdigit()})).then_some(id)
}
fn permanently_gone(session:&Value)->Option<String> {
    let status=session.get("status").and_then(|v|v.as_str()).unwrap_or("");
    let lifecycle=session.get("lifecycle").and_then(|v|v.as_str()).unwrap_or("");
    let explicit=session.get("permanentlyGone").and_then(|v|v.as_bool())==Some(true) ||
      ["permanently_deleted","deleted_permanently","permanently_closed"].contains(&status) ||
      ["permanently_deleted","deleted_permanently","permanently_closed"].contains(&lifecycle);
    // Ordinary hidden/archived/unavailable are explicitly not accepted.
    explicit.then(||format!("T3 lifecycle={lifecycle} status={status} permanentlyGone=true"))
}
fn write_proof(root:&Path,proof:&GoneProof)->Result<(),String>{
    let path=proof_file(root,&proof.task_id);
    fs::create_dir_all(path.parent().ok_or("Invalid recovery path")?).map_err(|e|e.to_string())?;
    let temp=path.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    let mut writer=fs::File::create_new(&temp).map_err(|e|e.to_string())?;
    writer.write_all(serde_json::to_string_pretty(proof).map_err(|e|e.to_string())?.as_bytes())
      .map_err(|e|e.to_string())?;
    writer.sync_all().map_err(|e|e.to_string())?;
    drop(writer);
    fs::rename(temp,path).map_err(|e|e.to_string())
}
/// t3=None means unavailable: do not start or advance evidence, and do not release.
pub(crate) fn reconcile(root:&Path,t3:Option<&Value>)->Result<Vec<String>,String>{
    let Some(sessions)=t3.and_then(|data|data.get("sessions")).and_then(|v|v.as_array()) else {
        return Ok(vec![]);
    };
    let policy=crate::agent_policy::read(root)?.values;
    let board=FsTaskStore::default().load(root).map_err(|e|e.to_string())?;
    let mut released=Vec::new();
    for task in board.tasks.into_iter().filter(|t|!t.archived && t.column=="in-progress") {
        let Some(owner)=task.claimed_by.as_deref() else {continue};
        let Some(id)=exact_t3(owner) else {continue};
        let path=proof_file(root,&task.id);
        let previous=fs::read(&path).ok().and_then(|bytes|
          serde_json::from_slice::<GoneProof>(&bytes).ok())
          .filter(|proof|proof.owner==owner);
        let session=sessions.iter().find(|s|s.get("id").and_then(|v|v.as_str())==Some(id));
        let evidence=session.and_then(permanently_gone);
        if session.is_some() && evidence.is_none() {
            // Authoritative evidence reversed: worker became present again.
            if path.is_file(){let _=fs::remove_file(&path);}
            continue;
        }
        let Some(proof)=(match (previous,evidence) {
            (Some(existing),_)=>Some(existing),
            (None,Some(evidence))=>Some(GoneProof {task_id:task.id.clone(),
                owner:owner.into(),first_confirmed_at_ms:now_ms(),evidence}),
            (None,None)=>None,
        }) else {continue};
        if !path.is_file(){write_proof(root,&proof)?;}
        if now_ms()-proof.first_confirmed_at_ms<policy.t3_orphan_grace_hours*3_600_000 {continue;}
        // The saved proof is tied to the exact owner. Update_as rechecks current
        // revision/claim inside the board lock and never spawns a replacement.
        let mut fresh=FsTaskStore::default().load(root).map_err(|e|e.to_string())?
          .tasks.into_iter().find(|t|t.id==task.id).ok_or("Task disappeared")?;
        if fresh.claimed_by.as_deref()!=Some(owner) || fresh.column!="in-progress" {continue;}
        fresh.claimed_by=None;fresh.column="ai-ready".into();
        FsTaskStore::default().update_as(root,fresh,MutationActor::Human)
          .map_err(|e|format!("Could not release confirmed-gone claim: {e}"))?;
        released.push(task.id.clone());
        let _=fs::remove_file(path);
    }
    Ok(released)
}
