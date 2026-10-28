//! TaskBoard-owned execution intent. No scheduler or OpenRig side effects live here.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode { #[default] Backlog, Queue }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionIntent {
    pub mode: ExecutionMode,
    /// Minted only on an explicit Backlog -> Queue transition; survives uncertain registration.
    pub queue_request: Option<String>,
    /// Set BEFORE network send. An unresolved attempt cannot be auto-replayed.
    pub queue_attempt: Option<String>,
    /// Exact canonical OpenRig qitem ID, never a local second queue.
    pub queue_receipt: Option<String>,
    /// Previous attempt was verified terminal in OpenRig; required before requeue.
    pub queue_retired_receipt: Option<String>,
    /// Stable ID for a single explicit foreground request, not a mode.
    pub start_now_request: Option<String>,
    /// Set by the eventual dispatcher after it durably accepts this request.
    pub start_now_receipt: Option<String>,
    pub revision: String,
}

fn key(s: &str) -> serde_yaml::Value { serde_yaml::Value::String(s.into()) }

fn intent(mapping: &serde_yaml::Mapping, text: &str) -> Result<ExecutionIntent, TaskStoreError> {
    let mode = mapping.get(key("execution_mode"))
        .map(|value| serde_yaml::from_value(value.clone())
            .map_err(|e| TaskStoreError::Parse(e.to_string())))
        .transpose()?.unwrap_or_default();
    let field = |name: &str| mapping.get(key(name)).and_then(|v| v.as_str()).map(str::to_owned);
    Ok(ExecutionIntent {
        mode,
        queue_request: field("queue_request"),
        queue_attempt: field("queue_registration_attempt"),
        queue_receipt: field("queue_receipt"),
        queue_retired_receipt: field("queue_retired_receipt"),
        start_now_request: field("start_now_request"),
        start_now_receipt: field("start_now_receipt"),
        revision: revision(text),
    })
}

fn load(path: &Path) -> Result<(String, serde_yaml::Mapping, String), TaskStoreError> {
    let text = fs::read_to_string(path)?;
    let (yaml, body) = split_frontmatter(&text)?;
    let mapping = serde_yaml::from_str(&yaml)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    Ok((text, mapping, body))
}

fn persist(path: &Path, mapping: &serde_yaml::Mapping, body: &str) -> Result<ExecutionIntent, TaskStoreError> {
    let yaml = serde_yaml::to_string(mapping)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    let text = format!("---\n{yaml}---\n{body}");
    write_text_atomic(path, &text)?;
    intent(mapping, &text)
}

impl FsTaskStore {
    pub fn execution_intent(&self, root: &Path, id: &str) -> Result<ExecutionIntent, TaskStoreError> {
        let path = find_task_path(root, id)?;
        let (text, mapping, _) = load(&path)?;
        intent(&mapping, &text)
    }

