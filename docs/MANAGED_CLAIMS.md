# TaskBoard managed claim contract (OW-0011)

Managed claims are **TaskBoard ownership lifetimes**, not OpenRig claimant generations.
They are deliberately independent of OpenRig queue attempt/claimant IDs.

CLI with `--root <board-path>`:

1. `task claim <task-id> --agent <display-name> --managed` returns the ordinary task JSON with an additional `claim_token` (UUID). Persist that receipt on the exact managed assignment.
2. `task release <task-id> --agent <display-name> --claim-token <token>` releases only that instance under the board lock. Repeating the release after successful release returns the unclaimed task without rewriting it, provided another claim has not begun. A stale release after reacquisition returns `claim_instance_mismatch` (exit 3), never drops the new owner.
3. `task complete-managed <task-id> --agent <display-name> --claim-token <token> --expected-revision <revision>` accepts a result only if both the current claim token and full Markdown revision match. A changed description/criteria gives `write_conflict` (exit 3); re-read and re-evaluate before any acceptance.

The token is stored as frontmatter `claim_token`, next to the existing human-readable `claimed_by`. Successful release clears the active token and retains `last_released_claim_token` solely for idempotent release reconciliation. Unrecognized frontmatter fields are retained. Do not infer a token from owner name, content hash, or OpenRig generation.

Legacy `task claim/release --agent` remains available for **manual/name-only** claims. A managed claim cannot be released through legacy name-only release. An existing legacy claim cannot be upgraded to managed merely by matching the owner name: reconcile explicitly. Do not automate a release or repair if the token was never recorded. Unknown delivery/claim outcomes must be reconciled, not blindly repeated.

The OpenRig managed task-claim adapter must preserve the TaskBoard `claim_token` independently from its own generation on future qualified TaskBoard assignments; as of OW-0011's source change, the current OpenRig queue implementation does not itself call this TaskBoard claim CLI. The adapter/cutover belongs to Omniwork's forthcoming managed assignment integration.
