import test from "node:test";
import assert from "node:assert/strict";
import { isHiddenByCollapsedEpic, toggleAllEpicCollapses } from "./epicCollapse.ts";
import { withEpicProxyRows } from "./epicProxyRows.ts";

const card = (id, kind, parent = null, column = "ai-ready") => ({
  id, kind, parent, column, depends_on: [], related_to: [],
});
const graph = [
  card("WTE-0001", "epic", null, "combat-implementation"),
  card("WT-0101", "task", "WTE-0001", "ai-ready"),
  card("WT-0102", "task", "WTE-0001", "combat-implementation"),
  card("WT-0103", "for_user", "WTE-0001", "ai-ready"),
  card("WTE-0002", "epic", null, "ai-ready"),
  card("WT-0201", "task", "WTE-0002", "ai-ready"),
];
const byId = new Map(graph.map((task) => [task.id, task]));

test("collapsing epic hides ordinary tasks in its own and other columns", () => {
  const collapsed = new Set(["WTE-0001"]);
  for (const id of ["WT-0101", "WT-0102"]) {
    assert.equal(isHiddenByCollapsedEpic(byId.get(id), collapsed, byId), true, id);
  }
  assert.equal(isHiddenByCollapsedEpic(byId.get("WTE-0001"), collapsed, byId), false);
  assert.equal(isHiddenByCollapsedEpic(byId.get("WT-0201"), collapsed, byId), false);
});

test("orange For user tasks remain visible under collapsed epic, even cross-column", () => {
  const collapsed = new Set(["WTE-0001"]);
  const visible = graph.filter((task) => !isHiddenByCollapsedEpic(task, collapsed, byId));
  assert.ok(visible.some((task) => task.id === "WT-0103"));
  const remote = withEpicProxyRows(visible.filter((t) => t.column === "ai-ready"), byId);
  assert.ok(remote.some((row) => row.type === "proxy" && row.epic.id === "WTE-0001"),
    "orange task still receives its cross-column epic proxy");
  assert.ok(!visible.some((task) => task.id === "WT-0101"));
});

test("collapse all folds all epics; second click expands all", () => {
  const all = ["WTE-0001", "WTE-0002"];
  let collapsed = toggleAllEpicCollapses(new Set(), all);
  assert.deepEqual([...collapsed].sort(), all);
  collapsed = toggleAllEpicCollapses(collapsed, all);
  assert.deepEqual([...collapsed], []);
});

test("collapse all repairs partially expanded state instead of toggling individual epics", () => {
  const all = ["WTE-0001", "WTE-0002"];
  const collapsed = toggleAllEpicCollapses(new Set(["WTE-0001"]), all);
  assert.deepEqual([...collapsed].sort(), all);
});

test("collapse state persists as ID list without affecting tasks or dependencies", () => {
  const all = ["WTE-0001", "WTE-0002"];
  const collapsed = toggleAllEpicCollapses(new Set(), all);
  const restored = new Set(JSON.parse(JSON.stringify([...collapsed])));
  assert.equal(isHiddenByCollapsedEpic(byId.get("WT-0101"), restored, byId), true);
  assert.equal(isHiddenByCollapsedEpic(byId.get("WT-0103"), restored, byId), false);
});

test("old collapsed regular-task IDs do not hide other work", () => {
  assert.equal(isHiddenByCollapsedEpic(byId.get("WT-0101"), new Set(["WT-0102"]), byId), false);
});

test("missing parent or cyclic old metadata cannot hang collapse traversal", () => {
  const orphan = card("WT-9999", "task", "WT-missing");
  const a = card("WT-a", "task", "WT-b");
  const b = card("WT-b", "task", "WT-a");
  const m = new Map([...byId, [orphan.id, orphan], [a.id, a], [b.id, b]]);
  assert.equal(isHiddenByCollapsedEpic(orphan, new Set(["WTE-0001"]), m), false);
  assert.equal(isHiddenByCollapsedEpic(a, new Set(["WTE-0001"]), m), false);
});
