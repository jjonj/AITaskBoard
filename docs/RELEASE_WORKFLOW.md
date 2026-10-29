# Rolling Releases and Agent Blocked Self-Review (MISC-0040)

## Scope and safety
Releases are **opt-in per board** and live separately in `<board>/Releases/REL-####.md` (first-class `kind: release`). Existing `Active/` and `Archive/` Kanban tasks and CLI command arguments remain supported. Nothing builds, deploys, restarts T3/TaskBoard/OpenRig, stops agents or freezes source. A release is bookkeeping, not proof that source has reached production.

The Tauri toolbar has a **Releases** entry with its own panel, counters, members, source references, immutable attempt history, manual include/exclude/move and evidence controls. The board setting is controlled there or via `release enable true|false`. Default is OFF. On first enablement, existing Done tasks are **not** enrolled unless `release reconcile --historical` is invoked explicitly (or **Import historical Done** is confirmed in UI). The feature is independent of the original task prefix, and release IDs use `REL-####` in their own folder.

## Agent workflow
Agents **keep using their existing** `task complete ID` or `task move ID done` commands after their implementation and source checks. The shared store creates the first collecting release lazily on eligible Done, idempotently enrolling each member once. Completion does not mean deployed. On a release-enabled board, ordinary `kind: task` cards are deployable by default; `task update ID --deployable false` opts an individual task out (the desktop task editor has the equivalent checkbox). Epics, For user cards and nondeployable cards never enroll. No worker must manage releases.

Release membership and metadata are reconciled under the same cross-process board lock on normal core loads, UI refreshes, and CLI writes. This also catches direct Markdown/sync edits to task status, removing reopened tasks from collecting membership unless they are already in an immutable deployment attempt. Historical enrollment requires explicit action. Existing completed tasks are never reopened merely because deployment or verification failed.

## Release CLI (also usable unchanged through OMCP's tokenized `taskboard({arguments,root})`)
Use `--root "<board>" --json` for machine-readable output. Commands operate only on metadata:

```text
release enable true
release list
release show REL-0001
release reconcile
release reconcile --historical
release include REL-0001 T3-0046
release exclude REL-0001 T3-0046
release move REL-0001 T3-0046 REL-0002
release source REL-0001 T3-0046 --repo "T3 Code" --commit "<exact commit>" --integrated-ref "<exact deployed build ref>" --evidence "<integration evidence>"
release source REL-0001 T3-0046 --repo "OpenRig" --commit "<commit>" --integrated-ref "<exact integrated ref>"
release prepare REL-0001 --evidence "manifest preflight"
release outcome REL-0001 1 deployed --evidence "operator-provided deployment evidence"
release outcome REL-0001 1 verified --evidence "actual acceptance verification"
```

`--integrated-ref` is deliberately omitted for detached/unmerged commits: such members remain **awaiting integration** and are not selectable for a deployment attempt. Each attempt snapshots the exact multi-repository members and integrated references. A running attempt does **not** prevent further Done tasks; they remain collecting for the next attempt. A build failure (`build_failed`) or verification failure (`verification_failed`) records evidence and creates a focused linked repair task, preserving original implementation Done status. A new attempt can be prepared after the failed attempt. No external deployment is performed.

## Agent Blocked transition: deliberate two steps
For CLI/OMCP agents, **all** first transitions to Blocked through `task move ID blocked`, `task update ID --add-attention blocked`, or `task create --column blocked` / `--attention blocked` require a concrete `--blocked-reason "..."`. The first request rejects with JSON `error.code="blocked_confirmation_required"`, explanation and an opaque single-use 10-minute token. It asks the agent to reconsider:
* Source complete and tested, waiting deployment/restart/live validation? Move to **Done**, let Releases handle it.
* Can more implementation work continue? Keep **In Progress**.
* Genuine inability to continue implementation? Retry the **same** action with identical reason, adding the rejection-provided hidden `--confirm-genuine-block TOKEN` flag. The flag is intentionally not documented in normal CLI help.

The token is board/task/action/reason bound, expiring, and consumed on success. Arbitrary, stale, wrong-action or replay tokens fail. Agents judge genuine blockers themselves; the gate is a prompt for self-review rather than human approval or keyword policing. Direct human GUI edits remain exempt. `--blocked-reason` and `blocked_reason` frontmatter explain the blocker; unblocking clears the metadata.

