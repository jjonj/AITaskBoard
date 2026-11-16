export type Column = {
  id: string;
  name: string;
  order: number;
  terminal: boolean;
  hidden: boolean;
  snoozed_until?: string | null;
  awake_until?: string | null;
  ai_ready: boolean;
  advisory?: boolean;
  auto_queue?: boolean;
  color: string;
};

export type BoardConfig = {
  schema: number;
  name: string;
  task_prefix?: string | null;
  epic_prefix?: string | null;
  columns: Column[];
  releases_enabled?: boolean;
  release_enabled_at?: string | null;
  revision: string;
};

export type TaskAttention = "question" | "needs-user" | "user-note" | "blocked" | "priority" | "prepare" | "announce";
export type TaskIcon = "bug" | "feature" | "refactor" | "content" | "validation" | "design" | "visual-work" | "visual-implementation" | "hard";

export type TaskQuestion = {
  request_type?: "decision" | "verification" | "action" | null;
  prompt: string;
  options: string[];
  answer: string | null;
};

export type TaskKind = "task" | "epic" | "for_user" | "recurring" | "release";
export type ReviewMode = "inbox" | "session";
export type TaskLive = {
  with_me: boolean;
  state?: "ready" | "in_session" | null;
  brief?: string | null;
  est_minutes?: number | null;
  ready_at?: string | null;
  later_until?: string | null;
};
export type Task = {
  kind: TaskKind;
  deployable?: boolean | null;
  repeat_every_days?: number | null;
  next_due?: string | null;
  depends_on: string[];
  related_to: string[];
  position_after?: string | null;
  positioned_at?: string | null;
  schema: number;
  id: string;
  title: string;
  column: string;
  parent: string | null;
  category: string | null;
  icon: TaskIcon | null;
  claimed_by: string | null;
  /** Persistent record of prior agent work; never inferred from the current column. */
  agent_interacted?: boolean;
  /** Absent on legacy tasks, which default to Inbox review. */
  review_mode?: ReviewMode;
  live?: TaskLive | null;
  attention: TaskAttention[];
  question: TaskQuestion | null;
  /** AI-authored, read-only in the desktop app; cleared when unblocked. */
  blocked_reason?: string | null;
  review_status?: "open" | "awaiting_revision" | "resolved" | null;
  review_round?: number | null;
  created: string;
  /** Latest observed file write, not necessarily a substantive task edit. */
  observed_modified_at?: string | null;
  completed: string | null;
  completed_at?: string | null;
  last_run_at?: string | null;
  run_count?: number;
  description: string;
  acceptance_criteria: string[];
  ai_notes: string;
  user_notes: string;
  notes: string;
  archived: boolean;
  path: string;
  revision: string;
};

export type TaskDraft = {
  reserved_id?: string | null;
  reservation_token?: string | null;
  kind: TaskKind;
  review_mode?: ReviewMode;
  deployable?: boolean | null;
  repeat_every_days?: number | null;
  next_due?: string | null;
  depends_on: string[];
  related_to: string[];
  title: string;
  column: string;
  parent: string | null;
  category: string | null;
  icon: TaskIcon | null;
  attention: TaskAttention[];
  question: TaskQuestion | null;
  blocked_reason?: string | null;
  description: string;
  acceptance_criteria: string[];
  ai_notes: string;
  user_notes: string;
  notes: string;
};

export type BoardSnapshot = {
  config: BoardConfig;
  tasks: Task[];
  warnings: string[];
};
