# TaskBoard: multi-project groundwork and remaining generalization

## Implemented (2026-10-22)

### Live board discovery (2026-10-23)

The desktop now merges its local catalog with existing `jjj\[ProjectName]\Tasks\_board.md` boards on each project-list read. Agent-created boards appear without manual catalog registration. Discovery reads only metadata, preserves existing tab IDs and folder associations, uses stable path-based IDs for discovered boards, and follows board names from Markdown. It never loads task stores or writes the catalog merely to list projects. The UI refreshes every two seconds and on window focus, pauses during editors/dialogs/drags/operations, and preserves the window's selected project when another window changes the shared active tab. Explicit tab removal records a backward-compatible `hiddenPaths` list in the local catalog; explicit project addition clears that suppression.

The desktop application now hosts a project tab strip. A native per-user project catalog
is stored in `%APPDATA%\TaskBoard\projects.json` (with a prior-copy backup as
`projects.json.bak`). A fresh installation uses the existing Wartribes board as
its sole initial tab; no Wartribes task files are migrated, renamed, or copied.

The **+ Project** dialog uses the Windows-native **Browse…** folder picker for source/working directories and suggests a name from the first folder. The path field precedes the name, and **+ Add folder** can associate multiple source, assets, or working directories with one project. The automatically managed Markdown board is always placed in `B:\GDrive\SharedWithPhone\Obsidian\jjj\[ProjectName]\Tasks`; existing board data is reused without overwriting it. There is no initialization checkbox or manual board-root selection. A gear icon on each tab allows editing associated folders later; editing folder associations never moves task files. Legacy project catalog entries without the new `folders` field load unchanged. Duplicate board tab paths are rejected. Project tab removal **only** removes the catalog entry; it never deletes the project board or its task files. At least one tab must remain. The last selected tab is restored on startup.

On switching tabs, React completely unmounts the previous project board and
mounts a new one keyed by the registered project ID. This isolates open editors,
user queues, drag state, undo histories, snapshots, polling, and selection. Tab
switching and removal are disabled during an active editor, drag, or operation
to prevent silent draft loss. Per-project local UI storage keeps activity
indicators, user-queue snoozes and collapsed-task preferences apart.
Wartribes' existing storage keys are unchanged for compatibility.

Existing Tauri task APIs already accept a board `path` on every operation;
the tabbed desktop passes its active project path explicitly. The default
CLI root remains Wartribes to avoid silently redirecting existing agents
when someone switches UI tabs. For a different project agents must specify
`--root "<project Tasks folder>"` explicitly.

**ID namespace:** A board is currently identified by its full directory path,
not by the task ID. Different projects may therefore both contain `WT-0001`,
because all reads, writes, task links and conflict guards operate inside a
single board root. No cross-project dependencies are available or implied.

## Important current limitations / next phase

1. **Project-specific ID prefixes (implemented in source 2026-10-23; deployment pending):**
   `_board.md` optional `task_prefix` and `epic_prefix` govern only NEW IDs. The
   legacy default remains WT/WTE when absent, including Wartribes. Core scans
   task filenames by general ID shape rather than current prefix, so legacy
   WT/WTE and earlier custom-prefix Markdown stay intact after configuration.
   The desktop creation dialog suggests editable prefixes; tab settings edit
   them, storing the result in `_board.md` for `--root` CLI compatibility.
   T3Code must be explicitly set to T3/T3E after safe deployment; no live
   board files are changed by source-stage implementation. No bulk renaming.
2. **Shared project catalog API for agents:** Extract the catalog module from
   the Tauri desktop to a reusable Rust crate or library. Add explicit
   `taskboard --project <stable-id>` resolution while retaining `--root`
   and the old Wartribes CLI default, with a clear precedence contract.
   Never implicitly target the currently selected UI tab from an agent CLI.
3. **Board workflow metadata:** `draft`, `ai-ready`, `in-progress` and
   `blocked` are currently meaningful built-in column IDs. Add schema-level
   workflow role metadata before supporting arbitrarily named custom flows.
   Preserve present core dependency/claim/Done semantics on existing boards.
4. **Optional integrations:** Fomni highlighting and Obsidian file opening are
   Windows-specific actions. Treat them as optional per-project integrations
   rather than requirements for creating or using a project.
5. **Configuration durability:** Catalog persistence is local to the Windows
   user, separate from all Markdown boards. Adding a tab is not a task
   migration. Back up projects.json when migrating machines; do not sync
   two running desktop processes' catalog writes without conflict control.
6. **Project-level identity and UX:** Native folder browsing and multi-folder metadata are now implemented. Later work may expose project colors, rename/reorder, per-project status badges, and explicit import/migration flows. Cross-project references must use a qualified project ID/path, not only `WT-####`. Multiple associated source folders do not mean the task system scans them: the Markdown task store remains a single Obsidian folder per project.

## Safety / acceptance boundaries

- Existing Wartribes Markdown is not modified merely by registering or
  switching project tabs except for normal core maintenance that already
  occurs when a board loads (archive age, dependency unblock, etc.).
- No user files are deleted on removal; app configuration only.
- Never merge tasks solely by matching `WT-` IDs across boards.
- An invalid project catalog must surface an error; never silently overwrite it.
- Switching is blocked until an open editor/operation is finished or cancelled.
- Each new board remains immediately usable with the existing CLI
  `--root` path even before the later generic prefix/CLI phases.
- Registered source folders are read-only metadata in TaskBoard: creating a project never generates files in or moves the original source folders.

## Live changes 2026-10-24: safe project tab and task moves

- The accidental closure of **Citizen** was restored using its original Markdown board. Closed tabs suppress rediscovery by adding their board path to the local catalog's `hiddenPaths`; source folders and Markdown tasks are never deleted. The naked top-bar **×** has been removed. To close a tab: click that project's ⚙, then **Close tab…**, then **Confirm close**. Ctrl+Z or the **↶ Undo close** control reopens the most recently closed tab with its original stable catalog ID and folder associations. Ctrl+Z also undoes project tab reordering. A project catalog backup was retained during the Citizen repair.
- Tasks can be dragged from their cards onto a *different project tab* to transfer them. The native backend reserves a destination ID using its own board's allocator, keeps the original task body/notes, verifies unchanged source revision, and deletes the original only after a valid destination is persisted. It switches to the destination after success and displays `oldID → project/newID`. Transfer intentionally refuses claimed tasks, epics, cards with review media, attachment references, hierarchy, dependency/related links, or incoming references so the destination cannot silently orphan related tasks. Unlinked active tasks transfer directly. Movement of a card within the board is unchanged.
- **Related to** now searches all registered project boards and stores remote task links as `<board root>#<task ID>`. These are informational references, never hard dependencies. The picker displays the short project name and task ID; the backend stores the qualified string, and validation accepts it without requiring the remote board to be online. Ordinary `Depends on` stays strictly local.
- Manual migration requested: OMN-0005 → MISC-0033 (Inbox improvements), OMN-0004 → MISC-0034 (Better Undo support), preserving all text and removing Omni originals only after verifying the destination content. Originals backed up in `artifacts/cross-project-transfer-20261024-omn/`.
- The 2026-10-24 Tauri release built and restarted successfully; screenshot confirmed the Citizen tab visible and the project tabs no longer have × buttons. 
