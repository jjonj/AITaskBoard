import test from "node:test";
import assert from "node:assert/strict";
import {
  appendDescriptionUpdates,
  parseDescriptionTimeline,
  prepareTaskDraftForSave,
  updateInitialDescription,
} from "./descriptionTimeline.ts";

const time = "2026-10-21T18:30:00.000Z";

test("preserves editable initial description and appends chronological updates", () => {
  let description = appendDescriptionUpdates("Original task description.", [
    { kind: "update", text: "First progress note" },
    { kind: "user-note", text: "Please revise the animation" },
  ], time);
  description = updateInitialDescription(description, "New original brief");
  const result = parseDescriptionTimeline(description);
  assert.equal(result.initial, "New original brief");
  assert.deepEqual(result.updates.map(({ kind, text }) => ({ kind, text })), [
    { kind: "update", text: "First progress note" },
    { kind: "user-note", text: "Please revise the animation" },
  ]);
  assert.equal(result.updates[1].at, time);
});

test("quoted note headings stay inside the Rust ## Description section", () => {
  const note = "A heading follows:\n## Acceptance Criteria\nDo not lose this line\n\nThanks";
  const description = appendDescriptionUpdates("Task", [{ kind: "user-note", text: note }], time);
  assert.ok(description.includes("> ## Acceptance Criteria"));
  assert.equal(description.split("\n").filter((line) => line.startsWith("## ")).length, 0);
  assert.equal(parseDescriptionTimeline(description).updates[0].text, note);
});

test("malformed timeline data stays visible as original text rather than being lost", () => {
  const raw = "Important original\n\n<!-- taskboard:update:user-note|bad date -->\n> Keep me";
  assert.equal(parseDescriptionTimeline(raw).initial, raw);
  assert.equal(parseDescriptionTimeline(raw).updates.length, 0);
});

test("saving moves Note to AI and historical AI notes into description and clears their fields", () => {
  const draft = {
    kind: "task", attention: ["needs-user"], description: "Initial",
    ai_notes: "Old AI log", user_notes: "User says this",
  };
  const saved = prepareTaskDraftForSave(draft, "Progress update", null);
  assert.equal(saved.kind, "task");
  assert.equal(saved.ai_notes, "");
  assert.equal(saved.user_notes, "");
  assert.deepEqual(saved.attention, ["user-note"]);
  assert.deepEqual(parseDescriptionTimeline(saved.description).updates.map((value) => value.kind), [
    "legacy-ai", "update", "user-note",
  ]);
  assert.deepEqual(parseDescriptionTimeline(saved.description).updates.map((value) => value.text), [
    "Old AI log", "Progress update", "User says this",
  ]);
});

test("legacy NeedsUser without new instructions becomes canonical For user if unclaimed", () => {
  const draft = {
    kind: "task", attention: ["needs-user"], description: "Original",
    ai_notes: "", user_notes: "",
  };
  assert.equal(prepareTaskDraftForSave(draft, "", null).kind, "for_user");
  const claimed = prepareTaskDraftForSave(draft, "", "active-agent");
  assert.equal(claimed.kind, "task");
  assert.deepEqual(claimed.attention, ["needs-user"]);
});

test("epic description updates are persisted without affecting epic coordination notes", () => {
  const draft = {
    kind: "epic", attention: [], description: "Epic brief",
    ai_notes: "", user_notes: "Unrelated epic coordination notes",
  };
  const saved = prepareTaskDraftForSave(draft, "Milestone updated", null);
  assert.equal(saved.user_notes, draft.user_notes);
  assert.deepEqual(parseDescriptionTimeline(saved.description).updates.map((entry) => entry.text), ["Milestone updated"]);
});
