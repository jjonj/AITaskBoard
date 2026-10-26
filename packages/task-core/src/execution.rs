//! TaskBoard-owned execution intent. No scheduler or OpenRig side effects live here.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode { #[default] Backlog, Queue }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionIntent {
    pub mode: ExecutionMode,
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
    /// Revision must match; setting Queue does not submit work to OpenRig.
    pub fn set_execution_mode(&self, root: &Path, id: &str, expected: &str, mode: ExecutionMode) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        if revision(&text) != expected { return Err(TaskStoreError::Conflict(format!("{id} changed on disk"))); }
        mapping.insert(key("execution_mode"), serde_yaml::to_value(mode).map_err(|e| TaskStoreError::Parse(e.to_string()))?);
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
