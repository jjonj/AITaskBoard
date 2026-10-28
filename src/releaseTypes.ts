export type ReleaseStage = "awaiting_integration" | "ready" | "deployed" | "verified" | "verification_failed" | "unknown";
export type SourceRef = { repository: string; commit: string; integrated_ref?: string | null };
export type ReleaseMember = { task_id: string; stage: ReleaseStage; sources: SourceRef[]; evidence: string[]; attempt_ids: number[] };
export type ReleaseAttempt = { number: number; created: string; status: string; members: ReleaseMember[]; manifest: SourceRef[]; run_id?: string | null; evidence: string[]; repair_task_id?: string | null };
export type ReleaseRecord = { schema: number; kind: "release"; id: string; status: string; created: string; members: ReleaseMember[]; attempts: ReleaseAttempt[]; excluded: string[] };
export type ReleaseSummary = { releases: ReleaseRecord[]; integration_needed: number; ready: number; verified: number; deployed: number; verification_failed: number };
