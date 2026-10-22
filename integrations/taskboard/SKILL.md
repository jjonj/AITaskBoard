---
name: taskboard
description: Use the Wartribes Markdown TaskBoard to discover, inspect, create, update, move, categorize, parent, complete, archive, claim/release work, ask the user questions, handle human notes to AI, mark user attention/blockers, or manage board columns. Use for durable Wartribes backlog/work-management tasks. Do not replace Wartribes source/build/editor coordination with TaskBoard.
---

# Wartribes TaskBoard

Canonical board: `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks`

CLI: `D:\SSDProjects\Tools\TaskBoard\TaskBoard.exe` (same executable launches GUI when called without arguments; supports `--help`, `--json task ...` directly). Alternative: `D:\SSDProjects\Tools\TaskBoard\taskboard.cmd`.

For mutations, inspect each command's exit code and JSON before continuing, and verify the final state with `task show`. Do not hide errors with `| head` or run dependent writes after failure.

Use the CLI instead of directly editing TaskBoard frontmatter when a semantic operation exists. For automation, pass `--json`.

## AI-ready column semantics

Every column has a persistent `ai_ready` boolean. Autonomous agents may select work from **any** column where `ai_ready: true`.

- The canonical **AI Ready** column (`id: ai-ready`) starts with `ai_ready: true` and is the default destination for newly created tasks.
- Other non-terminal columns can be marked AI-ready by the user, so do not infer readiness from the column name.
- **Draft** and terminal columns cannot be AI-ready.
- Discover autonomous work with `task list --ai-ready`, which follows the current board markers.

## Epics, hard dependencies and agent-safe work selection

**Strict epic-only hierarchy:** Only `kind: epic` may be used as a `parent`. Epics themselves must have no parent (no nested epics). Regular tasks and human-only For user cards are leaves, not containers. The CLI/shared core rejects creating or reparenting a task under another regular task, and rejects parented epics. For epics, never add acceptance criteria, AI Notes, icons, attention flags or questions; put scope, design and retained historical handoffs into `description` or `notes`. The UI presents a prominent Task/Epic switch and an epic-only parent selector. Dragging an ordinary task onto another ordinary task **moves it below the target in its column**, rather than adopting it as a subtask; dropping onto a real epic creates the parent relationship. Do not mistake visual purple cross-column epic proxies for independent tasks.

**Epics and For user cards are not agent-executable tasks.** Task types are `kind: task` (AI-executable), `kind: epic` (coordination), and `kind: for_user` (**human-only**). Newly created epics receive `WTE-####` IDs; historical epic IDs (`WT-####`) are retained for compatibility and must be identified by their `kind: epic` value. Never infer executability from the `WT-` prefix. Do not claim, assign, move, or implement an epic. Epic descriptions are for scope, design, requirements and coordination notes. Select one of its executable subtasks instead. The shared Rust core rejects epic claims/moves; epic status and Done state follow descendant executable tasks automatically.

**For user** (`kind: for_user`) is a dedicated **orange** task type, not merely the `needs-user` attention flag. Use it for a discrete human-only deliverable: user playtest/feel choice, design sign-off, supplied or approved meshes/animations, selection of canonical art assets, or a decision blocking a final integration. Prefer splitting that human action out from agent-owned implementation rather than converting a task currently claimed by another agent. Agents may create user cards and add technical notes but **must not claim, perform, complete, or archive them**, and must not reclassify them as executable through the CLI to bypass the restriction. `task show` marks readiness as `for_user`; `task list --ai-ready` omits them; the CLI rejects claim, complete, move-to-terminal and archive. The user moves the orange card to Done in the desktop application, satisfying any dependent tasks. To create: `task create --kind for_user --title "Approve final grip animations" --parent WT-0071 --related-to WT-0162 --description "..."`. Without `--column`, these default to In Progress. Only add a **hard** `depends_on` to downstream implementation when that entire task genuinely cannot start until the human finishes; use `related_to` otherwise.

**Readiness is separate from column.** Regular tasks may declare:
- `depends_on: [WT-0012, WT-0034]`: hard prerequisites. Claims and attempts to start blocked work are rejected until those tasks are Done or archived. Cycles, missing targets, self-dependencies, duplicates, and dependencies on epics are invalid.
- `related_to: [WT-0012]`: informational links only, no claim or move restriction.
- `task show WT-####` returns `readiness`, `waiting_for` and `blocks` (plus `epic_progress` for epics). Check these before executing.
- `task list --ai-ready` returns only executable tasks with no unfinished hard prerequisites or explicit blockers. A task being in an AI-ready column does not alone guarantee that it is ready.
- Avoid speculative hard dependencies: if two tasks can proceed independently and later integrate, allow both to proceed and put hard dependencies on the integration task.

Use `task create --kind epic` for new epic containers; `task create --kind task --depends-on WT-0012 --related-to WT-0013` for executable work. Set/replace links with `task update WT-#### --depends-on WT-0012` or `--related-to WT-0013`; clear with `--clear-dependencies` / `--clear-related`. Multiple dependencies can be passed by repeating the flag.

