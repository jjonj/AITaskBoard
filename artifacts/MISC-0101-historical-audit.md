# MISC-0101 — Historical TaskBoard audit, 2026-11-01

**Scope:** Registered project boards in `%APPDATA%/TaskBoard/projects.json`; 1,027 active Markdown task cards across 11 registered boards. The exact per-board dry-run findings are retained in `artifacts/misc0101-audit-dry-run.json`; rerun with `python scripts/audit_historical_tasks.py`.

## Findings and dispositions

- **Claims outside In Progress:** 42 nonterminal claims are attached to blocked/backlog task cards, primarily old Wartribes and t3code work. These claims may still represent valid worker/session provenance. No mass claim reset was performed, because missing/errored/unloaded T3 sessions are UNKNOWN, not proof of abandonment. The new core releases the claim on a *subsequent explicit human move* out of In Progress; the recovery coordinator requires authoritative permanently-gone evidence and grace.
- **Blocked with unfinished dependencies:** Eight tasks (seven Wartribes, one Misc) have a genuine blocker or blocked state *as well as* prerequisite waits. Do not delete formal dependencies or a blocker merely because they coexist. These remain unchanged; the new stale technical blocker review excludes tasks awaiting prerequisites.
- **Blocked without explicit reason:** T3-0018 is a legacy blocked card without a structured blocked reason. It remains for owner review rather than fabricating one.
- **Unclaimed In Progress:** WTE-0002/WTE-0003, MISCE-0002/MISCE-0003 and OTFE-0006 are epics, which are not executable/claimable. These are normal and need no correction.
- **Other boards:** Altraea, Citizen, Filler-backlog, MonkuNoMori and TaskBoard yielded no suspicious task metadata; boards with zero active tasks remain listed rather than silently omitted.

## Safe normalization and verification

- Read-only audit across every registered board. No task body, historical answer, working-tree reference, blocker, claim, or dependency was rewritten; no possible owner was evicted.
- Future human changes now clear obsolete claimant when explicitly moving a task out of In Progress. New session lineage and handoff APIs preserve prior session references across reassignment.
- Repeat the audit after owners decide the flagged legacy cards' disposition. The output is a *candidate list*, not authorization for speculative automated metadata rewrites.

**Outcome:** Zero historical mutations justified by authoritative evidence. The existing records are deliberately preserved. The implemented TaskBoard policy prevents the same classes of future metadata drift without silently reassigning prior work.