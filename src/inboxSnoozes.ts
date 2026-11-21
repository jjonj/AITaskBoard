import type { Task } from "./types";

export type InboxSnooze = number | { until: number; requestKey: string };
export type InboxSnoozes = Record<string, InboxSnooze>;

type InboxRequest = Pick<Task, "kind" | "attention" | "question" | "review_status" | "review_round">;

export const hasPendingQuestion = (task: InboxRequest): boolean =>
  task.kind === "task" && task.attention.includes("question") &&
  !!task.question?.prompt.trim() && !task.question.answer;

// A snooze belongs to one request, not every future question on the same task.
export const inboxRequestKey = (task: InboxRequest): string =>
  task.review_status === "open"
    ? `review:${task.review_round ?? 0}:${task.question?.prompt ?? ""}`
    : hasPendingQuestion(task) ? `question:${task.question!.prompt}` : "user";

export const readInboxSnoozes = (): InboxSnoozes => {
  try {
    const value: unknown = JSON.parse(window.localStorage.getItem("taskboard.inboxSnoozes") || "{}");
    if (!value || typeof value !== "object" || Array.isArray(value)) return {};
    return value as InboxSnoozes;
  } catch { return {}; }
};

export const isInboxItemSnoozed = (
  snoozes: InboxSnoozes, path: string, taskId: string, requestKey: string, now = Date.now(),
): boolean => {
  const entry = snoozes[`${path}|${taskId}`];
  // Preserve legacy snoozes for For user work, but never hide a new question/review.
  if (typeof entry === "number") return requestKey === "user" && entry > now;
  return !!entry && entry.requestKey === requestKey && entry.until > now;
};
