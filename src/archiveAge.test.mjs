import test from "node:test";
import assert from "node:assert/strict";
import { eligibleForAgeArchive } from "./archiveAge.ts";

const cutoff = "2026-10-12T16:30:00Z";
const task = (id, completed_at, kind = "task", parent = null, column = "done") =>
  ({ id, kind, parent, column, archived: false, completed_at });

test("archives only Done entries at least 72 hours old", () => {
  const items = [
    task("old", "2026-10-11T16:30:00Z"),
    task("boundary", cutoff),
    task("recent", "2026-10-14T16:30:00Z"),
    task("undated", null),
    task("elsewhere", "2026-10-09T16:30:00Z", "task", null, "review"),
    { ...task("already-archived", "2026-10-05T16:30:00Z"), archived: true },
  ];
  assert.deepEqual(eligibleForAgeArchive(items, "done", cutoff).map(t => t.id), ["old", "boundary"]);
});

test("never archive an epic while its younger or undated child would be orphaned", () => {
  const items = [
    task("epic", "2026-10-10T16:30:00Z", "epic"),
    task("old-child", "2026-10-11T16:30:00Z", "task", "epic"),
    task("new-child", "2026-10-14T16:30:00Z", "task", "epic"),
  ];
  assert.deepEqual(eligibleForAgeArchive(items, "done", cutoff).map(t => t.id), ["old-child"]);
});

test("can archive epic and every eligible older child together", () => {
  const items = [
    task("epic", "2026-10-10T16:30:00Z", "epic"),
    task("child", "2026-10-11T16:30:00Z", "task", "epic"),
  ];
  assert.deepEqual(eligibleForAgeArchive(items, "done", cutoff).map(t => t.id), ["epic", "child"]);
});

test("will not archive an epic with still-active children elsewhere", () => {
  const items = [
    task("epic", "2026-10-10T16:30:00Z", "epic"),
    task("child", null, "task", "epic", "ai-ready"),
  ];
  assert.deepEqual(eligibleForAgeArchive(items, "done", cutoff), []);
});

test("invalid or unknown timestamps are never assumed old", () => {
  const items = [task("garbled", "not-a-date"), task("legacy", null)];
  assert.deepEqual(eligibleForAgeArchive(items, "done", cutoff), []);
  assert.deepEqual(eligibleForAgeArchive(items, "done", "bad-cutoff"), []);
});
