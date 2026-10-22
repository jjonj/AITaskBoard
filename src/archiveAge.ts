import type { Task } from "./types";

/** Use timestamp stamped on entry to Done, not creation time or file mtime.
 * Missing/invalid timestamps are deliberately not guessed. Epics remain
 * until all of their still-active children can also be archived.
 */
export function eligibleForAgeArchive<T extends Pick<Task, "id" | "parent" | "kind" | "column" | "archived" | "completed_at">>(
  tasks: readonly T[],
  columnId: string,
  cutoffIso: string,
) {
  const cutoff = Date.parse(cutoffIso);
  if (!Number.isFinite(cutoff)) return [];
  const qualified = tasks.filter((task) => {
    if (task.archived || task.column !== columnId || !task.completed_at) return false;
    const completedAt = Date.parse(task.completed_at);
    return Number.isFinite(completedAt) && completedAt <= cutoff;
  });
  const selected = new Set(qualified.map((task) => task.id));
  // No auto-unparenting as part of age cleanup. This safeguard also handles
  // potential historical nested-epic data through iterative pruning.
  let changed = true;
  while (changed) {
    changed = false;
    for (const task of qualified) {
      if (!selected.has(task.id) || task.kind !== "epic") continue;
      if (tasks.some((child) => !child.archived && child.parent === task.id && !selected.has(child.id))) {
        selected.delete(task.id);
        changed = true;
      }
    }
  }
  return qualified.filter((task) => selected.has(task.id));
}
