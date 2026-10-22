# TaskBoard epic/dependency migration (2026-10-15)

The live Wartribes board at `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks` was upgraded from untyped parent cards to explicit executable tasks and non-executable **epics**.

## Applied migration

- **184** existing Markdown task files (Active and Archive) now carry `kind: task` or `kind: epic`, plus `depends_on` and `related_to` metadata.
- **9** historical coordination containers classified as epics: `WT-0020`, `WT-0052`, `WT-0075`, `WT-0086`, `WT-0096`, `WT-0102`, `WT-0132`, `WT-0145`, `WT-0163`.
- Other tasks, even if they have a few subtasks, remain executable tasks. Parentage alone never makes an epic.
- Existing epic IDs deliberately remain `WT-####` to avoid breaking cross-references. New epics use the visually distinct `WTE-####` namespace.
- Epic `WT-0102` had a legacy claim; it was cleared during migration because epics are no longer assignable.
- No content or titles were rewritten. Normal status changes on later loads are intentional: epic columns are computed from executable descendants.
- One task (`WT-0148`) changed during the initial sweep and was skipped to avoid overwriting concurrent agent work, then migrated separately against its fresh content.

## Explicit links

Safe, evidence-supported examples were seeded:
- Hard prerequisites: `WT-0182 -> WT-0079, WT-0080` (bow math and ballistics), `WT-0183 -> WT-0082` (throw ownership rules), and `WT-0184 -> WT-0083` (poison dose validation). These prerequisite tasks were already completed or archived, so they do not interrupt ongoing work.
- Informational references: `WT-0150 <-> WT-0147`, `WT-0186 <-> WT-0070/WT-0177`, and `WT-0087 <-> WT-0179` (stored as forward `related_to` links; the reverse view can be computed).

**Other hard dependencies were intentionally left empty**, rather than guessing based on mentions of adjacent tasks or partial prerequisites. Only whole-task gating dependencies should be hard.

## Rollback and verification

Backups and the manifest are in `artifacts/migrations/`:
- `20261015T011133_before_epic_migration.zip`
- `20261015T011322_WT-0148_before_retry.zip`
- `20261015T011133_report.json`
- `20261015_migration_script.py`

Shared core enforces epic claim/move prohibition, unresolved hard-dependency claim/start prevention, missing/self/duplicate links and dependency-cycle checks; epics automatically acquire derived columns and completion from descendants. The UI shows purple epic cards with progress and readiness cues for executable tasks. The CLI `task show` returns readiness, waiting_for, blocks and epic_progress; `task list --ai-ready` filters out epics and waiting/blocked work.

Verified via 28 shared Rust tests, 9 CLI tests, frontend TypeScript/Vite build, release Tauri build and a live UI screenshot. Production `TaskBoard.exe` was replaced and restarted.


## Additional migration and visualization pass — 2026-10-15

Follow-up audit found legacy executable parents in the Combat Implementation column, including WT-0072 -> WT-0176, which should have been an epic. Six additional existing parent cards were converted to explicit `kind: epic`: WT-0009, WT-0059, WT-0072, WT-0074, WT-0162, WT-0187. They keep historical WT IDs. Four incorrect or obsolete parent edges were corrected without deleting any tasks: WT-0016 no longer under WT-0006; WT-0015 no longer under WT-0016; unrelated WT-0041 removed from WT-0009; historical design WT-0179 detached from implementation task WT-0087 (now `related_to`).

To preserve implementable work that formerly lived directly in these new epic cards, created executable children: WT-0193 (remaining local spear/club prototype and feel tour; depends on WT-0176), WT-0194 (remaining Stage 2 network cutover; related to WT-0175), WT-0195 (spear visual/bake parity; depends on WT-0155/WT-0159), WT-0196 (Real RAI simulator production harness; related to WT-0190). Hard dependencies were not placed on work that can be parallelized.

Two actively claimed executable parents WT-0043 (gecko) and WT-0051 (kiwi) were explicitly left as regular executable tasks. Migrating those without coordinating ongoing agents would invalidate their active ownership or make their current work unclaimable. They are not Stage 1 / Stage 2 Combat Implementation epics and can be revisited when claims are released.

Migration backup and report: `artifacts/migrations/20261015T040102_hierarchy_before.zip` and `artifacts/migrations/20261015T040102_hierarchy_report.json`.

UI: New `src/dependencyLayout.ts` performs stable, per-column topological sorting, including visible parent-before-child ordering. `src/DependencyArrows.tsx` uses viewport-coordinate SVG paths with red arrows, a distinct routing track, clipping to each column's visible card-list region and geometry recomputation on nested scrolling and resizing. Lines are only drawn when both cards are visible in the same expanded column; cross-column prerequisites remain navigable through the hover preview. Graph-sort tests: `node --experimental-strip-types --test src/dependencyLayout.test.mjs`.


## Cross-column dependency overlay refinement

The dependency SVG renders independently from cards and never modifies card-list padding, card dimensions, or column positioning. Same-column links use a compact edge overlay. Cross-column links use smooth directional red curves and a per-edge SVG mask that covers **unrelated intermediary column rectangles**, making those portions of the wire appear *behind* those columns while the source/destination endpoints remain exposed. Routing is bidirectional, clipped to the visible board, and remeasured after card-list scrolling, board panning, column resizing or task updates.

Implementation: `src/DependencyArrows.tsx`, `src/dependencyRoutes.ts`. Geometry tests: `node --experimental-strip-types --test src/dependencyRoutes.test.mjs`.
