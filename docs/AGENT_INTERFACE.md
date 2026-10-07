# TaskBoard shared interface

Canonical core: `D:\SSDProjects\Tools\TaskBoard\packages\task-core`

Wartribes board: `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks`

Public model: `BoardConfig`, `Column`, `Task`, `TaskDraft`, `TaskAttention`, `TaskQuestion`, `BoardSnapshot`, `TaskStore`.

## Board/column structure

- `Column.ai_ready`: authoritative autonomous-work marker. Agents should discover tasks from all marked columns, not infer readiness from names.
- `BoardConfig.revision`: SHA-256 revision of the loaded `_board.md`; mutations reject stale revisions.
- `Task.revision`: SHA-256 revision of the loaded task Markdown; mutations reject stale revisions.

## Task structure

- `Task.parent`: finishable task/milestone hierarchy.
- `Task.category`: open-ended grouping.
- `Task.claimed_by`: optional exclusive agent claim.
- `Task.ai_notes`: agent-authored current-state/handoff context in `## AI Notes`.
- `Task.user_notes`: human-authored instructions/feedback in `## User Notes`.
- `Task.attention`: `question`, `needs-user`, `user-note`, `blocked`.
- `Task.question`: structured prompt, answer options, persisted user answer.

`user-note` means User Notes contain a new/changed human instruction needing agent attention. The desktop app raises it when the human edits User Notes. Agents clear the flag after acting, without erasing the note text by default.

Unanswered structured questions require `question`; answering preserves the answer and clears the flag.

Core writes preserve unknown frontmatter and extra Markdown sections. Mutations use a process-wide filesystem lock plus SHA-256 revisions, so stale app/agent/manual writes fail instead of overwriting newer content. Markdown remains canonical.
