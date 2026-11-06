---
name: taskboard
description: Manage project backlogs and work across Markdown TaskBoard boards, including tasks, epics, dependencies, claims, human feedback, visual reviews, and board columns. Use for durable work management in any project; follow that project's separate source/build/runtime coordination rules.
---

# TaskBoard

## Select the project board

TaskBoard supports multiple projects. Use the board associated with the user's request or repository; there is no universal canonical board.

- Boards normally live at `B:\GDrive\SharedWithPhone\Obsidian\jjj\<ProjectName>\Tasks`; `_board.md` identifies a board.
- Resolve the board from project instructions or the local catalog at `%APPDATA%\TaskBoard\projects.json`, which records board paths and associated source/working folders. Existing boards can also be discovered under the normal board location. If the intended board is still ambiguous, clarify before writing.
- Pass `--root "<board Tasks folder>"` explicitly on every CLI call. The CLI's legacy default is the Wartribes board; it does not follow the selected desktop tab.
- Task IDs are scoped to a board and may use project-specific prefixes configured by `task_prefix` and `epic_prefix` in `_board.md`. Use actual IDs returned by that board, never infer the project or task kind from a prefix. The `TASK-` IDs below are examples, not a required prefix.
- Hard dependencies stay within one board. Cross-project informational `related_to` links use `<board root>#<task ID>`.

CLI: `D:\SSDProjects\Tools\TaskBoard\TaskBoard.exe` (same executable launches GUI when called without arguments; supports `--help`, `--root "<board Tasks folder>" --json task ...` directly). Alternative: `D:\SSDProjects\Tools\TaskBoard\taskboard.cmd`.

When **rebuilding or restarting the TaskBoard desktop app itself**, use `npm run rebart` from `D:\SSDProjects\Tools\TaskBoard`. It stops both known production paths, runs a proper Tauri production build, repairs the canonical root `TaskBoard.exe` symlink to `src-tauri\target\release\taskboard.exe`, and relaunches the root path. Do not copy an EXE to the root path: that replaces/breaks the symlink. Direct Cargo release and Tauri release builds use the same target and therefore update both names automatically while the link exists, but `npm run build` builds only frontend assets and a debug or alternate-target build is not deployed. `npm run canonicalize` checks/repairs the link without rebuilding. See the repository README for details.

For mutations, inspect each command's exit code and JSON before continuing, and verify the final state with `task show`. Do not hide errors with `| head` or run dependent writes after failure.

Use the CLI instead of directly editing TaskBoard frontmatter when a semantic operation exists. For automation, pass `--json`.

## AI-ready column semantics

Every column has a persistent `ai_ready` boolean. Autonomous agents may select work from **any** column where `ai_ready: true`.

- The canonical **AI Ready** column (`id: ai-ready`) starts with `ai_ready: true` and is the default destination for newly created tasks.
- Newly created columns default to AI ready except columns called Draft, Done, Blocked or Review; existing boards were backfilled on 2026-10-23. Users may still change the marker.
- **Draft** and terminal columns cannot be AI-ready.
- Discover autonomous work with `task list --ai-ready`, which follows the current board markers.

## Epics, hard dependencies and agent-safe work selection

**Strict epic-only hierarchy:** Only `kind: epic` may be used as a `parent`. Epics themselves must have no parent (no nested epics). Regular tasks and human-only For user cards are leaves, not containers. The CLI/shared core rejects creating or reparenting a task under another regular task, and rejects parented epics. For epics, never add acceptance criteria, AI Notes, icons, attention flags or questions; put scope, design and retained historical handoffs into `description` or `notes`. The UI presents a prominent Task/Epic switch and an epic-only parent selector. Dragging an ordinary task onto another ordinary task **moves it below the target in its column**, rather than adopting it as a subtask; dropping onto a real epic creates the parent relationship. Do not mistake visual purple cross-column epic proxies for independent tasks.

**Epics and For user cards are not agent-executable tasks.** Task types are `kind: task` (AI-executable), `kind: epic` (coordination), and `kind: for_user` (**human-only**). New epic IDs use the board's configured epic prefix (legacy default `WTE`); historical IDs are retained for compatibility. Identify epics by `kind: epic`, and never infer executability from an ID prefix. Do not claim, assign, move, or implement an epic. Epic descriptions are for scope, design, requirements and coordination notes. Select one of its executable subtasks instead. The shared Rust core rejects epic claims/moves; epic status and Done state follow descendant executable tasks automatically.

