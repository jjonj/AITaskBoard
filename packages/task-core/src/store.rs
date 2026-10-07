use crate::{
    BoardConfig, BoardSnapshot, Column, Task, TaskAttention, TaskDraft, TaskQuestion,
    AI_READY_COLUMN_ID, DRAFT_COLUMN_ID, IN_PROGRESS_COLUMN_ID, SCHEMA_VERSION,
};
use chrono::{Datelike, Local, NaiveDate};
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
    schema: u32,
    id: String,
    title: String,
    column: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    claimed_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    attention: Vec<TaskAttention>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    question: Option<TaskQuestion>,
    created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    completed: Option<String>,
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
                ai_ready: false,
                color: "#8e7da8".into(),
            },
            Column {
                id: AI_READY_COLUMN_ID.into(),
                name: "AI Ready".into(),
                order: 1,
                terminal: false,
                hidden: false,
                ai_ready: true,
                color: "#718f78".into(),
            },
            Column {
                id: IN_PROGRESS_COLUMN_ID.into(),
                name: "In Progress".into(),
                order: 2,
                terminal: false,
                hidden: false,
                ai_ready: false,
                color: "#7388a8".into(),
            },
            Column {
                id: "blocked".into(),
                name: "Blocked".into(),
                order: 3,
                terminal: false,
                hidden: false,
                ai_ready: false,
                color: "#a48669".into(),
            },
            Column {
                id: "done".into(),
                name: "Done".into(),
                order: 4,
                terminal: true,
                hidden: false,
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
        }

        Ok(BoardSnapshot {
            config,
            tasks,
            warnings,
        })
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

            if task.completed != normalized {
                let path = PathBuf::from(&task.path);
                let existing = fs::read_to_string(&path)?;
                task.completed = normalized;
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

impl TaskStore for FsTaskStore {
    fn load(&self, root: &Path) -> Result<BoardSnapshot, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        if self.maintain_completed_tasks(root, &snapshot)? {
            self.load_unlocked(root)
        } else {
            Ok(snapshot)
        }
    }

    fn create(&self, root: &Path, draft: TaskDraft) -> Result<Task, TaskStoreError> {
        let _lock = acquire_lock(root)?;
        let snapshot = self.load_unlocked(root)?;
        validate_draft(&snapshot, &draft)?;

        let next = snapshot
            .tasks
            .iter()
            .filter_map(|t| t.id.strip_prefix("WT-")?.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        let id = format!("WT-{next:04}");
        let created = Local::now().date_naive().to_string();
        let path = root.join(ACTIVE_DIR).join(format!("{id}.md"));
        if path.exists() {
            return Err(TaskStoreError::Conflict(format!("{id} already exists")));
        }

        let mut task = Task {
            schema: SCHEMA_VERSION,
            id,
            title: draft.title.trim().to_string(),
            column: draft.column,
            parent: draft.parent,
            category: draft.category,
            claimed_by: None,
            attention: draft.attention,
            question: draft.question,
            created,
            completed: None,
            description: draft.description,
            acceptance_criteria: draft.acceptance_criteria,
            ai_notes: draft.ai_notes,
            user_notes: draft.user_notes,
            notes: draft.notes,
            archived: false,
            path: path.to_string_lossy().to_string(),
            revision: String::new(),
        };
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
        if !task.archived {
            apply_completion_state(&snapshot.config, &mut task);
        }
        validate_task(&snapshot, &task)?;
        let text = render_task(&task, Some(&existing))?;
        write_text_atomic(&path, &text)?;
        task.revision = revision(&text);
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
        if task
            .completed
            .as_deref()
            .and_then(parse_task_date)
            .is_none()
        {
            task.completed = Some(Local::now().date_naive().to_string());
        }
    } else {
        task.completed = None;
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
    if let Some(parent) = &draft.parent {
        if !snapshot
            .tasks
            .iter()
            .any(|t| &t.id == parent && !t.archived)
        {
            return Err(TaskStoreError::Invalid(format!(
                "parent {parent} does not exist"
            )));
        }
    }
    validate_attention(&draft.attention, draft.question.as_ref())?;
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
    if let Some(parent) = &task.parent {
        if parent == &task.id {
            return Err(TaskStoreError::Invalid("task cannot parent itself".into()));
        }
        if !snapshot
            .tasks
            .iter()
            .any(|t| &t.id == parent && !t.archived)
        {
            return Err(TaskStoreError::Invalid(format!(
                "parent {parent} does not exist"
            )));
        }
        if creates_cycle(snapshot, &task.id, parent) {
            return Err(TaskStoreError::Invalid(
                "parent change creates a cycle".into(),
            ));
        }
    }
    validate_attention(&task.attention, task.question.as_ref())?;
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

fn creates_cycle(snapshot: &BoardSnapshot, task_id: &str, parent_id: &str) -> bool {
    let parents: HashMap<&str, Option<&str>> = snapshot
        .tasks
        .iter()
        .map(|t| (t.id.as_str(), t.parent.as_deref()))
        .collect();
    let mut current = Some(parent_id);
    let mut visited = HashSet::new();
    while let Some(id) = current {
        if id == task_id || !visited.insert(id) {
            return true;
        }
        current = parents.get(id).copied().flatten();
    }
    false
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
        if !file_name.starts_with("WT-") || path.extension().and_then(|x| x.to_str()) != Some("md")
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
        title: meta.title,
        column: meta.column,
        parent: meta.parent,
        category: meta.category,
        claimed_by: meta.claimed_by,
        attention: meta.attention,
        question: meta.question,
        created: meta.created,
        completed: meta.completed,
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
        title: task.title.clone(),
        column: task.column.clone(),
        parent: task.parent.clone(),
        category: task.category.clone(),
        claimed_by: task.claimed_by.clone(),
        attention: task.attention.clone(),
        question: task.question.clone(),
        created: task.created.clone(),
        completed: task.completed.clone(),
    };
    let known = serde_yaml::to_value(&meta).map_err(|e| TaskStoreError::Parse(e.to_string()))?;
    if let serde_yaml::Value::Mapping(known) = known {
        mapping.remove(serde_yaml::Value::String("parent".into()));
        mapping.remove(serde_yaml::Value::String("category".into()));
        mapping.remove(serde_yaml::Value::String("claimed_by".into()));
        mapping.remove(serde_yaml::Value::String("attention".into()));
        mapping.remove(serde_yaml::Value::String("question".into()));
        mapping.remove(serde_yaml::Value::String("completed".into()));
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
    body = replace_markdown_section(&body, "## Acceptance Criteria", &criteria);
    body = replace_markdown_section(&body, "## AI Notes", task.ai_notes.trim());
    body = replace_markdown_section(&body, "## User Notes", task.user_notes.trim());
    if !task.notes.trim().is_empty() || body.lines().any(|line| line.trim() == "## Notes") {
        body = replace_markdown_section(&body, "## Notes", task.notes.trim());
    }
    body
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
