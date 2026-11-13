use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool { !*value }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Column {
    pub id: String,
    pub name: String,
    pub order: i32,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awake_until: Option<String>,
    #[serde(default)]
    pub ai_ready: bool,
    /// Advisory columns contain temporary board-wide instructions, not tasks.
    #[serde(default, skip_serializing_if = "is_false")]
    pub advisory: bool,
    /// Durable column-wide default for future executable tasks; never a scheduler.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto_queue: bool,
    #[serde(default)]
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardConfig {
    pub schema: u32,
    pub name: String,
    // Canonical ID namespaces for NEW entries only. None retains legacy WT/WTE.
    // Historical IDs are immutable and remain valid after a prefix change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_prefix: Option<String>,
    pub columns: Vec<Column>,
    #[serde(default)] pub releases_enabled: bool,
    #[serde(default)] pub release_enabled_at: Option<String>,
    #[serde(default)]
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TaskAttention {
    Question,
    NeedsUser,
    UserNote,
    Blocked,
    Priority,
    Prepare,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskQuestion {
    /// decision | verification | action. Legacy questions remain untyped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_type: Option<String>,
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    #[default]
    Task,
    Epic,
    ForUser,
    Recurring,
    Release,
}

/// Optional live collaboration state. A missing block is a normal task.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveTaskState { Ready, InSession }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskLive {
    pub with_me: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<LiveTaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub est_minutes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ready_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub later_until: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default)] pub deployable: Option<bool>,
    /// Optional interval: None means immediately eligible again after completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_every_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub run_count: u32,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub related_to: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position_after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub positioned_at: Option<String>,
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub column: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub claimed_by: Option<String>,
    /// Remains true after an agent claim or write, even if the claim is released.
    #[serde(default)]
    pub agent_interacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<TaskLive>,
    #[serde(default)]
    pub attention: Vec<TaskAttention>,
    #[serde(default)]
    pub question: Option<TaskQuestion>,
    /// AI-authored reason the task is blocked. Required when an agent moves a
    /// task into the Blocked column or adds the `blocked` flag; cleared
    /// automatically once the task is no longer blocked. Human edits through
    /// the desktop editor cannot change it.
    #[serde(default)]
    pub blocked_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_round: Option<u32>,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub ai_notes: String,
    #[serde(default)]
    pub user_notes: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub archived: bool,
    pub path: String,
    #[serde(default)]
    pub revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardSnapshot {
    pub config: BoardConfig,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskDraft {
    // Only the desktop creation dialog sends these; CLI creates normally.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservation_token: Option<String>,
    #[serde(default)]
    pub kind: TaskKind,
    #[serde(default)] pub deployable: Option<bool>,
    /// Optional interval: None means immediately eligible again after completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_every_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_due: Option<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub related_to: Vec<String>,
    pub title: String,
    pub column: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub attention: Vec<TaskAttention>,
    #[serde(default)]
    pub question: Option<TaskQuestion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_reason: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub ai_notes: String,
    #[serde(default)]
    pub user_notes: String,
    #[serde(default)]
    pub notes: String,
}
