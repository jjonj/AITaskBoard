# TaskBoard

Markdown-first desktop task boards for multiple projects. The original Wartribes board remains the default on first launch.

Default Wartribes board: `B:\GDrive\SharedWithPhone\Obsidian\jjj\Wartribes\Tasks`

## Project tabs

TaskBoard now opens with a tab strip for individual projects. Click **+ Project** to register an existing board folder containing `_board.md`, or check **Create a new TaskBoard board** to initialize an existing empty project folder. The selected tab persists across restarts in `%APPDATA%\TaskBoard\projects.json`. Each tab isolates task state, drafts, undo history, queues, snoozes, refresh and board preferences; tabs are locked while an editor or an operation is active. Removing a tab never deletes any tasks or directories. The original Wartribes tab, files, and user preferences are preserved.

**CLI:** Other projects can already be accessed with `TaskBoard.exe --root "D:\Projects\Other\Tasks" --json task list`. For safety, the CLI default remains Wartribes rather than whichever desktop tab was last active. New projects still generate `WT-####` task IDs within their independent board root; project-specific prefixes and custom workflow roles are **planned**, not yet implemented. See [multi-project groundwork and limitations](docs/MULTI_PROJECT_FOUNDATION_20261022.md).

Tasks can be edited directly in Obsidian. The editor presents **Description** as an editable original brief followed by timestamped, read-only updates. Updates (including submitted **Note to AI** text) are stored inside the Markdown `## Description` section using quoted text and comment markers, so an update cannot accidentally become a new Markdown section. Historical `AI Notes` and `User Notes` remain readable for compatibility; saving in TaskBoard moves prior AI notes and submitted Note to AI text into the description history and clears those separate fields.

## Human app

Double-click `Open TaskBoard.cmd`, or launch `TaskBoard.exe` without arguments. The same `TaskBoard.exe` now also runs the agent CLI whenever arguments are supplied (`TaskBoard.exe --help`, `TaskBoard.exe --json task show WT-0158`); the desktop UI is launched only with no arguments.

Key interactions:
- drag cards between columns;
- drag a card onto another card to make it a subtask of that task;
- drag column headers to reorder columns without native browser drag/drop;
- click the ✓ in a non-terminal column header to mark/unmark that entire column as AI-ready;
- collapse columns and parent tasks; right-click any column (including its collapsed bar) for Rename, Archive all, and snooze for 1 day, 3 days, 1 week, or 1 calendar month; snoozed columns automatically reopen after the deadline, display a temporary sun/awake icon for 24 hours, and can be woken manually;
- right-drag empty board space to pan horizontally, or use the vertical mouse wheel over board/column whitespace to scroll horizontally; dragging a task or column header near either horizontal screen edge auto-scrolls the board until the drag leaves the edge or is released; card lists retain their normal vertical wheel scrolling;
- right-click tasks for Edit, Move to done, Move to... (column submenu), New subtask, Remove parent, clipboard copy (ID, ID + title, full Markdown file path, ready-to-paste agent execute message), Highlight in Fomni, Open in Obsidian, and Archive; bulk Archive all automatically unparents active children in other columns (preserving their work and allowing a single Undo);
- cards observed created, moved (including placement changes), or edited while TaskBoard is open gain a blue border for 10 minutes, with a hover timestamp; the indicator survives a window restart via local UI storage and does not change the Markdown task or add an Attention flag;
- tasks entering a terminal column such as Done receive a completion date and precise completion timestamp, clear their **For user** status (including legacy `needs-user`), appear at the very top (newest first, including cards completed by agents), and are automatically archived after 7 days; reopening them clears both timestamps;
- hover cards for a full preview including task ID, description and its update timeline, acceptance criteria, question, and any historical user/AI notes; previews dismiss quickly when leaving the card (unless moused into the preview to inspect it); title text is relatively light, and agents can bold selected title phrases using Markdown `**important words**` (rendered safely in card/hover, stored verbatim in Markdown);
- Refresh forces a fresh Markdown reload and confirms when finished; the board also refreshes automatically every few seconds while you are not interacting with it;
- click cards for full details, question answering, and an optional vector task-type icon (Bug, New feature, Refactor, Content, Validation, Design, Visual Work);
- task icons are stored as optional `icon` values in Markdown frontmatter, and can also be set using the CLI `task create --icon` or `task update --icon` (`--clear-icon` to remove);
- when agents inspect or claim `Visual Work` tasks, the CLI returns a non-blocking Claude-only advisory; `Design` tasks return a non-blocking Opus/Astra-only advisory for initial drafts (subsequent revisions are unrestricted);
- The yellow **User queue** top-bar button counts outstanding, unsnoozed **For user** tasks and opens them in the editor newest-first (creation date, then ID). **Skip** leaves the task untouched and moves forward; **Snooze 24h** likewise moves forward but temporarily removes the task from the queue and its count. Snoozes survive an app restart via local UI storage, automatically expire after 24 hours, and never edit the underlying Markdown task. **Save** persists edits and advances to the next queued task; changing the column to Done clears that task from the queue. **Close & Followup** in queue review opens the new follow-up for editing, then saving or closing that follow-up resumes the queued review where it left off. The pass ends quietly without a notice bar; Cancel/Escape on a queued task exits the pass, and skipped tasks remain available next time.
- `Ctrl+T` creates a task in the default AI Ready column; `Ctrl+S` saves/closes an open task (or saves and advances during User queue review); `Esc` closes it; `Ctrl+Z` undoes the last persisted board action when the editor is closed.

