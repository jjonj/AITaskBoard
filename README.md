# TaskBoard

Markdown-first desktop task board for Wartribes.

Default board: `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks`

Tasks can be edited directly in Obsidian. Standard body sections are `Description`, `Acceptance Criteria`, `AI Notes`, and `User Notes`.

## Human app

Double-click `Open TaskBoard.cmd`.

Key interactions:
- drag cards between columns;
- drag a card onto another card to make it a subtask of that task;
- drag column headers to reorder columns without native browser drag/drop;
- click the ✓ in a non-terminal column header to mark/unmark that entire column as AI-ready;
- collapse columns and parent tasks;
- right-drag empty board space to pan horizontally;
- right-click tasks for Edit, New subtask, Remove parent, Highlight in Fomni, Open in Obsidian, and Archive;
- tasks entering a terminal column such as Done receive a completion date and are automatically archived after 7 days; reopening them clears that date;
- hover cards to see AI Notes and User Notes;
- click cards for full details and question answering;
- `Ctrl+T` creates a task in the default AI Ready column; `Ctrl+S` saves/closes an open task; `Esc` closes it; `Ctrl+Z` undoes the last persisted board action when the editor is closed.

Attention icons:
- **? Question** — answerable user question; floats to top.
- **◆ NeedsUser** — user action/review/input needed; floats to top.
- **✎ UserNote** — you wrote/changed a Note to AI; floats to top until an agent handles it.
- **× Blocked** — cannot proceed; floats to bottom.

The **Note to AI** field is a human instruction channel. Editing it automatically sets UserNote. Agents are instructed to act on it, then clear the flag while preserving your note as context.

`AI Ready` is the default new-task column. Any non-terminal column can also be marked AI-ready with its ✓ control; Draft and terminal columns cannot. Task/board writes use SHA-256 content revisions plus a cross-process lock, so a concurrent Obsidian/agent edit is rejected as a conflict rather than silently overwritten.

## Developer commands

```powershell
npm run build
cargo test --manifest-path packages/task-core/Cargo.toml
cargo test --manifest-path packages/task-cli/Cargo.toml
npm run tauri dev
npm run tauri build
```
