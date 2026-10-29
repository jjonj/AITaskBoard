import type { BoardSnapshot, Task } from "./types";

export type UserQueueSnoozes = Record<string, number>;

/**
 * "For user" indicates the eventual owner of a decision, not that the
 * decision is actionable today. A hard prerequisite must be finished (or
 * archived/terminal per TaskBoard's existing completion semantics) before
 * the downstream item enters the human inbox.
 *
 * Missing prerequisite IDs are not silently treated as complete.
 * Attention/question flags do not bypass prerequisites.
 */
export const hasCompletedPrerequisites = (task: Task, board: BoardSnapshot): boolean => {
  if (!task.depends_on.length) return true;
  const byId = new Map(board.tasks.map((item) => [item.id, item]));
  return task.depends_on.every((id) => {
    const prerequisite = byId.get(id);
    return !!prerequisite && (
      prerequisite.archived ||
      !!prerequisite.completed ||
      board.config.columns.some((column) =>
        column.id === prerequisite.column && column.terminal)
    );
  });
};

export const isUserQueueEligible = (
  task: Task, board: BoardSnapshot, snoozes: UserQueueSnoozes, now = Date.now(),
): boolean => {
  // An unanswered question on an AI task is an Inbox request, not a transfer
  // of ownership. It may remain open even when implementation is Done.
  const pendingQuestion = task.kind === "task" && task.attention.includes("question") &&
    !!task.question?.prompt && !task.question.answer;
  // Visual reviews have their own queue. A rejected verdict is finished human
  // work even while the agent is preparing a new round, and an approved
  // review must not reappear simply because the card is still For user.
  if (task.review_status === "open" || task.review_status === "awaiting_revision" ||
      (task.review_status === "resolved" && !pendingQuestion)) return false;
  const userOwned = task.kind === "for_user" ||
    (task.kind === "task" && task.attention.includes("needs-user"));
  const terminal = !!task.completed || board.config.columns.some((column) =>
    column.id === task.column && column.terminal);
  return !task.archived && (pendingQuestion || (userOwned && !terminal)) &&
    hasCompletedPrerequisites(task, board) && (snoozes[task.id] ?? 0) <= now;
};