**For user** (`kind: for_user`) is a dedicated **orange** task type for work actually owned/performed by the human, not an agent's request for approval or verification. Do not create separate review or verification tasks duplicating the implementation card. Attach questions (`task update ID --question ... --option ...`) and visual reviews (`task review request ID ...`) directly to the original task. Use a For user task only for a substantial independent action the human must perform, such as providing materials. Prefer splitting that human action out from agent-owned implementation rather than converting a task currently claimed by another agent. Agents may create user cards and add technical notes but **must not claim, perform, complete, or archive them**, and must not reclassify them as executable through the CLI to bypass the restriction. `task show` marks readiness as `for_user`; `task list --ai-ready` omits them; the CLI rejects claim, complete, move-to-terminal and archive. The user moves the orange card to Done in the desktop application, satisfying any dependent tasks. To create a genuinely human-owned action: `task create --kind for_user --title "Provide approved artwork assets" --parent TASK-0071 --related-to TASK-0162 --description "..."`. Without `--column`, these default to In Progress. Only add a **hard** `depends_on` to downstream implementation when that entire task genuinely cannot start until the human finishes; use `related_to` otherwise.

**Readiness is separate from column.** Regular tasks may declare:
- `depends_on: [TASK-0012, TASK-0034]`: hard prerequisites. Claims and attempts to start blocked work are rejected until those tasks are Done or archived. Cycles, missing targets, self-dependencies, duplicates, and dependencies on epics are invalid.
- `related_to: [TASK-0012]`: informational links only, no claim or move restriction.
- `task show TASK-####` returns `readiness`, `waiting_for` and `blocks` (plus `epic_progress` for epics). Check these before executing.
- `task list --ai-ready` returns only executable tasks with no unfinished hard prerequisites or explicit blockers. A task being in an AI-ready column does not alone guarantee that it is ready.
- Avoid speculative hard dependencies: if two tasks can proceed independently and later integrate, allow both to proceed and put hard dependencies on the integration task.

Use `task create --kind epic` for new epic containers; `task create --kind task --depends-on TASK-0012 --related-to TASK-0013` for executable work. Set/replace links with `task update TASK-#### --depends-on TASK-0012` or `--related-to TASK-0013`; clear with `--clear-dependencies` / `--clear-related`. Multiple dependencies can be passed by repeating the flag.

## Attention flags and notes

- **`question`** — a concrete question the user can answer. Always provide a structured question prompt; add `--option` values when useful. Question cards float to the top.
- **`needs-user`** — the user must do/review/provide/approve something that is not best represented as one structured question. Explain exactly what is needed in AI Notes. Floats to the top.
- **`user-note`** — the user wrote or changed `## User Notes` for the AI to act on. Treat this as an explicit human instruction/comment. Read it before doing more work. Address it, then clear only the `user-note` attention flag; normally leave the User Notes text intact as durable context. Floats to the top.
- **`priority`** — user-defined urgency. Implement actionable priority tasks before other ready work, respecting prerequisites; the board and CLI list them first.
- **`blocked`** — the task cannot currently proceed. **You must supply a concise, easy-to-read `--blocked-reason "..."`** whenever you add the `blocked` flag or move a task into the Blocked column (`task update ID --add-attention blocked --blocked-reason "..."`, `task move ID blocked --blocked-reason "..."`, or `task create ... --blocked-reason "..."`); the CLI rejects it otherwise with `blocked_reason_required`. The reason is stored in `blocked_reason` frontmatter, shown by `task show`/`task list`, and recorded once as a red **Blocked** Description timeline update (a changed reason adds a new entry; ordinary edits do not). It is cleared automatically when the task leaves Blocked. Tasks already blocked before this field existed are grandfathered (no reason required to keep editing them; `task show` returns `blocked_reason_source: legacy_ai_notes` with the first AI Notes line as fallback). Blocked cards sort to the bottom. If no useful work can continue, normally move the task to the board's blocked column too. The human unblocks with **Authorize Unblock** in the desktop editor, which moves the task to AI Ready and adds a human note: “Agents have full authority to do what it takes to unblock this task.” Treat that as explicit authorization.

