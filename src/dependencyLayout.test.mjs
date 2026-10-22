import test from "node:test";
import assert from "node:assert/strict";
import { orderCardsByDependencies } from "./dependencyLayout.ts";
const card = (id, depends_on = [], parent = null) => ({ id, parent, depends_on });
test("transitive prerequisites precede dependent and unrelated stay stable", () => {
 const ids = orderCardsByDependencies([card("C",["B"]),card("Q"),card("B",["A"]),card("Z"),card("A")]).map(t=>t.id);
 assert.ok(ids.indexOf("A") < ids.indexOf("B"));
 assert.ok(ids.indexOf("B") < ids.indexOf("C"));
 assert.ok(ids.indexOf("Q") < ids.indexOf("Z"));
 assert.equal(ids.length,5);
});
test("missing prerequisites and unrelated tasks preserve order", () => {
 assert.deepEqual(orderCardsByDependencies([card("B",["OTHER"]),card("A")]).map(t=>t.id),["B","A"]);
});
test("epic before children, dependency between children",()=>{
 assert.deepEqual(orderCardsByDependencies([
 card("WT-0193",["WT-0176"],"WT-0072"),card("WT-0072"),card("WT-0176",[],"WT-0072")
 ]).map(t=>t.id),["WT-0072","WT-0176","WT-0193"]);
});
test("legacy cycle does not drop cards",()=>{
 assert.equal(orderCardsByDependencies([card("X",["Y"]),card("Y",["X"])]).length,2);
});
test("already ordered Done column remains stable",()=>{
 const cs=[card("A"),card("B"),card("C")];assert.deepEqual(orderCardsByDependencies(cs),cs);
});

import { applyCardPlacements } from "./dependencyLayout.ts";

test("dropping a card on another moves it directly beneath its target", () => {
  const cards = [
    {...card("A")}, {...card("B")}, {...card("C"), position_after: "A", positioned_at: "2026-10-15T10:00:00Z"},
  ];
  assert.deepEqual(applyCardPlacements(cards).map(t => t.id), ["A", "C", "B"]);
});

test("most recent repeated drop sits directly below anchor", () => {
  const cards = [
    {...card("A")},
    {...card("C"), position_after: "A", positioned_at: "2026-10-15T10:00:00Z"},
    {...card("B"), position_after: "A", positioned_at: "2026-10-15T11:00:00Z"},
  ];
  assert.deepEqual(applyCardPlacements(cards).map(t => t.id), ["A", "B", "C"]);
});

test("hard prerequisites still win over manual positions", () => {
  const cards = [
    {...card("Dependent", ["Required"]), position_after: "Middle", positioned_at: "2026-10-15T12:00:00Z"},
    {...card("Middle")}, {...card("Required")},
  ];
  const ids = orderCardsByDependencies(applyCardPlacements(cards)).map(t => t.id);
  assert.ok(ids.indexOf("Required") < ids.indexOf("Dependent"));
});

test("orphaned placement anchor is ignored without losing a card", () => {
  const cards = [{...card("B"), position_after: "OLD"}, card("A")];
  assert.deepEqual(applyCardPlacements(cards).map(t => t.id), ["B", "A"]);
});
