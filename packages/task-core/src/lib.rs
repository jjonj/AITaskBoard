mod model;
mod store;
mod release;
mod block_gate;
mod handoff;
pub use release::*;
pub use block_gate::challenge_agent_block;

pub use model::*;
pub use handoff::{handoff_bundle, HandoffPacket};
pub use store::review::*;
pub use store::{default_board_config, default_column_ai_ready, recurring_due, is_task_blocked, queue_task_ready, ExecutionIntent, ExecutionMode, QueueState, FsTaskStore, MutationActor, TaskIdReservation, TaskStore, TaskStoreError, BLOCKED_COLUMN_ID};

pub const SCHEMA_VERSION: u32 = 1;
pub const DRAFT_COLUMN_ID: &str = "draft";
pub const AI_READY_COLUMN_ID: &str = "ai-ready";
pub const IN_PROGRESS_COLUMN_ID: &str = "in-progress";
pub const BUSY_RECOVERY_IDLE_HOURS: i64 = 3;
pub const BUSY_STARTING_RECOVERY_IDLE_MINUTES: i64 = 10;

/// Starting is a short-lived launch phase; other busy work retains the 3-hour idle window.
pub fn busy_recovery_idle_threshold(state: Option<QueueState>) -> chrono::Duration {
    if state == Some(QueueState::Starting) {
        chrono::Duration::minutes(BUSY_STARTING_RECOVERY_IDLE_MINUTES)
    } else {
        chrono::Duration::hours(BUSY_RECOVERY_IDLE_HOURS)
    }
}
