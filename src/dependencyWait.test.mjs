import test from "node:test";
import assert from "node:assert/strict";
import { createDependencyWaitInspector } from "./dependencyWait.ts";

const columns = [
  { id: "draft", name: "User Drafts", terminal: false, ai_ready: false },
  { id: "ai-ready", name: "AI Ready", terminal: false, ai_ready: true, auto_queue: true },
  { id: "manual", name: "Manual", terminal: false, ai_ready: true },
  { id: "blocked", name: "Waiting", terminal: false, ai_ready: false },
  { id: "done", name: "Done", terminal: true, ai_ready: false },
];
const task = (id, depends_on = [], column = "ai-ready", extra = {}) => ({
  id, column, depends_on, kind: "task", attention: [], question: null,
  archived: false, completed: null, claimed_by: null, ...extra,
});
const inspect = (tasks, executionModes = {}, remoteTasks = new Map(), remoteBoards = new Map()) =>
  createDependencyWaitInspector({ tasks, columns, executionModes, remoteTasks, remoteBoards,
    pendingTaskQueue: {}, pendingColumnQueue: {}, claimStates: {} });

test("red WAIT explains an unqueued dependency through a multi-hop chain", () => {
  const a = task("A", ["B"]);
  const b = task("B", ["C"]);
  const c = task("C", [], "manual");
  const result = inspect([a,b,c])(a);
  assert.equal(result.stalled, true);
  assert.match(result.reasons.join("\n"), /C: not queued/);
});

test("queued or claimed work stays amber, while User Drafts and unanswered questions turn red", () => {
  const a = task("A", ["B"]);
  const b = task("B", ["C"], "manual", { claimed_by: "t3:worker" });
  const c = task("C");
  assert.equal(inspect([a,b,c])(a).stalled, false);
  c.column = "draft";
  assert.match(inspect([a,b,c])(a).reasons.join("\n"), /C: in User Drafts/);
  c.column = "ai-ready";
  c.attention = ["question"];
  c.question = { prompt: "Choose", answer: null };
  assert.match(inspect([a,b,c])(a).reasons.join("\n"), /waiting for your answer/);
});

test("remote and missing links are diagnosed, completed prerequisites do not block", () => {
  const a = task("A", ["elsewhere#B"]);
  const remote = new Map([["elsewhere#B", task("B", [], "manual")]]);
  const boards = new Map([["elsewhere", { columns, executionModes: { B: {mode: "queue"} } }]]);
  assert.equal(inspect([a], {}, remote, boards)(a).stalled, false);
  boards.get("elsewhere").executionModes.B.mode = "backlog";
  assert.match(inspect([a], {}, remote, boards)(a).reasons.join("\n"), /not queued/);
  assert.match(inspect([a])(a).reasons.join("\n"), /prerequisite missing/);
  remote.get("elsewhere#B").column = "done";
  assert.equal(inspect([a], {}, remote, boards)(a).stalled, false);
});
