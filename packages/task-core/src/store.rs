use crate::{
    BoardConfig, BoardSnapshot, Column, Task, TaskAttention, TaskDraft, TaskKind, TaskQuestion,
    AI_READY_COLUMN_ID, DRAFT_COLUMN_ID, IN_PROGRESS_COLUMN_ID, SCHEMA_VERSION,
};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
#[path = "execution.rs"]
mod execution;
pub use execution::{ExecutionIntent, ExecutionMode};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[path = "review.rs"]
pub mod review;
#[path = "store/review_link.rs"]
mod review_link;

const BOARD_FILE: &str = "_board.md";
const ACTIVE_DIR: &str = "Active";
const ARCHIVE_DIR: &str = "Archive";
const COMPLETED_RETENTION_DAYS: i64 = 7;

#[derive(Debug, thiserror::Error)]
pub enum TaskStoreError {
    #[error("blocked_confirmation_required: {task} {action}; retry only for a genuine blocker with --confirm-genuine-block {token}")]
    BlockedChallenge { token: String, task: String, action: String },
    #[error("board data is invalid: {0}")]
    Invalid(String),
    #[error("task not found: {0}")]
    NotFound(String),
    #[error("write conflict: {0}")]
    Conflict(String),
    #[error("claim_instance_mismatch: {0}")]
    ClaimInstanceMismatch(String),
    #[error("task {task_id} is claimed by {claimed_by}")]
    AlreadyClaimed { task_id: String, claimed_by: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error: {0}")]
    Parse(String),
}

pub trait TaskStore {
    fn load(&self, root: &Path) -> Result<BoardSnapshot, TaskStoreError>;
    fn create(&self, root: &Path, draft: TaskDraft) -> Result<Task, TaskStoreError>;
    fn update(&self, root: &Path, task: Task) -> Result<Task, TaskStoreError>;
    fn claim(&self, root: &Path, id: &str, agent: &str) -> Result<Task, TaskStoreError>;
    fn release_claim(&self, root: &Path, id: &str, agent: &str) -> Result<Task, TaskStoreError>;
    fn archive(&self, root: &Path, id: &str) -> Result<Task, TaskStoreError>;
    fn archive_column(&self, root: &Path, column_id: &str, expected: &[(String, String)]) -> Result<BoardSnapshot, TaskStoreError>;
    fn archive_column_older_than(&self, root: &Path, column_id: &str, cutoff_iso: &str, expected: &[(String, String)]) -> Result<BoardSnapshot, TaskStoreError>;
    fn save_config(&self, root: &Path, config: BoardConfig) -> Result<BoardConfig, TaskStoreError>;
    fn restore_snapshot(
        &self,
        root: &Path,
        before: BoardSnapshot,
        after: BoardSnapshot,
    ) -> Result<BoardSnapshot, TaskStoreError>;
}

#[derive(Default)]
pub struct FsTaskStore;

/// Temporary, lock-protected ID claim used by the desktop New Task dialog.
/// It does not create a phantom task in the board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskIdReservation {
    pub id: String,
    pub token: String,
}

const RESERVATIONS_DIR: &str = ".id-reservations";