Attention icons:
- **? Question** — answerable user question; floats to top.
- **✎ UserNote** — you wrote/changed a Note to AI; floats to top until an agent handles it.
- **× Blocked** — cannot proceed; floats to bottom.

The **Note to AI** field is a submission composer rather than a second permanent notes section: starting an instruction clears **For user** immediately in the editor, and **Save** appends it to the Description timeline with a timestamp, empties the composer, and raises `UserNote` for AI attention. **For user** is now the single human-input status in the UI and user queue; the old `NeedsUser` attention flag remains readable as a backward-compatible alias. New CLI `--attention needs-user` creations become For user cards. The independent AI Notes editor was removed; historical agent notes are displayed in the timeline and imported into Description on save.

`AI Ready` is the default new-task column. Any non-terminal column can also be marked AI-ready with its ✓ control; Draft and terminal columns cannot. Task/board writes use SHA-256 content revisions plus a cross-process lock, so a concurrent Obsidian/agent edit is rejected as a conflict rather than silently overwritten.

## Epics and task dependencies

- **Task** (`kind: task`, normally `WT-####`) is an executable work unit an AI can claim and move. **Only epics can be parents.** Tasks and For user cards can be children of an epic but cannot have subtasks of their own. Epics are root-level containers, not nested within other tasks or epics.
- **For user** (`kind: for_user`, normal `WT-####` IDs) is **human-only work**: asset/mesh supply, animation references, playtesting, approvals, design decisions. It has a conspicuous **orange** card and FOR USER badge, and sorts toward the top of its column. Agents can create it and attach planning notes, but cannot claim it or finish/archive it through the agent CLI. A human completes it by moving it to Done in the TaskBoard app. The card can be a hard prerequisite (`depends_on`) for downstream AI implementation; `task list --ai-ready` omits all user-only tasks. When the human marks the card Done, its persisted kind changes back to an ordinary task and its legacy NeedsUser flag is removed.
- **Epic** (`kind: epic`, newly created IDs `WTE-####`) is a coordination container: scope, designs, requirements, notes and subtask progress. **Epics do not have acceptance criteria, AI Notes, card icons, question/attention flags, hard dependencies or a parent.** Their editor instead exposes a dedicated planning/coordination Notes field. Epics cannot be claimed, assigned, or moved directly. Their column is derived from descendant executable tasks (In Progress takes priority; all completed => Done; entirely blocked/waiting => Blocked; otherwise the most active ready work column). A new epic with no children stays Draft.
- Existing epic IDs retain their historical `WT-####` identifier for compatibility with code, docs and agents; they still get a distinctive purple epic card (without a redundant EPIC badge), non-executable behavior and progress tracker.
- **Cross-column epic proxies:** When descendants are visible in a column other than their nearest epic ancestor, TaskBoard renders a **thin purple epic proxy** (without an EPIC badge) just above each contiguous group of those descendants. Hovering the proxy highlights the one real epic card and its column; clicking jumps to the real card (or opens it if its column is intentionally hidden). Proxies are visual references only: no duplicate task records, claims, status changes, or drag handles. They respect search, collapsed descendants and dependency ordering.
- **Collapse epics:** Each epic has a collapse/expand chevron; the toolbar has **Collapse all epics / Expand all epics**. Folding hides ordinary descendants across all columns, including while searching, while **orange For user cards always remain visible**. Fold state is stored locally and actions support Undo. Cross-column epic proxies remain for visible orange tasks.
- **Done archive by age:** Right-click a terminal/Done column and choose **Archive older than 3 days**. This archives only cards with an actual `completed_at` timestamp at least 72 hours old. Recently completed, undated and still-active epic cards are preserved; younger children are never detached just to archive a parent. The action checks revisions under a write lock and supports Undo. The existing **Archive all** remains separate.
- Each executable task can declare `depends_on: [WT-####, ...]` (hard prerequisite) and `related_to: [WT-####, WTE-####, ...]` (informational). Enforced by shared Rust core: no claiming or starting an unfinished prerequisite's dependent, no dangling/self/duplicate links, no hard dependency cycles, and no hard dependency on an epic. Completion/archiving satisfies a prerequisite; reopening makes dependents wait again.
- In the task editor, the **Task / Epic** selector is at the top (with a separate orange For user toggle for ordinary cards). Only executable/user tasks get an optional parent dropdown, containing epics exclusively. Dropping a task onto an epic makes it a child. **Dropping a task onto another ordinary task places it directly below that card in the target column**, without changing the hierarchy. The placement is persisted in task frontmatter as `position_after`/`positioned_at`, and hard dependencies still take precedence. Tasks expose Ready, Waiting and Blocked states separately from their columns. The app edits dependency links using searchable multi-select task pickers (search by ID or title), and shows clickable links in hover previews. **Directional red prerequisite-to-dependent lines** connect only *unfinished* visible prerequisites to their dependents, never from Done cards. Once every dependency is complete, an ordinary task in Blocked automatically returns to AI Ready (or In Progress when already agent-claimed); explicit Blocked attention and human-only tasks remain blocked. They are drawn in an independent pointer-transparent SVG overlay, never reserve layout space, track scrolling/resizing, and are visually occluded behind unrelated intermediate columns. The column uses a stable topological ordering so a dependency always appears above the task it gates; unrelated tasks retain their normal sort order, and epic parents remain above their displayed children. `task list --ai-ready` excludes epics, tasks with unfinished dependencies and explicitly blocked tasks.
- CLI usage: `task create --kind epic --title "Work stream"`, `task create --depends-on WT-0012 --related-to WT-0015 ...`, `task update WT-0034 --depends-on WT-0012 --related-to WT-0015`, `task update WT-0034 --clear-dependencies`; `task show` includes `readiness`, `waiting_for`, `blocks` and `epic_progress`.
- Only declare hard dependencies when the **entire task cannot meaningfully begin**. Use `related_to` for partially shared work or cross-functional coordination to avoid unnecessarily serializing autonomous agents.

