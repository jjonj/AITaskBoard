// Stable, column-local precedence ordering. Dependencies outrank the usual
// attention/hierarchy rank, while unrelated cards retain their existing order.
// Parent-before-child is a visual ordering constraint, not a hard dependency.
export type OrderedCard = {
  id: string;
  parent: string | null;
  depends_on: string[];
  position_after?: string | null;
  positioned_at?: string | null;
};

/** Local, persistent card insertion. The explicit positions override the
 * default attention/hierarchy sort only when both cards share a column.
 * Apply oldest positions first so the latest drop immediately follows its
 * chosen anchor; hard dependencies are reconciled by the next sorting pass.
 */
export function applyCardPlacements<T extends OrderedCard>(baseOrder: readonly T[]): T[] {
  const arranged = [...baseOrder];
  const positioned = baseOrder
    .filter((item) => item.position_after && item.position_after !== item.id)
    .sort((a, b) => (a.positioned_at || "").localeCompare(b.positioned_at || "") ||
      a.id.localeCompare(b.id));
  for (const item of positioned) {
    const original = arranged.findIndex((candidate) => candidate.id === item.id);
    if (original < 0) continue;
    const anchorId = item.position_after;
    if (!arranged.some((candidate) => candidate.id === anchorId)) continue;
    arranged.splice(original, 1);
    const anchor = arranged.findIndex((candidate) => candidate.id === anchorId);
    if (anchor >= 0) arranged.splice(anchor + 1, 0, item);
  }
  return arranged;
}

export function orderCardsByDependencies<T extends OrderedCard>(baseOrder: readonly T[]): T[] {
  const byId = new Map(baseOrder.map((task) => [task.id, task]));
  const originalRank = new Map(baseOrder.map((task, index) => [task.id, index]));
  const incoming = new Map(baseOrder.map((task) => [task.id, 0]));
  const outgoing = new Map<string, Set<string>>();
  for (const task of baseOrder) {
    const prerequisites = new Set(task.depends_on || []);
    if (task.parent && byId.has(task.parent)) prerequisites.add(task.parent);
    for (const prerequisite of prerequisites) {
      if (!byId.has(prerequisite) || prerequisite === task.id) continue;
      if (!outgoing.has(prerequisite)) outgoing.set(prerequisite, new Set());
      if (!outgoing.get(prerequisite)!.has(task.id)) {
        outgoing.get(prerequisite)!.add(task.id);
        incoming.set(task.id, incoming.get(task.id)! + 1);
      }
    }
  }

  const available = baseOrder.filter((task) => incoming.get(task.id) === 0);
  const sorted: T[] = [];
  const visited = new Set<string>();
  while (available.length) {
    available.sort((a, b) => originalRank.get(a.id)! - originalRank.get(b.id)!);
    const current = available.shift()!;
    sorted.push(current);
    visited.add(current.id);
    for (const id of outgoing.get(current.id) || []) {
      incoming.set(id, incoming.get(id)! - 1);
      if (incoming.get(id) === 0) available.push(byId.get(id)!);
    }
  }
  // Corrupt/manual cycles should not make cards disappear. The backend
  // prevents dependency cycles, but a hierarchy+dependency cycle is possible.
  for (const card of baseOrder) {
    if (!visited.has(card.id)) sorted.push(card);
  }
  return sorted;
}