fn next_task_id(root: &Path, snapshot: &BoardSnapshot, kind: TaskKind) -> String {
    let configured = if kind == TaskKind::Epic {
        snapshot.config.epic_prefix.as_deref().unwrap_or("WTE")
    } else {
        snapshot.config.task_prefix.as_deref().unwrap_or("WT")
    };
    let prefix = format!("{configured}-");
    let mut next = snapshot.tasks.iter()
        .filter_map(|task| task.id.strip_prefix(&prefix)?.parse::<u32>().ok())
        .max().unwrap_or(0) + 1;
    loop {
        let id = format!("{prefix}{next:04}");
        if !root.join(RESERVATIONS_DIR).join(format!("{id}.txt")).exists()
            && !root.join(ACTIVE_DIR).join(format!("{id}.md")).exists()
            && !root.join(ARCHIVE_DIR).join(format!("{id}.md")).exists() {
            return id;
        }
        next += 1;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskMeta {
    #[serde(default)]
    kind: TaskKind,
    #[serde(default, skip_serializing_if = "Option::is_none")] deployable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] repeat_every_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")] next_due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] last_run_at: Option<String>,
    #[serde(default)] run_count: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    depends_on: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    related_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    positioned_at: Option<String>,
    schema: u32,
    id: String,
    title: String,
    column: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claimed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    live: Option<crate::TaskLive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attention: Vec<TaskAttention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    question: Option<TaskQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    review_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    review_round: Option<u32>,
    created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
}

pub(crate) struct LockGuard {
    file: fs::File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn default_column_ai_ready(id: &str, name: &str, terminal: bool) -> bool {
    !terminal && !["draft", "done", "blocked", "review"]
        .iter()
        .any(|excluded| id.trim().eq_ignore_ascii_case(excluded) ||
            name.trim().eq_ignore_ascii_case(excluded))
}

pub fn default_board_config(name: impl Into<String>) -> BoardConfig {
    BoardConfig {
        schema: SCHEMA_VERSION,
        name: name.into(),
        releases_enabled: false,
        release_enabled_at: None,
        task_prefix: None,
        epic_prefix: None,
        columns: vec![
            Column {
                id: DRAFT_COLUMN_ID.into(),
                name: "User Drafts".into(),
                order: 0,
                terminal: false,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: false,
                advisory: false,
                auto_queue: false,
                color: "#8e7da8".into(),
            },
            Column {
                id: AI_READY_COLUMN_ID.into(),
                name: "AI Ready".into(),
                order: 1,
                terminal: false,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: true,
                advisory: false,
                auto_queue: false,
                color: "#718f78".into(),
            },
            Column {
                id: IN_PROGRESS_COLUMN_ID.into(),
                name: "In Progress".into(),
                order: 2,
                terminal: false,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: true,
                advisory: false,
                auto_queue: false,
                color: "#7388a8".into(),
            },
            Column {
                id: "blocked".into(),
                name: "Blocked".into(),
                order: 3,
                terminal: false,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: false,
                advisory: false,
                auto_queue: false,
                color: "#a48669".into(),
            },
            Column {
                id: "done".into(),
                name: "Done".into(),
                order: 4,
                terminal: true,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: false,
                advisory: false,
                auto_queue: false,
                color: "#6f8679".into(),
            },
        ],
        revision: String::new(),
    }
}

impl FsTaskStore {
    pub fn reserve_task_id(&self, root: &Path, kind: TaskKind) -> Result<TaskIdReservation, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let dir = root.join(RESERVATIONS_DIR);
        fs::create_dir_all(&dir)?;
        // Crashed or discarded dialogs cannot hold IDs indefinitely.
        for entry in fs::read_dir(&dir)?.flatten() {
            let old = entry.metadata().ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|elapsed| elapsed > Duration::from_secs(24 * 60 * 60));
            if old && entry.path().extension().is_some_and(|extension| extension == "txt") {
                let _ = fs::remove_file(entry.path());
            }
        }
        let id = next_task_id(root, &snapshot, kind);
        let token = revision(&format!("{}:{}:{}:{:?}", id, std::process::id(),
            Local::now().timestamp_nanos_opt().unwrap_or_default(), std::thread::current().id()));
        fs::write(dir.join(format!("{id}.txt")), &token)?;
        Ok(TaskIdReservation { id, token })
    }

    pub fn release_task_id(&self, root: &Path, reservation: &TaskIdReservation) -> Result<(), TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let file = root.join(RESERVATIONS_DIR).join(format!("{}.txt", reservation.id));
        if file.is_file() && fs::read_to_string(&file)? == reservation.token {
            fs::remove_file(file)?;
        }
        Ok(())
    }

    /// Store an immutable image under the synced Tasks folder. The owning task
    /// must exist or have a live New Task reservation.
    pub fn attach_task_image(
        &self, root: &Path, task_id: &str, bytes: &[u8], extension: &str,
    ) -> Result<String, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        if task_id.is_empty() || !task_id.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') {
            return Err(TaskStoreError::Invalid("invalid attachment task ID".into()));
        }
        if bytes.is_empty() || bytes.len() > 20 * 1024 * 1024 {
            return Err(TaskStoreError::Invalid("clipboard image must be 1 byte to 20 MB".into()));
        }
        if !matches!(extension, "png" | "jpg" | "webp" | "gif") {
            return Err(TaskStoreError::Invalid("unsupported clipboard image type".into()));
        }
        let is_active = root.join(ACTIVE_DIR).join(format!("{task_id}.md")).is_file();
        let is_reserved = root.join(RESERVATIONS_DIR).join(format!("{task_id}.txt")).is_file();
        if !is_active && !is_reserved {
            return Err(TaskStoreError::NotFound(task_id.into()));
        }
        let dir = root.join("Attachments").join(task_id);
        fs::create_dir_all(&dir)?;
        let digest = Sha256::digest(bytes);
        let name = format!("{digest:x}.{extension}");
        let target = dir.join(&name);
        if !target.exists() {
            // New immutable name; concurrent writes serialize through the board lock.
            fs::write(&target, bytes)?;
        }
        Ok(format!("Attachments/{task_id}/{name}"))
    }

    pub fn ensure_board(&self, root: &Path, name: &str) -> Result<(), TaskStoreError> {
        fs::create_dir_all(root.join(ACTIVE_DIR))?;
        fs::create_dir_all(root.join(ARCHIVE_DIR))?;
        let board_path = root.join(BOARD_FILE);
        if !board_path.exists() {
            let config = default_board_config(name);
            write_text_atomic(&board_path, &render_board(&config, None)?)?;
        }
        Ok(())
    }

    pub(crate) fn load_unlocked(&self, root: &Path) -> Result<BoardSnapshot, TaskStoreError> {
        self.ensure_board(root, "TaskBoard")?;
        let board_text = fs::read_to_string(root.join(BOARD_FILE))?;
        let mut config = parse_board(&board_text)?;
        config.revision = revision(&board_text);
        validate_config(&config)?;

        let mut warnings = Vec::new();
        let mut tasks = Vec::new();
        collect_tasks(&root.join(ACTIVE_DIR), false, &mut tasks, &mut warnings)?;
        collect_tasks(&root.join(ARCHIVE_DIR), true, &mut tasks, &mut warnings)?;
        tasks.sort_by(|a, b| a.id.cmp(&b.id));

        let mut ids = HashSet::new();
        for task in &tasks {
            if !ids.insert(task.id.clone()) {
                warnings.push(format!("duplicate task id {}", task.id));
            }
            if !task.archived && !config.columns.iter().any(|c| c.id == task.column) {
                warnings.push(format!("{} uses unknown column {}", task.id, task.column));
            }
            if let Err(error) = validate_links(&tasks, Some(&task.id), task.kind, &task.depends_on, &task.related_to) {
                warnings.push(format!("{}: {}", task.id, error));
            }
            if task.kind == TaskKind::Epic && task.claimed_by.is_some() {
                warnings.push(format!("{} is an epic but has a legacy claim; remove claimed_by", task.id));
            }
        }

        Ok(BoardSnapshot {
            config,
            tasks,
            warnings,
        })
    }

    // Epic columns are derived from executable descendants, not direct user/agent
    // edits. Persist the computed state under the existing board lock so raw
    // Markdown readers and the CLI see the same status as the app.
    fn maintain_epic_columns(&self, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        let mut changed = false;
        for epic in snapshot.tasks.iter().filter(|t| !t.archived && t.kind == TaskKind::Epic) {
            let desired = derived_epic_column(snapshot, &epic.id);
            if epic.column == desired { continue; }
            let path = PathBuf::from(&epic.path);
            let existing = fs::read_to_string(&path)?;
            if revision(&existing) != epic.revision {
                return Err(TaskStoreError::Conflict(format!("{} was changed during epic status reconciliation", epic.id)));
            }
            let mut updated = epic.clone();
            updated.column = desired;
            apply_completion_state(&snapshot.config, &mut updated);
            write_text_atomic(&path, &render_task(&updated, Some(&existing))?)?;
            changed = true;
        }
        Ok(changed)
    }

    // One-time reconciliation for reviews rejected before the Inbox distinguished
    // a completed human verdict from an agent's pending next submission.
    fn maintain_review_verdicts(&self, root: &Path, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        let mut changed = false;
        for task in snapshot.tasks.iter().filter(|task| !task.archived &&
            task.review_status.as_deref() == Some("open") &&
            review::awaiting_revised_round(root, &task.id)) {
            let path = PathBuf::from(&task.path);
            let existing = fs::read_to_string(&path)?;
            if revision(&existing) != task.revision {
                return Err(TaskStoreError::Conflict(format!("{} changed during review reconciliation", task.id)));
            }
            let mut updated = task.clone();
            updated.review_status = Some("awaiting_revision".into());
            updated.attention.retain(|flag| *flag != TaskAttention::Question);
            updated.question = None;
            write_text_atomic(&path, &render_task(&updated, Some(&existing))?)?;
            changed = true;
        }
        Ok(changed)
    }

    // Move dependency-blocked work back to an executable ready column once
    // every prerequisite is complete. Never release an explicitly Blocked task,
    // a human-only card, or a card without prerequisites: those may have other
    // reasons to remain in Blocked. Also runs on load so manually edited
    // Markdown and older board states are reconciled.
    fn maintain_dependency_unblocks(&self, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        let ready_column = snapshot.config.columns.iter()
            .find(|col| col.id == AI_READY_COLUMN_ID && !col.terminal)
            .or_else(|| snapshot.config.columns.iter()
                .filter(|col| col.ai_ready && !col.terminal && col.id != "blocked")
                .min_by_key(|col| col.order))
            .or_else(|| snapshot.config.columns.iter()
                .filter(|col| !col.terminal && col.id != "blocked" && col.id != DRAFT_COLUMN_ID)
                .min_by_key(|col| col.order));
        let Some(ready_column) = ready_column else { return Ok(false); };
        let mut changed = false;
        for task in snapshot.tasks.iter().filter(|task| {
            !task.archived &&
            task.kind == TaskKind::Task &&
            task.column == "blocked" &&
            !task.attention.contains(&TaskAttention::Blocked) &&
            !task.attention.contains(&TaskAttention::NeedsUser) &&
            !task.depends_on.is_empty() &&
            ensure_dependencies_complete_for(&snapshot.tasks, &task.depends_on).is_ok()
        }) {
            let path = PathBuf::from(&task.path);
            let existing = fs::read_to_string(&path)?;
            if revision(&existing) != task.revision {
                return Err(TaskStoreError::Conflict(format!("{} changed during dependency release", task.id)));
            }
            let mut released = task.clone();
            released.column = if released.claimed_by.is_some() &&
                snapshot.config.columns.iter().any(|c| c.id == IN_PROGRESS_COLUMN_ID && !c.terminal) {
                IN_PROGRESS_COLUMN_ID.to_string()
            } else {
                ready_column.id.clone()
            };
            released.blocked_reason = None;
            apply_completion_state(&snapshot.config, &mut released);
            write_text_atomic(&path, &render_task(&released, Some(&existing))?)?;
            changed = true;
        }
        Ok(changed)
    }

    /// Inherit a column Queue default only when prerequisites have become ready.
    /// No polling or dispatch occurs here; repeated loads are no-ops after the
    /// first durable mode/request transition.
    fn maintain_auto_queue(&self, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        let mut changed = false;
        for task in snapshot.tasks.iter().filter(|task| {
            !task.archived && task.completed.is_none() && task.claimed_by.is_none() &&
            matches!(task.kind, TaskKind::Task | TaskKind::Recurring) &&
            (task.attention.contains(&TaskAttention::Priority) ||
                snapshot.config.columns.iter().any(|c| c.id == task.column && c.auto_queue))
        }) {
            let path = PathBuf::from(&task.path);
            let before = fs::read_to_string(&path)?;
            if revision(&before) != task.revision { continue; }
            let after = queue_policy_text(&before, task, snapshot, Some(task))?;
            if after != before {
                write_text_atomic(&path, &after)?;
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Repair existing unanswered Inbox requests left in an AI Todo column.
    fn maintain_inbox_placement(&self, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        let mut changed = false;
        for task in snapshot.tasks.iter().filter(|task| !task.archived
            && pending_human_request(task) && is_todo(&snapshot.config, &task.column)) {
            let path = PathBuf::from(&task.path);
            let existing = fs::read_to_string(&path)?;
            if revision(&existing) != task.revision { continue; }
            let mut corrected = task.clone();
            corrected.column = IN_PROGRESS_COLUMN_ID.into();
            let rendered = update_activity(&render_task(&corrected, Some(&existing))?,
                &existing, task, &corrected, &snapshot.config)?;
            write_text_atomic(&path, &rendered)?;
            changed = true;
        }
        Ok(changed)
    }

    /// On each board read, recover genuinely idle In Progress tasks.
    /// Direct Markdown edits also count because they update the file timestamp.
    fn maintain_abandoned_tasks(&self, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        // An inactive T3 browser session is not a dead worker. A suspended, errored,
        // throttled or offline T3 session must retain its claim until a separate
        // authoritative permanently-gone receipt has completed its recovery grace.
        // External agent claims use a configurable inactivity lease instead.
        let hours = std::env::var("TASKBOARD_EXTERNAL_CLAIM_EXPIRY_HOURS")
            .ok().and_then(|v| v.parse::<i64>().ok())
            .filter(|hours| (1..=24 * 90).contains(hours))
            .unwrap_or(ABANDON_AFTER_HOURS);
        let cutoff = Utc::now() - ChronoDuration::hours(hours);
        let mut changed = false;
        for task in snapshot.tasks.iter().filter(|task| !task.archived
            && matches!(task.kind, TaskKind::Task | TaskKind::Recurring)
            && task.column == IN_PROGRESS_COLUMN_ID
            && task.completed.is_none() && !pending_human_request(task)) {
            // Never infer permanent T3 disappearance from a clock or a failed
            // inventory query. The separate session-recovery process may restore it.
            if task.claimed_by.as_deref().is_some_and(|owner| {
                let raw = owner.strip_prefix("t3-browser:").unwrap_or(owner);
                raw.len() == 36 && raw.bytes().enumerate().all(|(i, b)|
                    if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() })
            }) { continue; }
            let path = PathBuf::from(&task.path);
            let existing = fs::read_to_string(&path)?;
            if revision(&existing) != task.revision { continue; }
            let persisted = claim_metadata(&existing, "last_activity_at")?
                .and_then(|stamp| DateTime::parse_from_rfc3339(&stamp).ok())
                .map(|stamp| stamp.with_timezone(&Utc));
            let file_activity = fs::metadata(&path)?.modified().ok().map(DateTime::<Utc>::from);
            let latest = match (persisted, file_activity) {
                (Some(a), Some(b)) => Some(a.max(b)),
                (Some(a), None) | (None, Some(a)) => Some(a),
                (None, None) => None,
            };
            if latest.is_none_or(|activity| activity > cutoff) { continue; }
            let mut abandoned = task.clone();
            let previous_owner = abandoned.claimed_by.clone().unwrap_or_else(|| "unassigned".into());
            let origin = claim_metadata(&existing, "in_progress_origin")?;
            abandoned.column = return_column(&snapshot.config, origin.as_deref());
            abandoned.claimed_by = None;
            abandoned.description = append_description_update(&abandoned.description, "abandonment",
                &format!("External claim expired after {hours} hours without task updates (previous owner: {previous_owner}). Moved out of In Progress. Please finish."));
            let rendered = stamp_activity(&render_with_claim_token(&abandoned, &existing, None)?, None, true)?;
            write_text_atomic(&path, &rendered)?;
            changed = true;
        }
        Ok(changed)
    }

    /// Uncommitted creation dialogs are saved as user drafts with a marker.
    /// Expire only untouched, unreferenced drafts after 24 hours; manual saves
    /// remove the marker and ordinary user-created drafts are never pruned.
    fn maintain_auto_drafts(&self, snapshot: &BoardSnapshot) -> Result<bool, TaskStoreError> {
        let cutoff = Utc::now() - ChronoDuration::hours(24);
        let mut changed = false;
        for task in snapshot.tasks.iter().filter(|task| !task.archived && task.column == DRAFT_COLUMN_ID) {
            let Some(raw_stamp) = task.notes.split("<!-- taskboard:auto-draft:").nth(1)
                .and_then(|value| value.split(" -->").next()) else { continue; };
            let Ok(created) = DateTime::parse_from_rfc3339(raw_stamp) else { continue; };
            if created.with_timezone(&Utc) > cutoff { continue; }
            if snapshot.tasks.iter().any(|other| other.id != task.id &&
                (other.parent.as_deref() == Some(&task.id) ||
                 other.depends_on.contains(&task.id) || other.related_to.contains(&task.id))) { continue; }
            let path = PathBuf::from(&task.path);
            let metadata = fs::metadata(&path)?;
            if metadata.modified().ok().map(DateTime::<Utc>::from)
                .is_none_or(|modified| modified > cutoff) { continue; }
            let current = fs::read_to_string(&path)?;
            if revision(&current) == task.revision {
                fs::remove_file(&path)?;
                changed = true;
            }
        }
        Ok(changed)
    }

    // Expiration is reconciled by the shared core on every load, including CLI
    // loads. A snoozed column therefore wakes even if the app was closed.
    fn maintain_snoozed_columns(
        &self,
        root: &Path,
        config: &BoardConfig,
    ) -> Result<bool, TaskStoreError> {
        let now = Utc::now();
        let mut next = config.clone();
        let mut changed = false;
        for column in &mut next.columns {
            if let Some(until) = column.snoozed_until.as_deref() {
                if let Ok(expiry) = DateTime::parse_from_rfc3339(until) {
                    if expiry.with_timezone(&Utc) <= now {
                        let awake_end = expiry.with_timezone(&Utc) + ChronoDuration::hours(24);
                        column.hidden = false;
                        column.snoozed_until = None;
                        column.awake_until = (awake_end > now)
                            .then(|| awake_end.to_rfc3339_opts(SecondsFormat::Millis, true));
                        changed = true;
                    }
                }
            }
            if let Some(until) = column.awake_until.as_deref() {
                if DateTime::parse_from_rfc3339(until).is_ok_and(|expiry| expiry <= now) {
                    column.awake_until = None;
                    changed = true;
                }
            }
        }
        if changed {
            let path = root.join(BOARD_FILE);
            let existing = fs::read_to_string(&path)?;
            let text = render_board(&next, Some(&existing))?;
            write_text_atomic(&path, &text)?;
        }
        Ok(changed)
    }

    fn maintain_completed_tasks(
        &self,
        root: &Path,
        snapshot: &BoardSnapshot,
    ) -> Result<bool, TaskStoreError> {
        let today = Local::now().date_naive();
        let terminal_columns: HashSet<&str> = snapshot
            .config
            .columns
            .iter()
            .filter(|column| column.terminal)
            .map(|column| column.id.as_str())
            .collect();
        let mut active_tasks: Vec<Task> = snapshot
            .tasks
            .iter()
            .filter(|task| !task.archived)
            .cloned()
            .collect();
        let mut changed = false;

        // Normalize completion timestamps. Existing terminal tasks that predate
        // this field receive today's date, giving them a full retention period.
        for task in &mut active_tasks {
            if task.kind == TaskKind::Release { continue; }
            if task.kind == TaskKind::Recurring && terminal_columns.contains(task.column.as_str()) {
                let path = PathBuf::from(&task.path);
                let existing = fs::read_to_string(&path)?;
                apply_completion_state(&snapshot.config, task);
                let text = render_task(task, Some(&existing))?;
                write_text_atomic(&path, &text)?;
                task.revision = revision(&text);
                changed = true;
                continue;
            }
            let is_terminal = terminal_columns.contains(task.column.as_str());
            let normalized = if is_terminal {
                task.completed
                    .as_deref()
                    .and_then(parse_task_date)
                    .map(|date| date.to_string())
                    .or_else(|| Some(today.to_string()))
            } else {
                None
            };

            let stale_user_status = is_terminal && (
                task.kind == TaskKind::ForUser || task.attention.contains(&TaskAttention::NeedsUser));
            if task.completed != normalized || stale_user_status {
                let path = PathBuf::from(&task.path);
                let existing = fs::read_to_string(&path)?;
                let newly_completed = task.completed.is_none() && normalized.is_some();
                task.completed = normalized;
                if newly_completed && task.completed_at.is_none() {
                    task.completed_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true));
                }
                if stale_user_status {
                    if task.kind == TaskKind::ForUser { task.kind = TaskKind::Task; }
                    task.attention.retain(|flag| flag != &TaskAttention::NeedsUser);
                }
                if task.completed.is_none() { task.completed_at = None; }
                let text = render_task(task, Some(&existing))?;
                write_text_atomic(&path, &text)?;
                task.revision = revision(&text);
                changed = true;
            }
        }

        let mut remaining: HashSet<String> = active_tasks
            .iter()
            .filter(|task| terminal_columns.contains(task.column.as_str()))
            // Implementation can be Done while human acceptance is still open.
            .filter(|task| task.review_status.as_deref() != Some("open") &&
                !(task.attention.contains(&TaskAttention::Question) &&
                  task.question.as_ref().is_some_and(|q| q.answer.is_none())))
            .filter_map(|task| {
                let completed = task.completed.as_deref().and_then(parse_task_date)?;
                (today.signed_duration_since(completed).num_days() >= COMPLETED_RETENTION_DAYS)
                    .then(|| task.id.clone())
            })
            .collect();
        let mut active_ids: HashSet<String> =
            active_tasks.iter().map(|task| task.id.clone()).collect();

        // Archive leaves before parents so a completed hierarchy can drain in
        // one maintenance pass without leaving active children orphaned.
        loop {
            let mut progressed = false;
            let candidates: Vec<String> = remaining.iter().cloned().collect();
            for id in candidates {
                let has_active_child = active_tasks.iter().any(|task| {
                    active_ids.contains(&task.id) && task.parent.as_deref() == Some(id.as_str())
                });
                if has_active_child {
                    continue;
                }

                let Some(task) = active_tasks.iter().find(|task| task.id == id) else {
                    remaining.remove(&id);
                    continue;
                };
                let source = PathBuf::from(&task.path);
                let destination_dir = root.join(ARCHIVE_DIR).join(today.year().to_string());
                fs::create_dir_all(&destination_dir)?;
                let destination = destination_dir.join(format!("{id}.md"));
                if destination.exists() {
                    // A duplicate is already reported by load_unlocked; do not
                    // make the board unusable just because cleanup cannot proceed.
                    remaining.remove(&id);
                    continue;
                }
                fs::rename(&source, &destination)?;
                active_ids.remove(&id);
                remaining.remove(&id);
                progressed = true;
                changed = true;
            }
            if !progressed {
                break;
            }
        }

        Ok(changed)
    }
}

