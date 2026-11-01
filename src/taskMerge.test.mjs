import test from "node:test";
import assert from "node:assert/strict";
import { mergeTaskEdits } from "./taskMerge.ts";

const base = {
  id: "MISC-0097", revision: "base", title: "Base", column: "ai-ready",
  description: "Original", ai_notes: "Initial", acceptance_criteria: ["A"],
  user_notes: "", attention: [], notes: "",
};

test("preserves independent edits made after editor opened", () => {
  const edited = { ...base, title: "User title" };
  const disk = { ...base, revision: "new", ai_notes: "Agent decision" };
  const { merged, conflicts } = mergeTaskEdits(base, edited, disk);
  assert.deepEqual(conflicts, []);
  assert.equal(merged.title, "User title");
  assert.equal(merged.ai_notes, "Agent decision");
  assert.equal(merged.revision, "new");
});

test("flags same-field conflicts rather than discarding either version", () => {
  const edited = { ...base, description: "User new requirements" };
  const disk = { ...base, revision: "new", description: "Agent updated requirements" };
  const { merged, conflicts } = mergeTaskEdits(base, edited, disk);
  assert.deepEqual(conflicts, ["description"]);
  assert.equal(merged.description, "Agent updated requirements");
});

test("permits identical concurrent edits without false conflict", () => {
  const edited = { ...base, acceptance_criteria: ["A", "B"] };
  const disk = { ...base, revision: "new", acceptance_criteria: ["A", "B"] };
  const { merged, conflicts } = mergeTaskEdits(base, edited, disk);
  assert.deepEqual(conflicts, []);
  assert.deepEqual(merged.acceptance_criteria, ["A", "B"]);
});
