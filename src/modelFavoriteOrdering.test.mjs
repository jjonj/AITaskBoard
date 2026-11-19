import test from "node:test";
import assert from "node:assert/strict";
import { orderTaskModelFavorites } from "./modelFavoriteOrdering.ts";

test("T3-style provider grouping and model family order do not depend on favorite insertion order", () => {
  const source = [
    { provider: "opencode", model: "openrouter/deepseek/deepseek-v4.1-flash" },
    { provider: "claudeAgent", model: "claude-haiku-5-5" },
    { provider: "codex", model: "gpt-6-luna" },
    { provider: "claudeAgent", model: "claude-opus-5-5" },
    { provider: "codex", model: "gpt-6.2-astra" },
    { provider: "codex", model: "gpt-6.1-terra" },
    { provider: "claudeAgent", model: "claude-sonnet-5-5" },
    { provider: "codex", model: "gpt-6.2-sol" },
    { provider: "opencode", model: "openrouter/xiaomi/mimo-v2.6-flash" },
  ];
  assert.deepEqual(orderTaskModelFavorites(source).map(row => row.model), [
    "gpt-6.2-sol", "gpt-6.2-astra", "gpt-6.1-terra", "gpt-6-luna",
    "claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5",
    "openrouter/deepseek/deepseek-v4.1-flash", "openrouter/xiaomi/mimo-v2.6-flash",
  ]);
  assert.deepEqual(source.map(row => row.provider).slice(0, 2), ["opencode", "claudeAgent"],
    "sort must not mutate T3's source favorites");
});

test("new favorites participate immediately and other provider order stays stable", () => {
  const before = [{ provider: "opencode", model: "first" }, { provider: "codex", model: "gpt-6-luna" }];
  const after = [...before, { provider: "codex", model: "gpt-6.3-sol" }];
  assert.deepEqual(orderTaskModelFavorites(after).map(row => row.model),
    ["gpt-6.3-sol", "gpt-6-luna", "first"]);
});
