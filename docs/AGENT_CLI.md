# Agent CLI

The agent-facing TaskBoard CLI lives in `packages/task-cli` and consumes the same `task-core` crate used by the desktop app.

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

## Attention metadata

Canonical values: `question`, `needs-user`, `user-note`, `blocked`.

- `question`: unresolved structured question for the user.
- `needs-user`: user action/review/input required.
- `user-note`: human-authored User Notes require AI attention. After handling them, clear this flag but normally preserve the note text.
- `blocked`: cannot proceed.

Questions can include repeated `--option` values. The desktop app persists the answer and removes `question` automatically.

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