impl FsTaskStore {
    fn load_impl(&self, root: &Path) -> Result<BoardSnapshot, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let mut snapshot = self.load_unlocked(root)?;
        if self.maintain_snoozed_columns(root, &snapshot.config)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_review_verdicts(root, &snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_completed_tasks(root, &snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_inbox_placement(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_abandoned_tasks(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_dependency_unblocks(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_auto_queue(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_auto_drafts(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_epic_columns(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        let synced_review_bytes = review::measure_review_bytes(root);
        if synced_review_bytes > 1024 * 1024 * 1024 {
            snapshot.warnings.push(format!(
                "Review media above 1 GB soft cap ({} MB). Nothing blocked.",
                synced_review_bytes / (1024 * 1024)));
        }
        let stale = review::stale_open_review_ids(root);
        if !stale.is_empty() {
            snapshot.warnings.push(format!(
                "Open reviews older than 30 days: {} (never auto-deleted).",
                stale.join(", ")));
        }
        crate::release::reconcile_unlocked(root, &snapshot.config, &snapshot.tasks, false)?;
        if crate::release::reconcile_repairs_unlocked(root)? {
            snapshot = self.load_unlocked(root)?;
        }
        Ok(snapshot)
    }

    /// `create` with an explicit actor. The desktop app creates as `Human`;
    /// agents/CLI use the trait method, which requires a blocked reason.
    /** OW-0023: phone archive must compare the exact source revision inside
     * the PC TaskBoard core's existing board lock, not in a separate preflight.
     * Existing desktop archive behavior remains unchanged. */
    pub fn archive_if_revision(&self, root: &Path, id: &str, expected: &str) -> Result<Task, TaskStoreError> {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TaskStoreError::Invalid("phone archive requires an exact SHA-256 revision".into()));
        }
        let _lock = acquire_lock(root)?;
        let source = find_task_path(root, id)?;
        if source.starts_with(root.join(ARCHIVE_DIR)) {
            return Err(TaskStoreError::Conflict(format!("{id} is already archived")));
        }
        let existing = fs::read_to_string(&source)?;
        if revision(&existing) != expected {
            return Err(TaskStoreError::Conflict(format!("{id} changed since the phone snapshot")));
        }
        let snapshot = self.load_unlocked(root)?;
        if snapshot.tasks.iter().any(|task| !task.archived && task.parent.as_deref() == Some(id)) {
            return Err(TaskStoreError::Invalid(format!("{id} has active subtasks")));
        }
        let year = Local::now().year().to_string();
        let destination_dir = root.join(ARCHIVE_DIR).join(year);
        fs::create_dir_all(&destination_dir)?;
        let destination = destination_dir.join(format!("{id}.md"));
        if destination.exists() {
            return Err(TaskStoreError::Conflict(format!("archive already contains {id}")));
        }
        fs::rename(&source, &destination)?;
        parse_task(&destination, true, &existing)
    }

    /// OW-0023: the phone's create request validates the source board revision
    /// under the SAME lock as ID allocation and Markdown creation.
    /// OW-0023: phone column edits have exact-revision CAS AND referential
    /// integrity for every active task, under one canonical TaskBoard lock.
    /// A column cannot disappear while task cards still point to it.
    /** OW-0023: the PC alone creates project TaskBoards inside the configured
     * vault. Atomic directory creation prevents colliding phone/PC project
     * names; leave partial state visible on I/O failure, never erase an
     * unexpected concurrent file. */
    pub fn create_project_in_vault(&self, vault: &Path, name: &str) -> Result<PathBuf, TaskStoreError> {
        let name = name.trim();
        if !(2..=60).contains(&name.chars().count()) ||
            !name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ' || c == '_' || c == '-') {
            return Err(TaskStoreError::Invalid("project name must be 2..60 characters (letters, digits, space, hyphen, underscore)".into()));
        }
        if matches!(name.to_ascii_uppercase().as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" |
            "LPT1" | "LPT2" | "LPT3") {
            return Err(TaskStoreError::Invalid("reserved Windows project folder name".into()));
        }
        let vault = fs::canonicalize(vault)?;
        let project = vault.join(name);
        fs::create_dir(&project)?;
        let tasks = project.join("Tasks");
        fs::create_dir(&tasks)?;
        let mut config = default_board_config(name);
        let letters: String = name.chars().filter(|ch| ch.is_ascii_alphanumeric())
            .map(|ch| ch.to_ascii_uppercase()).take(9).collect();
        let prefix = if letters.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic()) {
            letters
        } else {
            format!("TB{letters}")
        };
        let prefix = format!("{prefix:T<2}").chars().take(10).collect::<String>();
        config.task_prefix = Some(prefix.clone());
        config.epic_prefix = Some(format!("{prefix}E").chars().take(11).collect());
        validate_config(&config)?;
        let text = render_board(&config, None)?;
        write_new(&tasks.join(BOARD_FILE), &text)?;
        fs::create_dir(&tasks.join(ACTIVE_DIR))?;
        fs::create_dir(&tasks.join(ARCHIVE_DIR))?;
        Ok(tasks)
    }

    pub fn save_phone_columns(&self, root: &Path, columns: Vec<Column>, expected: &str) -> Result<BoardConfig, TaskStoreError> {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TaskStoreError::Invalid("phone board edit needs exact revision".into()));
        }
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        if snapshot.config.revision != expected {
            return Err(TaskStoreError::Conflict("board changed since phone snapshot".into()));
        }
        if snapshot.tasks.iter().any(|task| !task.archived &&
            !columns.iter().any(|column| column.id == task.column)) {
            return Err(TaskStoreError::Conflict("a column still holds active tasks; move them first".into()));
        }
        let mut config = snapshot.config;
        config.columns = columns;
        validate_config(&config)?;
        let path = root.join(BOARD_FILE);
        let original = fs::read_to_string(&path)?;
        if revision(&original) != expected {
            return Err(TaskStoreError::Conflict("board changed during phone save".into()));
        }
        let text = render_board(&config, Some(&original))?;
        write_text_atomic(&path, &text)?;
        config.revision = revision(&text);
        Ok(config)
    }

    pub fn create_if_board_revision(&self, root: &Path, draft: TaskDraft, expected: &str) -> Result<Task, TaskStoreError> {
        if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(TaskStoreError::Invalid("phone create requires exact board revision".into()));
        }
        let _lock = acquire_lock(root)?;
        let current = self.load_unlocked(root)?;
        if current.config.revision != expected {
            return Err(TaskStoreError::Conflict("board changed since phone snapshot".into()));
        }
        self.create_as_unlocked(root, draft, MutationActor::Human)
    }

    pub fn create_as(&self, root: &Path, draft: TaskDraft, actor: MutationActor) -> Result<Task, TaskStoreError> {
        self.create_as_confirmed(root, draft, actor, None)
    }

    /// Agent confirmation is verified under the same board lock as creation.
    /// The ordinary public Agent create route cannot bypass the review.
    pub fn create_as_confirmed(&self, root: &Path, draft: TaskDraft, actor: MutationActor, confirmation: Option<&str>) -> Result<Task, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let blocking = draft.column == BLOCKED_COLUMN_ID || draft.attention.contains(&TaskAttention::Blocked);
        if actor == MutationActor::Agent && blocking {
            crate::block_gate::challenge_agent_block_unlocked(root, &format!("new:{}", draft.title), "create", draft.blocked_reason.as_deref().unwrap_or(""), confirmation)?;
        } else if confirmation.is_some() {
            return Err(TaskStoreError::Invalid("blocked_confirmation_invalid: no new agent Blocked transition".into()));
        }
        self.create_as_unlocked(root, draft, actor)
    }

    pub(crate) fn create_as_unlocked(&self, root: &Path, draft: TaskDraft, actor: MutationActor) -> Result<Task, TaskStoreError> {
        let snapshot = self.load_unlocked(root)?;
        validate_draft(&snapshot, &draft)?;

        let reserved = match (&draft.reserved_id, &draft.reservation_token) {
            (None, None) => None,
            (Some(id), Some(token)) => {
                // Require a live reservation for the exact task/epic namespace.
                let configured = if draft.kind == TaskKind::Epic {
                    snapshot.config.epic_prefix.as_deref().unwrap_or("WTE")
                } else {
                    snapshot.config.task_prefix.as_deref().unwrap_or("WT")
                };
                if !id.strip_prefix(&format!("{configured}-"))
                    .is_some_and(|digits| digits.len() >= 4 && digits.bytes().all(|b| b.is_ascii_digit())) {
                    return Err(TaskStoreError::Invalid("reserved ID has wrong task kind or prefix".into()));
                }
                let file = root.join(RESERVATIONS_DIR).join(format!("{id}.txt"));
                if !file.is_file() || fs::read_to_string(&file)? != *token {
                    return Err(TaskStoreError::Conflict(format!("{id} reservation expired or changed")));
                }
                Some((id.clone(), file))
            }
            _ => return Err(TaskStoreError::Invalid("reserved ID and reservation token must be supplied together".into())),
        };
        let id = reserved.as_ref().map(|(id, _)| id.clone())
            .unwrap_or_else(|| next_task_id(root, &snapshot, draft.kind));
        let created = Local::now().date_naive().to_string();
        let path = root.join(ACTIVE_DIR).join(format!("{id}.md"));
        if path.exists() {
            return Err(TaskStoreError::Conflict(format!("{id} already exists")));
        }

        let mut task = Task {
            schema: SCHEMA_VERSION,
            id,
            kind: draft.kind,
            deployable: draft.deployable,
            repeat_every_days: draft.repeat_every_days,
            next_due: draft.next_due,
            last_run_at: None,
            run_count: 0,
            depends_on: draft.depends_on,
            related_to: draft.related_to,
            position_after: None,
            positioned_at: None,
            title: draft.title.trim().to_string(),
            column: draft.column,
            parent: draft.parent,
            category: draft.category,
            icon: draft.icon,
            claimed_by: None,
            live: None,
            attention: draft.attention,
            question: draft.question,
            blocked_reason: draft.blocked_reason,
            review_status: None,
            review_round: None,
            created,
            completed: None,
            completed_at: None,
            description: draft.description,
            acceptance_criteria: draft.acceptance_criteria,
            ai_notes: draft.ai_notes,
            user_notes: draft.user_notes,
            notes: draft.notes,
            archived: false,
            path: path.to_string_lossy().to_string(),
            revision: String::new(),
        };
        if task.kind == TaskKind::Epic {
            task.column = DRAFT_COLUMN_ID.to_string();
        } else if pending_human_request(&task) && is_todo(&snapshot.config, &task.column) {
            task.column = IN_PROGRESS_COLUMN_ID.into();
        }
        apply_completion_state(&snapshot.config, &mut task);
        apply_blocked_state(None, &mut task, actor)?;
        validate_task(&snapshot, &task)?;
        let text = queue_policy_text(&render_task(&task, None)?, &task, &snapshot, None)?;
        write_new(&path, &text)?;
        if let Some((_, reservation_file)) = reserved {
            // Task creation is already durable; stale reservation cleanup is safe.
            let _ = fs::remove_file(reservation_file);
        }
        task.revision = revision(&text);
        mirror_local_related_links(root, &snapshot, &task.id, &[], &task.related_to)?;
        if !task.archived && task.completed.is_some() {
            let current = self.load_unlocked(root)?;
            crate::release::reconcile_unlocked(root, &current.config, &current.tasks, false)?;
        }
        Ok(task)
    }

    /// Preview completed linked implementation reviews, or replay them under the
    /// board lock. Only explicit, unprocessed reviews are candidates.
    pub fn reconcile_completed_reviews(&self, root: &Path, apply: bool) -> Result<Vec<String>, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let candidates: Vec<Task> = snapshot.tasks.iter()
            .filter(|task| task.kind == TaskKind::ForUser
                && task.completed.is_some()
                && review_link::is_implementation_review(task))
            .cloned().collect();
        let mut report = Vec::new();
        for review in candidates {
            // Re-read after each applied review: multiple reviews of one parent
            // must never overwrite another review's reconciliation.
            let current = if apply { self.load_unlocked(root)? } else { snapshot.clone() };
            if let Some(result) = review_link::reconcile(&current, &review, !apply)? {
                report.push(result);
            }
        }
        Ok(report)
    }
    /// `update` with an explicit actor. Human edits can never author or change
    /// `blocked_reason`; agent edits must supply one when entering Blocked.
    pub fn update_as(&self, root: &Path, task: Task, actor: MutationActor) -> Result<Task, TaskStoreError> {
        self.update_as_confirmed(root, task, actor, None, "update")
    }

    /// Validates a one-use agent Blocked confirmation inside the transaction.
    /// `action` distinguishes move from update, matching the CLI challenge.
    pub fn update_as_confirmed(&self, root: &Path, mut task: Task, actor: MutationActor, confirmation: Option<&str>, action: &str) -> Result<Task, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        let path = find_task_path(root, &task.id)?;
        let existing = fs::read_to_string(&path)?;
        if !task.revision.is_empty() && revision(&existing) != task.revision {
            return Err(TaskStoreError::Conflict(format!(
                "{} changed on disk",
                task.id
            )));
        }

        task.archived = path.starts_with(root.join(ARCHIVE_DIR));
        task.path = path.to_string_lossy().to_string();
        let previous = parse_task(&path, task.archived, &existing)?;
        // A user opening and explicitly saving an auto-draft adopts it as a
        // normal Draft. Never expire a draft the human intentionally retained.
        if actor == MutationActor::Human && previous.notes.contains("<!-- taskboard:auto-draft:") {
            task.notes = task.notes.lines().filter(|line|
                !line.trim().starts_with("<!-- taskboard:auto-draft:")).collect::<Vec<_>>().join("\n");
        }
        if actor == MutationActor::Human && previous.kind == TaskKind::ForUser &&
            review_link::is_implementation_review(&task) {
            let answer = task.question.as_ref().and_then(|q| q.answer.as_deref()).unwrap_or("").trim();
            if ["approve", "approved", "accepted", "needs changes", "rejected"].iter()
                .any(|value| answer.eq_ignore_ascii_case(value)) {
                if let Some(done) = snapshot.config.columns.iter().find(|col| col.terminal) {
                    task.column = done.id.clone();
                }
            }
        }
        if previous.kind == TaskKind::Epic && task.column != previous.column {
            return Err(TaskStoreError::Invalid(format!("{} is an epic: its column is derived from child tasks and cannot be moved directly", task.id)));
        }
        if matches!(task.kind, TaskKind::Epic | TaskKind::ForUser) && task.claimed_by.is_some() {
            return Err(TaskStoreError::Invalid("epics and For user tasks cannot be claimed or assigned to agents".into()));
        }
        if !task.archived && task.kind != TaskKind::Epic
           && previous.column != task.column
           && (task.column == IN_PROGRESS_COLUMN_ID || snapshot.config.columns.iter().any(|c| c.id == task.column && c.terminal)) {
            ensure_dependencies_complete(&snapshot, &task)?;
        }
        if !task.archived && pending_human_request(&task) && is_todo(&snapshot.config, &task.column)
            && (!pending_human_request(&previous) || is_todo(&snapshot.config, &previous.column)) {
            task.column = IN_PROGRESS_COLUMN_ID.into();
        }
        // A human moving a task out of the working column is also revoking
        // the old worker's authority. Preserve normal Inbox answers but never
        // leave an orphan claimant on a nonworking task.
        if actor == MutationActor::Human && task.column != IN_PROGRESS_COLUMN_ID {
            task.claimed_by = None;
        }
        if !task.archived {
            apply_completion_state(&snapshot.config, &mut task);
        }
        review_link::close_question(&mut task,
            actor == MutationActor::Human && previous.kind == TaskKind::ForUser);
        let becoming_blocked = !is_task_blocked(&previous) && is_task_blocked(&task);
        if actor == MutationActor::Agent && becoming_blocked {
            crate::block_gate::challenge_agent_block_unlocked(root, &task.id, action, task.blocked_reason.as_deref().unwrap_or(""), confirmation)?;
        } else if confirmation.is_some() {
            return Err(TaskStoreError::Invalid("blocked_confirmation_invalid: no new agent Blocked transition".into()));
        }
        apply_blocked_state(Some(&previous), &mut task, actor)?;
        validate_task(&snapshot, &task)?;
        // An advisory is temporary board-wide information. Completing it
        // removes the Markdown card instead of leaving a misleading Done task.
        // The update remains protected by the revision check + board lock.
        let was_advisory = snapshot.config.columns.iter()
            .any(|column| column.id == previous.column && column.advisory);
        let now_terminal = snapshot.config.columns.iter()
            .any(|column| column.id == task.column && column.terminal);
        if was_advisory && now_terminal && !task.archived {
            fs::remove_file(&path)?;
            task.revision.clear();
            return Ok(task);
        }
        let rendered = if task.claimed_by != previous.claimed_by {
            render_with_claim_token(&task, &existing, None)?
        } else { render_task(&task, Some(&existing))? };
        let rendered = queue_policy_text(&rendered, &task, &snapshot, Some(&previous))?;
        let text = update_activity(&rendered, &existing, &previous, &task, &snapshot.config)?;
        write_text_atomic(&path, &text)?;
        task.revision = revision(&text);
        if previous.related_to != task.related_to {
            mirror_local_related_links(root, &snapshot, &task.id, &previous.related_to, &task.related_to)?;
        }
        if actor == MutationActor::Human && previous.kind == TaskKind::ForUser && previous.completed.is_none() && task.completed.is_some() {
            let _ = review_link::reconcile(&snapshot, &task, false)?;
        }
        // Complete dependent release during the same board lock. The next load
        // also covers out-of-band edits and archived prerequisites.
        if !task.archived && task.completed.is_some() && previous.completed.is_none() {
            let current = self.load_unlocked(root)?;
            self.maintain_dependency_unblocks(&current)?;
        }
        let current = self.load_unlocked(root)?;
        crate::release::reconcile_unlocked(root, &current.config, &current.tasks, false)?;
        Ok(task)
    }
}


