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

Canonical new attention values: `question`, `user-note`, `blocked`, `priority`. The old `needs-user` value is a backward-compatible alias for the **For user** task kind, not a separate UI status. New `task create --attention needs-user` and `task update --add-attention needs-user` requests become `kind: for_user` (the latter requires an unclaimed non-epic task). Legacy tasks still carrying `needs-user` are considered human-only for CLI readiness, claim, Done, and archive operations; only the user can clear this legacy status through TaskBoard.

- `question`: unresolved structured question for the user.
- `user-note`: human-authored instructions requiring AI attention. The desktop editor submits Note to AI as a timestamped update inside Description and clears the composer while setting `user-note`; agents should clear this flag after handling the instruction, without removing the recorded history.
- `blocked`: cannot proceed. Agents must pass a concise `--blocked-reason <text>` when adding this flag or moving/creating a task in the Blocked column; otherwise the command fails with `blocked_reason_required` (exit 4). The reason is stored as `blocked_reason` frontmatter, appears in `task show` (top-level `blocked_reason` + `blocked_reason_source`) and `task list`, and is appended once to Description as a red `blocked` timeline update (`<!-- taskboard:update:blocked|<ts> -->`). A different reason appends a new entry; ordinary edits do not. Leaving Blocked clears it. The desktop editor never edits the reason; its **Authorize Unblock** button moves the task to AI Ready and appends a human note: "Agents have full authority to do what it takes to unblock this task." Legacy tasks blocked before this field existed are grandfathered: agents may keep editing them without a reason, and `task show` falls back to the first AI Notes line (`blocked_reason_source: legacy_ai_notes`, or `missing_legacy`).
- `priority`: user-set urgency. Actionable priority tasks appear first in `task list` and `task list --ai-ready`. Agents should handle them before ordinary ready work, respecting dependencies. Set or clear it via `task update ID --add-attention priority` / `--remove-attention priority`, or right-click the card in the desktop app.

Questions can include repeated `--option` values. The desktop app persists the answer and removes `question` automatically. Historical `AI Notes` still load for compatibility; the new desktop editor displays them in the description timeline and imports them to Description when the user saves. CLI `--ai-notes` remains supported for older integrations.

## Work-type warnings on task start

For tasks tagged with `icon: visual-work` or `icon: design`, `task show` (preflight) and `task claim` (start) include a top-level `warnings` array in the JSON result.

- Visual Work: Claude Opus agents only.
- Design: Opus/Astra agents only for the **initial draft**; later revisions may be done by other agents.

Example warning shape: `{"warnings":[{"code":"visual_work_claude_opus_only","message":"...","advisory_only":true}]}`. These are agent-facing instructions, not enforced permissions: the command still succeeds (exit 0), the claim still happens, and existing result fields remain at their previous paths. Agents must self-check and abstain/release when ineligible.

## Main commands

