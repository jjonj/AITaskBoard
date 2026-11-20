import type { Column, Task } from "./types";

export type HumanDependencyStop = {
  task: Task;
  reason: "draft" | "for-user" | "answer";
  affectedIds: string[];
};

const causeOf = (task: Task): HumanDependencyStop["reason"] | null => {
  if (task.column === "draft") return "draft";
  if (task.kind === "for_user") return "for-user";
  if (task.question && !task.question.answer && task.attention.includes("question")) return "answer";
  return null;
};

/** Walk unresolved prerequisite chains, including cards in hidden columns. */
export function humanDependencyStops(tasks: Task[], columns: Column[]) {
  const byId = new Map(tasks.map((task) => [task.id, task]));
  const terminal = new Set(columns.filter((column) => column.terminal).map((column) => column.id));
  const completed = (task: Task) => task.archived || !!task.completed || terminal.has(task.column);

  const rootsFor = (id: string, path: Set<string>): Set<string> => {
    const task = byId.get(id);
    if (!task || completed(task) || path.has(id)) return new Set();
    const nextPath = new Set(path);
    nextPath.add(id);
    const roots = new Set<string>(causeOf(task) ? [id] : []);
    for (const prerequisite of task.depends_on) {
      for (const root of rootsFor(prerequisite, nextPath)) roots.add(root);
    }
    return roots;
  };

  const forTask = new Map<string, string[]>();
  const affected = new Map<string, Set<string>>();
  for (const task of tasks) {
    if (completed(task)) continue;
    const roots = new Set<string>();
    for (const prerequisite of task.depends_on) {
      for (const root of rootsFor(prerequisite, new Set([task.id]))) roots.add(root);
    }
    if (!roots.size) continue;
    const ids = [...roots].sort();
    forTask.set(task.id, ids);
    for (const id of ids) {
      if (!affected.has(id)) affected.set(id, new Set());
      affected.get(id)!.add(task.id);
    }
  }

  const stops: HumanDependencyStop[] = [...affected.entries()]
    .map(([id, downstream]) => {
      const task = byId.get(id)!;
      return { task, reason: causeOf(task)!, affectedIds: [...downstream].sort() };
    })
    .sort((a, b) => b.affectedIds.length - a.affectedIds.length || a.task.id.localeCompare(b.task.id));

  return { forTask, stops };
}
