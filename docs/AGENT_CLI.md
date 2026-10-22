# Agent CLI

The agent-facing TaskBoard CLI lives in `packages/task-cli` and consumes the same `task-core` crate used by the desktop app. The installed desktop executable now runs that exact CLI when given arguments; with no arguments it opens the GUI. `TaskBoard.exe --help` prints command usage, and `TaskBoard.exe --json task show WT-0158` works directly (no `taskboard.cmd` wrapper required). Inspect every write's JSON and exit code; verify with `task show` afterward. Do not hide errors through `| head` or execute follow-up mutations after a failed claim.

Windows entry point:

```console
D:\SSDProjects\Tools\TaskBoard\taskboard.cmd --json task list
```

Default board: `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks`

## AI-ready columns

Each column has a persisted `ai_ready` boolean. Autonomous agents should use:

```console
taskboard.cmd --json task list --ai-ready
```

That returns tasks from every currently marked AI-ready column. The canonical `ai-ready` column starts marked and remains the default for newly created tasks, but other non-terminal columns may also be marked by the user. Draft and terminal columns cannot be marked AI-ready.

Column readiness can be changed explicitly:

```console
taskboard.cmd --json column ready rancui true
taskboard.cmd --json column ready rancui false
```

## Epic-only hierarchy and UI placement

Tasks may be children **only of epics**: `task parent WT-0042 --parent WTE-0001` (or a historical `WT-####` that has `kind: epic`). Regular tasks and For user cards cannot be parents. Epics themselves have no parent and cannot be nested. Epic content is limited to description/design, notes, user notes, category and related-task references; they have **no** acceptance criteria, AI Notes, icon, attention/question flags or hard prerequisites. The shared core rejects invalid input. The desktop editor has a Task/Epic toggle at top, an optional epic-only parent selector for non-epics, and a dedicated Notes field for epics. Dragging a card onto a regular card places it below the target (also changes column if needed), persisting `position_after`/`positioned_at`; dragging onto an epic establishes parentage.

## Dedicated For user cards

`kind: for_user` is a distinct orange human-only card, not simply an attention flag. Create these for decisions, assets, playtesting or animation approvals only the user can carry out. Example: `task create --kind for_user --parent WT-0071 --title "Approve ALS spear parry references" --related-to WT-0162`. By default, they go into In Progress. Agents may supply notes and links, but cannot claim, complete, move to Done, archive, or reclassify an existing For user card via the CLI. The user can complete it normally in the desktop application, unblocking any `depends_on` relationships. CLI `task show` reports `readiness: for_user`, and `task list --ai-ready` excludes it even if its column is marked AI-ready. For preexisting agent-claimed tasks, create a separate For user action instead of attempting to change the in-progress agent task.

## Attention metadata

Canonical new attention values: `question`, `user-note`, `blocked`. The old `needs-user` value is a backward-compatible alias for the **For user** task kind, not a separate UI status. New `task create --attention needs-user` and `task update --add-attention needs-user` requests become `kind: for_user` (the latter requires an unclaimed non-epic task). Legacy tasks still carrying `needs-user` are considered human-only for CLI readiness, claim, Done, and archive operations; only the user can clear this legacy status through TaskBoard.

- `question`: unresolved structured question for the user.
- `user-note`: human-authored instructions requiring AI attention. The desktop editor submits Note to AI as a timestamped update inside Description and clears the composer while setting `user-note`; agents should clear this flag after handling the instruction, without removing the recorded history.
- `blocked`: cannot proceed.

Questions can include repeated `--option` values. The desktop app persists the answer and removes `question` automatically. Historical `AI Notes` still load for compatibility; the new desktop editor displays them in the description timeline and imports them to Description when the user saves. CLI `--ai-notes` remains supported for older integrations.

## Work-type warnings on task start

For tasks tagged with `icon: visual-work` or `icon: design`, `task show` (preflight) and `task claim` (start) include a top-level `warnings` array in the JSON result.

- Visual Work: Claude agents only.
- Design: Opus/Astra agents only for the **initial draft**; later revisions may be done by other agents.

Example warning shape: `{"warnings":[{"code":"visual_work_claude_only","message":"...","advisory_only":true}]}`. These are agent-facing instructions, not enforced permissions: the command still succeeds (exit 0), the claim still happens, and existing result fields remain at their previous paths. Agents must self-check and abstain/release when ineligible.

## Main commands

```text
task list [--ai-ready]
task show <id-or-exact-title>
task search <query>
task create --title <title> [--column <column>] [--category <category>] [--parent <task>]
            [--ai-notes <text>] [--user-notes <text>]
            [--attention question|needs-user|user-note|blocked]
            [--question <prompt>] [--option <answer>]...
task update <task> [--title ...] [--description ...] [--category ... | --clear-category]
            [--ai-notes <text> | --clear-ai-notes]
            [--user-notes <text> | --clear-user-notes]
            [--add-attention question|needs-user|user-note|blocked]
            [--remove-attention question|needs-user|user-note|blocked]
            [--question <prompt> --option <answer>...] [--clear-question]
task move <task> <column>
task parent <task> --parent <task>
task complete <task>
task archive <task>
task claim <task> --agent <stable-agent-name>
task release <task> --agent <stable-agent-name>

column list
column create
column rename
column reorder
column ready <column> true|false
column remove
```

Search includes User Notes, AI Notes, question prompts/options/answers, task text, IDs, titles, and categories.

Agents should not invent or rewrite User Notes as if they came from the user. They are a human-authored channel. Handle them and clear `user-note` when done.

The CLI preserves valid manual extensions through the shared core. Semantic writes carry SHA-256 content revisions and use the same cross-process mutation lock as the desktop app. If a task or board changed after it was loaded, the write is rejected instead of overwriting the concurrent edit; reload/reconcile and retry. TaskBoard coordination remains separate from Wartribes source/build/editor coordination.
