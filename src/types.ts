export type Column = {
  id: string;
  name: string;
  order: number;
  terminal: boolean;
  hidden: boolean;
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

export type TaskQuestion = {
  prompt: string;
  options: string[];
  answer: string | null;
};

export type Task = {
  schema: number;
  id: string;
  title: string;
  column: string;
  parent: string | null;
  category: string | null;
  claimed_by: string | null;
  attention: TaskAttention[];
  question: TaskQuestion | null;
  created: string;
  completed: string | null;
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
  title: string;
  column: string;
  parent: string | null;
  category: string | null;
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