```text
task list [--ai-ready]
task show <id-or-exact-title>
task search <query>
task create --title <title> [--column <column>] [--category <category>] [--parent <task>]
            [--ai-notes <text>] [--user-notes <text>]
            [--attention question|needs-user|user-note|blocked]
            [--question <prompt>] [--option <answer>]... [--blocked-reason <text>]
task update <task> [--title ...] [--description ...] [--category ... | --clear-category]
            [--ai-notes <text> | --clear-ai-notes]
            [--user-notes <text> | --clear-user-notes]
            [--add-attention question|needs-user|user-note|blocked]
            [--remove-attention question|needs-user|user-note|blocked]
            [--question <prompt> --option <answer>...] [--clear-question]
            [--blocked-reason <text>]
task move <task> <column> [--blocked-reason <text>]
task parent <task> --parent <task>
task complete <task>                        # legacy autonomous completion
task finish <task> [--verification none|user --instructions "<deployment + exact checks>"]
task ask-user <task> --type decision|visual-review|verification|action --question <prompt>
             [--option <choice>]... [--artifact <path=label>]... [--mode approve|pick|rank|markup]
             [--previous <path=label>] [--poster <path>]
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

`task ask-user` attaches one typed human request to the original AI task and rejects a duplicate open request. Decision requires 2+ choices; verification uses Done/Needs changes. Visual review requires real screenshot/video/HTML evidence. `task finish --verification user` atomically moves implementation to Done and opens a verification request; normal `task finish` closes technical/trivial work without a human gate. The Inbox Done button closes the request; Needs changes requeues the original AI task. Agents should not invent or rewrite User Notes as if they came from the user. They are a human-authored channel. Handle them and clear `user-note` when done.

The CLI preserves valid manual extensions through the shared core. Semantic writes carry SHA-256 content revisions and use the same cross-process mutation lock as the desktop app. If a task or board changed after it was loaded, the write is rejected instead of overwriting the concurrent edit; reload/reconcile and retry. TaskBoard coordination remains separate from Wartribes source/build/editor coordination.


## Visual Review mode (desktop and phone)
The task-oriented review API is available on every board using the same explicit `--root`.
Use it for visual or usability decisions instead of a vague "Needs user: review the look" handoff.
The reviewer, never the agent who produced the artifacts, records the verdict.

```powershell
TaskBoard.exe --root "B:\GDrive\SharedWithPhone\Obsidian\jjj\Misc\Tasks" --json task review request MISC-0006 --mode pick --judge "Which design is easiest to scan?" --artifact "D:\mockups\A.png=Variant A" --artifact "D:\mockups\B.png=Variant B"
TaskBoard.exe --root "B:\GDrive\SharedWithPhone\Obsidian\jjj\Misc\Tasks" --json task review show MISC-0006
```

Supported modes: `pick` (2–4 variants), `approve` (a single new design is valid; `--previous` is optional), `rank` (`--top N`), and `markup` (single artifact). All require `--judge` plus one or more `--artifact path=label` values. Optional `--criteria`, `--reference`, `--poster`, `--implemented-by`, and `--openrig-gate`. Small files are synced under `Tasks/Reviews/ID/rN`. Media over 25 MB stays in the PC's local review cache and requires a synced poster. Videos prefer an extracted ffmpeg poster where available.

**Media presentation contract (desktop):** Each `--artifact` becomes one full-size item in a filmstrip, with a single-artifact default view, optional image-to-image compare/overlay, video playback, a dedicated interactive web viewport, and collapsible queue/feedback. For reviews of multiple screens or animation beats, supply **separate screenshots and video files as individually labeled artifacts** (and a distinct HTML artifact if the prototype must be interactive). Do **not** package all frames, clips, and text in a long HTML report and expect the reviewer to zoom/scroll through it. `--previous` is only usable for visual comparison when both old and proposed media are images; when approving a motion clip or HTML prototype, `--previous` is optional and the viewer intentionally defaults to single view unless both sides support image comparison. For artifact-intensive approvals with more than four items, use `approve` (not `pick`). Preserve aspect ratio; do not embed the images into a webpage just to make them reviewable.

For the MISC-0006 bootstrap review, two HTML options (Compact A and Balanced B) were requested on 2026-10-24 and linked to implementation task MISC-0007. Both reuse actual TaskBoard styling; the existing interactive wireframe supplies functional layout and visual details only where TaskBoard has no precedent. Human picked B (`picked`, choice `B`) in MISC-0006's official manifest on 2026-10-24. Selected-B Review UI is implemented in desktop and Android source; actual desktop before/after images and HTML reference have been submitted in MISC-0007 `mode=approve` round 1. The reviewer subsequently issued a genuine `approved` verdict for MISC-0007 in Review at 2026-10-24T00:45:28Z. It is not agent-generated. The selected-B Android OTA failed on the phone with `INSTALL_FAILED_VERIFICATION_FAILURE`, and device verification remains outstanding. TaskBoard correctly blocks Done on unfinished MISC-0006/MISC-0005 dependencies; do not bypass hard dependencies or claim a phone screenshot that is only a mockup.

Requests mark `review_status: open` and `attention: question`. Verdicts (choice, rank, coordinates/timestamps of pins, note) live in `manifest.json` and are appended to the Description timeline. Humans may resolve via the desktop Review UI or explicit `task review resolve --human-confirmed ...`; **agents must never resolve their own requests or supply `--human-confirmed`**. A review may be rejected or returned for more variants. MISC-0004 is responsible for cleanup; do not manually prune review folders.

### Review retention maintenance
`task review gc --dry-run [--all-boards]` previews due file/archive actions. `task review gc` runs a single-board sweep; `--all-boards` reads the desktop project catalog and skips unavailable boards. Deletions move into the Windows Recycle Bin and are logged in `Reviews/_gc.log`. Do not bypass the review manifest or directly delete its winning media. A hidden Windows scheduled task runs daily and at logon, while desktop TaskBoard sweeps on startup and every six hours. Use `scripts/Install-TaskBoardReviewGc.ps1 -Uninstall` to remove the background task.

## Board-wide advisories (MISC-0024)

Columns cycle through three mutually exclusive modes: **Off** (grey), **AI Ready**
(green checkmark), and **Advisory** (yellow exclamation mark). Draft, Done,
and canonical AI Ready cannot become advisories. Agent-facing task-list,
task-show, and successful task-claim JSON responses include an "advisories"
array. Read and obey that array before any implementation work on the board:
it contains temporary instructions for all working agents, not tasks to
claim or complete.

The normal --ai-ready execution queue excludes advisory cards. Moving a
notice to Draft removes it from normal agent discovery. Moving a card
directly from an advisory column to Done deletes its active Markdown file,
instead of retaining it in Done; save revisions are checked first.
Do not use advisory columns as permanent documentation.

CLI: column advisory <column> true|false, or column ready <column> true|false.
Enabling one mode disables the other. The Android build may not support this
new marker until updated.

## Desktop shortcuts and batch creation (MISC-0023/0025/0027)

At board level, Arrow keys or WASD navigate cards (Space opens the selected
card); Ctrl+Space/Ctrl+T creates a task, Ctrl+R opens Review, Ctrl+Q starts
the User queue, Ctrl+K focuses search, Ctrl+Z undoes the previous board
action, and Ctrl+V on empty board UI opens Task Dump prefilled from the
plain-text clipboard. Inputs retain ordinary paste and typing behavior.

In the editor, Ctrl+Tab toggles Task/Epic, Ctrl+U toggles For user, Enter
saves except in multiline inputs, Ctrl+S saves, and Ctrl+Enter marks an
existing non-epic task Done. Save and Done positions have been swapped,
with Save at the right edge. Task Dump accepts one item per newline,
allows per-item Priority toggles, and creates separate tasks with
overflow in Description; successfully created items survive a partial
batch failure.

## Unified feedback inbox (MISC-0028)

The desktop toolbar now has one **Inbox** entry, combining live Visual Review requests and
**For user** task cards; the board remains visible behind an overlay. Visual Review preserves
its pin/variant workflow. A For user card opens a compact answer form with Note to AI,
For user toggle, acceptance criteria and Save/Skip/Snooze; **Full task editor** exposes
all metadata when necessary. **Ctrl+R** opens Inbox; **Ctrl+Q** starts the direct For user
queue. Review verdicts, design approval and gate releases remain human-only.

For an Animation Lab live `url` artifact on loopback with `embed=1`, Review mode can
send `{source:'taskboard-review',type:'time'|'play'|'pause',t?}` to the iframe and record
real playback timestamps from `animation-lab-embed` messages when placing pins. Use
`anim.py embed-url` for the stable asset-SHA-pinned URL or `anim.py review-snapshot`
for a standalone immutable HTML fallback. The direct 127.0.0.1:5178 URL is preferred:
the Gateway launch link may not preserve query arguments.

OpenRig human gates may be registered through `task review request --openrig-gate <qitem>`.
Registration only adds a display link; it grants no authority. The installed OpenRig daemon
has a separate typed gate/receipt API, but **a genuine T3 Desktop MAIN human action and
verified wake receipt are still required**. Never infer a gate has been released merely
from a Review manifest or a CLI request result.

Android TaskBoard now uses a locally scoped desktop-style dark blue-slate palette, colored
priority/epic/For user badges, improved list and board columns, compact tools and long-press
card actions. Its APK builds, but an OTA reaching `installer_started` alone is not
installation or physical-device screenshot proof.

## Rolling release commands (MISC-0040)
On release-enabled boards, a worker's existing `task complete ID` / `task move ID done` automatically enrolls eligible Done implementations exactly once into the current collecting `kind: release` record. The board's Releases panel handles deploy bookkeeping; it does **not** restart or deploy T3. `release list|show` inspect membership, stages and evidence; `release source` records exact source and integrated revisions; `release prepare` snapshots an immutable attempt; `release outcome` records evidence and generates focused corrective tasks on failure. See [RELEASE_WORKFLOW.md](RELEASE_WORKFLOW.md) for complete examples. Board collection defaults off and requires `release enable true`. `release reconcile --historical` intentionally enrolls prior Done work and must not be invoked on live data without an explicit request.

Blocked is not the default for completed implementation waiting on deployment. New agent CLI blocking actions first return `blocked_confirmation_required` and a ten-minute one-use challenge plus self-review message; a genuine inability to proceed can be confirmed with the rejection-provided hidden flag. Human GUI operations remain direct. Existing OMCP `taskboard({arguments,root})` tokenization is unchanged.

Legacy blocked migration is intentionally not automatic. Use `release blocked-candidates` for an evidence-only review. `release migrate-blocked <ID> --validated --evidence "<source test proof>"` requires explicit authorization and source confirmation before it moves a single legacy blocker to Done.