/// Mirror added/removed informational links in the destination task. The
/// same path works for edits from the desktop and CLI. Cross-project links
/// use the absolute source board plus '#task-id' as their inverse identity.
fn mirror_local_related_links(
    root: &Path, snapshot: &BoardSnapshot, source: &str,
    previous: &[String], current: &[String],
) -> Result<(), TaskStoreError> {
    let old: HashSet<&str> = previous.iter().map(String::as_str).collect();
    let new: HashSet<&str> = current.iter().map(String::as_str).collect();
    for reference in old.symmetric_difference(&new) {
        let external = reference.rsplit_once('#');
        let (destination, id) = external.map(|(board, id)| (Path::new(board), id))
            .unwrap_or((root, *reference));
        // Remote board files must be protected against concurrent writes too.
        // A contended remote board fails explicitly instead of overwriting it.
        let _remote_lock = if external.is_some() && destination != root {
            Some(acquire_lock(destination)?)
        } else { None };
        let path = if external.is_none() {
            snapshot.tasks.iter().find(|task| task.id == id)
                .map(|_| find_task_path(root, id)).transpose()?
        } else { Some(find_task_path(destination, id)?) };
        let Some(path) = path else { continue; };
        let original = fs::read_to_string(&path)?;
        let mut target = parse_task(&path, path.starts_with(destination.join(ARCHIVE_DIR)), &original)?;
        let reverse = external.is_some().then(|| format!("{}#{source}", root.display()))
            .unwrap_or_else(|| source.to_owned());
        if new.contains(reference) {
            if !target.related_to.iter().any(|link| link == &reverse) {
                target.related_to.push(reverse);
            }
        } else {
            target.related_to.retain(|link| link != &reverse);
        }
        let rendered = render_task(&target, Some(&original))?;
        if rendered != original { write_text_atomic(&path, &rendered)?; }
    }
    Ok(())
}

const ABANDON_AFTER_HOURS: i64 = 24;

/// Human Inbox requests remain in place even when their agent is inactive.
fn pending_human_request(task: &Task) -> bool {
    task.review_status.as_deref() == Some("open")
        || (task.attention.contains(&TaskAttention::Question)
            && task.question.as_ref().is_some_and(|q| q.answer.is_none() && !q.prompt.trim().is_empty()))
        || task.attention.contains(&TaskAttention::NeedsUser)
}

fn is_todo(config: &BoardConfig, column: &str) -> bool {
    column != IN_PROGRESS_COLUMN_ID && column != DRAFT_COLUMN_ID
        && config.columns.iter().any(|item| item.id == column
            && !item.terminal && !item.advisory && item.ai_ready)
}

fn return_column(config: &BoardConfig, original: Option<&str>) -> String {
    original.filter(|column| is_todo(config, column))
        .map(str::to_owned)
        .unwrap_or_else(|| config.columns.iter()
            .find(|c| c.id == AI_READY_COLUMN_ID && !c.terminal)
            .or_else(|| config.columns.iter().filter(|c| is_todo(config, &c.id))
                .min_by_key(|c| c.order))
            .map(|c| c.id.clone()).unwrap_or_else(|| AI_READY_COLUMN_ID.to_owned()))
}

/// Maintain origin and activity timestamps in existing Markdown frontmatter.
/// Legacy cards without an origin return to AI Ready.
fn stamp_activity(rendered: &str, origin: Option<&str>, touch: bool) -> Result<String, TaskStoreError> {
    let (yaml, body) = split_frontmatter(rendered)?;
    let mut map: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
        .map_err(|error| TaskStoreError::Parse(error.to_string()))?;
    let key = serde_yaml::Value::String("in_progress_origin".into());
    map.remove(&key);
    if let Some(origin) = origin {
        map.insert(key, serde_yaml::Value::String(origin.to_owned()));
    }
    if touch {
        map.insert(serde_yaml::Value::String("last_activity_at".into()),
            serde_yaml::Value::String(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)));
    }
    Ok(format!("---\n{}---\n{}", serde_yaml::to_string(&map)
        .map_err(|error| TaskStoreError::Parse(error.to_string()))?, body))
}

fn update_activity(rendered: &str, existing: &str, before: &Task, after: &Task, config: &BoardConfig)
    -> Result<String, TaskStoreError> {
    let origin = if after.column == IN_PROGRESS_COLUMN_ID {
        if before.column == IN_PROGRESS_COLUMN_ID {
            claim_metadata(existing, "in_progress_origin")?
                .or_else(|| Some(return_column(config, None)))
        } else {
            Some(return_column(config, Some(&before.column)))
        }
    } else { None };
    stamp_activity(rendered, origin.as_deref(), before != after)
}

fn release_to_original(task: &mut Task, config: &BoardConfig, existing: &str)
    -> Result<(), TaskStoreError> {
    if task.column == IN_PROGRESS_COLUMN_ID && !pending_human_request(task) && task.completed.is_none() {
        let origin = claim_metadata(existing, "in_progress_origin")?;
        task.column = return_column(config, origin.as_deref());
    }
    task.claimed_by = None;
    Ok(())
}

/// TaskBoard-owned lifetime token; separate from OpenRig claimant generation.
#[derive(Debug, Clone, Serialize)]
pub struct ManagedClaim {
    pub task: Task,
    pub claim_token: String,
}

fn claim_token(text: &str) -> Result<Option<String>, TaskStoreError> {
    claim_metadata(text, "claim_token")
}

fn claim_metadata(text: &str, key: &str) -> Result<Option<String>, TaskStoreError> {
    let (yaml, _) = split_frontmatter(text)?;
    let mapping: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    Ok(mapping.get(serde_yaml::Value::String(key.into()))
        .and_then(serde_yaml::Value::as_str).map(str::to_owned))
}

fn render_with_claim_token(task: &Task, existing: &str, token: Option<&str>) -> Result<String, TaskStoreError> {
    let rendered = render_task(task, Some(existing))?;
    let (yaml, body) = split_frontmatter(&rendered)?;
    let mut mapping: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    mapping.remove(serde_yaml::Value::String("claim_token".into()));
    if let Some(token) = token {
        mapping.insert(serde_yaml::Value::String("claim_token".into()),
            serde_yaml::Value::String(token.to_owned()));
    }
    let yaml = serde_yaml::to_string(&mapping).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    Ok(format!("---\n{yaml}---\n{body}"))
}

impl FsTaskStore {
    /// A managed claim never upgrades an existing manual/name-only claim.
    pub fn claim_managed(&self, root: &Path, id: &str, agent: &str) -> Result<ManagedClaim, TaskStoreError> {
        let agent = agent.trim();
        if agent.is_empty() { return Err(TaskStoreError::Invalid("agent name cannot be empty".into())); }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        if path.starts_with(root.join(ARCHIVE_DIR)) {
            return Err(TaskStoreError::Invalid(format!("cannot claim archived task {id}")));
        }
        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, false, &existing)?;
        if !matches!(task.kind, TaskKind::Task | TaskKind::Recurring)
            || task.attention.contains(&TaskAttention::NeedsUser)
            || !recurring_due(&task) {
            return Err(TaskStoreError::Invalid("task is not executable or its recurrence is not due".into()));
        }
        let snapshot = self.load_unlocked(root)?;
        ensure_dependencies_complete(&snapshot, &task)?;
        if snapshot.config.columns.iter().any(|col| col.id == task.column && col.terminal)
            || !snapshot.config.columns.iter().any(|col| col.id == IN_PROGRESS_COLUMN_ID) {
            return Err(TaskStoreError::Invalid("claim requires an unfinished task and an In Progress column".into()));
        }
        if let Some(owner) = task.claimed_by.as_deref() {
            return Err(if owner == agent {
                TaskStoreError::ClaimInstanceMismatch(format!("{id}: existing claim requires its original token; cannot recover it from a display name"))
            } else {
                TaskStoreError::AlreadyClaimed { task_id: id.into(), claimed_by: owner.into() }
            });
        }
        let origin = if task.column == IN_PROGRESS_COLUMN_ID {
            claim_metadata(&existing, "in_progress_origin")?
                .unwrap_or_else(|| return_column(&snapshot.config, None))
        } else {
            return_column(&snapshot.config, Some(&task.column))
        };
        task.column = IN_PROGRESS_COLUMN_ID.into();
        let token = uuid::Uuid::new_v4().to_string();
        task.claimed_by = Some(agent.to_owned());
        let rendered = stamp_activity(&render_with_claim_token(&task, &existing, Some(&token))?,
            Some(&origin), true)?;
        write_text_atomic(&path, &rendered)?;
        task.revision = revision(&rendered);
        Ok(ManagedClaim { task, claim_token: token })
    }

    /// Conditional acceptance checks both ownership lifetime and the full content
    /// revision so changed criteria cannot be silently accepted from a stale result.
    pub fn complete_managed(&self, root: &Path, id: &str, agent: &str, token: &str, expected_revision: &str) -> Result<Task, TaskStoreError> {
        if token.is_empty() || expected_revision.is_empty() {
            return Err(TaskStoreError::Invalid("managed completion requires claim token and content revision".into()));
        }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        if path.starts_with(root.join(ARCHIVE_DIR)) {
            return Err(TaskStoreError::Invalid("cannot complete archived task".into()));
        }
        let existing = fs::read_to_string(&path)?;
        if claim_token(&existing)?.as_deref() != Some(token) {
            return Err(TaskStoreError::ClaimInstanceMismatch(format!("{id}: claim lifetime changed; no changes made")));
        }
        if revision(&existing) != expected_revision {
            return Err(TaskStoreError::Conflict(format!("{id}: task content or criteria changed; re-read before accepting result")));
        }
        let mut task = parse_task(&path, false, &existing)?;
        if task.claimed_by.as_deref() != Some(agent) {
            return Err(TaskStoreError::ClaimInstanceMismatch(format!("{id}: owner mismatch")));
        }
        let snapshot = self.load_unlocked(root)?;
        ensure_dependencies_complete(&snapshot, &task)?;
        if !recurring_due(&task) {
            return Err(TaskStoreError::Invalid("recurring task is not yet due".into()));
        }
        let terminal = snapshot.config.columns.iter().filter(|column| column.terminal)
            .min_by_key(|column| column.order)
            .ok_or_else(|| TaskStoreError::Invalid("board has no terminal column".into()))?;
        task.column = terminal.id.clone();
        apply_completion_state(&snapshot.config, &mut task);
        validate_task(&snapshot, &task)?;
        let rendered = if task.kind == TaskKind::Recurring {
            render_with_claim_token(&task, &existing, None)?
        } else { render_task(&task, Some(&existing))? };
        write_text_atomic(&path, &rendered)?;
        task.revision = revision(&rendered);
        let current = self.load_unlocked(root)?;
        self.maintain_dependency_unblocks(&current)?;
        Ok(task)
    }

    /// Conditional under board lock. Same token + already released is idempotent only
    /// if no new claim exists; an absent claim has no verified history after restart.
    pub fn release_managed(&self, root: &Path, id: &str, agent: &str, expected: &str) -> Result<Task, TaskStoreError> {
        if expected.is_empty() { return Err(TaskStoreError::ClaimInstanceMismatch("missing token".into())); }
        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, path.starts_with(root.join(ARCHIVE_DIR)), &existing)?;
        let actual = claim_token(&existing)?;
        if task.claimed_by.is_none() && claim_metadata(&existing, "last_released_claim_token")?.as_deref() == Some(expected) {
            return Ok(task);
        }
        if task.claimed_by.as_deref() != Some(agent) || actual.as_deref() != Some(expected) {
            return Err(TaskStoreError::ClaimInstanceMismatch(format!("{id}: expected token does not match current claim; no changes made")));
        }
        let snapshot = self.load_unlocked(root)?;
        release_to_original(&mut task, &snapshot.config, &existing)?;
        let rendered = stamp_activity(&render_with_claim_token(&task, &existing, None)?, None, true)?;
        let (yaml, body) = split_frontmatter(&rendered)?;
        let mut map: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
        map.insert(serde_yaml::Value::String("last_released_claim_token".into()),
            serde_yaml::Value::String(expected.into()));
        let rendered = format!("---\n{}---\n{}", serde_yaml::to_string(&map)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?, body);
        write_text_atomic(&path, &rendered)?;
        task.revision = revision(&rendered);
        Ok(task)
    }
}

impl TaskStore for FsTaskStore {
    fn load(&self, root: &Path) -> Result<BoardSnapshot, TaskStoreError> {
        self.load_impl(root)
    }

    fn create(&self, root: &Path, draft: TaskDraft) -> Result<Task, TaskStoreError> {
        self.create_as(root, draft, MutationActor::Agent)
    }

    fn update(&self, root: &Path, task: Task) -> Result<Task, TaskStoreError> {
        self.update_as(root, task, MutationActor::Agent)
    }

    fn claim(&self, root: &Path, id: &str, agent: &str) -> Result<Task, TaskStoreError> {
        let agent = agent.trim();
        if agent.is_empty() {
            return Err(TaskStoreError::Invalid("agent name cannot be empty".into()));
        }

        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        if path.starts_with(root.join(ARCHIVE_DIR)) {
            return Err(TaskStoreError::Invalid(format!(
                "cannot claim archived task {id}"
            )));
        }

        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, false, &existing)?;
        if task.kind == TaskKind::Epic {
            return Err(TaskStoreError::Invalid(format!("{} is an EPIC, not executable. Select a subtask; epics cannot be claimed or assigned", task.id)));
        }
        if task.kind == TaskKind::ForUser || task.attention.contains(&TaskAttention::NeedsUser) {
            return Err(TaskStoreError::Invalid(format!("{} is a FOR USER task requiring user input: agents may not claim it", task.id)));
        }
        let snapshot = self.load_unlocked(root)?;
        ensure_dependencies_complete(&snapshot, &task)?;
        if snapshot.config.columns.iter().any(|col| col.id == task.column && col.terminal)
            || !snapshot.config.columns.iter().any(|col| col.id == IN_PROGRESS_COLUMN_ID) {
            return Err(TaskStoreError::Invalid("claim requires an unfinished task and an In Progress column".into()));
        }
        if !recurring_due(&task) {
            return Err(TaskStoreError::Invalid("recurring task is not yet due".into()));
        }
        if let Some(owner) = task.claimed_by.as_deref() {
            if owner != agent {
                return Err(TaskStoreError::AlreadyClaimed {
                    task_id: id.to_string(),
                    claimed_by: owner.to_string(),
                });
            }
            return Ok(task);
        }

