import test from "node:test";
import assert from "node:assert/strict";
import { interveningColumns, crossColumnCurve } from "./dependencyRoutes.ts";
const col = (id, x) => ({ id, rect: { x, y: 120, width: 286, height: 750 } });

test("unrelated middle columns hide the cross-column wire", () => {
  const columns = [col("combat", 40), col("progress", 340), col("blocked", 640), col("done", 940)];
  const middle = interveningColumns(columns[0], columns[3], columns);
  assert.deepEqual(middle.map((r) => r.x), [337, 637]);
  assert.ok(middle.every((r) => r.height === 756 && r.width === 292));
});

test("the endpoint columns never occlude their own arrows", () => {
  const columns = [col("source", 40), col("target", 340)];
  assert.deepEqual(interveningColumns(columns[0], columns[1], columns), []);
});

test("occluder selection works in reverse column direction too", () => {
  const columns = [col("left", 40), col("middle", 340), col("right", 640)];
  assert.deepEqual(interveningColumns(columns[2], columns[0], columns).map((r) => r.x), [337]);
});

test("cross-column routing supports both directions with endpoints fixed", () => {
  assert.match(crossColumnCurve(100, 220, 800, 490, 1), /^M 100.0 220.0 C .* 800.0 490.0$/);
  assert.match(crossColumnCurve(800, 490, 100, 220, -1), /^M 800.0 490.0 C .* 100.0 220.0$/);
});
