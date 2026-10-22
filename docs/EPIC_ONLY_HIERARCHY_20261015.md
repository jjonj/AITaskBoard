# Epic-only task hierarchy and card drop behavior — 2026-10-15

TaskBoard now has a strict invariant in shared Rust `task-core` (both desktop UI and agent CLI): only an epic may be used as a parent. Epics are root containers and cannot have parents, including other epics. Regular implementation and For user cards are leaves. The parent selector only displays actual epics.

## Dragging
- Drop a normal/For user task **on an epic card**: it is assigned to that epic (retains its current column).
- Drop a normal/For user task **on another non-epic task card**: it moves to that card's column **directly below it**, without changing its epic relationship. Placement persists in `position_after` and `positioned_at` frontmatter. Dependency precedence still takes priority over purely visual position.
- Drop on empty column area: move to that column; clear explicit placement.
- The UI displays distinct drag affordances: blue parent highlight for epics, green **Place below** for ordinary cards.
- Epic cards remain non-draggable; they follow their children automatically.

## Editor
- At the top, use the **Task | Epic** selector; the **For user** option remains available on non-epic cards.
- The **Epic (optional parent)** selector is only visible for non-epics and lists only real epics.
- Epics contain design/description, related links, user notes and editable coordination Notes. No AI Notes, acceptance criteria, card icons, question/attention flags or hard dependencies. Shared core rejects invalid data, and future epic Markdown writes do not produce these sections.
- If a regular task is deliberately converted into an epic using the UI, previously authored AI Notes and acceptance criteria are retained in its Notes instead of discarded. An epic with children cannot be reclassified into a regular task.

## Existing-board migration
All **203** active and archived task Markdown files were inspected; **22** were changed: 15 epics were normalized (legacy acceptance criteria and AI Notes preserved within `## Notes`; icons cleared and nested epic parentage removed), and 7 non-epic child references were reparented to their nearest actual epic (or detached when none exists), with the former association retained as `related_to`. No tasks or agent claims were deleted.

Backup:
`artifacts/migrations/20261015T150514_epic_only_hierarchy_before.zip`

Migration report:
`artifacts/migrations/20261015T150514_epic_only_hierarchy_report.json`

Migration script:
`artifacts/migrations/migrate_epic_only_hierarchy_20261015.py`

The live CLI read all **170 active** tasks with **zero board warnings** after the migration. The initial migration script's YAML block-list formatting error affected WT-0204 and WT-0205; both files were immediately repaired and their parsed metadata verified. The migration script has been corrected to prevent repetition.

Verification: frontend TypeScript/Vite production build, 34 Rust task-core tests, 10 Rust task-cli tests, 20 frontend sort/dependency/proxy tests, `git diff --check`, and Tauri native production release build.