**AI Notes** are agent-authored current-state/handoff notes. Keep them concise and current.

**User Notes** are human-authored instructions/feedback such as “make this clearer,” “make a proposed design,” or “here are my notes for your proposal.” Do not rewrite or erase them just because you handled them. Handle the requested work, update the task/AI Notes as appropriate, then remove `user-note` from attention. If the user later edits the note in the desktop app, TaskBoard raises `user-note` again automatically.

A structured unanswered question is stored in frontmatter. When the user answers in the desktop app, TaskBoard persists `question.answer` and automatically removes `question`.

## Formatting titles

To emphasize specific words in a task title, use Markdown `**bold words**` in the title field, e.g. `Investigate **critical regression** in the editor`. The board safely renders this as bold in the card overview/hover, and stores the title unchanged in Markdown frontmatter. Prefer emphasizing just the critical phrase instead of making the whole title bold.

## Categories vs tasks

Open-ended collections such as `Feature Ideas` and `UI` are categories, not permanent parent tasks. Only a dedicated epic (`kind: epic`) may organize subtasks. Convert legitimate multi-work parents to epics; otherwise detach/reparent legacy child links to the nearest real epic, retaining prior associations via `related_to`.

## Task-type work advisories (non-blocking)

The optional `icon` frontmatter classifies tasks. `task show` and `task claim` return structured `warnings` when relevant. **Read the warnings before beginning work** and self-check model eligibility; the CLI intentionally does not block or verify identity.

- **Visual Work** (`visual-work`): only **Claude Opus** agents may author/iterate the visual design. All other models leave it for Claude Opus.
- **Visual Implementation** (`visual-implementation`): an approved mockup/design is ready, and non-Claude agents may implement it.
- **Design** (`design`): only **Opus or Astra** should produce an **initial design draft**. Other agents may refine a draft that already exists, but should not create the initial one.
- Other icons have no special work advisories. A warning is *not* the same as a `blocked` attention flag.

If your agent/model is not eligible, do not claim/move the task; if already claimed, release it. Never treat the warning as a CLI error or an authorization mechanism.

## Tasks that need a live conversation (With me)

For a task marked `live.with_me`, do all independent preparation before asking the human to join. Park the **same task** using `task live ready TASK-#### --brief "Done: ... / Need from you: ... / Minutes: ..." --est 15`; keep the brief to three short lines. This sets `live.state: ready` and `needs-user`, so TaskBoard and T3 can present one **Ready for you** item. End your turn rather than pinging OpenRig human inbox, AI Speak, or creating a separate TaskBoard feedback request. One-off questions and visual approvals still use ordinary TaskBoard question/review actions.

The session link remains the existing `claimed_by`; don't create or send to a second session. Use `task live start TASK-####` when the human starts the live exchange, `task live later TASK-#### --until <RFC3339>` to postpone, and `task live clear TASK-####` if no longer needed. Queue-dispatched With-me workers receive the same prep/park guidance when OW-0017's dispatcher is active; manual agents follow it immediately.

## Normal workflow

1. Discover autonomous work with `task list --ai-ready` or targeted search. This may return tasks from several columns.
2. Inspect the task with `task show`, including User Notes, AI Notes, attention flags, any question answer, **and work-type warnings**. Follow the model eligibility advisory before starting.
3. If `user-note` is present, treat the User Notes as the user's current instruction before continuing.
4. Reject `kind: epic` and `kind: for_user` as agent work targets. Inspect epics for executable subtasks; leave For user tasks to the human. For a regular task, confirm `readiness: ready` and no `waiting_for` blockers. Claim only work actually being started; move it to `in-progress`.
5. Work under the repository's separate source/build/editor coordination rules.
6. Request input on the SAME implementation task with `task ask-user TASK-ID --type decision|visual-review|verification|action --question "..."`. Decision requires at least two `--option` values; visual review requires actual `--artifact` previews (and optionally `--mode`); verification provides Done/Needs changes options. Requests become Inbox items without For user cards or duplicates.
7. After handling a User Note, clear `user-note` but preserve the note text unless the user asks to remove it.
8. Use `task finish TASK-ID` to complete technical/trivial work autonomously. If human verification is explicitly required and implementation is already testable, use `task finish TASK-ID --verification user --instructions "<deployed build and exact checks>"` to atomically move implementation to Done and open one Inbox verification on the same card. Pending acceptance requests remain Inbox-visible and must not be age-archived; they do not make a finished implementation Blocked.

