import type { Task } from "./types";

/** A proxy is only visual. The original epic remains the sole mutable task. */
export type EpicProxyRow =
  | { type: "task"; task: Task }
  | { type: "proxy"; epic: Task };

/** Nearest actual epic ancestor which lives outside the child's column.
 * If the nearest epic is already in this column, that real card acts as the
 * parent and another (more remote) proxy would only add misleading nesting.
 */
export function remoteEpicAncestor(
  task: Pick<Task, "parent" | "column">,
  byId: ReadonlyMap<string, Task>,
): Task | null {
  let parentId = task.parent;
  const visited = new Set<string>();
  while (parentId && !visited.has(parentId)) {
    visited.add(parentId);
    const parent = byId.get(parentId);
    if (!parent) return null; // Archived or otherwise absent parent.
    if (parent.kind === "epic") {
      return parent.column === task.column ? null : parent;
    }
    parentId = parent.parent;
  }
  return null;
}

/** Add a thin identity-only proxy above each uninterrupted run of cards
 * whose nearest epic is in a different column. If another group splits
 * the run (e.g. a topological dependency ordering), repeat the proxy so
 * no later card is left visibly orphaned. Never change actual task order.
 */
export function withEpicProxyRows(
  orderedTasks: readonly Task[],
  byId: ReadonlyMap<string, Task>,
): EpicProxyRow[] {
  const result: EpicProxyRow[] = [];
  let previousRemoteEpicId: string | null = null;
  for (const task of orderedTasks) {
    const epic = remoteEpicAncestor(task, byId);
    const remoteId = epic?.id ?? null;
    if (epic && remoteId !== previousRemoteEpicId) {
      result.push({ type: "proxy", epic });
    }
    result.push({ type: "task", task });
    previousRemoteEpicId = remoteId;
  }
  return result;
}
