# TaskBoard: multi-project groundwork and remaining generalization

## Implemented (2026-10-22)

The desktop application now hosts a project tab strip. A native per-user project catalog
is stored in `%APPDATA%\TaskBoard\projects.json` (with a prior-copy backup as
`projects.json.bak`). A fresh installation uses the existing Wartribes board as
its sole initial tab; no Wartribes task files are migrated, renamed, or copied.

The **+ Project** dialog can register an existing TaskBoard root (a directory
containing `_board.md`) or explicitly initialize a new board in an **existing**
directory. The directory is canonicalized and duplicate tab paths are rejected.
Project tab removal **only** removes the catalog entry; it never deletes the
project board or its task files. At least one tab must remain. The last selected
tab is restored on startup.

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

1. **Project-specific ID prefixes:** The core still creates `WT-####` tasks
   and `WTE-####` epics and discovers Markdown files with those prefixes.
   Define a backward-compatible `task_prefix` in `_board.md`, then update
   generators, scanners, dependency parsing, search, and UI link pickers. Never
   silently rename existing Wartribes IDs or rewrite cross-references.
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
6. **Project-level identity and UX:** Future work may expose project colors,
   rename/reorder, a native folder picker, per-project status badges, and
   explicit import/migration flows. Cross-project references must use a
   qualified project ID/path, not only `WT-####`.

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
