import test from "node:test";
import assert from "node:assert/strict";
import { inboxRequestKey, isInboxItemSnoozed } from "./inboxSnoozes.ts";

const question = { kind: "task", attention: ["question"],
  question: { prompt: "Approve this motion?", answer: null }, review_status: "open", review_round: 1 };

test("new questions and review rounds are not hidden by an earlier snooze", () => {
  const until = Date.now() + 86400000;
  const key = inboxRequestKey(question);
  assert.equal(isInboxItemSnoozed({ "board|task": until }, "board", "task", key), false);
  const snoozes = { "board|task": { until, requestKey: key } };
  assert.equal(isInboxItemSnoozed(snoozes, "board", "task", key), true);
  assert.equal(isInboxItemSnoozed(snoozes, "board", "task", inboxRequestKey({ ...question, review_round: 2 })), false);
  assert.equal(isInboxItemSnoozed(snoozes, "board", "task", inboxRequestKey({ ...question,
    question: { ...question.question, prompt: "Different question?" } })), false);
  assert.equal(isInboxItemSnoozed({ "board|task": until }, "board", "task", "user"), true);
});