## Corrected hierarchy migration (2026-10-15)

The earlier migration missed executable-looking parent cards in Combat Implementation. This follow-up explicitly promoted WT-0072 and WT-0074 to epics, along with WT-0009, WT-0059, WT-0162 and WT-0187. To preserve outstanding execution work, four dedicated subtasks were created (WT-0193–WT-0196); WT-0193 depends on WT-0176 and WT-0195 depends on WT-0155/WT-0159, making the dependency arrows visible in Combat Implementation. Four unrelated/misleading historical parent edges were detached (see migration report and backup in `artifacts/migrations`). Existing parent tasks WT-0043 and WT-0051 remain executable **temporarily because other agents currently claim them**; converting those would invalidate running work. The board still permits executable parent tasks where necessary.

## Human-action cards (2026-10-15)

Four orange **For user** cards were split from ongoing combat/animation work without changing tasks currently claimed by agents:

- `WT-0202` — Decide combat controls and CS99 assumptions (split from user request on `WT-0103`).
- `WT-0203` — Playtest retreat attacks and novice parry; choose preferred feel (split from `WT-0111`).
- `WT-0204` — Provide/approve human and boar skeletal mesh/anatomy references (from asset work `WT-0118`). Final anatomy QA `WT-0189` now depends on this human action, but content authoring can continue independently.
- `WT-0205` — Choose ALS-ready spear/club stance, parry and reaction animation references (`WT-0071`).

The earlier agent-owned tasks are preserved. Only the final anatomy QA was given a new **hard** dependency because its acceptance explicitly requires actual approved meshes; other user actions are linked informationally to avoid unnecessarily blocking agent progress.

## Developer commands

```powershell
npm run build
cargo test --manifest-path packages/task-core/Cargo.toml
cargo test --manifest-path packages/task-cli/Cargo.toml
npm run tauri dev
npm run tauri build
```
