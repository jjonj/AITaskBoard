import test from "node:test";
import assert from "node:assert/strict";
import { isUserQueueEligible } from "./userQueueEligibility.ts";

const task = (id, overrides = {}) => ({
  id, kind: "task", column: "ai-ready", attention: [],
  depends_on: [], completed: null, archived: false, ...overrides,
});
const board = (...tasks) => ({
  tasks, config: { columns: [
    { id: "ai-ready", terminal: false },
    { id: "blocked", terminal: false },
    { id: "done", terminal: true },
  ] },
});
const user = task("WT-0310", { kind: "for_user", column: "review-and-refactors", depends_on: ["WT-0307"] });

test("club review WT-0310 is not in human inbox until WT-0307 is done", () => {
  const parent = task("WT-0306", { kind: "for_user", column: "blocked" });
  const worker = task("WT-0307", { depends_on: ["WT-0306"] });
  const source = board(parent, worker, user);
  assert.equal(isUserQueueEligible(user, source, {}), false);
  assert.equal(isUserQueueEligible(parent, source, {}), true);
  assert.equal(isUserQueueEligible(worker, source, {}), false);
  assert.equal(isUserQueueEligible(user, board(parent, { ...worker, completed: "2026-10-25" }, user), {}), true);
});

test("an unanswered AI question appears in Inbox even when implementation is Done", () => {
  const pending = task("T3-0072", {
    column: "done", completed: "2026-10-27", attention: ["question"],
    question: { prompt: "Verify Open Session works", options: ["Done", "Needs changes"], answer: null },
  });
  assert.equal(isUserQueueEligible(pending, board(pending), {}), true);
  assert.equal(isUserQueueEligible({ ...pending, question: { ...pending.question, answer: "Done" } }, board(pending), {}), false);
  assert.equal(isUserQueueEligible({ ...pending, attention: [] }, board(pending), {}), false);
});

test("ordinary AI questions request feedback without creating For user cards", () => {
  const pending = task("T3-0100", { attention: ["question"],
    question: { prompt: "Choose A or B", options: ["A", "B"], answer: null } });
  assert.equal(isUserQueueEligible(pending, board(pending), {}), true);
});

test("missing prerequisites fail closed; terminal and archived prerequisites count complete", () => {
  assert.equal(isUserQueueEligible(user, board(user), {}), false);
  for (const finished of [ { completed: "2026-10-25" }, { column: "done" }, { archived: true } ]) {
    assert.equal(isUserQueueEligible(user, board(task("WT-0307", finished), user), {}), true);
  }
});

test("multi-prerequisite and normal queue filtering continue to work", () => {
  const multi = task("WT-0311", {
    kind: "task", attention: ["needs-user"], depends_on: ["WT-0307", "WT-0306"]
  });
  const prereq = task("WT-0307", { completed: "2026-10-25" });
  const other = task("WT-0306");
  assert.equal(isUserQueueEligible(multi, board(prereq, other, multi), {}), false);
  assert.equal(isUserQueueEligible(multi, board(prereq, { ...other, completed: "2026-10-25" }, multi), {}), true);
  assert.equal(isUserQueueEligible(multi, board(prereq, { ...other, completed: "2026-10-25" }, multi), { "WT-0311": 1000 }, 500), false);
  assert.equal(isUserQueueEligible(multi, board(prereq, { ...other, completed: "2026-10-25" }, multi), { "WT-0311": 1000 }, 1000), true);
  assert.equal(isUserQueueEligible({ ...multi, column: "done" }, board(prereq, other, multi), {}), false);
  assert.equal(isUserQueueEligible({ ...multi, completed: "2026-10-25" }, board(prereq, other, multi), {}), false);
});
