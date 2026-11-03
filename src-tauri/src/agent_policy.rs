//! Per-board recovery policy, independent of T3 session identities.
use std::{fs,path::{Path,PathBuf},io::Write};
use serde::{Serialize,Deserialize};
use sha2::{Digest,Sha256};

#[derive(Clone,Debug,Serialize,Deserialize)]
#[serde(rename_all="camelCase",default)]
pub(crate) struct AgentPolicy {
    pub resume_window_hours:i64,
    pub wake_recent_idle:bool,
    pub t3_orphan_grace_hours:i64,
    pub external_lease_hours:i64,
    pub technical_recheck_days:i64,
}
impl Default for AgentPolicy {
    fn default()->Self { Self{resume_window_hours:2,wake_recent_idle:true,
        t3_orphan_grace_hours:2,external_lease_hours:4,technical_recheck_days:7} }
}
impl AgentPolicy {
    fn validate(&self)->Result<(),String>{
        if !(1..=168).contains(&self.resume_window_hours) ||
           !(1..=168).contains(&self.t3_orphan_grace_hours) ||
           !(1..=2160).contains(&self.external_lease_hours) ||
           !(1..=365).contains(&self.technical_recheck_days) {
            return Err("Recovery policy values are out of bounds".into());
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub(crate) struct AgentPolicyResponse { pub values:AgentPolicy,pub revision:String }
fn policy_file(root:&Path)->PathBuf {root.join(".agent-resume-policy.json")}
fn revision(raw:&[u8])->String {
    Sha256::digest(raw).iter().map(|b|format!("{b:02x}")).collect()
}
pub(crate) fn read(root:&Path)->Result<AgentPolicyResponse,String>{
    let path=policy_file(root);
    if !path.is_file() {
        return Ok(AgentPolicyResponse{values:AgentPolicy::default(),revision:"missing".into()});
    }
    let raw=fs::read(&path).map_err(|e|e.to_string())?;
    let values:AgentPolicy=serde_json::from_slice(&raw).map_err(|e|format!("Recovery policy malformed: {e}"))?;
    values.validate()?;
    Ok(AgentPolicyResponse{values,revision:revision(&raw)})
}
#[tauri::command]
pub(crate) fn task_read_agent_policy(path:String)->Result<AgentPolicyResponse,String>{
    read(&PathBuf::from(path))
}
#[tauri::command]
pub(crate) fn task_save_agent_policy(path:String,values:AgentPolicy,expected_revision:String)
  ->Result<AgentPolicyResponse,String>{
    let root=PathBuf::from(path);
    if !root.join("_board.md").is_file(){return Err("Board manifest missing".into());}
    values.validate()?;
    if read(&root)?.revision!=expected_revision {
        return Err("Recovery settings changed elsewhere; reload before saving".into());
    }
    let file=policy_file(&root);
    let temp=file.with_extension(format!("{}.tmp",uuid::Uuid::new_v4()));
    let raw=serde_json::to_vec_pretty(&values).map_err(|e|e.to_string())?;
    let mut writer=fs::File::create_new(&temp).map_err(|e|e.to_string())?;
    writer.write_all(&raw).map_err(|e|e.to_string())?;
    writer.sync_all().map_err(|e|e.to_string())?;
    drop(writer);
    fs::rename(&temp,&file).map_err(|e|e.to_string())?;
    read(&root)
}
