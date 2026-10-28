use crate::TaskStoreError;
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}};

#[derive(Debug, Serialize, Deserialize)]
struct BlockChallenge { task: String, action: String, reason: String, expires: String }
fn challenges_dir(root: &Path) -> PathBuf {
    let hash = format!("{:x}", Sha256::digest(root.to_string_lossy().to_ascii_lowercase().as_bytes()));
    std::env::temp_dir().join("TaskBoard").join("blocked-gate").join(&hash[..24])
}
pub fn challenge_agent_block(root: &Path, task: &str, action: &str, reason: &str, supplied: Option<&str>) -> Result<(), TaskStoreError> {
    let _guard = super::store::acquire_lock(root)?;
    challenge_agent_block_unlocked(root, task, action, reason, supplied)
}

/// Only call while holding the board lock; used by direct agent mutations
/// so callers cannot bypass the challenge by skipping the CLI.
pub(crate) fn challenge_agent_block_unlocked(root: &Path, task: &str, action: &str, reason: &str, supplied: Option<&str>) -> Result<(), TaskStoreError> {
    if reason.trim().is_empty() { return Err(TaskStoreError::Invalid("blocked_reason_required: provide the concrete inability to continue".into())); }
    let folder = challenges_dir(root);
    fs::create_dir_all(&folder)?;
    if let Some(token) = supplied {
        if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TaskStoreError::Invalid("blocked_confirmation_invalid: no valid challenge token".into()));
        }
        let path = folder.join(format!("{token}.yaml"));
        let text = fs::read_to_string(&path).map_err(|_| TaskStoreError::Invalid("blocked_confirmation_invalid: expired, unknown or used token; retry without confirmation to self-review".into()))?;
        let entry: BlockChallenge = serde_yaml::from_str(&text).map_err(|_| TaskStoreError::Invalid("blocked_confirmation_invalid".into()))?;
        if entry.expires.as_str() < Utc::now().to_rfc3339().as_str() || entry.task != task || entry.action != action || entry.reason != reason {
            return Err(TaskStoreError::Invalid("blocked_confirmation_invalid: wrong task, action, reason or expired".into()));
        }
        fs::remove_file(path)?;
        return Ok(());
    }
    let tmp = tempfile::Builder::new().prefix("challenge-").tempfile_in(&folder)?;
    let secret = format!("{:x}", Sha256::digest(format!("{}:{}", tmp.path().display(), Utc::now().to_rfc3339()).as_bytes()));
    let data = BlockChallenge { task: task.into(), action: action.into(), reason: reason.into(), expires: (Utc::now() + Duration::minutes(10)).to_rfc3339() };
    fs::write(folder.join(format!("{secret}.yaml")), serde_yaml::to_string(&data).map_err(|e| TaskStoreError::Parse(e.to_string()))?)?;
    Err(TaskStoreError::BlockedChallenge { token: secret, task: task.into(), action: action.into() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expired_challenge_is_rejected() {
        let root = std::env::temp_dir().join(format!("gate-expiry-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let err = challenge_agent_block(&root, "T3-0001", "move", "missing private SDK", None).unwrap_err();
        let TaskStoreError::BlockedChallenge { token, .. } = err else { panic!("first request must challenge") };
        let path = challenges_dir(&root).join(format!("{token}.yaml"));
        let mut saved: BlockChallenge = serde_yaml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        saved.expires = "2000-01-01T00:00:00+00:00".into();
        fs::write(&path, serde_yaml::to_string(&saved).unwrap()).unwrap();
        assert!(challenge_agent_block(&root, "T3-0001", "move", "missing private SDK", Some(&token)).is_err());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(challenges_dir(&root)).unwrap();
    }
}
