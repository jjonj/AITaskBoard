export type Column = {
  id: string;
  name: string;
  order: number;
  terminal: boolean;
  hidden: boolean;
  snoozed_until?: string | null;
  awake_until?: string | null;
  ai_ready: boolean;
  color: string;
};

export type BoardConfig = {
  schema: number;
  name: string;
  columns: Column[];
  revision: string;
};

export type TaskAttention = "question" | "needs-user" | "user-note" | "blocked";
export type TaskIcon = "bug" | "feature" | "refactor" | "content" | "validation" | "design" | "visual-work";

export type TaskQuestion = {
  prompt: string;
  options: string[];
  answer: string | null;
};

export type TaskKind = "task" | "epic" | "for_user";
export type Task = {
  kind: TaskKind;
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
  attention: TaskAttention[];
  question: TaskQuestion | null;
  created: string;
  completed: string | null;
  completed_at?: string | null;
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
  kind: TaskKind;
  depends_on: string[];
  related_to: string[];
  title: string;
  column: string;
  parent: string | null;
  category: string | null;
  icon: TaskIcon | null;
  attention: TaskAttention[];
  question: TaskQuestion | null;
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
