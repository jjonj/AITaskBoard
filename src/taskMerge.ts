import type { Task } from "./types";

/** Keep independent edits from other agents when saving an older editor draft.
 * Never silently overwrite a field changed differently on disk. */
export function mergeTaskEdits(base: Task, edited: Task, disk: Task): {
  merged: Task;
  conflicts: string[];
} {
  const fields: (keyof Task)[] = [
    "title", "column", "parent", "category", "kind", "deployable",
    "repeat_every_days", "next_due", "blocked_reason", "depends_on", "related_to",
    "icon", "attention", "question", "description", "acceptance_criteria",
    "ai_notes", "user_notes", "notes",
  ];
  const merged = { ...disk };
  const conflicts: string[] = [];
  for (const field of fields) {
    const original = JSON.stringify(base[field]);
    const proposed = JSON.stringify(edited[field]);
    const latest = JSON.stringify(disk[field]);
    if (original === proposed) continue;
    if (original !== latest && proposed !== latest) {
      conflicts.push(field);
    } else {
      Object.assign(merged, { [field]: edited[field] });
    }
  }
  return { merged, conflicts };
}
