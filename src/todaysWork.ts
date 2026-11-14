import type { Column, Task } from "./types";

const DAY_MS = 24 * 60 * 60 * 1000;
const MAX_PICKUPS = 10;

// Compare calendar days in the viewer's local timezone, not UTC midnight.
const localDay = (date: Date) =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;

const timestamp = (value?: string | null) => {
  // TaskBoard date-only values represent local dates (UTC parsing shifts them).
  const normalized = value && /^\d{4}-\d{2}-\d{2}$/.test(value) ? `${value}T00:00:00` : value;
  const ms = Date.parse(normalized || "");
  return Number.isFinite(ms) ? ms : 0;
};

const onDay = (value: string | null | undefined, day: string) => {
  if (!value) return false;
  // TaskBoard's 'created' and legacy 'completed' are local YYYY-MM-DD.
  if (/^\d{4}-\d{2}-\d{2}$/.test(value)) return value === day;
  const ms = timestamp(value);
  return !!ms && localDay(new Date(ms)) === day;
};

const recentlyCreated = (task: Task, now: number) => {
  const created = timestamp(task.created);
  return created > 0 && created <= now && now - created < 7 * DAY_MS;
};

/**
 * A focused view of the current board, not a second dispatcher.
 * Tiers mirror T3 Keep Busy / GetSuggestedTasks: Priority, stale In Progress
 * (24h), stale technical Blocked (6h), recent AI-ready (7d), then AI-ready.
 * Claimed, human-owned, draft, complete and dependency-waiting work cannot be
 * suggested for pickup. Today activity is a UNION with the best ten pickups.
 */
export function todaysWorkIds(
  tasks: Task[],
  columns: Column[],
  isComplete: (id: string) => boolean,
  now: number = Date.now(),
): Set<string> {
  const day = localDay(new Date(now));
  const columnById = new Map(columns.map(column => [column.id, column]));
  const candidates: Array<{ task: Task; tier: number }> = [];

  for (const task of tasks) {
    if (task.archived) continue;
    const column = columnById.get(task.column);
    const complete = !!task.completed || !!column?.terminal;
    const touchedToday = [
      task.created, task.observed_modified_at, task.completed_at,
      task.completed, task.last_run_at, task.positioned_at,
    ].some(value => onDay(value, day));
    // Selection includes completed work and blocked work actually touched today.
    if (touchedToday) candidates.push({ task, tier: 0 });

    if (complete || task.claimed_by ||
        !["task", "recurring"].includes(task.kind) ||
        task.column === "draft" ||
        task.kind === "task" && task.attention.includes("needs-user") ||
        task.attention.includes("question") && task.question?.answer == null ||
        task.depends_on.some(id => !isComplete(id)) ||
        task.attention.includes("prepare")) continue;

    const changed = timestamp(task.observed_modified_at) || timestamp(task.created);
    const age = now - changed;
    const isTechnicalBlocked = (task.column === "blocked" || task.attention.includes("blocked")) &&
      !!task.blocked_reason?.trim();
    const isReady = !!column?.ai_ready && !column.terminal &&
      task.column !== "blocked" && !task.attention.includes("blocked");
    let tier: number | null = null;
    if (task.attention.includes("priority")) tier = 1;
    else if (task.column === "in-progress" && age >= DAY_MS) tier = 2;
    else if (isTechnicalBlocked && age >= 6 * 60 * 60 * 1000) tier = 3;
    else if (isReady) tier = recentlyCreated(task, now) ? 4 : 5;
    if (tier != null) candidates.push({ task, tier });
  }

  const result = new Set(candidates.filter(item => item.tier === 0).map(item => item.task.id));
  // Only the ten highest-priority outstanding pickup candidates, even when
  // those same tasks also appear in today's activity set.
  candidates.filter(item => item.tier > 0)
    .sort((a, b) => a.tier - b.tier ||
      timestamp(b.task.created) - timestamp(a.task.created) ||
      a.task.id.localeCompare(b.task.id))
    .slice(0, MAX_PICKUPS)
    .forEach(item => result.add(item.task.id));
  return result;
}