## Safety and migration
No existing Done/Blocked tasks are changed as part of deployment. **Never run an unsolicited historical migration**. Release collection is off on existing boards until enabled. Updating app binaries is a separate explicitly authorized operator action. The original Markdown remains canonical, with the new Release files separately persisted; cross-process locks prevent duplicate enrollment. Any tests of migration must run on disposable temporary boards, not live project boards.

## Current integration boundary
2026-10-25 release: source reconciled in isolated worktree commit db2baaf and grafted into the shared main working tree with a per-file foreign-change hash check and local backups. TaskBoard desktop, standalone agent CLI, and OMCP Tools sidecar were rebuilt and restarted; an actual native Releases panel and disposable-board OMCP blocked confirmation/Done auto-enrollment were observed. The main checkout still contains other owners' uncommitted work, so this is a **deployed working-tree integration, not a clean main Git commit**. Existing Done/Blocked were not migrated automatically. Release collection was explicitly enabled for T3Code and Misc only; other boards remain opt-in.

## Explicit review and migration of legacy Blocked tasks
`release blocked-candidates` lists current Blocked tasks and their actual reason without changing anything or guessing source completeness. Only after independent source validation may an operator explicitly issue
`release migrate-blocked T3-0056 --validated --evidence "commit <SHA> / tests <results>"`.
This moves that one executable implementation from Blocked to Done, records the supplied evidence on task and release, and enrolls it through normal completion. **Never run this against existing T3 tasks without the user's explicit migration authorization.** No routine worker should need it.

`release new` (or **New release** in the UI) is an optional **explicit human/operator** override to open a subsequent collecting release so an individual member can be moved into it. Existing members and immutable in-flight attempts are preserved, and ongoing agents keep completing work without any pause. Normal workers **never need to create a release manually**; the first automatically collects lazily.

## 2026-10-26: executable source-bound release bridge (MISC-0049)

