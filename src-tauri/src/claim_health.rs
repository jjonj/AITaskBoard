//! Claim telemetry is observational: a failed T3 lookup never means a claim died.
use std::{collections::HashMap,fs,path::PathBuf};
use serde::Serialize;
use task_core::{FsTaskStore,TaskStore};

#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct ClaimHealth {
    owner: String,
    task_activity_at: Option<String>,
    agent_activity_at: Option<i64>,
    agent_state: String,
    verified_at: Option<String>,
}

fn exact_t3(owner:&str)->Option<&str> {
    let raw=owner.strip_prefix("t3-browser:").unwrap_or(owner);
    (raw.len()==36 && raw.bytes().enumerate().all(|(i,c)|
        if [8,13,18,23].contains(&i) {c==b'-'} else {c.is_ascii_hexdigit()})).then_some(raw)
}

#[tauri::command]
pub(crate) async fn task_claim_health(path:Option<String>)->Result<HashMap<String,ClaimHealth>,String>{
    tauri::async_runtime::spawn_blocking(move || {
        let root=PathBuf::from(path.ok_or("Board path required")?);
        let board=FsTaskStore::default().load(&root).map_err(|e|e.to_string())?;
        let t3_ids=board.tasks.iter().filter_map(|t| t.claimed_by.as_deref().and_then(exact_t3)).collect::<Vec<_>>();
        let t3=if !t3_ids.is_empty() {crate::agent_notices::shared_t3_inventory()} else {None};
        let states=t3.as_ref().and_then(|v|v.get("sessions")).and_then(|v|v.as_array());
        let mut health=HashMap::new();
        for task in board.tasks.into_iter().filter(|t|t.claimed_by.is_some() && !t.archived) {
            let owner=task.claimed_by.unwrap_or_default();
            let activity=fs::read_to_string(&task.path).ok().and_then(|text| {
                // Metadata only, never confuse filesystem mtime with verified worker progress.
                text.lines().skip(1).take_while(|line|line.trim()!="---")
                    .find_map(|line|line.strip_prefix("last_activity_at:")
                        .map(|value|value.trim().trim_matches('"').trim_matches('\'').to_string()))
            });
            let session=exact_t3(&owner).and_then(|id|states.and_then(|items|items.iter()
                .find(|item|item.get("id").and_then(|v|v.as_str())==Some(id))));
            let state=if exact_t3(&owner).is_none(){"External / unverified".into()}
                else if let Some(s)=session {
                    if s.get("activity").and_then(|v|v.as_str())==Some("working") &&
                       matches!(s.get("status").and_then(|v|v.as_str()), Some("generating" | "working" | "streaming")) {"Working".into()}
                    else {match s.get("status").and_then(|v|v.as_str()).unwrap_or("") {
                        "done_or_idle"=>"Idle", "unavailable"=>"Errored", "pending_or_stalled"=>"Unknown",
                        _=>"Unknown" }.into()}
                }else {"Unknown".into()};
            let agent_activity_at=session.and_then(|s|s.get("lastWorkingAt"))
                .and_then(|v|v.as_i64()).filter(|n|*n>0)
;
            let verified_at=session.and_then(|s|s.get("statusObservedAt"))
                .and_then(|v|v.as_str()).map(str::to_owned);
            health.insert(task.id,ClaimHealth{owner,task_activity_at:activity,
                agent_activity_at,agent_state:state,verified_at});
        }
        Ok(health)
    }).await.map_err(|e|e.to_string())?
}