    /// Explicit mode change only. Legacy missing metadata is Backlog.
    /// A new Queue request ID is minted on each Backlog -> Queue transition;
    /// re-saving Queue never creates another request or OpenRig side effect.
    pub fn set_execution_mode(&self, root: &Path, id: &str, expected: &str, mode: ExecutionMode) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        if revision(&text) != expected { return Err(TaskStoreError::Conflict(format!("{id} changed on disk"))); }
        let previous = intent(&mapping, &text)?;
        if previous.mode == mode { return Ok(previous); }
        if mode == ExecutionMode::Queue {
            // Never erase an old pending/UNKNOWN registration. Verify prior
            // OpenRig qitem terminal state before minting a requeue identity.
            if previous.queue_attempt.is_some() {
                let old_qid = previous.queue_request.as_ref().map(|request| format!("qitem-taskboard-{request}"));
                if previous.queue_retired_receipt.as_deref() != old_qid.as_deref() {
                    return Err(TaskStoreError::Conflict(format!("{id} previous Queue attempt is not verified terminal; reconcile before requeue")));
                }
            }
            mapping.insert(key("queue_request"), key(&uuid::Uuid::new_v4().to_string()));
            mapping.remove(key("queue_registration_attempt"));
            mapping.remove(key("queue_receipt"));
            mapping.remove(key("queue_retired_receipt"));
        }
        // When returning to Backlog, preserve prior request/receipt for explicit
        // cancel reconciliation. This method never cancels running OpenRig work.
        mapping.insert(key("execution_mode"), serde_yaml::to_value(mode).map_err(|e| TaskStoreError::Parse(e.to_string()))?);
        persist(&path, &mapping, &body)
    }

    /// Persist the pre-send intent before any network write. The marker may be
    /// reconciled by exact qitem ID, but never blindly resent after UNKNOWN.
    pub fn begin_queue_registration(&self, root: &Path, id: &str, request: &str, expected: &str) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.revision != expected || previous.mode != ExecutionMode::Queue || previous.queue_request.as_deref() != Some(request) {
            return Err(TaskStoreError::Conflict(format!("{id} Queue intent was edited or canceled")));
        }
        if previous.queue_attempt.is_some() || previous.queue_receipt.is_some() {
            return Err(TaskStoreError::Conflict(format!("{id} Queue registration already attempted; reconcile exact qitem")));
        }
        mapping.insert(key("queue_registration_attempt"), key(request));
        persist(&path, &mapping, &body)
    }

    /// Compare-and-set a canonical OpenRig registration receipt, after durable
    /// acknowledgement. A modified task, canceled Queue, or UNKNOWN send may
    /// not be silently accepted as an old revision's registration.
    pub fn receipt_queue(&self, root: &Path, id: &str, request: &str, expected: &str, qitem_id: &str) -> Result<ExecutionIntent, TaskStoreError> {
        if qitem_id.trim().is_empty() { return Err(TaskStoreError::Invalid("queue item ID cannot be empty".into())); }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.queue_request.as_deref() != Some(request) || previous.queue_attempt.as_deref() != Some(request) || previous.mode != ExecutionMode::Queue {
            return Err(TaskStoreError::Conflict(format!("{id} queue request changed or was canceled")));
        }
        if let Some(existing) = previous.queue_receipt.as_deref() {
            if existing == qitem_id { return Ok(previous); }
            return Err(TaskStoreError::Conflict(format!("{id} has a different OpenRig receipt")));
        }
        if previous.revision != expected { return Err(TaskStoreError::Conflict(format!("{id} changed during registration"))); }
        mapping.insert(key("queue_receipt"), key(qitem_id));
        persist(&path, &mapping, &body)
    }

    /// Record a verified terminal state of an earlier OpenRig Queue request.
    /// A timeout or missing row is not terminal evidence.
    pub fn retire_queue(&self, root: &Path, id: &str, request: &str, expected: &str, qitem_id: &str) -> Result<ExecutionIntent, TaskStoreError> {
        if qitem_id != format!("qitem-taskboard-{request}") {
            return Err(TaskStoreError::Invalid("retired queue qitem does not match request".into()));
        }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.queue_request.as_deref() != Some(request) || previous.queue_attempt.as_deref() != Some(request) {
            return Err(TaskStoreError::Conflict(format!("{id} queue attempt changed")));
        }
        if let Some(retired) = previous.queue_retired_receipt.as_deref() {
            if retired == qitem_id { return Ok(previous); }
            return Err(TaskStoreError::Conflict(format!("{id} different retired queue receipt")));
        }
        if previous.revision != expected { return Err(TaskStoreError::Conflict(format!("{id} changed during Queue retirement"))); }
        mapping.insert(key("queue_retired_receipt"), key(qitem_id));
        persist(&path, &mapping, &body)
    }

    /// A foreground click creates one stable request. Re-read before retrying:
    /// the request ID is durable even if the caller loses its response.
    /// A pending request cannot be silently replaced/replayed.
    pub fn request_start_now(&self, root: &Path, id: &str, expected: &str) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        if revision(&text) != expected { return Err(TaskStoreError::Conflict(format!("{id} changed on disk"))); }
        if mapping.get(key("start_now_request")).is_some() && mapping.get(key("start_now_receipt")).is_none() {
            return Err(TaskStoreError::Conflict(format!("{id} has an unreceipted Start now request; reconcile first")));
        }
        mapping.insert(key("start_now_request"), key(&uuid::Uuid::new_v4().to_string()));
        mapping.remove(key("start_now_receipt"));
        persist(&path, &mapping, &body)
    }

    /// Exact request-id compare-and-set. No network send, automatic replay, or dispatcher.
    pub fn receipt_start_now(&self, root: &Path, id: &str, request: &str, receipt: &str) -> Result<ExecutionIntent, TaskStoreError> {
        if receipt.trim().is_empty() { return Err(TaskStoreError::Invalid("receipt cannot be empty".into())); }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        if mapping.get(key("start_now_request")).and_then(|v| v.as_str()) != Some(request) {
            return Err(TaskStoreError::Conflict(format!("{id} Start now request changed")));
        }
        if let Some(existing) = mapping.get(key("start_now_receipt")).and_then(|v| v.as_str()) {
            if existing == receipt { return intent(&mapping, &text); }
            return Err(TaskStoreError::Conflict(format!("{id} Start now already receipted differently")));
        }
        mapping.insert(key("start_now_receipt"), key(receipt));
        persist(&path, &mapping, &body)
    }
}