        let origin = if task.column == IN_PROGRESS_COLUMN_ID {
            claim_metadata(&existing, "in_progress_origin")?
                .unwrap_or_else(|| return_column(&snapshot.config, None))
        } else {
            return_column(&snapshot.config, Some(&task.column))
        };
        task.column = IN_PROGRESS_COLUMN_ID.into();
        task.claimed_by = Some(agent.to_string());
        let text = stamp_activity(&render_task(&task, Some(&existing))?, Some(&origin), true)?;
        write_text_atomic(&path, &text)?;
        task.revision = revision(&text);
        Ok(task)
    }

    fn release_claim(&self, root: &Path, id: &str, agent: &str) -> Result<Task, TaskStoreError> {
        let agent = agent.trim();
        if agent.is_empty() {
            return Err(TaskStoreError::Invalid("agent name cannot be empty".into()));
        }

        let _lock = acquire_lock(root)?;
        let path = find_task_path(root, id)?;
        let existing = fs::read_to_string(&path)?;
        let mut task = parse_task(&path, path.starts_with(root.join(ARCHIVE_DIR)), &existing)?;

        match task.claimed_by.as_deref() {
            None => return Ok(task),
            Some(owner) if owner != agent => {
                return Err(TaskStoreError::AlreadyClaimed {
                    task_id: id.to_string(),
                    claimed_by: owner.to_string(),
                });
            }
            Some(_) => {}
        }

        if claim_token(&existing)?.is_some() {
            return Err(TaskStoreError::ClaimInstanceMismatch(format!("{id}: managed claim requires token-based release")));
        }
        let snapshot = self.load_unlocked(root)?;
        release_to_original(&mut task, &snapshot.config, &existing)?;
        let text = stamp_activity(&render_task(&task, Some(&existing))?, None, true)?;
        write_text_atomic(&path, &text)?;
        task.revision = revision(&text);
        Ok(task)
    }

    fn archive(&self, root: &Path, id: &str) -> Result<Task, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let source = find_task_path(root, id)?;
        if source.starts_with(root.join(ARCHIVE_DIR)) {
            let text = fs::read_to_string(&source)?;
            return parse_task(&source, true, &text);
        }
        let snapshot = self.load_unlocked(root)?;
        if snapshot.tasks.iter().any(|task| !task.archived && task.parent.as_deref() == Some(id)) {
            return Err(TaskStoreError::Invalid(format!(
                "cannot archive {id} while active subtasks exist; complete, archive or detach them first"
            )));
        }

        let year = Local::now().year().to_string();
        let destination_dir = root.join(ARCHIVE_DIR).join(year);
        fs::create_dir_all(&destination_dir)?;
        let destination = destination_dir.join(format!("{id}.md"));
        if destination.exists() {
            return Err(TaskStoreError::Conflict(format!(
                "archive already contains {id}"
            )));
        }
        // Explicit human archival revokes ownership and future Queue intent,
        // while retaining exact old queue request/receipt IDs for safe remote
        // cancellation on the next reconciliation (never blindly retry).
        let old_text = fs::read_to_string(&source)?;
        let mut archived = parse_task(&source, false, &old_text)?;
        archived.claimed_by = None;
        archived.live = None;
        let rendered = render_task(&archived, Some(&old_text))?;
        let (yaml, body) = split_frontmatter(&rendered)?;
        let mut metadata: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
        metadata.insert(serde_yaml::Value::String("execution_mode".into()),
            serde_yaml::Value::String("backlog".into()));
        metadata.insert(serde_yaml::Value::String("queue_manual".into()),
            serde_yaml::Value::Bool(true));
        write_text_atomic(&source, &format!("---\n{}---\n{}", serde_yaml::to_string(&metadata)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?, body))?;
        fs::rename(&source, &destination)?;
        let text = fs::read_to_string(&destination)?;
        parse_task(&destination, true, &text)
    }

    fn archive_column(
        &self,
        root: &Path,
        column_id: &str,
        expected: &[(String, String)],
    ) -> Result<BoardSnapshot, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        if !snapshot.config.columns.iter().any(|column| column.id == column_id) {
            return Err(TaskStoreError::Invalid(format!("unknown column {column_id}")));
        }
        let selected: Vec<&Task> = snapshot.tasks.iter()
            .filter(|task| !task.archived && task.column == column_id)
            .collect();
        let revisions: HashMap<&str, &str> = expected.iter()
            .map(|(id, rev)| (id.as_str(), rev.as_str())).collect();
        if revisions.len() != selected.len() || selected.iter().any(|task| {
            revisions.get(task.id.as_str()) != Some(&task.revision.as_str())
        }) {
            return Err(TaskStoreError::Conflict(
                "column membership or task contents changed; refresh before archiving all".into(),
            ));
        }
        let selected_ids: HashSet<&str> = selected.iter().map(|task| task.id.as_str()).collect();
        // A bulk archive should never strand active subtasks in other columns.
        // Detach only immediate children outside this column; leave all their
        // other fields, descendants and active work completely untouched.
        // Retain original bytes for rollback and for snapshot-level Undo.
        let mut detachments: Vec<(PathBuf, String, String)> = Vec::new();
        for child in snapshot.tasks.iter().filter(|task| {
            !task.archived && !selected_ids.contains(task.id.as_str())
                && task.parent.as_deref().is_some_and(|parent| selected_ids.contains(parent))
        }) {
            let path = PathBuf::from(&child.path);
            let original = fs::read_to_string(&path)?;
            if revision(&original) != child.revision {
                return Err(TaskStoreError::Conflict(format!(
                    "{} changed on disk while preparing bulk archive", child.id
                )));
            }
            let mut detached = child.clone();
            detached.parent = None;
            let updated = render_task(&detached, Some(&original))?;
            detachments.push((path, original, updated));
        }

        let archive_dir = root.join(ARCHIVE_DIR).join(Local::now().year().to_string());
        let destinations: Vec<(PathBuf, PathBuf)> = selected.iter().map(|task| (
            PathBuf::from(&task.path),
            archive_dir.join(format!("{}.md", task.id)),
        )).collect();
        if destinations.iter().any(|(_, dest)| dest.exists()) {
            return Err(TaskStoreError::Conflict(
                "archive destination already exists; no tasks were archived".into(),
            ));
        }
        if destinations.is_empty() {
            return Ok(snapshot);
        }
        fs::create_dir_all(&archive_dir)?;
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (source, destination) in destinations {
            if let Err(error) = fs::rename(&source, &destination) {
                for (old, new) in moved.into_iter().rev() {
                    let _ = fs::rename(new, old);
                }
                return Err(TaskStoreError::Io(error));
            }
            moved.push((source, destination));
        }

        let mut detached_count = 0;
        for (path, _, updated) in &detachments {
            if let Err(error) = write_text_atomic(path, updated) {
                // Restore both the archived parents and all changed children
                // on I/O failure; the successful path is one logical Undo.
                for (path, original, _) in detachments[..detached_count].iter().rev() {
                    let _ = write_text_atomic(path, original);
                }
                for (old, new) in moved.into_iter().rev() {
                    let _ = fs::rename(new, old);
                }
                return Err(error);
            }
            detached_count += 1;
        }
        self.load_unlocked(root)
    }

    fn archive_column_older_than(
        &self,
        root: &Path,
        column_id: &str,
        cutoff_iso: &str,
        expected: &[(String, String)],
    ) -> Result<BoardSnapshot, TaskStoreError> {
        let cutoff = DateTime::parse_from_rfc3339(cutoff_iso)
            .map_err(|_| TaskStoreError::Invalid("archive cutoff must be a valid RFC3339 timestamp".into()))?
            .with_timezone(&Utc);
        if cutoff > Utc::now() {
            return Err(TaskStoreError::Invalid("archive cutoff must not be in the future".into()));
        }
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        if !snapshot.config.columns.iter().any(|column| column.id == column_id && column.terminal) {
            return Err(TaskStoreError::Invalid("age-based archival is only available for Done/terminal columns".into()));
        }
        let selected = age_archive_selection(&snapshot, column_id, cutoff);
        let revisions: HashMap<&str, &str> = expected.iter()
            .map(|(id, rev)| (id.as_str(), rev.as_str())).collect();
        if revisions.len() != expected.len() || revisions.len() != selected.len()
            || selected.iter().any(|task| revisions.get(task.id.as_str()) != Some(&task.revision.as_str()))
        {
            return Err(TaskStoreError::Conflict(
                "eligible Done tasks or task revisions changed; refresh before archiving".into(),
            ));
        }
        if selected.is_empty() {
            return Ok(snapshot);
        }
        let archive_dir = root.join(ARCHIVE_DIR).join(Local::now().year().to_string());
        let destinations = selected.iter().map(|task| (
            PathBuf::from(&task.path),
            archive_dir.join(format!("{}.md", task.id)),
        )).collect::<Vec<_>>();
        if destinations.iter().any(|(_, destination)| destination.exists()) {
            return Err(TaskStoreError::Conflict("an archive destination already exists; no tasks archived".into()));
        }
        fs::create_dir_all(&archive_dir)?;
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        for (source, destination) in destinations {
            if let Err(error) = fs::rename(&source, &destination) {
                for (old, new) in moved.into_iter().rev() {
                    let _ = fs::rename(new, old);
                }
                return Err(TaskStoreError::Io(error));
            }
            moved.push((source, destination));
        }
        self.load_unlocked(root)
    }

    fn save_config(
        &self,
        root: &Path,
        mut config: BoardConfig,
    ) -> Result<BoardConfig, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let path = root.join(BOARD_FILE);
        let existing = fs::read_to_string(&path)?;
        if !config.revision.is_empty() && revision(&existing) != config.revision {
            return Err(TaskStoreError::Conflict(
                "board columns changed on disk".into(),
            ));
        }
        validate_config(&config)?;
        let text = render_board(&config, Some(&existing))?;
        write_text_atomic(&path, &text)?;
        config.revision = revision(&text);
        Ok(config)
    }

    fn restore_snapshot(
        &self,
        root: &Path,
        before: BoardSnapshot,
        after: BoardSnapshot,
    ) -> Result<BoardSnapshot, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let current = self.load_unlocked(root)?;

        let config_changed = !config_payload_eq(&before.config, &after.config);
        if config_changed
            && (current.config.revision != after.config.revision
                || !config_payload_eq(&current.config, &after.config))
        {
            return Err(TaskStoreError::Conflict(
                "cannot undo because board columns changed after that action".into(),
            ));
        }

        let before_map: HashMap<&str, &Task> = before
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect();
        let after_map: HashMap<&str, &Task> = after
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect();
        let current_map: HashMap<&str, &Task> = current
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect();

        let mut changed_ids = HashSet::new();
        for id in before_map.keys().chain(after_map.keys()) {
            let changed = match (before_map.get(id), after_map.get(id)) {
                (Some(a), Some(b)) => !task_payload_eq(a, b),
                (None, None) => false,
                _ => true,
            };
            if changed {
                changed_ids.insert((*id).to_string());
            }
        }

        for id in &changed_ids {
            match (after_map.get(id.as_str()), current_map.get(id.as_str())) {
                (Some(expected), Some(actual))
                    if expected.revision == actual.revision
                        && task_payload_eq(expected, actual) => {}
                (None, None) => {}
                _ => {
                    return Err(TaskStoreError::Conflict(format!(
                        "cannot undo because {id} changed after that action"
                    )));
                }
            }
        }

        for id in &changed_ids {
            if before_map.get(id.as_str()).is_none() && after_map.get(id.as_str()).is_some() {
                if let Some(child) = current.tasks.iter().find(|task| {
                    task.parent.as_deref() == Some(id.as_str()) && !changed_ids.contains(&task.id)
                }) {
                    return Err(TaskStoreError::Conflict(format!(
                        "cannot undo creation of {id} because {} now depends on it",
                        child.id
                    )));
                }
            }
        }

        if config_changed {
            let before_columns: HashSet<&str> = before
                .config
                .columns
                .iter()
                .map(|column| column.id.as_str())
                .collect();
            let removed_on_undo: HashSet<&str> = after
                .config
                .columns
                .iter()
                .map(|column| column.id.as_str())
                .filter(|id| !before_columns.contains(id))
                .collect();
            for task in current.tasks.iter().filter(|task| !task.archived) {
                if removed_on_undo.contains(task.column.as_str()) && !changed_ids.contains(&task.id)
                {
                    return Err(TaskStoreError::Conflict(format!(
                        "cannot undo because {} now uses column {}",
                        task.id, task.column
                    )));
                }
            }
        }

        // Restore board configuration first. This is safe because all affected
        // task files were revision-checked above and the process-wide lock is held.
        if config_changed {
            let path = root.join(BOARD_FILE);
            let existing = fs::read_to_string(&path)?;
            let mut restored = before.config.clone();
            restored.revision = revision(&existing);
            let text = render_board(&restored, Some(&existing))?;
            write_text_atomic(&path, &text)?;
        }

        for id in changed_ids {
            match (before_map.get(id.as_str()), after_map.get(id.as_str())) {
                (None, Some(_created)) => {
                    let path = find_task_path(root, &id)?;
                    fs::remove_file(path)?;
                }
                (Some(previous), Some(_)) => {
                    let current_path = find_task_path(root, &id)?;
                    let desired_dir = if previous.archived {
                        root.join(ARCHIVE_DIR)
                            .join(previous.created.get(0..4).unwrap_or("unknown"))
                    } else {
                        root.join(ACTIVE_DIR)
                    };
                    fs::create_dir_all(&desired_dir)?;
                    let desired_path = desired_dir.join(format!("{id}.md"));

                    let mut path = current_path;
                    if path != desired_path {
                        if desired_path.exists() {
                            return Err(TaskStoreError::Conflict(format!(
                                "cannot undo because {} already exists",
                                desired_path.display()
                            )));
                        }
                        fs::rename(&path, &desired_path)?;
                        path = desired_path;
                    }

                    let existing = fs::read_to_string(&path)?;
                    let mut restored = (*previous).clone();
                    restored.path = path.to_string_lossy().to_string();
                    restored.revision = revision(&existing);
                    let text = render_task(&restored, Some(&existing))?;
                    write_text_atomic(&path, &text)?;
                }
                (Some(previous), None) => {
                    let desired_dir = if previous.archived {
                        root.join(ARCHIVE_DIR)
                            .join(previous.created.get(0..4).unwrap_or("unknown"))
                    } else {
                        root.join(ACTIVE_DIR)
                    };
                    fs::create_dir_all(&desired_dir)?;
                    let path = desired_dir.join(format!("{id}.md"));
                    if path.exists() {
                        return Err(TaskStoreError::Conflict(format!(
                            "cannot undo because {id} already exists"
                        )));
                    }
                    let mut restored = (*previous).clone();
                    restored.path = path.to_string_lossy().to_string();
                    restored.revision.clear();
                    let text = render_task(&restored, None)?;
                    write_new(&path, &text)?;
                }
                (None, None) => {}
            }
        }

        self.load_unlocked(root)
    }
}


