//! TaskBoard-owned execution intent. No scheduler or OpenRig side effects live here.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode { #[default] Backlog, Queue }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueState { Queued, Reserved, Starting, Working, Completed }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionIntent {
    pub mode: ExecutionMode,
    /// Stable identity of the user's queue intent.
    pub queue_request: Option<String>,
    /// Legacy OpenRig registration marker. Never dispatch while present.
    pub queue_attempt: Option<String>,
    pub queue_receipt: Option<String>,
    pub queue_retired_receipt: Option<String>,
    /// One T3-owned lease / idempotency key for a selected task.
    pub queue_state: Option<QueueState>,
    pub queue_reservation_id: Option<String>,
    pub queue_reservation_expires_at: Option<String>,
    pub queue_session_id: Option<String>,
    pub queue_error: Option<String>,
    /// Stable ID for a single explicit foreground request, not a mode.
    pub start_now_request: Option<String>,
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
        queue_state: mapping.get(key("queue_state")).map(|value|
            serde_yaml::from_value(value.clone())
                .map_err(|error| TaskStoreError::Parse(error.to_string()))).transpose()?,
        queue_reservation_id: field("queue_reservation_id"),
        queue_reservation_expires_at: field("queue_reservation_expires_at"),
        queue_session_id: field("queue_session_id"),
        queue_error: field("queue_error"),
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

    /// Atomically reserve an eligible queue item under the board lock.
    pub fn reserve_queue_task(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
        lease_seconds: u64,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        if uuid::Uuid::parse_str(request_id).is_err() || !(30..=900).contains(&lease_seconds) {
            return Err(TaskStoreError::Invalid("reservation needs a UUID request ID and a 30..900 second lease".into()));
        }
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let task = snapshot.tasks.iter().find(|task| task.id == id && !task.archived)
            .ok_or_else(|| TaskStoreError::NotFound(id.into()))?;
        if !queue_task_ready(&snapshot, task) || !recurring_due(task) {
            return Err(TaskStoreError::Conflict(format!("{id} is not currently ready and unclaimed")));
        }
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue || previous.queue_request.as_deref() != Some(queue_request) {
            return Err(TaskStoreError::Conflict(format!("{id} queue intent changed")));
        }
        if (previous.queue_attempt.is_some() || previous.queue_receipt.is_some()) &&
            previous.queue_retired_receipt.as_deref() != previous.queue_request.as_ref()
                .map(|request| format!("qitem-taskboard-{request}")).as_deref() {
            return Err(TaskStoreError::Conflict(format!("{id} is a legacy OpenRig-owned queue item; reconcile ownership first")));
        }
        match previous.queue_state.unwrap_or(QueueState::Queued) {
            QueueState::Queued => {}
            QueueState::Reserved => {
                if previous.queue_reservation_id.as_deref() == Some(request_id) {
                    return Ok(previous);
                }
                let expires = previous.queue_reservation_expires_at.as_deref()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.timestamp())
                    .ok_or_else(|| TaskStoreError::Conflict(format!("{id} reservation lease is unreadable; reconcile before replacing it")))?;
                if expires > chrono::Utc::now().timestamp() {
                    return Err(TaskStoreError::Conflict(format!("{id} is already reserved")));
                }
            }
            state => return Err(TaskStoreError::Conflict(format!("{id} is already {state:?}"))),
        }
        mapping.insert(key("queue_state"), serde_yaml::to_value(QueueState::Reserved)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        mapping.insert(key("queue_reservation_id"), key(request_id));
        mapping.insert(key("queue_reservation_expires_at"), serde_yaml::Value::String(
            (chrono::Utc::now() + chrono::Duration::seconds(lease_seconds as i64))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
        mapping.remove(key("queue_session_id"));
        mapping.remove(key("queue_error"));
        persist(&path, &mapping, &body)
    }

    /// Move a reserved request to starting. Starting reservations never expire automatically.
    pub fn start_queue_task(&self, root: &Path, id: &str, request_id: &str) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.queue_reservation_id.as_deref() != Some(request_id) {
            return Err(TaskStoreError::Conflict(format!("{id} reservation identity changed")));
        }
        match previous.queue_state {
            Some(QueueState::Reserved) => {
                let expires = previous.queue_reservation_expires_at.as_deref()
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.timestamp());
                if !expires.is_some_and(|expires| expires > chrono::Utc::now().timestamp()) {
                    return Err(TaskStoreError::Conflict(format!("{id} reservation lease expired; obtain a fresh reservation")));
                }
            }
            Some(QueueState::Starting | QueueState::Working) => return Ok(previous),
            _ => return Err(TaskStoreError::Conflict(format!("{id} is not reserved"))),
        }
        mapping.insert(key("queue_state"), serde_yaml::to_value(QueueState::Starting)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        mapping.remove(key("queue_reservation_expires_at"));
        persist(&path, &mapping, &body)
    }

    /// Accept the exact T3 session and claim the task in one atomic board write.
    pub fn claim_queue_working(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
        session_id: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        if uuid::Uuid::parse_str(session_id).is_err() {
            return Err(TaskStoreError::Invalid("T3 session ID must be a UUID".into()));
        }
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let path = find_task_path(root, id)?;
        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, false, &existing)?;
        let (text, mut mapping, _original_body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue || previous.queue_request.as_deref() != Some(queue_request) ||
            previous.queue_reservation_id.as_deref() != Some(request_id) {
            return Err(TaskStoreError::Conflict(format!("{id} queue reservation identity changed")));
        }
        if previous.queue_state == Some(QueueState::Working) &&
            previous.queue_session_id.as_deref() == Some(session_id) &&
            task.claimed_by.as_deref() == Some(&format!("t3:{session_id}")) {
            return Ok(previous);
        }
        if previous.queue_state != Some(QueueState::Starting) {
            return Err(TaskStoreError::Conflict(format!("{id} has not entered starting")));
        }
        if task.claimed_by.is_some() || !queue_task_ready(&snapshot, &task) || !recurring_due(&task) {
            return Err(TaskStoreError::Conflict(format!("{id} is no longer available to this reservation")));
        }
        ensure_dependencies_complete(&snapshot, &task)?;
        let origin = if task.column == IN_PROGRESS_COLUMN_ID {
            claim_metadata(&existing, "in_progress_origin")?
                .unwrap_or_else(|| return_column(&snapshot.config, None))
        } else {
            return_column(&snapshot.config, Some(&task.column))
        };
        task.column = IN_PROGRESS_COLUMN_ID.into();
        task.claimed_by = Some(format!("t3:{session_id}"));
        task.agent_interacted = true;
        let rendered = stamp_activity(&render_with_claim_token(&task, &existing, None)?, Some(&origin), true)?;
        let (yaml, body) = split_frontmatter(&rendered)?;
        mapping = serde_yaml::from_str(&yaml).map_err(|error| TaskStoreError::Parse(error.to_string()))?;
        mapping.insert(key("queue_state"), serde_yaml::to_value(QueueState::Working)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        mapping.insert(key("queue_session_id"), key(session_id));
        mapping.remove(key("queue_reservation_expires_at"));
        let final_text = format!("---\n{}---\n{}", serde_yaml::to_string(&mapping)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?, body);
        write_text_atomic(&path, &final_text)?;
        task.revision = revision(&final_text);
        self.load_unlocked(root)?;
        let _ = body;
        Ok(intent(&mapping, &final_text)?)
    }

    /// Retire a queue worker only after T3 has durably closed the exact guest.
    /// The caller must verify that it never bootstrapped or performed work.
    pub fn reclaim_closed_queue_worker(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
        session_id: &str, reason: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        if uuid::Uuid::parse_str(session_id).is_err() {
            return Err(TaskStoreError::Invalid("T3 session ID must be a UUID".into()));
        }
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let path = find_task_path(root, id)?;
        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, false, &existing)?;
        let (text, original_mapping, _) = load(&path)?;
        let previous = intent(&original_mapping, &text)?;
        if previous.mode != ExecutionMode::Queue ||
            previous.queue_request.as_deref() != Some(queue_request) ||
            previous.queue_reservation_id.as_deref() != Some(request_id) ||
            previous.queue_session_id.as_deref() != Some(session_id) ||
            previous.queue_state != Some(QueueState::Working) {
            return Err(TaskStoreError::Conflict(format!("{id} queue worker identity changed")));
        }
        if task.archived || task.completed.is_some() ||
            snapshot.config.columns.iter().any(|column| column.terminal && column.id == task.column) {
            return Err(TaskStoreError::Conflict(format!("{id} is already terminal")));
        }
        let owner = format!("t3:{session_id}");
        if task.claimed_by.as_deref().is_some_and(|claim| claim != owner.as_str()) ||
            (task.claimed_by.is_none() && !queue_task_ready(&snapshot, &task)) {
            return Err(TaskStoreError::Conflict(format!("{id} ownership/readiness changed")));
        }
        let had_claim = task.claimed_by.is_some();
        if had_claim {
            if claim_metadata(&existing, "claim_token")?.is_some() {
                return Err(TaskStoreError::Conflict(format!("{id} has a managed claim token")));
            }
            release_to_original(&mut task, &snapshot.config, &existing)?;
        }
        let rendered = if had_claim {
            stamp_activity(&render_with_claim_token(&task, &existing, None)?, None, true)?
        } else {
            existing.clone()
        };
        let (yaml, body) = split_frontmatter(&rendered)?;
        let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?;
        mapping.insert(key("queue_state"), serde_yaml::to_value(QueueState::Queued)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        mapping.remove(key("queue_reservation_id"));
        mapping.remove(key("queue_reservation_expires_at"));
        mapping.remove(key("queue_session_id"));
        mapping.insert(key("queue_error"), serde_yaml::Value::String(reason.chars().take(500).collect()));
        persist(&path, &mapping, &body)
    }

    /// Retry only after T3 has positively rejected the exact start before creating a session.
    pub fn requeue_failed_start(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str, reason: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue || previous.queue_request.as_deref() != Some(queue_request) ||
            previous.queue_reservation_id.as_deref() != Some(request_id) ||
            previous.queue_state != Some(QueueState::Starting) {
            return Err(TaskStoreError::Conflict(format!("{id} failed-start receipt does not match the active reservation")));
        }
        let task = parse_task(&path, false, &text)?;
        if task.claimed_by.is_some() {
            return Err(TaskStoreError::Conflict(format!("{id} is claimed; cannot return it to Queue")));
        }
        mapping.insert(key("queue_state"), serde_yaml::to_value(QueueState::Queued)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        mapping.remove(key("queue_reservation_id"));
        mapping.remove(key("queue_reservation_expires_at"));
        mapping.remove(key("queue_session_id"));
        mapping.insert(key("queue_error"), serde_yaml::Value::String(reason.chars().take(500).collect::<String>()));
        persist(&path, &mapping, &body)
    }

    /// A manual Start now preempts only work that has not entered starting.
    pub fn prepare_manual_start(&self, root: &Path, id: &str) -> Result<(), TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.queue_attempt.is_some() || previous.queue_receipt.is_some() {
            let legacy_qitem = previous.queue_request.as_ref()
                .map(|request| format!("qitem-taskboard-{request}"));
            if previous.queue_retired_receipt.as_deref() != legacy_qitem.as_deref() {
                return Err(TaskStoreError::Conflict(format!("{id} is still owned by a legacy OpenRig queue request")));
            }
        }
        if matches!(previous.queue_state, Some(QueueState::Starting | QueueState::Working)) {
            return Err(TaskStoreError::Conflict(format!("{id} is already starting or working; reconcile its existing T3 session")));
        }
        if previous.mode == ExecutionMode::Queue {
            mapping.insert(key("execution_mode"), key("backlog"));
            mapping.insert(key("queue_manual"), serde_yaml::Value::Bool(true));
            mapping.remove(key("queue_reservation_id"));
            mapping.remove(key("queue_reservation_expires_at"));
            mapping.remove(key("queue_session_id"));
            mapping.remove(key("queue_state"));
        }
        if mapping != load(&path)?.1 { persist(&path, &mapping, &body)?; }
        Ok(())
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
            if matches!(previous.queue_state, Some(QueueState::Reserved | QueueState::Starting | QueueState::Working)) {
                return Err(TaskStoreError::Conflict(format!("{id} has an active T3 reservation; reconcile it before re-queuing")));
            }
            if previous.start_now_request.is_some() && previous.start_now_receipt.is_none() {
                return Err(TaskStoreError::Conflict(format!("{id} has an unresolved manual Start now request; reconcile it before queuing")));
            }
            if previous.queue_attempt.is_some() || previous.queue_receipt.is_some() {
                let old_qid = previous.queue_request.as_ref().map(|request| format!("qitem-taskboard-{request}"));
                if previous.queue_retired_receipt.as_deref() != old_qid.as_deref() {
                    return Err(TaskStoreError::Conflict(format!("{id} previous Queue attempt is not verified terminal; reconcile before requeue")));
                }
            }
            mapping.insert(key("queue_request"), key(&uuid::Uuid::new_v4().to_string()));
            mapping.insert(key("queue_state"), serde_yaml::to_value(QueueState::Queued)
                .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
            mapping.remove(key("queue_reservation_id"));
            mapping.remove(key("queue_reservation_expires_at"));
            mapping.remove(key("queue_session_id"));
            mapping.remove(key("queue_error"));
            mapping.remove(key("queue_registration_attempt"));
            mapping.remove(key("queue_receipt"));
            mapping.remove(key("queue_retired_receipt"));
            mapping.remove(key("start_now_request"));
            mapping.remove(key("start_now_receipt"));
        } else if matches!(previous.queue_state, Some(QueueState::Starting | QueueState::Working)) {
            return Err(TaskStoreError::Conflict(format!("{id} is already starting or working")));
        } else {
            mapping.remove(key("queue_state"));
            mapping.remove(key("queue_reservation_id"));
            mapping.remove(key("queue_reservation_expires_at"));
            mapping.remove(key("queue_session_id"));
        }
        // When returning to Backlog, preserve prior request/receipt for explicit
        // cancel reconciliation. This method never cancels running OpenRig work.
        mapping.insert(key("queue_manual"), serde_yaml::Value::Bool(true));
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
        let queue = intent(&mapping, &text)?;
        if queue.mode == ExecutionMode::Queue {
            if (queue.queue_attempt.is_some() || queue.queue_receipt.is_some()) &&
                queue.queue_retired_receipt.as_deref() != queue.queue_request.as_ref()
                    .map(|request| format!("qitem-taskboard-{request}")).as_deref() {
                return Err(TaskStoreError::Conflict(format!("{id} still has an outstanding legacy Queue request. Reconcile/cancel it before Start now")));
            }
            if matches!(queue.queue_state, Some(QueueState::Starting | QueueState::Working)) {
                return Err(TaskStoreError::Conflict(format!("{id} is already starting or working")));
            }
            mapping.insert(key("execution_mode"), key("backlog"));
            mapping.insert(key("queue_manual"), serde_yaml::Value::Bool(true));
            mapping.remove(key("queue_state"));
            mapping.remove(key("queue_reservation_id"));
            mapping.remove(key("queue_reservation_expires_at"));
            mapping.remove(key("queue_session_id"));
        }
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
