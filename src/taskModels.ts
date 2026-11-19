import type { TaskModelSelection } from "./types";

const GLOBAL_DEFAULT_KEY = "taskboard:global-default-task-model:v1";
const EFFORTS = ["low", "medium", "high", "xhigh"] as const;

export function readDefaultTaskModel(): TaskModelSelection | null {
  try {
    const saved = window.localStorage.getItem(GLOBAL_DEFAULT_KEY);
    if (!saved) return null;
    const selection: unknown = JSON.parse(saved);
    if (!selection || typeof selection !== "object") return null;
    const row = selection as Record<string, unknown>;
    return typeof row.provider === "string" && row.provider.trim() &&
      typeof row.model === "string" && row.model.trim() &&
      EFFORTS.includes(row.effort as typeof EFFORTS[number])
      ? { provider: row.provider, model: row.model, effort: row.effort as TaskModelSelection["effort"] }
      : null;
  } catch { return null; }
}

export function saveDefaultTaskModel(selection: TaskModelSelection | null): void {
  window.localStorage.setItem(GLOBAL_DEFAULT_KEY, selection ? JSON.stringify(selection) : "");
}

export const taskModelKey = (provider: string, model: string) => JSON.stringify([provider, model]);
