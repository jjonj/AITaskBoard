---
name: taskboard
description: Use the Wartribes Markdown TaskBoard to discover, inspect, create, update, move, categorize, parent, complete, archive, claim/release work, ask the user questions, handle human notes to AI, mark user attention/blockers, or manage board columns. Use for durable Wartribes backlog/work-management tasks. Do not replace Wartribes source/build/editor coordination with TaskBoard.
---

# Wartribes TaskBoard

Canonical board: `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks`

CLI: `D:\SSDProjects\Tools\TaskBoard\taskboard.cmd`

Use the CLI instead of directly editing TaskBoard frontmatter when a semantic operation exists. For automation, pass `--json`.

## AI-ready column semantics

Every column has a persistent `ai_ready` boolean. Autonomous agents may select work from **any** column where `ai_ready: true`.

- The canonical **AI Ready** column (`id: ai-ready`) starts with `ai_ready: true` and is the default destination for newly created tasks.
- Other non-terminal columns can be marked AI-ready by the user, so do not infer readiness from the column name.
- **Draft** and terminal columns cannot be AI-ready.
- Discover autonomous work with `task list --ai-ready`, which follows the current board markers.

## Attention flags and notes

- **`question`** — a concrete question the user can answer. Always provide a structured question prompt; add `--option` values when useful. Question cards float to the top.
- **`needs-user`** — the user must do/review/provide/approve something that is not best represented as one structured question. Explain exactly what is needed in AI Notes. Floats to the top.
- **`user-note`** — the user wrote or changed `## User Notes` for the AI to act on. Treat this as an explicit human instruction/comment. Read it before doing more work. Address it, then clear only the `user-note` attention flag; normally leave the User Notes text intact as durable context. Floats to the top.
- **`blocked`** — the task cannot currently proceed. Explain blocker + unblock condition in AI Notes. Blocked cards sort to the bottom. If no useful work can continue, normally move the task to the board's blocked column too.

**AI Notes** are agent-authored current-state/handoff notes. Keep them concise and current.

**User Notes** are human-authored instructions/feedback such as “make this clearer,” “make a proposed design,” or “here are my notes for your proposal.” Do not rewrite or erase them just because you handled them. Handle the requested work, update the task/AI Notes as appropriate, then remove `user-note` from attention. If the user later edits the note in the desktop app, TaskBoard raises `user-note` again automatically.

A structured unanswered question is stored in frontmatter. When the user answers in the desktop app, TaskBoard persists `question.answer` and automatically removes `question`.

## Categories vs tasks

Open-ended collections such as `Feature Lab` and `Rancorous UI` are categories, not permanent parent tasks. A parent task should represent finishable larger work or a milestone.

## Normal workflow

1. Discover autonomous work with `task list --ai-ready` or targeted search. This may return tasks from several columns.
2. Inspect the task with `task show`, including User Notes, AI Notes, attention flags, and any question answer.
3. If `user-note` is present, treat the User Notes as the user's current instruction before continuing.
4. Claim only work actually being started; move it to `in-progress`.
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
