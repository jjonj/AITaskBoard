//! TaskBoard-owned execution intent. No scheduler or OpenRig side effects live here.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode { #[default] Backlog, Queue }

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QueueState { Queued, Reserved, Starting, Working, Completed, RecoveryRequired }

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
    /// Durable phase entry, used by TaskBoard busy-work FIFO recovery across restarts.
    pub queue_state_since: Option<String>,
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
        queue_state_since: field("queue_state_since"),
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
    let mut mapping = mapping.clone();
    let current = load(path)?.1;
    // Every authoritative transition carries a persistent clock; stale
    // reservations must not become "new" whenever the desktop restarts.
    if mapping.get(key("queue_state")) != current.get(key("queue_state")) ||
        mapping.get(key("queue_reservation_id")) != current.get(key("queue_reservation_id")) {
        mapping.insert(key("queue_state_since"), key(
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
    } else if mapping.get(key("queue_state_since")).is_none() {
        // Legacy in-flight tasks enter monitoring with a durable first sighting.
        mapping.insert(key("queue_state_since"), key(
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
    }
    let yaml = serde_yaml::to_string(&mapping)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    let text = format!("---\n{yaml}---\n{body}");
    write_text_atomic(path, &text)?;
    intent(&mapping, &text)
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
        let resolving_incident = previous.queue_state == Some(QueueState::RecoveryRequired);
        if previous.queue_state != Some(QueueState::Starting) && !resolving_incident {
            return Err(TaskStoreError::Conflict(format!("{id} has not entered starting")));
        }
        if resolving_incident && (mapping.get(key("queue_recovery_question_added")).and_then(|v| v.as_bool()) != Some(true) ||
            task.question.as_ref().is_none_or(|q|
                q.request_type.as_deref() != Some("queue_recovery") || q.answer.is_some()) ||
            task.column == "blocked") {
            return Err(TaskStoreError::Conflict(format!("{id} has a human-owned recovery decision")));
        }
        if task.claimed_by.is_some() || task.archived || task.completed.is_some() ||
            snapshot.config.columns.iter().any(|column| column.terminal && column.id == task.column) ||
            (!resolving_incident && (!queue_task_ready(&snapshot, &task) || !recurring_due(&task))) {
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
        mapping.insert(key("queue_state_since"), key(
            &chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)));
        mapping.insert(key("queue_session_id"), key(session_id));
        mapping.remove(key("queue_reservation_expires_at"));
        if resolving_incident {
            if mapping.get(key("queue_recovery_attention_added")).and_then(|v| v.as_bool()) == Some(true) {
                let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
                    .map(|v| serde_yaml::from_value(v.clone())
                        .map_err(|error| TaskStoreError::Parse(error.to_string())))
                    .transpose()?.unwrap_or_default();
                attention.retain(|flag| *flag != TaskAttention::NeedsUser);
                mapping.insert(key("attention"), serde_yaml::to_value(&attention)
                    .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
            }
            mapping.remove(key("queue_recovery_attention_added"));
            if mapping.get(key("queue_recovery_question_added")).and_then(|v| v.as_bool()) == Some(true) {
                mapping.remove(key("question"));
                let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
                    .map(|v| serde_yaml::from_value(v.clone())
                        .map_err(|error| TaskStoreError::Parse(error.to_string())))
                    .transpose()?.unwrap_or_default();
                attention.retain(|flag| *flag != TaskAttention::Question);
                mapping.insert(key("attention"), serde_yaml::to_value(&attention)
                    .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
            }
            mapping.remove(key("queue_recovery_question_added"));
            mapping.remove(key("queue_error"));
        }
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

    /// Requeue an already-released worker only after its exact old T3 session
    /// is confirmed inactive. Do not alter claims or replace active workers.
    pub fn requeue_released_worker(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
        session_id: &str, reason: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue ||
            previous.queue_request.as_deref() != Some(queue_request) ||
            previous.queue_reservation_id.as_deref() != Some(request_id) ||
            previous.queue_session_id.as_deref() != Some(session_id) ||
            previous.queue_state != Some(QueueState::Working) {
            return Err(TaskStoreError::Conflict(format!("{id} released worker identity changed")));
        }
        let task = parse_task(&path, false, &text)?;
        if task.claimed_by.is_some() || !queue_task_ready(&snapshot, &task) ||
            !recurring_due(&task) || task.completed.is_some() {
            return Err(TaskStoreError::Conflict(format!("{id} has an active claim or is no longer ready")));
        }
        ensure_dependencies_complete(&snapshot, &task)?;
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

    /// Recover a reservation that never entered Starting. No T3 spawn can
    /// have been authorized from Reserved, so expiration is safe without a
    /// session close receipt. Idempotent under a board-wide lock.
    pub fn expire_reserved_queue_task(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue ||
            previous.queue_request.as_deref() != Some(queue_request) ||
            previous.queue_reservation_id.as_deref() != Some(request_id) ||
            previous.queue_state != Some(QueueState::Reserved) {
            return Err(TaskStoreError::Conflict(format!("{id}: reservation changed")));
        }
        let expired = previous.queue_reservation_expires_at.as_deref()
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .is_some_and(|expires| expires <= chrono::Utc::now());
        if !expired {
            return Err(TaskStoreError::Conflict(format!("{id}: reservation is unexpired or unreadable")));
        }
        mapping.insert(key("queue_state"), key("queued"));
        mapping.remove(key("queue_reservation_id"));
        mapping.remove(key("queue_reservation_expires_at"));
        mapping.insert(key("queue_error"), key("Expired pre-spawn reservation returned to Queue."));
        persist(&path, &mapping, &body)
    }

    /// Hard upper bound on ambiguous queue phases. An incident is *not*
    /// permission to replay a possibly-delivered prompt or discard an owner.
    /// It is a durable, user-visible recovery state with exact identifiers.
    pub fn flag_queue_recovery_required(
        &self, root: &Path, id: &str, queue_request: &str,
        state: QueueState, request_id: &str, reason: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        if !matches!(state, QueueState::Queued | QueueState::Reserved | QueueState::Starting | QueueState::Working) {
            return Err(TaskStoreError::Invalid("Only nonterminal queue phases can be escalated".into()));
        }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue ||
            previous.queue_request.as_deref().unwrap_or("__missing__") != queue_request ||
            previous.queue_state.unwrap_or(QueueState::Queued) != state ||
            previous.queue_reservation_id.as_deref().unwrap_or("__missing__") != request_id {
            return Err(TaskStoreError::Conflict(format!("{id}: recovery identity changed")));
        }
        let task = parse_task(&path, false, &text)?;
        if task.completed.is_some() || task.archived {
            return Err(TaskStoreError::Conflict(format!("{id}: task is terminal")));
        }
        mapping.insert(key("queue_state"), key("recovery_required"));
        mapping.insert(key("queue_error"), key(&reason.chars().take(500).collect::<String>()));
        let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
            .map(|value| serde_yaml::from_value(value.clone())
                .map_err(|error| TaskStoreError::Parse(error.to_string())))
            .transpose()?.unwrap_or_default();
        if !attention.contains(&TaskAttention::NeedsUser) {
            attention.push(TaskAttention::NeedsUser);
            mapping.insert(key("queue_recovery_attention_added"), serde_yaml::Value::Bool(true));
        }
        mapping.insert(key("attention"), serde_yaml::to_value(attention)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        // Reuse the existing human Inbox instead of inventing a second queue
        // error UI. A recovery decision is actionable and deliberately asks
        // permission before a potentially duplicated remote execution.
        let pending = task.question.as_ref().is_some_and(|q| q.answer.is_none());
        if !pending {
            let question = TaskQuestion {
                request_type: Some("queue_recovery".into()),
                prompt: format!("Queue recovery cannot prove whether the old session completed. {} The old prompt may still run. Retry creates a new execution and can duplicate work.", reason),
                options: vec!["Retry anyway (may duplicate work)".into(), "Keep paused".into()],
                answer: None,
            };
            mapping.insert(key("question"), serde_yaml::to_value(question)
                .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
            mapping.insert(key("queue_recovery_question_added"), serde_yaml::Value::Bool(true));
            let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
                .map(|value| serde_yaml::from_value(value.clone())
                    .map_err(|error| TaskStoreError::Parse(error.to_string())))
                .transpose()?.unwrap_or_default();
            if !attention.contains(&TaskAttention::Question) { attention.push(TaskAttention::Question); }
            mapping.insert(key("attention"), serde_yaml::to_value(attention)
                .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        }
        persist(&path, &mapping, &body)
    }

    /// Reset stale temporary working state after the common inactivity timeout.
    /// Preserve task content and restore its last pre-work column.
    /// All identities and timestamps are rechecked under the board lock.
    pub fn recover_stale_busy_task(
        &self, root: &Path, id: &str, expected_owner: Option<&str>,
        expected_queue_request: Option<&str>, expected_reservation: Option<&str>,
        expected_state: Option<QueueState>, expected_manual_request: Option<&str>,
        last_worker_activity_ms: Option<i64>,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let path = find_task_path(root, id)?;
        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, false, &existing)?;
        let (yaml, _) = split_frontmatter(&existing)?;
        let old: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
        let previous = intent(&old, &existing)?;
        if task.archived || task.completed.is_some() ||
            !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) ||
            snapshot.config.columns.iter().any(|col| col.terminal && col.id == task.column) ||
            task.claimed_by.as_deref() != expected_owner ||
            previous.queue_request.as_deref() != expected_queue_request ||
            previous.queue_reservation_id.as_deref() != expected_reservation ||
            previous.queue_state != expected_state ||
            previous.start_now_request.as_deref() != expected_manual_request {
            return Err(TaskStoreError::Conflict(format!("{id}: task changed since FIFO inspection")));
        }
        let busy = task.claimed_by.is_some() || task.column == IN_PROGRESS_COLUMN_ID ||
            matches!(previous.queue_state, Some(
                QueueState::Reserved | QueueState::Starting | QueueState::Working | QueueState::RecoveryRequired)) ||
            (previous.start_now_request.is_some() && previous.start_now_receipt.is_none());
        if !busy {
            return Err(TaskStoreError::Conflict(format!("{id}: no longer busy")));
        }
        // An unanswered user question or open visual review is not abandoned
        // execution. Leave its Inbox state untouched until answered.
        if task.review_status.as_deref() == Some("open") ||
            task.question.as_ref().is_some_and(|q| q.answer.is_none() &&
                q.request_type.as_deref() != Some("queue_recovery")) {
            return Err(TaskStoreError::Conflict(format!("{id}: awaiting user")));
        }
        let now = Utc::now();
        let dates = [
            previous.queue_state_since.as_deref(),
            old.get(key("last_activity_at")).and_then(serde_yaml::Value::as_str),
        ];
        let mut latest = dates.into_iter().flatten()
            .filter_map(|raw| chrono::DateTime::parse_from_rfc3339(raw).ok().map(|t| t.to_utc()))
            .max();
        if let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) {
            let time = chrono::DateTime::<Utc>::from(modified);
            latest = Some(latest.map_or(time, |v| v.max(time)));
        }
        if let Some(at) = last_worker_activity_ms.and_then(chrono::DateTime::<Utc>::from_timestamp_millis)
            .filter(|at| *at <= now) {
            latest = Some(latest.map_or(at, |v| v.max(at)));
        }
        let idle_threshold = crate::busy_recovery_idle_threshold(previous.queue_state);
        let idle_label = "3 hours";
        if latest.is_none_or(|at| now.signed_duration_since(at) < idle_threshold) {
            return Err(TaskStoreError::Conflict(format!("{id}: not inactive for {idle_label}")));
        }
        let previous_owner = task.claimed_by.clone().unwrap_or_else(|| "unassigned".into());
        release_to_original(&mut task, &snapshot.config, &existing)?;
        task.description = append_description_update(&task.description, "abandonment",
            &format!("FIFO recovery: over {idle_label} without activity (previous owner: {previous_owner}). Returned to {}.", task.column));
        let rendered = stamp_activity(&render_with_claim_token(&task, &existing, None)?, None, true)?;
        let (yaml, body) = split_frontmatter(&rendered)?;
        let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
        // A parked task must not retain a queue lease that can reactivate it.
        // Any executable Todo column preserves Queue intent after recovery.
        let auto_queued = snapshot.config.columns.iter().any(|column| column.id == task.column && column.auto_queue);
        if queue_enabled_column(&snapshot.config, &task.column) && (previous.mode == ExecutionMode::Queue || auto_queued) {
            mapping.insert(key("execution_mode"), key("queue"));
            mapping.insert(key("queue_request"), key(&uuid::Uuid::new_v4().to_string()));
            mapping.insert(key("queue_state"), key("queued"));
            mapping.insert(key("queue_state_since"), key(&now.to_rfc3339_opts(SecondsFormat::Millis, true)));
        } else {
            mapping.insert(key("execution_mode"), key("backlog"));
            for name in ["queue_request", "queue_state", "queue_state_since"] {
                mapping.remove(key(name));
            }
        }
        for name in ["queue_reservation_id", "queue_reservation_expires_at", "queue_session_id",
            "queue_registration_attempt", "queue_receipt", "queue_error"] {
            mapping.remove(key(name));
        }
        mapping.remove(key("start_now_request"));
        mapping.remove(key("start_now_receipt"));
        mapping.remove(key("queue_error"));
        let final_text = format!("---\n{}---\n{}", serde_yaml::to_string(&mapping)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?, body);
        write_text_atomic(&path, &final_text)?;
        intent(&mapping, &final_text)
    }

    /// One-time compatibility migration for incidents written by the obsolete
    /// watchdog. Preserve exact request/owner and original phase age, but remove
    /// the synthetic human-recovery question. No replay occurs here.
    pub fn normalize_legacy_queue_recovery(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.queue_state != Some(QueueState::RecoveryRequired) ||
            previous.queue_request.as_deref() != Some(queue_request) ||
            previous.queue_reservation_id.as_deref() != Some(request_id) {
            return Err(TaskStoreError::Conflict(format!("{id}: stale recovery attempt")));
        }
        let task = parse_task(&path, false, &text)?;
        if task.archived || task.completed.is_some() {
            return Err(TaskStoreError::Conflict(format!("{id}: task is terminal")));
        }
        let phase = if previous.queue_session_id.is_some() || task.claimed_by.is_some() {
            QueueState::Working
        } else {
            QueueState::Starting
        };
        mapping.insert(key("queue_state"), serde_yaml::to_value(phase)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?);
        if mapping.get(key("queue_recovery_attention_added")).and_then(|v| v.as_bool()) == Some(true) ||
            mapping.get(key("queue_recovery_question_added")).and_then(|v| v.as_bool()) == Some(true) {
            let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
                .map(|v| serde_yaml::from_value(v.clone())
                    .map_err(|e| TaskStoreError::Parse(e.to_string())))
                .transpose()?.unwrap_or_default();
            if mapping.get(key("queue_recovery_attention_added")).and_then(|v| v.as_bool()) == Some(true) {
                attention.retain(|v| *v != TaskAttention::NeedsUser);
            }
            if mapping.get(key("queue_recovery_question_added")).and_then(|v| v.as_bool()) == Some(true) &&
                task.question.as_ref().is_some_and(|q| q.request_type.as_deref() == Some("queue_recovery")) {
                mapping.remove(key("question"));
                attention.retain(|v| *v != TaskAttention::Question);
            }
            mapping.insert(key("attention"), serde_yaml::to_value(&attention)
                .map_err(|e| TaskStoreError::Parse(e.to_string()))?);
        }
        mapping.remove(key("queue_recovery_attention_added"));
        mapping.remove(key("queue_recovery_question_added"));
        mapping.remove(key("queue_error"));
        if task.column == BLOCKED_COLUMN_ID &&
            task.question.as_ref().is_some_and(|q| q.request_type.as_deref() == Some("queue_recovery")) {
            mapping.insert(key("column"), key(AI_READY_COLUMN_ID));
            mapping.remove(key("blocked_reason"));
        }
        // Do not call persist(): this is a presentation-state migration, not a
        // new execution attempt, so its original recovery age must not reset.
        let final_text = format!("---\n{}---\n{}", serde_yaml::to_string(&mapping)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?, body);
        write_text_atomic(&path, &final_text)?;
        intent(&mapping, &final_text)
    }

    /// Explicit operator resolution for an incident that cannot be proven
    /// safe to retry automatically. A fresh queue_request prevents stale old
    /// session acknowledgements from claiming the new attempt.
    pub fn retry_queue_incident(
        &self, root: &Path, id: &str, queue_request: &str, request_id: &str,
        reason: &str,
    ) -> Result<ExecutionIntent, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let (text, mut mapping, body) = load(&path)?;
        let previous = intent(&mapping, &text)?;
        if previous.mode != ExecutionMode::Queue ||
            previous.queue_state != Some(QueueState::RecoveryRequired) ||
            previous.queue_request.as_deref().unwrap_or("__missing__") != queue_request ||
            previous.queue_reservation_id.as_deref().unwrap_or("__missing__") != request_id {
            return Err(TaskStoreError::Conflict(format!("{id}: incident identity changed")));
        }
        let snapshot = self.load_unlocked(root)?;
        let task = snapshot.tasks.iter().find(|task| task.id == id && !task.archived)
            .ok_or_else(|| TaskStoreError::NotFound(id.into()))?;
        if task.completed.is_some() || task.claimed_by.is_some() {
            return Err(TaskStoreError::Conflict(format!("{id}: active claim/terminal task requires owner reconciliation")));
        }
        let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
            .map(|value| serde_yaml::from_value(value.clone())
                .map_err(|error| TaskStoreError::Parse(error.to_string())))
            .transpose()?.unwrap_or_default();
        if mapping.get(key("queue_recovery_attention_added")).and_then(|v| v.as_bool()) == Some(true) {
            attention.retain(|flag| *flag != TaskAttention::NeedsUser);
        }
        mapping.insert(key("attention"), serde_yaml::to_value(attention)
            .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        mapping.remove(key("queue_recovery_attention_added"));
        if mapping.get(key("queue_recovery_question_added")).and_then(|v| v.as_bool()) == Some(true) {
            mapping.remove(key("question"));
            let mut attention: Vec<TaskAttention> = mapping.get(key("attention"))
                .map(|value| serde_yaml::from_value(value.clone())
                    .map_err(|error| TaskStoreError::Parse(error.to_string())))
                .transpose()?.unwrap_or_default();
            attention.retain(|flag| *flag != TaskAttention::Question);
            mapping.insert(key("attention"), serde_yaml::to_value(attention)
                .map_err(|error| TaskStoreError::Parse(error.to_string()))?);
        }
        mapping.remove(key("queue_recovery_question_added"));
        mapping.insert(key("queue_request"), key(&uuid::Uuid::new_v4().to_string()));
        mapping.insert(key("queue_state"), key("queued"));
        mapping.remove(key("queue_reservation_id"));
        mapping.remove(key("queue_reservation_expires_at"));
        mapping.remove(key("queue_session_id"));
        mapping.insert(key("queue_error"), key(&format!("Operator retry: {}", reason).chars().take(500).collect::<String>()));
        persist(&path, &mapping, &body)
    }

    /// A manual Start now preempts only work that has not entered starting.
    pub fn prepare_manual_start(&self, root: &Path, id: &str) -> Result<(), TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        if snapshot.tasks.iter().any(|task| task.id == id &&
            snapshot.config.columns.iter().any(|column| column.id == task.column && column.auto_queue)) {
            return Err(TaskStoreError::Conflict(format!("{id} belongs to an auto-queue column")));
        }
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
        if mode == ExecutionMode::Backlog {
            let snapshot = self.load_unlocked(root)?;
            if snapshot.tasks.iter().any(|task| task.id == id &&
                snapshot.config.columns.iter().any(|column| column.id == task.column && column.auto_queue)) {
                return Err(TaskStoreError::Conflict(format!("{id} belongs to an auto-queue column")));
            }
        }
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
        let snapshot = self.load_unlocked(root)?;
        if snapshot.tasks.iter().any(|task| task.id == id &&
            snapshot.config.columns.iter().any(|column| column.id == task.column && column.auto_queue)) {
            return Err(TaskStoreError::Conflict(format!("{id} belongs to an auto-queue column")));
        }
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
