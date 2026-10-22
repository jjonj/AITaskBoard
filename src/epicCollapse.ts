import type { Task } from "./types";

type CollapseCard = Pick<Task, "id" | "kind" | "parent">;

/** Only epics hide their descendants. Human-owned (orange) cards must
 * remain visible even when their epic is collapsed, across all columns.
 */
export function isHiddenByCollapsedEpic(
  task: CollapseCard,
  collapsedIds: ReadonlySet<string>,
  cards: ReadonlyMap<string, CollapseCard>,
): boolean {
  if (task.kind === "for_user" || task.kind === "epic") return false;
  let parentId = task.parent;
  const seen = new Set<string>();
  while (parentId && !seen.has(parentId)) {
    seen.add(parentId);
    const parent = cards.get(parentId);
    if (!parent) break;
    if (parent.kind === "epic" && collapsedIds.has(parentId)) return true;
    parentId = parent.parent;
  }
  return false;
}

/** An all-epics button is reversible: first collapse every epic, then
 * expand every epic. Manually expanded epics are re-collapsed on the next
 * "Collapse all" operation rather than clearing the other collapsed ids.
 */
export function toggleAllEpicCollapses(
  current: ReadonlySet<string>,
  epicIds: readonly string[],
): Set<string> {
  const next = new Set(current);
  if (!epicIds.length) return next;
  const allCollapsed = epicIds.every((id) => current.has(id));
  for (const id of epicIds) {
    if (allCollapsed) next.delete(id);
    else next.add(id);
  }
  return next;
}