## Attention flags and notes

- **`question`** — a concrete question the user can answer. Always provide a structured question prompt; add `--option` values when useful. Question cards float to the top.
- **`needs-user`** — the user must do/review/provide/approve something that is not best represented as one structured question. Explain exactly what is needed in AI Notes. Floats to the top.
- **`user-note`** — the user wrote or changed `## User Notes` for the AI to act on. Treat this as an explicit human instruction/comment. Read it before doing more work. Address it, then clear only the `user-note` attention flag; normally leave the User Notes text intact as durable context. Floats to the top.
- **`blocked`** — the task cannot currently proceed. Explain blocker + unblock condition in AI Notes. Blocked cards sort to the bottom. If no useful work can continue, normally move the task to the board's blocked column too.

**AI Notes** are agent-authored current-state/handoff notes. Keep them concise and current.

**User Notes** are human-authored instructions/feedback such as “make this clearer,” “make a proposed design,” or “here are my notes for your proposal.” Do not rewrite or erase them just because you handled them. Handle the requested work, update the task/AI Notes as appropriate, then remove `user-note` from attention. If the user later edits the note in the desktop app, TaskBoard raises `user-note` again automatically.

A structured unanswered question is stored in frontmatter. When the user answers in the desktop app, TaskBoard persists `question.answer` and automatically removes `question`.

## Formatting titles

To emphasize specific words in a task title, use Markdown `**bold words**` in the title field, e.g. `Investigate **critical regression** in combat`. The board safely renders this as bold in the card overview/hover, and stores the title unchanged in Markdown frontmatter. Prefer emphasizing just the critical phrase instead of making the whole title bold.

## Categories vs tasks

Open-ended collections such as `Feature Lab` and `Rancorous UI` are categories, not permanent parent tasks. Only a dedicated epic (`kind: epic`) may organize subtasks. Convert legitimate multi-work parents to epics; otherwise detach/reparent legacy child links to the nearest real epic, retaining prior associations via `related_to`.

## Task-type work advisories (non-blocking)

The optional `icon` frontmatter classifies tasks. `task show` and `task claim` return structured `warnings` when relevant. **Read the warnings before beginning work** and self-check model eligibility; the CLI intentionally does not block or verify identity.

- **Visual Work** (`visual-work`): only **Claude** agents should perform the work. Non-Claude agents should leave it for Claude.
- **Design** (`design`): only **Opus or Astra** should produce an **initial design draft**. Other agents may refine a draft that already exists, but should not create the initial one.
- Other icons have no special work advisories. A warning is *not* the same as a `blocked` attention flag.

If your agent/model is not eligible, do not claim/move the task; if already claimed, release it. Never treat the warning as a CLI error or an authorization mechanism.

## Normal workflow

1. Discover autonomous work with `task list --ai-ready` or targeted search. This may return tasks from several columns.
2. Inspect the task with `task show`, including User Notes, AI Notes, attention flags, any question answer, **and work-type warnings**. Follow the model eligibility advisory before starting.
3. If `user-note` is present, treat the User Notes as the user's current instruction before continuing.
4. Reject `kind: epic` and `kind: for_user` as agent work targets. Inspect epics for executable subtasks; leave For user tasks to the human. For a regular task, confirm `readiness: ready` and no `waiting_for` blockers. Claim only work actually being started; move it to `in-progress`.
5. Work under the repository's separate source/build/editor coordination rules.
6. Use Question/NeedsUser/Blocked when appropriate and keep AI Notes truthful.
7. After handling a User Note, clear `user-note` but preserve the note text unless the user asks to remove it.
8. Complete only after acceptance criteria and required verification are satisfied. Completed tasks remain visible in a terminal column for 7 days and then auto-archive; do not archive them immediately just to clear Done.

## Examples

```console
taskboard.cmd --json task list --ai-ready
taskboard.cmd --json task show WT-0042

taskboard.cmd --json task create --title "Choose combat prompt style" --ai-notes "Implementation is ready except for the preferred interaction." --question "Which interaction should we use?" --option "Radial menu" --option "Context menu"

taskboard.cmd --json task update WT-0042 --ai-notes "Please inspect the animation and approve/reject it." --add-attention needs-user
taskboard.cmd --json task update WT-0042 --ai-notes "Blocked by missing source asset." --add-attention blocked

# After acting on a human note:
taskboard.cmd --json task update WT-0042 --remove-attention user-note
```

## Safety

Markdown remains canonical. The CLI and desktop app share `task-core`; semantic writes carry SHA-256 content revisions and use a cross-process mutation lock. Unknown frontmatter and extra Markdown sections are preserved. Treat `write_conflict` as a request to reload and reconcile—never overwrite a concurrent user/agent edit.

TaskBoard claims are durable work assignment only; they do not grant Wartribes editor/build/source ownership.