/// Only regular tasks can carry hard dependencies, and hard dependency
/// targets must also be executable tasks. Related links are informational.
fn validate_cross_project_id(reference: &str) -> Result<(&Path, &str), TaskStoreError> {
    let (board, id) = reference.rsplit_once('#').ok_or_else(||
        TaskStoreError::Invalid(format!("invalid cross-project link: {reference}")))?;
    if board.len() > 512 || !Path::new(board).is_absolute() ||
        id.len() > 32 || !id.contains('-') ||
        !id.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'-') {
        return Err(TaskStoreError::Invalid(format!("invalid cross-project link: {reference}")));
    }
    Ok((Path::new(board), id))
}

fn load_cross_project_task(reference: &str) -> Result<Task, TaskStoreError> {
    let (root, id) = validate_cross_project_id(reference)?;
    let active = root.join(ACTIVE_DIR).join(format!("{id}.md"));
    if active.is_file() {
        let content = fs::read_to_string(&active)?;
        return parse_task(&active, false, &content);
    }
    let mut archived = Vec::new();
    let mut warnings = Vec::new();
    collect_tasks(&root.join(ARCHIVE_DIR), true, &mut archived, &mut warnings)?;
    archived.into_iter().find(|task| task.id == id).ok_or_else(||
        TaskStoreError::Invalid(format!("cross-project prerequisite {reference} was not found")))
}

fn validate_links(tasks: &[Task], own_id: Option<&str>, kind: TaskKind, hard: &[String], related: &[String]) -> Result<(), TaskStoreError> {
    if kind == TaskKind::Epic && !hard.is_empty() {
        return Err(TaskStoreError::Invalid("epics cannot have hard dependencies; add them to subtasks".into()));
    }
    for links in [hard, related] {
        let mut seen = HashSet::new();
        for id in links {
            if id.trim().is_empty() || !seen.insert(id.as_str()) {
                return Err(TaskStoreError::Invalid("dependency links must be nonempty and unique".into()));
            }
            if own_id == Some(id.as_str()) {
                return Err(TaskStoreError::Invalid("task cannot link to itself".into()));
            }
            // Cross-project task links use the same board-path#task-id
            // identity as informational links. Hard links must resolve to
            // an executable task; unreadable remote boards remain unfinished.
            if id.contains('#') {
                if std::ptr::eq(links, hard) {
                    let target = load_cross_project_task(id)?;
                    if target.kind == TaskKind::Epic {
                        return Err(TaskStoreError::Invalid(format!(
                            "hard dependency {id} is an epic; depend on an executable task"
                        )));
                    }
                } else {
                    validate_cross_project_id(id)?;
                }
                continue;
            }
            let target = tasks.iter().find(|t| t.id == *id).ok_or_else(|| TaskStoreError::Invalid(format!("link references unknown task {id}")))?;
            if std::ptr::eq(links, hard) && target.kind == TaskKind::Epic {
                return Err(TaskStoreError::Invalid(format!("hard dependency {id} is an epic; depend on a specific executable subtask instead")));
            }
        }
    }
    if let Some(id) = own_id {
        for target in hard {
            if depends_transitively(tasks, target, id) {
                return Err(TaskStoreError::Invalid(format!("dependency cycle: {id} -> {target} -> ... -> {id}")));
            }
        }
    }
    Ok(())
}
fn depends_transitively(tasks: &[Task], start: &str, target: &str) -> bool {
    let mut pending = vec![start.to_string()];
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if id == target { return true; }
        if !seen.insert(id.clone()) { continue; }
        if let Some(t) = tasks.iter().find(|t| t.id == id) {
            pending.extend(t.depends_on.iter().cloned());
        }
    }
    false
}
fn ensure_dependencies_complete_for(tasks: &[Task], dependencies: &[String]) -> Result<(), TaskStoreError> {
    let waiting = dependencies.iter().filter(|id| {
        if id.contains('#') {
            return load_cross_project_task(id).map(|task| !task.archived && task.completed.is_none()).unwrap_or(true);
        }
        tasks.iter().find(|t| &t.id == *id).is_none_or(|t| !t.archived && t.completed.is_none())
    }).cloned().collect::<Vec<_>>();
    if !waiting.is_empty() {
        return Err(TaskStoreError::Invalid(format!("unfinished dependencies: {}. Complete these tasks before claiming or starting work", waiting.join(", "))));
    }
    Ok(())
}
fn ensure_dependencies_complete(snapshot: &BoardSnapshot, task: &Task) -> Result<(), TaskStoreError> {
    ensure_dependencies_complete_for(&snapshot.tasks, &task.depends_on)
}
fn derived_epic_column(snapshot: &BoardSnapshot, id: &str) -> String {
    let mut seen = HashSet::new();
    let mut queue = vec![id.to_string()];
    let mut leaves = Vec::<&Task>::new();
    while let Some(parent) = queue.pop() {
        for child in snapshot.tasks.iter().filter(|task| task.parent.as_deref() == Some(&parent)) {
            if !seen.insert(child.id.as_str()) { continue; }
            if child.kind == TaskKind::Epic { queue.push(child.id.clone()); }
            else { leaves.push(child); }
        }
    }
    if leaves.is_empty() { return DRAFT_COLUMN_ID.to_string(); }
    if leaves.iter().all(|t| t.archived || t.completed.is_some()) {
        return snapshot.config.columns.iter().find(|c| c.terminal).map(|c| c.id.clone()).unwrap_or_else(|| "done".into());
    }
    let active = leaves.iter().filter(|t| !t.archived && t.completed.is_none()).collect::<Vec<_>>();
    if active.iter().any(|t| t.column == IN_PROGRESS_COLUMN_ID) {
        return IN_PROGRESS_COLUMN_ID.into();
    }
    let usable = active.iter().filter(|t| {
        t.column != "blocked" && !t.attention.contains(&TaskAttention::Blocked) &&
        ensure_dependencies_complete_for(&snapshot.tasks, &t.depends_on).is_ok()
    }).collect::<Vec<_>>();
    if usable.is_empty() && !active.is_empty() { return "blocked".into(); }
    // Most populated active work column; never let a single Draft item
    // outweigh substantive pending implementation work.
    let mut counts = HashMap::<&str, usize>::new();
    for t in usable {
        if t.column != DRAFT_COLUMN_ID { *counts.entry(&t.column).or_default() += 1; }
    }
    if let Some((column, _)) = counts.into_iter().max_by(|a,b| a.1.cmp(&b.1).then_with(|| b.0.cmp(a.0))) {
        return column.to_string();
    }
    DRAFT_COLUMN_ID.to_string()
}

fn config_payload_eq(a: &BoardConfig, b: &BoardConfig) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.revision.clear();
    b.revision.clear();
    a == b
}

fn task_payload_eq(a: &Task, b: &Task) -> bool {
    let mut a = a.clone();
    let mut b = b.clone();
    a.revision.clear();
    b.revision.clear();
    a.path.clear();
    b.path.clear();
    a == b
}

fn parse_task_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").ok()
}
pub fn recurring_due(task: &Task) -> bool {
    task.kind != TaskKind::Recurring || task.next_due.as_deref()
        .and_then(parse_task_date).is_none_or(|date| date <= Local::now().date_naive())
}
fn validate_recurrence(kind: TaskKind, days: Option<u32>, due: Option<&str>) -> Result<(), TaskStoreError> {
    if kind != TaskKind::Recurring && (days.is_some() || due.is_some()) {
        return Err(TaskStoreError::Invalid("repeat interval and due date require kind recurring".into()));
    }
    if days.is_some_and(|days| !(1..=3650).contains(&days)) {
        return Err(TaskStoreError::Invalid("repeat interval must be 1 to 3650 days".into()));
    }
    if due.is_some_and(|value| parse_task_date(value).is_none()) {
        return Err(TaskStoreError::Invalid("next due must be YYYY-MM-DD".into()));
    }
    Ok(())
}

fn apply_completion_state(config: &BoardConfig, task: &mut Task) {
    let is_terminal = config
        .columns
        .iter()
        .any(|column| column.id == task.column && column.terminal);

    if task.kind == TaskKind::Recurring && is_terminal {
        let today = Local::now().date_naive();
        task.run_count = task.run_count.saturating_add(1);
        task.last_run_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
        task.next_due = task.repeat_every_days
            .map(|days| (today + chrono::Duration::days(i64::from(days))).to_string());
        task.column = AI_READY_COLUMN_ID.to_string();
        task.claimed_by = None;
        task.live = None;
        task.completed = None;
        task.completed_at = None;
        let due = task.next_due.as_deref().unwrap_or("immediately");
        task.description = append_description_update(&task.description, "recurring",
            &format!("Recurring run {} recorded; next eligible: {}.", task.run_count, due));
        return;
    }
    if is_terminal {
        task.live = None;
        // Done records completed implementation. A separate unanswered question
        // may still await human acceptance in the Inbox without reopening work.
        if task.kind == TaskKind::ForUser { task.kind = TaskKind::Task; }
        task.attention.retain(|flag| flag != &TaskAttention::NeedsUser);
        if task
            .completed
            .as_deref()
            .and_then(parse_task_date)
            .is_none()
        {
            task.completed = Some(Local::now().date_naive().to_string());
            task.completed_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true));
        }
    } else {
        task.completed = None;
        task.completed_at = None;
    }
}

/// Who is performing a task mutation. Only affects `blocked_reason` handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MutationActor {
    /// CLI/agents: entering Blocked requires a reason.
    Agent,
    /// Desktop editor: may block/unblock but never authors the reason.
    Human,
}

pub const BLOCKED_COLUMN_ID: &str = "blocked";

/// Executable work is blocked by the `blocked` flag or the Blocked column.
pub fn is_task_blocked(task: &Task) -> bool {
    task.kind != TaskKind::Epic &&
        (task.column == BLOCKED_COLUMN_ID || task.attention.contains(&TaskAttention::Blocked))
}

/// Enforce `blocked_reason` semantics and record the reason as a red
/// `blocked` Description timeline update when an agent blocks a task (or
/// replaces the reason). Leaving Blocked clears the reason. Tasks that were
/// already blocked before this field existed are grandfathered: they keep
/// working without a reason, so legacy blocked items are never stranded.
fn apply_blocked_state(previous: Option<&Task>, task: &mut Task, actor: MutationActor) -> Result<(), TaskStoreError> {
    if !is_task_blocked(task) {
        task.blocked_reason = None;
        return Ok(());
    }
    let was_blocked = previous.is_some_and(is_task_blocked);
    let previous_reason = previous.filter(|_| was_blocked).and_then(|p| p.blocked_reason.clone());
    if actor == MutationActor::Human {
        task.blocked_reason = previous_reason;
        return Ok(());
    }
    let reason = task.blocked_reason.as_deref().map(str::trim)
        .filter(|reason| !reason.is_empty()).map(str::to_string);
    match reason {
        None if !was_blocked => return Err(TaskStoreError::Invalid(format!(
            "{} cannot become blocked without a reason: supply a concise blocked_reason (CLI: --blocked-reason \"...\")",
            task.id))),
        None => task.blocked_reason = previous_reason,
        Some(text) => {
            if !was_blocked || previous_reason.as_deref() != Some(text.as_str()) {
                task.description = append_description_update(&task.description, "blocked", &text);
            }
            task.blocked_reason = Some(text);
        }
    }
    Ok(())
}

/// Appends one Description timeline block in the format of
/// `src/descriptionTimeline.ts`.
fn append_description_update(description: &str, kind: &str, text: &str) -> String {
    let stamp = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let quoted = text.trim().replace("\r\n", "\n").lines()
        .map(|line| if line.is_empty() { ">".to_string() } else { format!("> {line}") })
        .collect::<Vec<_>>().join("\n");
    let block = format!("<!-- taskboard:update:{kind}|{stamp} -->\n{quoted}\n<!-- taskboard:end-update -->");
    let base = description.trim_end();
    if base.is_empty() { block } else { format!("{base}\n\n{block}") }
}

fn validate_draft(snapshot: &BoardSnapshot, draft: &TaskDraft) -> Result<(), TaskStoreError> {
    if draft.title.trim().is_empty() {
        return Err(TaskStoreError::Invalid("title cannot be empty".into()));
    }
    if !snapshot.config.columns.iter().any(|c| c.id == draft.column) {
        return Err(TaskStoreError::Invalid(format!(
            "unknown column {}",
            draft.column
        )));
    }
    if draft.kind == TaskKind::Release { return Err(TaskStoreError::Invalid("releases use the Releases lifecycle".into())); }
    validate_parent_epic_only(snapshot, draft.kind, draft.parent.as_deref(), None)?;
    validate_recurrence(draft.kind, draft.repeat_every_days, draft.next_due.as_deref())?;
    validate_attention(&draft.attention, draft.question.as_ref())?;
    validate_icon(draft.icon.as_deref())?;
    validate_epic_fields(draft.kind, draft.icon.as_deref(), &draft.attention,
        draft.question.as_ref(), &draft.acceptance_criteria, &draft.ai_notes)?;
    if draft.kind == TaskKind::Epic && !draft.depends_on.is_empty() {
        return Err(TaskStoreError::Invalid("epics cannot declare hard dependencies; put prerequisites on executable subtasks".into()));
    }
    if draft.kind != TaskKind::Epic && draft.column == IN_PROGRESS_COLUMN_ID {
        ensure_dependencies_complete_for(&snapshot.tasks, &draft.depends_on)?;
    }
    validate_links(&snapshot.tasks, None, draft.kind, &draft.depends_on, &draft.related_to)?;
    Ok(())
}


