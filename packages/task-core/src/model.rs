use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Column {
    pub id: String,
    pub name: String,
    pub order: i32,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub ai_ready: bool,
    #[serde(default)]
    pub color: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardConfig {
    pub schema: u32,
    pub name: String,
    pub columns: Vec<Column>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskQuestion {
    pub prompt: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub schema: u32,
    pub id: String,
    pub title: String,
    pub column: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub claimed_by: Option<String>,
    #[serde(default)]
    pub attention: Vec<TaskAttention>,
    #[serde(default)]
    pub question: Option<TaskQuestion>,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<String>,
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
    pub title: String,
    pub column: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub attention: Vec<TaskAttention>,
    #[serde(default)]
    pub question: Option<TaskQuestion>,
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
