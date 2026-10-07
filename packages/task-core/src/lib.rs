mod model;
mod store;

pub use model::*;
pub use store::{default_board_config, FsTaskStore, TaskStore, TaskStoreError};

pub const SCHEMA_VERSION: u32 = 1;
pub const DRAFT_COLUMN_ID: &str = "draft";
pub const AI_READY_COLUMN_ID: &str = "ai-ready";
pub const IN_PROGRESS_COLUMN_ID: &str = "in-progress";