fn validate_live(task: &Task) -> Result<(), TaskStoreError> {
    let Some(live) = &task.live else { return Ok(()); };
    if !live.with_me || !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) {
        return Err(TaskStoreError::Invalid("live collaboration requires with_me on an agent task".into()));
    }
    if live.est_minutes.is_some_and(|minutes| !(1..=1440).contains(&minutes)) {
        return Err(TaskStoreError::Invalid("live estimate must be between 1 and 1440 minutes".into()));
    }
    if let Some(value) = live.ready_at.as_deref() {
        DateTime::parse_from_rfc3339(value).map_err(|_| TaskStoreError::Invalid("invalid ready_at ISO timestamp".into()))?;
    }
    if let Some(value) = live.later_until.as_deref() {
        DateTime::parse_from_rfc3339(value).map_err(|_| TaskStoreError::Invalid("invalid later_until ISO timestamp".into()))?;
    }
    match live.state {
        Some(crate::LiveTaskState::Ready) => {
            if !task.attention.contains(&TaskAttention::NeedsUser)
                || live.brief.as_deref().is_none_or(|brief| brief.trim().is_empty())
                || live.ready_at.is_none() {
                return Err(TaskStoreError::Invalid("Ready for you requires needs-user, brief and ready_at".into()));
            }
        }
        Some(crate::LiveTaskState::InSession) if task.attention.contains(&TaskAttention::NeedsUser) => {
            return Err(TaskStoreError::Invalid("in_session must clear needs-user".into()));
        }
        _ => {}
    }
    Ok(())
}

fn validate_task(snapshot: &BoardSnapshot, task: &Task) -> Result<(), TaskStoreError> {
    if task.title.trim().is_empty() {
        return Err(TaskStoreError::Invalid("title cannot be empty".into()));
    }
    if !task.archived && !snapshot.config.columns.iter().any(|c| c.id == task.column) {
        return Err(TaskStoreError::Invalid(format!(
            "unknown column {}",
            task.column
        )));
    }
    if task.kind == TaskKind::Release { return Err(TaskStoreError::Invalid("releases use the Releases lifecycle".into())); }
    validate_parent_epic_only(snapshot, task.kind, task.parent.as_deref(), Some(&task.id))?;
    validate_recurrence(task.kind, task.repeat_every_days, task.next_due.as_deref())?;
    validate_attention(&task.attention, task.question.as_ref())?;
    if pending_human_request(task) && is_todo(&snapshot.config, &task.column) {
        return Err(TaskStoreError::Invalid("Pending Inbox question/review cannot remain in a Todo column; use In Progress or Done".into()));
    }
    validate_live(task)?;
    validate_icon(task.icon.as_deref())?;
    validate_epic_fields(task.kind, task.icon.as_deref(), &task.attention,
        task.question.as_ref(), &task.acceptance_criteria, &task.ai_notes)?;
    if task.position_after.as_deref() == Some(&task.id) {
        return Err(TaskStoreError::Invalid("task cannot be positioned after itself".into()));
    }
    if task.kind == TaskKind::Epic && task.position_after.is_some() {
        return Err(TaskStoreError::Invalid("epics have derived status and cannot have manual task positions".into()));
    }
    if task.kind != TaskKind::Epic && snapshot.tasks.iter()
        .any(|existing| !existing.archived && existing.parent.as_deref() == Some(task.id.as_str()))
    {
        return Err(TaskStoreError::Invalid("a task with children cannot cease being an epic; reassign its children first".into()));
    }
    if matches!(task.kind, TaskKind::Epic | TaskKind::ForUser) && task.claimed_by.is_some() {
        return Err(TaskStoreError::Invalid("epics and For user tasks cannot be claimed or assigned".into()));
    }
    validate_links(&snapshot.tasks, Some(task.id.as_str()), task.kind, &task.depends_on, &task.related_to)?;
    Ok(())
}

/// Excludes recently completed tasks and unverifiable legacy timestamps.
 /// Also excludes epics that would orphan any still-active child, rather than
 /// silently detaching a newer child during age-based housekeeping.
fn age_archive_selection<'a>(
    snapshot: &'a BoardSnapshot,
    column_id: &str,
    cutoff: DateTime<Utc>,
) -> Vec<&'a Task> {
    let qualified = snapshot.tasks.iter().filter(|task| {
        !task.archived && task.column == column_id &&
            task.review_status.as_deref() != Some("open") &&
            !(task.attention.contains(&TaskAttention::Question) &&
              task.question.as_ref().is_some_and(|q| q.answer.is_none())) &&
            task.completed_at.as_deref()
                .and_then(|timestamp| DateTime::parse_from_rfc3339(timestamp).ok())
                .is_some_and(|date| date.with_timezone(&Utc) <= cutoff)
    }).collect::<Vec<_>>();
    let mut selected = qualified.iter().map(|task| task.id.as_str()).collect::<HashSet<_>>();
    loop {
        let mut changed = false;
        for task in &qualified {
            if task.kind != TaskKind::Epic || !selected.contains(task.id.as_str()) {
                continue;
            }
            if snapshot.tasks.iter().any(|child| {
                !child.archived && child.parent.as_deref() == Some(task.id.as_str())
                    && !selected.contains(child.id.as_str())
            }) {
                selected.remove(task.id.as_str());
                changed = true;
            }
        }
        if !changed { break; }
    }
    qualified.into_iter().filter(|task| selected.contains(task.id.as_str())).collect()
}

fn validate_parent_epic_only(snapshot: &BoardSnapshot, kind: TaskKind, parent: Option<&str>, own_id: Option<&str>) -> Result<(), TaskStoreError> {
    if kind == TaskKind::Epic && parent.is_some() {
        return Err(TaskStoreError::Invalid("epics are root containers and cannot have a parent".into()));
    }
    if let Some(parent_id) = parent {
        if own_id == Some(parent_id) {
            return Err(TaskStoreError::Invalid("task cannot parent itself".into()));
        }
        let parent_task = snapshot.tasks.iter().find(|t| t.id == parent_id && !t.archived)
            .ok_or_else(|| TaskStoreError::Invalid(format!("parent {parent_id} does not exist")))?;
        if parent_task.kind != TaskKind::Epic {
            return Err(TaskStoreError::Invalid(format!(
                "{parent_id} is not an epic; tasks can only be placed under epics"
            )));
        }
    }
    Ok(())
}

fn validate_epic_fields(kind: TaskKind, icon: Option<&str>, attention: &[TaskAttention],
    question: Option<&TaskQuestion>, acceptance: &[String], ai_notes: &str) -> Result<(), TaskStoreError> {
    if kind != TaskKind::Epic { return Ok(()); }
    if icon.is_some() || !attention.is_empty() || question.is_some() {
        return Err(TaskStoreError::Invalid("epics cannot have a card icon, attention flags or questions".into()));
    }
    if !acceptance.is_empty() || !ai_notes.trim().is_empty() {
        return Err(TaskStoreError::Invalid("epics cannot have acceptance criteria or AI Notes; move design and coordination material into Description or Notes".into()));
    }
    Ok(())
}

fn validate_icon(icon: Option<&str>) -> Result<(), TaskStoreError> {
    if let Some(icon) = icon {
        if !matches!(icon, "bug" | "feature" | "refactor" | "content" | "validation" | "design" | "visual-work" | "visual-implementation" | "hard") {
            return Err(TaskStoreError::Invalid(format!("unknown task icon {icon}")));
        }
    }
    Ok(())
}

fn validate_attention(
    attention: &[TaskAttention],
    question: Option<&TaskQuestion>,
) -> Result<(), TaskStoreError> {
    let has_question = attention.contains(&TaskAttention::Question);
    if let Some(question) = question {
        if question.prompt.trim().is_empty() {
            return Err(TaskStoreError::Invalid(
                "question prompt cannot be empty".into(),
            ));
        }
        if question
            .options
            .iter()
            .any(|option| option.trim().is_empty())
        {
            return Err(TaskStoreError::Invalid(
                "question options cannot be empty".into(),
            ));
        }
        let answered = question
            .answer
            .as_deref()
            .is_some_and(|answer| !answer.trim().is_empty());
        if answered && has_question {
            return Err(TaskStoreError::Invalid(
                "answered question must not retain the question attention flag".into(),
            ));
        }
        if !answered && !has_question {
            return Err(TaskStoreError::Invalid(
                "unanswered question requires the question attention flag".into(),
            ));
        }
    } else if has_question {
        return Err(TaskStoreError::Invalid(
            "question attention flag requires a structured question".into(),
        ));
    }
    Ok(())
}

fn validate_config(config: &BoardConfig) -> Result<(), TaskStoreError> {
    let task_prefix = config.task_prefix.as_deref().unwrap_or("WT");
    let epic_prefix = config.epic_prefix.as_deref().unwrap_or("WTE");
    for (label, prefix) in [("task", task_prefix), ("epic", epic_prefix)] {
        if !valid_prefix(prefix) {
            return Err(TaskStoreError::Invalid(format!(
                "{label}_prefix must be 2-12 uppercase ASCII letters/digits, starting with a letter (without '-')"
            )));
        }
    }
    if task_prefix == epic_prefix {
        return Err(TaskStoreError::Invalid("task_prefix and epic_prefix must differ".into()));
    }
    if config.columns.is_empty() {
        return Err(TaskStoreError::Invalid(
            "board needs at least one column".into(),
        ));
    }
    let mut ids = HashSet::new();
    for column in &config.columns {
        if column.id.trim().is_empty() || column.name.trim().is_empty() {
            return Err(TaskStoreError::Invalid(
                "column id/name cannot be empty".into(),
            ));
        }
        if !ids.insert(column.id.as_str()) {
            return Err(TaskStoreError::Invalid(format!(
                "duplicate column {}",
                column.id
            )));
        }
        if column.id == AI_READY_COLUMN_ID && !column.ai_ready {
            return Err(TaskStoreError::Invalid(
                "canonical AI Ready column must remain AI ready".into(),
            ));
        }
        if column.ai_ready && (column.id == DRAFT_COLUMN_ID || column.terminal) {
            return Err(TaskStoreError::Invalid(format!(
                "column {} cannot be AI ready",
                column.id
            )));
        }
        if column.advisory && (column.ai_ready || column.id == DRAFT_COLUMN_ID
            || column.id == AI_READY_COLUMN_ID || column.terminal) {
            return Err(TaskStoreError::Invalid(format!(
                "column {} cannot be advisory and must not be AI ready, Draft or terminal",
                column.id
            )));
        }
    }
    Ok(())
}

// Use the filename shape, not the currently configured prefix, to discover tasks.
// This retains WT/WTE and any earlier custom prefix when a board switches IDs.
fn valid_prefix(prefix: &str) -> bool {
    (2..=12).contains(&prefix.len())
        && prefix.as_bytes()[0].is_ascii_uppercase()
        && prefix.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn looks_like_task_id(file_name: &str) -> bool {
    let Some(stem) = file_name.strip_suffix(".md") else { return false; };
    let Some((prefix, number)) = stem.rsplit_once('-') else { return false; };
    valid_prefix(prefix) && number.len() >= 4 && number.bytes().all(|b| b.is_ascii_digit())
}

fn collect_tasks(
    dir: &Path,
    archived: bool,
    out: &mut Vec<Task>,
    warnings: &mut Vec<String>,
) -> Result<(), TaskStoreError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_tasks(&path, archived, out, warnings)?;
            continue;
        }
        let file_name = path.file_name().and_then(|x| x.to_str()).unwrap_or("");
        if !looks_like_task_id(file_name) {

            continue;
        }
        match fs::read_to_string(&path)
            .map_err(TaskStoreError::from)
            .and_then(|text| parse_task(&path, archived, &text))
        {
            Ok(task) if path.file_stem().and_then(|s| s.to_str()) == Some(task.id.as_str()) => out.push(task),
            Ok(task) => warnings.push(format!("{}: filename does not match task id {}", path.display(), task.id)),
            Err(err) => warnings.push(format!("{}: {err}", path.display())),
        }
    }
    Ok(())
}

fn parse_board(text: &str) -> Result<BoardConfig, TaskStoreError> {
    let (yaml, _) = split_frontmatter(text)?;
    let mut config: BoardConfig =
        serde_yaml::from_str(&yaml).map_err(|e| TaskStoreError::Parse(e.to_string()))?;

    // Backward compatibility: older boards predate per-column AI readiness.
    // If no column is explicitly marked ready, keep the canonical AI Ready
    // column ready so existing boards retain their old semantics.
    if !config.columns.iter().any(|column| column.ai_ready) {
        if let Some(column) = config
            .columns
            .iter_mut()
            .find(|column| column.id == AI_READY_COLUMN_ID)
        {
            column.ai_ready = true;
        }
    }

    Ok(config)
}

pub(crate) fn render_board(config: &BoardConfig, existing: Option<&str>) -> Result<String, TaskStoreError> {
    let (mut mapping, body) = if let Some(existing) = existing {
        let (yaml, body) = split_frontmatter(existing)?;
        let mapping = serde_yaml::from_str::<serde_yaml::Mapping>(&yaml)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
        (mapping, body)
    } else {
        (
            serde_yaml::Mapping::new(),
            format!("\n# {} Task Board\n", config.name),
        )
    };

    let mut clean = config.clone();
    clean.revision.clear();
    mapping.remove(serde_yaml::Value::String("revision".into()));
    let known = serde_yaml::to_value(&clean).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    if let serde_yaml::Value::Mapping(mut known) = known {
        known.remove(serde_yaml::Value::String("revision".into()));
        for (key, value) in known {
            mapping.insert(key, value);
        }
    }

    let yaml = serde_yaml::to_string(&mapping).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    Ok(format!("---\n{yaml}---\n{body}"))
}

fn parse_task(path: &Path, archived: bool, text: &str) -> Result<Task, TaskStoreError> {
    let (yaml, body) = split_frontmatter(text)?;
    let meta: TaskMeta =
        serde_yaml::from_str(&yaml).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    let (description, acceptance_criteria, ai_notes, user_notes, notes) = parse_sections(&body);
    Ok(Task {
        schema: meta.schema,
        id: meta.id,
        kind: meta.kind,
        deployable: meta.deployable,
        repeat_every_days: meta.repeat_every_days,
        next_due: meta.next_due,
        last_run_at: meta.last_run_at,
        run_count: meta.run_count,
        depends_on: meta.depends_on,
        related_to: meta.related_to,
        position_after: meta.position_after,
        positioned_at: meta.positioned_at,
        title: meta.title,
        column: meta.column,
        parent: meta.parent,
        category: meta.category,
        icon: meta.icon,
        claimed_by: meta.claimed_by,
        live: meta.live,
        attention: meta.attention,
        question: meta.question,
        blocked_reason: meta.blocked_reason,
        review_status: meta.review_status,
        review_round: meta.review_round,
        created: meta.created,
        completed: meta.completed,
        completed_at: meta.completed_at,
        description,
        acceptance_criteria,
        ai_notes,
        user_notes,
        notes,
        archived,
        path: path.to_string_lossy().to_string(),
        revision: revision(text),
    })
}