## Examples

```console
taskboard.cmd --root "<board Tasks folder>" --json task list --ai-ready
taskboard.cmd --root "<board Tasks folder>" --json task show TASK-0042

taskboard.cmd --root "<board Tasks folder>" --json task ask-user TASK-0042 --type decision --question "Which interaction should we use?" --option "Radial menu" --option "Context menu"

taskboard.cmd --root "<board Tasks folder>" --json task update TASK-0042 --question "Does the released animation look correct?" --option "Done" --option "Needs changes"
taskboard.cmd --root "<board Tasks folder>" --json task update TASK-0042 --add-attention blocked --blocked-reason "Missing source artwork; designer must supply it."

# After acting on a human note:
taskboard.cmd --root "<board Tasks folder>" --json task update TASK-0042 --remove-attention user-note
```

## Safety

Markdown remains canonical. The CLI and desktop app share `task-core`; semantic writes carry SHA-256 content revisions and use a cross-process mutation lock. Unknown frontmatter and extra Markdown sections are preserved. Treat `write_conflict` as a request to reload and reconcile—never overwrite a concurrent user/agent edit.

TaskBoard claims are durable work assignment only; follow the target project's separate editor/build/source ownership rules.

## Visual reviews

Use `task review request <id> --mode pick|approve|rank|markup --judge "<one sentence>" --artifact "<path>=<label>"` for subjective visual decisions; prefer 2–4 variants and a short clip for 3D/animated outcomes. **For multi-screen design/motion reviews, attach every screenshot and video as its own labeled `--artifact` so the full-window visual inspector presents a navigable filmstrip. Attach interactive HTML separately rather than bundling all images, video and copy into one long webpage.** `approve` supports one new proposal without a previous version. Supply `--previous` when useful for before/after comparison; only image-to-image comparisons enable side/overlay controls. URLs require a poster. `task review show <id>` returns the full manifest, labels, references and verdict history. The desktop **Review** view displays open requests, allows normalized pins/notes and accepts human approval, rejection or More variants. Never call `task review resolve` to fake a user verdict; its `--human-confirmed` flag is reserved strictly for the actual human reviewer. Results remain in the task Description timeline. Review attachments under each board's `Tasks/Reviews` folder are managed by the retention process; agents should not delete them themselves.

## Rolling releases and agent Blocked confirmation

Release collection is **off by default** and must be explicitly enabled per board (`release enable true` or the Releases UI). Continue using ordinary `task complete` or moving implementation to Done after source tests and commit; do not put otherwise finished implementations in Blocked awaiting build, restart, release verification or human deploy authority. TaskBoard enrolls eligible Done tasks into its rolling first-class Releases history, without blocking further work. A release attempt snapshots the exact integrated source manifest while newly completed tasks keep arriving for the next attempt. Do **not** auto-restart or auto-deploy any app. For CLI examples and stages see `D:\SSDProjects\Tools\TaskBoard\docs\RELEASE_WORKFLOW.md`.

For an actual inability to continue implementation, provide a concrete `--blocked-reason` on agent CLI `task move ID blocked`, `task update ID --add-attention blocked`, or direct blocked create. The first request deliberately rejects with structured code `blocked_confirmation_required`, self-review instructions, and a one-use ten-minute challenge flag/token. If and only if there is a true blocker, retry the same operation and reason with the specific hidden confirmation flag/token returned by that rejection. An arbitrary preselected token, token for another board/task/reason/operation, or reused token is not accepted. No human approval is necessary for legitimate blockers; the human GUI is exempt.

New release commands follow the ordinary opaque OMCP argument-array forwarding scheme (e.g. `["release","list"]`, `["release","show","REL-0001"]`, `["release","prepare","REL-0001"]`). Existing `task` and `column` command shapes are unchanged. Do not run `release reconcile --historical` on live boards unless explicitly authorized, because it opts historical Done work into the release stream.