**Source only; NOT deployed and NOT wired to a single canonical T3 restart entrypoint.** The real October 25 T3 restarts were made by ad-hoc external release workers, not by TaskBoard or one standardized runner. Their restart logs never invoked \`release prepare\` or \`release outcome\`; hence REL-0001 has valid Done members but no attempts. Do not invent an earlier preflight or backdate deployed state. Installing today's TaskBoard/sidecar requires a separately authorized coordinated release.

TaskBoard now supports \`release prepare REL-#### --run-id stable-run-id --evidence "..."\`, \`release outcome REL-#### N <deployed|build_failed|unknown> --run-id stable-run-id --evidence "..."\` and \`release verify REL-#### N TASK-ID --status <verified|verification_failed> --evidence "..."\`. Prepare is idempotent per run ID, attempts freeze exact task+source state, unknown results can reconcile after obtaining independently verified receipts, and feature verification is per immutable member, not implied by opening an Electron window. \`release source\` now deduplicates identical provenance/evidence.

**Opt-in build/restart integration**: Use \`scripts/release_deployment_bridge.py\` as the source/handoff boundary from an already authorized application release runner, never as a restart authorization.

1. An integrator lists existing eligible release members (Done tasks) and writes a JSON manifest in the board's TaskBoard-approved artifacts folder with \`board_root\`, \`release_id\`, stable globally unique \`run_id\`, and explicit \`inputs\`: \`task\`, \`repository\`, \`repo_path\`, \`source_commit\` (exact ancestor Git SHA or \`working-tree:<checkout_digest>\`), and \`file_hashes\` (map of paths relative to repo to SHA256). Include only files whose contents were inspected, tested and actually incorporated in the build tree. No guess from a Done card or chat report. The checkout fingerprint includes HEAD, all Git tracked diffs and untracked path names. An entirely untracked file must have its own hash.
2. Existing authorized release script calls \`python scripts/release_deployment_bridge.py preflight <manifest.json>\` before touching build/restart. The bridge checks each file, source ancestry/worktree fingerprint, Done membership and exact release ID. It records source refs and an immutable prepared attempt; it fails closed if any other member would accidentally be frozen. One \`<manifest>.release-receipt.json\` persists the exact attempt/inputs, idempotent on retries. **A successful preflight is not permission to build, nor proof of installation.**
3. The **same already-authorized build/restart workflow** performs its existing build/activation and captures its own real build log, artifact and process/activation receipt. Then \`python scripts/release_deployment_bridge.py outcome <manifest.json> --status deployed --artifact <installed artifact> --activation-receipt <observed startup evidence> --log <build log>\`. Absent or ambiguous runtime evidence: use \`--status unknown --log <diagnostic log>\`; failed compilation use \`--status build_failed --log <real log>\`. The bridge hashes objective files and records the result; do not supply a fabricated receipt. Recording a receipt proves only the operator-supplied evidence exists, not that the agent independently watched the app start.
4. The release owner verifies **individual** live task behavior and uses \`release verify REL-#### N TASK-ID --status verified --evidence <specific observed behavior>\` or \`verification_failed\`; neither ordinary source completion nor deployment marks features verified. Failures create a linked repair card without reopening implementation Done.

**Canonical T3 release hookup (SOURCE ONLY, not live-activated at writing):** The checked-in T3 restart runner D:/SSDProjects/Tools/t3code/compile_and_launch_t3.py now has optional TaskBoard bookkeeping in its existing detached worker. It validates an exact source manifest before compilation, and on actual observed activation records executable hash, matching process IDs, restart log, and activation receipt. It records cancellation or ambiguous errors as UNKNOWN, not deployed. The manifest is detected automatically when an integrator places it at t3code/.t3/taskboard-release-manifest.json; alternatively pass --taskboard-manifest MANIFEST to the existing T3 build/restart entrypoint. Operators must still authorize restarts. T3 --dry-run does not change release state. No T3 build or restart was run while wiring this.

**Source-discovery helper (2026-10-29):** `python scripts/release_deployment_bridge.py draft --root "<T3Code Tasks>" --release REL-0001 --repo "D:/SSDProjects/Tools/t3code" --repository "T3 Code" --output "<new scratch-path>/t3-release-manifest.json"` examines each Done member's AI notes for one unambiguous `committed <SHA>` / `commit <SHA>` reference. It verifies the referenced commit is in the **current** checkout history and hashes its current affected files. Missing, ambiguous, deleted and detached sources are skipped, reported, and **not promoted**. The draft is only a review candidate: the integrator MUST inspect task-to-commit attribution, uncommitted/foreign changes and build scope before providing `--taskboard-manifest <reviewed path>` to the already authorized runner. Multi-repo input rows may be added by the integrator and are checked by the same bridge. Draft never mutates release state and never triggers deployment; output is create-only to preserve attempt lineage.

**Run-ID reuse guard:** Once `outcome` records `deployed`, `build_failed`, or `unknown`, calling `preflight` on that same manifest fails with instructions to create a NEW manifest/run ID for the NEXT build. `outcome` can independently reconcile an UNKNOWN with actual later activation evidence; conflicting terminal outcomes are rejected. `deployed` also verifies the current sources still match the frozen preflight. This prevents a later build from silently borrowing a prior attempt or overwriting its provenance.

**UI reloads vs full restart:** The default T3 compile-and-reload path now accepts the same reviewed manifest. It records an immutable preflight before building and, after invoking Ctrl+R, honestly records `unknown` with the build log: sending a reload keystroke does not prove the renderer loaded or server/Electron code became active. The full restart path requires a matching process and executable SHA for `deployed`; neither path marks individual features verified automatically. No manifest means no attempt, and the runner announces this prominently rather than inventing provenance. Operators must not treat `unknown` as deployed.

**Remaining acceptance:** A source integrator must review and use a real reconciled manifest; merely finding a SHA in notes is not acceptance. Existing REL-0001 members without provenance remain awaiting integration. Only an actual later authorized T3 release can prove the hook and user-visible feature acceptance end-to-end. Other ad-hoc release entrypoints still need the bridge. No historical deployment or feature verification is inferred from old logs. The bridge does not run a build, restart, install, or spawn agent. It does not run ChatGPT scheduled tasks.

**Observed checks:** One disposable board completed an implementation and enrolled exactly once, preflight called twice with identical run ID yielded exactly one immutable attempt, later \`unknown\` recorded without deployment claims; source hash verification ran against a real TaskBoard checkout. Focused Rust tests passed for one immutable attempt, a post-preflight late Done, unknown→deployed reconciliation, per-member verification and recurring cadence. Current Misc REL-0001 and T3 REL-0001 remain untouched and awaiting legitimate integration evidence.