/// Single source of dispatch eligibility for the native OpenRig producer.
pub fn queue_task_ready(snapshot: &BoardSnapshot, task: &Task) -> bool {
    !task.archived && task.completed.is_none() && task.claimed_by.is_none() &&
        matches!(task.kind, TaskKind::Task | TaskKind::Recurring) &&
        task.column == AI_READY_COLUMN_ID &&
        is_todo(&snapshot.config, &task.column) &&
        !is_task_blocked(task) && !pending_human_request(task) &&
        ensure_dependencies_complete_for(&snapshot.tasks, &task.depends_on).is_ok()
}

/// Apply only durable policy-derived Queue intent. This is a metadata write,
/// never a worker start; the existing OpenRig bridge owns dispatch/reconciliation.
fn queue_policy_text(
    text: &str, task: &Task, snapshot: &BoardSnapshot, previous: Option<&Task>,
) -> Result<String, TaskStoreError> {
    let Some(column) = snapshot.config.columns.iter().find(|c| c.id == task.column) else {
        return Ok(text.to_owned());
    };
    if task.archived || task.completed.is_some() || task.claimed_by.is_some() ||
        !matches!(task.kind, TaskKind::Task | TaskKind::Recurring) ||
        task.column != AI_READY_COLUMN_ID || !is_todo(&snapshot.config, &task.column) ||
        task.attention.iter().any(|a| matches!(a, TaskAttention::Blocked | TaskAttention::NeedsUser | TaskAttention::Question)) ||
        task.question.as_ref().is_some_and(|q| q.answer.is_none()) ||
        task.review_status.as_deref() == Some("open") {
        return Ok(text.to_owned());
    }
    let newly_prioritized = task.attention.contains(&TaskAttention::Priority) &&
        previous.is_none_or(|before| !before.attention.contains(&TaskAttention::Priority) ||
            before.column != task.column);
    if !column.auto_queue && !task.attention.contains(&TaskAttention::Priority) {
        return Ok(text.to_owned());
    }
    let (yaml, body) = split_frontmatter(text)?;
    let mut map: serde_yaml::Mapping = serde_yaml::from_str(&yaml)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    let key = |s: &str| serde_yaml::Value::String(s.into());
    let mut changed = false;
    if newly_prioritized && map.get(key("queue_inherit")) != Some(&serde_yaml::Value::Bool(true)) {
        map.insert(key("queue_inherit"), serde_yaml::Value::Bool(true));
        changed = true;
    }
    let inherited = map.get(key("queue_inherit")).and_then(|v| v.as_bool()) == Some(true);
    let manual = map.get(key("queue_manual")).and_then(|v| v.as_bool()) == Some(true);
    let queued = map.get(key("execution_mode")).and_then(|v| v.as_str()) == Some("queue");
    if (column.auto_queue || inherited) && !manual && !queued &&
        map.get(key("queue_registration_attempt")).is_none() &&
        ensure_dependencies_complete_for(&snapshot.tasks, &task.depends_on).is_ok() {
        map.insert(key("execution_mode"), key("queue"));
        map.insert(key("queue_request"), key(&uuid::Uuid::new_v4().to_string()));
        changed = true;
    }
    if !changed { return Ok(text.to_owned()); }
    Ok(format!("---\n{}---\n{}", serde_yaml::to_string(&map)
        .map_err(|e| TaskStoreError::Parse(e.to_string()))?, body))
}

fn render_task(task: &Task, existing: Option<&str>) -> Result<String, TaskStoreError> {
    let (mut mapping, existing_body) = if let Some(existing) = existing {
        let (yaml, body) = split_frontmatter(existing)?;
        let mapping = serde_yaml::from_str::<serde_yaml::Mapping>(&yaml)
            .map_err(|e| TaskStoreError::Parse(e.to_string()))?;
        (mapping, Some(body))
    } else {
        (serde_yaml::Mapping::new(), None)
    };

    let meta = TaskMeta {
        schema: task.schema,
        id: task.id.clone(),
        kind: task.kind,
        deployable: task.deployable,
        repeat_every_days: task.repeat_every_days,
        next_due: task.next_due.clone(),
        last_run_at: task.last_run_at.clone(),
        run_count: task.run_count,
        depends_on: task.depends_on.clone(),
        related_to: task.related_to.clone(),
        position_after: task.position_after.clone(),
        positioned_at: task.positioned_at.clone(),
        title: task.title.clone(),
        column: task.column.clone(),
        parent: task.parent.clone(),
        category: task.category.clone(),
        icon: task.icon.clone(),
        claimed_by: task.claimed_by.clone(),
        live: task.live.clone(),
        attention: task.attention.clone(),
        question: task.question.clone(),
        blocked_reason: task.blocked_reason.clone(),
        review_status: task.review_status.clone(),
        review_round: task.review_round,
        created: task.created.clone(),
        completed: task.completed.clone(),
        completed_at: task.completed_at.clone(),
    };
    let known = serde_yaml::to_value(&meta).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    if let serde_yaml::Value::Mapping(known) = known {
        mapping.remove(serde_yaml::Value::String("deployable".into()));
        mapping.remove(serde_yaml::Value::String("repeat_every_days".into()));
        mapping.remove(serde_yaml::Value::String("next_due".into()));
        mapping.remove(serde_yaml::Value::String("last_run_at".into()));
        mapping.remove(serde_yaml::Value::String("run_count".into()));
        mapping.remove(serde_yaml::Value::String("blocked_reason".into()));
        mapping.remove(serde_yaml::Value::String("parent".into()));
        mapping.remove(serde_yaml::Value::String("depends_on".into()));
        mapping.remove(serde_yaml::Value::String("related_to".into()));
        mapping.remove(serde_yaml::Value::String("position_after".into()));
        mapping.remove(serde_yaml::Value::String("positioned_at".into()));
        mapping.remove(serde_yaml::Value::String("category".into()));
        mapping.remove(serde_yaml::Value::String("icon".into()));
        mapping.remove(serde_yaml::Value::String("claimed_by".into()));
        mapping.remove(serde_yaml::Value::String("live".into()));
        mapping.remove(serde_yaml::Value::String("attention".into()));
        mapping.remove(serde_yaml::Value::String("question".into()));
        mapping.remove(serde_yaml::Value::String("blocked_reason".into()));
        mapping.remove(serde_yaml::Value::String("review_status".into()));
        mapping.remove(serde_yaml::Value::String("review_round".into()));
        mapping.remove(serde_yaml::Value::String("completed".into()));
        mapping.remove(serde_yaml::Value::String("completed_at".into()));
        for (key, value) in known {
            mapping.insert(key, value);
        }
    }
    let yaml = serde_yaml::to_string(&mapping).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    let body = render_task_body(task, existing_body.as_deref());
    Ok(format!("---\n{yaml}---\n{body}"))
}

fn render_task_body(task: &Task, existing: Option<&str>) -> String {
    let criteria = task
        .acceptance_criteria
        .iter()
        .map(|item| format!("- [ ] {}", item.trim()))
        .collect::<Vec<_>>()
        .join("\n");

    let Some(existing) = existing else {
        if task.kind == TaskKind::Epic {
            let mut body = format!("\n## Description\n\n{}\n\n## User Notes\n\n{}\n",
                task.description.trim(), task.user_notes.trim());
            if !task.notes.trim().is_empty() {
                body.push_str(&format!("\n## Notes\n\n{}\n", task.notes.trim()));
            }
            return body;
        }
        return format!(
            "\n## Description\n\n{}\n\n## Acceptance Criteria\n\n{}\n\n## AI Notes\n\n{}\n\n## User Notes\n\n{}\n",
            task.description.trim(),
            criteria,
            task.ai_notes.trim(),
            task.user_notes.trim()
        );
    };

    let mut body = existing.to_string();
    body = replace_markdown_section(&body, "## Description", task.description.trim());
    if task.kind == TaskKind::Epic {
        body = remove_markdown_section(&body, "## Acceptance Criteria");
        body = remove_markdown_section(&body, "## AI Notes");
    } else {
        body = replace_markdown_section(&body, "## Acceptance Criteria", &criteria);
        body = replace_markdown_section(&body, "## AI Notes", task.ai_notes.trim());
    }
    body = replace_markdown_section(&body, "## User Notes", task.user_notes.trim());
    if !task.notes.trim().is_empty() || body.lines().any(|line| line.trim() == "## Notes") {
        body = replace_markdown_section(&body, "## Notes", task.notes.trim());
    }
    body
}

fn remove_markdown_section(body: &str, heading: &str) -> String {
    let normalized = body.replace("\r\n", "\n");
    let mut lines: Vec<&str> = normalized.lines().collect();
    if let Some(start) = lines.iter().position(|line| line.trim() == heading) {
        let end = (start + 1..lines.len())
            .find(|index| lines[*index].trim_start().starts_with("## "))
            .unwrap_or(lines.len());
        lines.drain(start..end);
    }
    let mut output = lines.join("\n");
    output.push('\n');
    output
}

fn replace_markdown_section(body: &str, heading: &str, content: &str) -> String {
    let normalized = body.replace("\r\n", "\n");
    let mut lines = normalized.lines().map(str::to_string).collect::<Vec<_>>();

    if let Some(start) = lines.iter().position(|line| line.trim() == heading) {
        let end = (start + 1..lines.len())
            .find(|index| lines[*index].trim_start().starts_with("## "))
            .unwrap_or(lines.len());

        let mut replacement = vec![lines[start].clone(), String::new()];
        if !content.is_empty() {
            replacement.extend(content.lines().map(str::to_string));
        }
        replacement.push(String::new());
        lines.splice(start..end, replacement);

        let mut output = lines.join("\n");
        output.push('\n');
        return output;
    }

    let mut output = normalized.trim_end_matches('\n').to_string();
    if !output.is_empty() {
        output.push_str("\n\n");
    }
    output.push_str(heading);
    output.push_str("\n\n");
    output.push_str(content);
    output.push('\n');
    output
}

fn parse_sections(body: &str) -> (String, Vec<String>, String, String, String) {
    enum Section {
        None,
        Description,
        Acceptance,
        AiNotes,
        UserNotes,
        Notes,
    }
    let mut section = Section::None;
    let mut description = Vec::new();
    let mut acceptance = Vec::new();
    let mut ai_notes = Vec::new();
    let mut user_notes = Vec::new();
    let mut notes = Vec::new();

    let normalized = body.replace("\r\n", "\n");
    for line in normalized.lines() {
        match line.trim() {
            "## Description" => {
                section = Section::Description;
                continue;
            }
            "## Acceptance Criteria" => {
                section = Section::Acceptance;
                continue;
            }
            "## AI Notes" => {
                section = Section::AiNotes;
                continue;
            }
            "## User Notes" => {
                section = Section::UserNotes;
                continue;
            }
            "## Notes" => {
                section = Section::Notes;
                continue;
            }
            heading if heading.starts_with("## ") => {
                section = Section::None;
                continue;
            }
            _ => {}
        }
        match section {
            Section::Description => description.push(line),
            Section::Acceptance => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                let item = trimmed
                    .strip_prefix("- [ ] ")
                    .or_else(|| trimmed.strip_prefix("- [x] "))
                    .or_else(|| trimmed.strip_prefix("- [X] "))
                    .or_else(|| trimmed.strip_prefix("- "))
                    .unwrap_or(trimmed);
                acceptance.push(item.to_string());
            }
            Section::AiNotes => ai_notes.push(line),
            Section::UserNotes => user_notes.push(line),
            Section::Notes => notes.push(line),
            Section::None => {}
        }
    }
    (
        description.join("\n").trim().to_string(),
        acceptance,
        ai_notes.join("\n").trim().to_string(),
        user_notes.join("\n").trim().to_string(),
        notes.join("\n").trim().to_string(),
    )
}

fn split_frontmatter(text: &str) -> Result<(String, String), TaskStoreError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let normalized = text.replace("\r\n", "\n");
    if !normalized.starts_with("---\n") {
        return Err(TaskStoreError::Parse("missing YAML frontmatter".into()));
    }
    let rest = &normalized[4..];
    let marker = "\n---\n";
    let Some(index) = rest.find(marker) else {
        return Err(TaskStoreError::Parse(
            "unterminated YAML frontmatter".into(),
        ));
    };
    Ok((
        rest[..index].to_string(),
        rest[index + marker.len()..].to_string(),
    ))
}

pub(crate) fn acquire_lock(root: &Path) -> Result<LockGuard, TaskStoreError> {
    let lock_dir = std::env::temp_dir().join("TaskBoard").join("locks");
    fs::create_dir_all(&lock_dir)?;

    let key = revision(&root.to_string_lossy());
    let path = lock_dir.join(format!("{}.lock", &key[..24]));

    for _ in 0..80 {
        let file = match fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(err) if is_lock_contention(&err) => {
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(err) => return Err(err.into()),
        };

        match file.try_lock_exclusive() {
            Ok(()) => return Ok(LockGuard { file }),
            Err(err) if is_lock_contention(&err) => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(err) => return Err(err.into()),
        }
    }

    Err(TaskStoreError::Conflict("board is busy".into()))
}

fn is_lock_contention(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::WouldBlock || matches!(error.raw_os_error(), Some(32 | 33))
}

fn find_task_path(root: &Path, id: &str) -> Result<PathBuf, TaskStoreError> {
    let active = root.join(ACTIVE_DIR).join(format!("{id}.md"));
    if active.exists() {
        return Ok(active);
    }
    let mut matches = Vec::new();
    find_named(&root.join(ARCHIVE_DIR), &format!("{id}.md"), &mut matches)?;
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(TaskStoreError::NotFound(id.into())),
        _ => Err(TaskStoreError::Invalid(format!(
            "multiple archived files for {id}"
        ))),
    }
}

fn find_named(dir: &Path, file_name: &str, out: &mut Vec<PathBuf>) -> Result<(), TaskStoreError> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            find_named(&path, file_name, out)?;
        } else if path.file_name().and_then(|x| x.to_str()) == Some(file_name) {
            out.push(path);
        }
    }
    Ok(())
}

fn revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn write_new(path: &Path, text: &str) -> Result<(), TaskStoreError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

pub(crate) fn write_text_atomic(path: &Path, text: &str) -> Result<(), TaskStoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| TaskStoreError::Invalid("target has no parent directory".into()))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;

    match temp.persist(path) {
        Ok(_) => Ok(()),
        Err(error) if is_replace_not_supported(&error.error) => {
            // Some synced/mapped filesystems (notably the Google Drive Windows
            // mount) allow normal file writes but reject rename-over-existing.
            // We still hold the TaskBoard cross-process lock here, so fall back
            // to a truncate/write of the validated destination.
            let mut file = fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(path)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            Ok(())
        }
        Err(error) => Err(TaskStoreError::Io(error.error)),
    }
}

fn is_replace_not_supported(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::PermissionDenied
        || matches!(error.raw_os_error(), Some(5 | 32 | 33))
}
