import test from "node:test";
import assert from "node:assert/strict";
import { remoteEpicAncestor, withEpicProxyRows } from "./epicProxyRows.ts";

const task = (id, column, parent = null, kind = "task") => ({
  id, column, parent, kind, depends_on: [],
});
const asMap = (...items) => new Map(items.map((item) => [item.id, item]));

test("subtask in different column receives a visual proxy for its epic", () => {
  const epic = task("WT-0075", "in-progress", null, "epic");
  const child = task("WT-0118", "combat-implementation", epic.id);
  const byId = asMap(epic, child);
  assert.equal(remoteEpicAncestor(child, byId), epic);
  const rows = withEpicProxyRows([child], byId);
  assert.deepEqual(rows.map((row) => row.type), ["proxy", "task"]);
  assert.equal(rows[0].epic?.id, epic.id);
});

test("the actual epic is never replaced by a proxy in its own column", () => {
  const epic = task("WTE-0001", "in-progress", null, "epic");
  const child = task("WT-0010", "in-progress", epic.id);
  const rows = withEpicProxyRows([epic, child], asMap(epic, child));
  assert.deepEqual(rows.map((row) => row.type), ["task", "task"]);
});

test("an intermediate regular task does not hide the missing epic ancestor", () => {
  const epic = task("WTE-0002", "in-progress", null, "epic");
  const parent = task("WT-0010", "ai-ready", epic.id);
  const child = task("WT-0011", "ai-ready", parent.id);
  const rows = withEpicProxyRows([parent, child], asMap(epic, parent, child));
  assert.deepEqual(rows.map((row) => row.type), ["proxy", "task", "task"]);
  assert.equal(rows[0].epic?.id, epic.id);
});

test("nearer epic within column provides the real parent: no grandparent proxy", () => {
  const grandparent = task("WTE-0002", "in-progress", null, "epic");
  const epic = task("WTE-0003", "ai-ready", grandparent.id, "epic");
  const child = task("WT-0011", "ai-ready", epic.id);
  const rows = withEpicProxyRows([epic, child], asMap(grandparent, epic, child));
  assert.deepEqual(rows.map((row) => row.type), ["proxy", "task", "task"]);
  assert.equal(rows[0].epic?.id, grandparent.id);
  assert.equal(remoteEpicAncestor(child, asMap(grandparent, epic, child)), null);
});

test("separated descendants each get a proxy, preserving dependency sort", () => {
  const epic = task("WTE-0004", "in-progress", null, "epic");
  const first = task("WT-0001", "ai-ready", epic.id);
  const unrelated = task("WT-0002", "ai-ready");
  const second = task("WT-0003", "ai-ready", epic.id);
  const ordered = [first, unrelated, second];
  const rows = withEpicProxyRows(ordered, asMap(epic, ...ordered));
  assert.deepEqual(rows.map((row) => row.type === "task" ? row.task.id : `proxy:${row.epic.id}`),
    ["proxy:WTE-0004", "WT-0001", "WT-0002", "proxy:WTE-0004", "WT-0003"]);
  assert.deepEqual(ordered.map((x) => x.id), ["WT-0001", "WT-0002", "WT-0003"]);
});

test("collapsed/search filtered children do not produce ghost proxies", () => {
  const epic = task("WTE-0005", "in-progress", null, "epic");
  const child = task("WT-0005", "ai-ready", epic.id);
  assert.deepEqual(withEpicProxyRows([], asMap(epic, child)), []);
  assert.deepEqual(withEpicProxyRows([task("WT-0006", "ai-ready")], asMap(epic, child)).map((r) => r.type), ["task"]);
});

test("broken/cyclic legacy parent pointers cannot create infinite ancestor traversal", () => {
  const a = task("WT-0001", "ai-ready", "WT-0002");
  const b = task("WT-0002", "ai-ready", "WT-0001");
  assert.equal(remoteEpicAncestor(a, asMap(a, b)), null);
});
