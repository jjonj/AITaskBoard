use crate::{
    BoardConfig, BoardSnapshot, Column, Task, TaskAttention, TaskDraft, TaskKind, TaskQuestion,
    AI_READY_COLUMN_ID, DRAFT_COLUMN_ID, IN_PROGRESS_COLUMN_ID, SCHEMA_VERSION,
};
use chrono::{DateTime, Datelike, Duration as ChronoDuration, Local, NaiveDate, SecondsFormat, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

const BOARD_FILE: &str = "_board.md";
const ACTIVE_DIR: &str = "Active";
const ARCHIVE_DIR: &str = "Archive";
const COMPLETED_RETENTION_DAYS: i64 = 7;

#[derive(Debug, thiserror::Error)]
pub enum TaskStoreError {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TaskMeta {
    #[serde(default)]
    kind: TaskKind,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attention: Vec<TaskAttention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    question: Option<TaskQuestion>,
    created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
}

struct LockGuard {
    file: fs::File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

pub fn default_board_config(name: impl Into<String>) -> BoardConfig {
    BoardConfig {
        schema: SCHEMA_VERSION,
        name: name.into(),
        columns: vec![
            Column {
                id: DRAFT_COLUMN_ID.into(),
                name: "Draft".into(),
                order: 0,
                terminal: false,
                hidden: false,
                snoozed_until: None,
                awake_until: None,
                ai_ready: false,
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
                ai_ready: false,
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
                color: "#6f8679".into(),
            },
        ],
        revision: String::new(),
    }
}

impl FsTaskStore {
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

    fn load_unlocked(&self, root: &Path) -> Result<BoardSnapshot, TaskStoreError> {
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
            apply_completion_state(&snapshot.config, &mut released);
            write_text_atomic(&path, &render_task(&released, Some(&existing))?)?;
            changed = true;
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
                task.completed = normalized;
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
        if task.kind != TaskKind::Task || task.attention.contains(&TaskAttention::NeedsUser) {
            return Err(TaskStoreError::Invalid("task is not executable".into()));
        }
        ensure_dependencies_complete(&self.load_unlocked(root)?, &task)?;
        if let Some(owner) = task.claimed_by.as_deref() {
            return Err(if owner == agent {
                TaskStoreError::ClaimInstanceMismatch(format!("{id}: existing claim requires its original token; cannot recover it from a display name"))
            } else {
                TaskStoreError::AlreadyClaimed { task_id: id.into(), claimed_by: owner.into() }
            });
        }
        let token = uuid::Uuid::new_v4().to_string();
        task.claimed_by = Some(agent.to_owned());
        let rendered = render_with_claim_token(&task, &existing, Some(&token))?;
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
        let terminal = snapshot.config.columns.iter().filter(|column| column.terminal)
            .min_by_key(|column| column.order)
            .ok_or_else(|| TaskStoreError::Invalid("board has no terminal column".into()))?;
        task.column = terminal.id.clone();
        apply_completion_state(&snapshot.config, &mut task);
        validate_task(&snapshot, &task)?;
        let rendered = render_task(&task, Some(&existing))?;
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
        task.claimed_by = None;
        let rendered = render_with_claim_token(&task, &existing, None)?;
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
        let _lock = acquire_lock(root)?;
        let mut snapshot = self.load_unlocked(root)?;
        if self.maintain_snoozed_columns(root, &snapshot.config)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_completed_tasks(root, &snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_dependency_unblocks(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        if self.maintain_epic_columns(&snapshot)? {
            snapshot = self.load_unlocked(root)?;
        }
        Ok(snapshot)
    }

    fn create(&self, root: &Path, draft: TaskDraft) -> Result<Task, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        validate_draft(&snapshot, &draft)?;

        let prefix = if draft.kind == TaskKind::Epic { "WTE-" } else { "WT-" };
        let next = snapshot
            .tasks
            .iter()
            .filter_map(|t| t.id.strip_prefix(prefix)?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let id = format!("{prefix}{next:04}");
        let created = Local::now().date_naive().to_string();
        let path = root.join(ACTIVE_DIR).join(format!("{id}.md"));
        if path.exists() {
            return Err(TaskStoreError::Conflict(format!("{id} already exists")));
        }

        let mut task = Task {
            schema: SCHEMA_VERSION,
            id,
            kind: draft.kind,
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
            attention: draft.attention,
            question: draft.question,
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
        }
        apply_completion_state(&snapshot.config, &mut task);
        validate_task(&snapshot, &task)?;
        let text = render_task(&task, None)?;
        write_new(&path, &text)?;
        task.revision = revision(&text);
        Ok(task)
    }

    fn update(&self, root: &Path, mut task: Task) -> Result<Task, TaskStoreError> {
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
        if !task.archived {
            apply_completion_state(&snapshot.config, &mut task);
        }
        validate_task(&snapshot, &task)?;
        let text = if task.claimed_by != previous.claimed_by {
            render_with_claim_token(&task, &existing, None)?
        } else { render_task(&task, Some(&existing))? };
        write_text_atomic(&path, &text)?;
        task.revision = revision(&text);
        // Complete dependent release during the same board lock. The next load
        // also covers out-of-band edits and archived prerequisites.
        if !task.archived && task.completed.is_some() && previous.completed.is_none() {
            let current = self.load_unlocked(root)?;
            self.maintain_dependency_unblocks(&current)?;
        }
        Ok(task)
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
        if let Some(owner) = task.claimed_by.as_deref() {
            if owner != agent {
                return Err(TaskStoreError::AlreadyClaimed {
                    task_id: id.to_string(),
                    claimed_by: owner.to_string(),
                });
            }
            return Ok(task);
        }

        task.claimed_by = Some(agent.to_string());
        let text = render_task(&task, Some(&existing))?;
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
        task.claimed_by = None;
        let text = render_task(&task, Some(&existing))?;
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

fn apply_completion_state(config: &BoardConfig, task: &mut Task) {
    let is_terminal = config
        .columns
        .iter()
        .any(|column| column.id == task.column && column.terminal);

    if is_terminal {
        // Done is always final work, never pending human input.
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
    validate_parent_epic_only(snapshot, draft.kind, draft.parent.as_deref(), None)?;
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
    validate_parent_epic_only(snapshot, task.kind, task.parent.as_deref(), Some(&task.id))?;
    validate_attention(&task.attention, task.question.as_ref())?;
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
        if !matches!(icon, "bug" | "feature" | "refactor" | "content" | "validation" | "design" | "visual-work") {
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
    }
    Ok(())
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
        if !(file_name.starts_with("WT-") || file_name.starts_with("WTE-")) || path.extension().and_then(|x| x.to_str()) != Some("md")
        {
            continue;
        }
        match fs::read_to_string(&path)
            .map_err(TaskStoreError::from)
            .and_then(|text| parse_task(&path, archived, &text))
        {
            Ok(task) => out.push(task),
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

fn render_board(config: &BoardConfig, existing: Option<&str>) -> Result<String, TaskStoreError> {
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
        attention: meta.attention,
        question: meta.question,
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
        attention: task.attention.clone(),
        question: task.question.clone(),
        created: task.created.clone(),
        completed: task.completed.clone(),
        completed_at: task.completed_at.clone(),
    };
    let known = serde_yaml::to_value(&meta).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    if let serde_yaml::Value::Mapping(known) = known {
        mapping.remove(serde_yaml::Value::String("parent".into()));
        mapping.remove(serde_yaml::Value::String("depends_on".into()));
        mapping.remove(serde_yaml::Value::String("related_to".into()));
        mapping.remove(serde_yaml::Value::String("position_after".into()));
        mapping.remove(serde_yaml::Value::String("positioned_at".into()));
        mapping.remove(serde_yaml::Value::String("category".into()));
        mapping.remove(serde_yaml::Value::String("icon".into()));
        mapping.remove(serde_yaml::Value::String("claimed_by".into()));
        mapping.remove(serde_yaml::Value::String("attention".into()));
        mapping.remove(serde_yaml::Value::String("question".into()));
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

fn acquire_lock(root: &Path) -> Result<LockGuard, TaskStoreError> {
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

fn write_text_atomic(path: &Path, text: &str) -> Result<(), TaskStoreError> {
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
