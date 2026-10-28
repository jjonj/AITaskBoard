import { invoke } from "@tauri-apps/api/core";
import type { BoardConfig, BoardSnapshot, Task, TaskDraft } from "./types";

export type ProjectTab = { id: string; name: string; path: string; folders: string[] };
export type ProjectCatalog = { schema: number; activeId: string; projects: ProjectTab[] };
export const listProjects = () => invoke<ProjectCatalog>("list_projects");
export const addProject = (name: string, folders: string[], taskPrefix: string, epicPrefix: string) =>
  invoke<ProjectCatalog>("add_project", { name, folders, taskPrefix, epicPrefix });
export const setProjectFolders = (id: string, folders: string[]) =>
  invoke<ProjectCatalog>("set_project_folders", { id, folders });
export const activateProject = (id: string) =>
  invoke<ProjectCatalog>("activate_project", { id });
export const reorderProjects = (ids: string[]) =>
  invoke<ProjectCatalog>("reorder_projects", { ids });
export const removeProject = (id: string) =>
  invoke<ProjectCatalog>("remove_project", { id });
export const restoreProject = (project: ProjectTab, index: number) =>
  invoke<ProjectCatalog>("restore_project", { project, index });
export const moveTaskBetweenProjects = (
  sourceProjectId: string, destinationProjectId: string, taskId: string, expectedRevision: string,
) => invoke<Task>("move_task_between_projects", { sourceProjectId, destinationProjectId, taskId, expectedRevision });

export const getDefaultBoardPath = () =>
  invoke<string>("default_board_path");

export const loadBoard = (path?: string) =>
  invoke<BoardSnapshot>("load_board", { path });

export type UserQueueProjectIds = { path: string; eligibleIds: string[]; error: string | null };
export const userQueueProjectIds = (paths: string[]) =>
  invoke<UserQueueProjectIds[]>("user_queue_project_ids", { paths });

export type BoardChangeToken = { path: string; token: string | null };
export const boardChangeTokens = (paths: string[]) =>
  invoke<BoardChangeToken[]>("board_change_tokens", { paths });

export type TaskIdReservation = { id: string; token: string };
export const reserveTaskId = (kind: TaskDraft["kind"], path?: string) =>
  invoke<TaskIdReservation>("reserve_task_id", { path, kind });
export const releaseTaskId = (reservation: TaskIdReservation, path?: string) =>
  invoke<void>("release_task_id", { path, reservation });

export const createTask = (draft: TaskDraft, path?: string) =>
  invoke<Task>("create_task", { path, draft });

export type ExecutionMode = "backlog" | "queue";
export type ExecutionIntent = {
  mode: ExecutionMode;
  queue_request: string | null;
  queue_attempt: string | null;
  queue_receipt: string | null;
  queue_retired_receipt: string | null;
  start_now_request: string | null;
  start_now_receipt: string | null;
  revision: string;
};
export const taskExecutionIntent = (id: string, path?: string) =>
  invoke<ExecutionIntent>("task_execution_intent", { path, id });
export const taskSetExecutionMode = (id: string, expectedRevision: string, mode: ExecutionMode, path?: string) =>
  invoke<ExecutionIntent>("task_set_execution_mode", { path, id, expectedRevision, mode });
export const taskRequestStartNow = (id: string, expectedRevision: string, path?: string) =>
  invoke<ExecutionIntent>("task_request_start_now", { path, id, expectedRevision });

export type QueueSyncResult = {
  status: "not_queued" | "unavailable" | "registered" | "terminal" | "running" | "canceled" | "unknown" | "rejected" | "conflict";
  qitemId: string | null;
  reason: string | null;
};
/** Native authenticated one-shot bridge; never exposes OpenRig bearer to renderer. */
export const taskSyncQueue = (id: string, path?: string) =>
  invoke<QueueSyncResult>("task_sync_queue", { path, id });

export type QueueOverviewItem = {
  qitemId: string; taskId: string | null; summary: string | null;
  state: string; priority: "routine" | "urgent" | "critical";
  createdAt: string; updatedAt: string; claimedAt: string | null;
  pickup: unknown | null; waiting: unknown | null; t3SessionId: string | null;
};
export type QueueOverview = {
  schema: "openrig-taskboard-queue-overview/v1";
  observedAt: string; environmentId: "pc"; boardRoot: string;
  activeCount: number | null; hasMore: boolean; limit: number;
  items: QueueOverviewItem[];
};
/** Read-only canonical OpenRig board queue. Credentials never reach the renderer. */
export const taskQueueOverview = (path?: string) =>
  invoke<QueueOverview>("task_queue_overview", { path });

export type QueueCapacity = {
  observedAt: string; limit: number; working: number; reserved: number;
  unknown: number; used: number; available: number; admissible: boolean;
  reason: string | null;
};
export const taskQueueCapacity = () =>
  invoke<QueueCapacity>("task_queue_capacity");
export const taskLiveWithMe = (id: string, expectedRevision: string, withMe: boolean, path?: string) =>
  invoke<Task>("task_live_with_me", { path, id, expectedRevision, withMe });

export const saveTaskImage = (taskId: string, data: string, mime: string, path?: string) =>
  invoke<string>("save_task_image", { path, taskId, data, mime });
export const readTaskImage = (relativePath: string, path?: string) =>
  invoke<string>("read_task_image", { path, relativePath });

export const updateTask = (task: Task, path?: string) =>
  invoke<Task>("update_task", { path, task });

export const archiveTask = (id: string, path?: string) =>
  invoke<Task>("archive_task", { path, id });

export const archiveColumn = (columnId: string, expected: Array<[string, string]>, path?: string) =>
  invoke<BoardSnapshot>("archive_column", { columnId, expected, path });

export const archiveColumnOlderThan = (
  columnId: string, cutoffIso: string, expected: Array<[string, string]>, path?: string,
) => invoke<BoardSnapshot>("archive_column_older_than", { columnId, cutoffIso, expected, path });

export const saveBoardConfig = (config: BoardConfig, path?: string) =>
  invoke<BoardConfig>("save_board_config", { path, config });

export const restoreSnapshot = (
  before: BoardSnapshot,
  after: BoardSnapshot,
  path?: string,
) => invoke<BoardSnapshot>("restore_snapshot", { path, before, after });

export const highlightInFomni = (file: string) =>
  invoke<void>("highlight_in_fomni", { file });

export const openInObsidian = (file: string) =>
  invoke<void>("open_in_obsidian", { file });

export const openClaimedT3Session = (claimedBy: string) =>
  invoke<{ sessionId: string; reopened: boolean }>("open_claimed_t3_session", { claimedBy });
/** Open the board-qualified task's current T3 session, or request one safely. */
export const openTaskSession = (id: string, path?: string) =>
  invoke<Record<string, unknown>>("open_task_session", { path, id });

export const showMainWindow = () =>
  invoke<void>("show_main_window");

// Native release bookkeeping; never deploys or restarts an application.
export const listReleases = (path?: string) => invoke<import("./releaseTypes").ReleaseSummary>("list_releases", { path });
export const enableReleases = (enabled: boolean, path?: string) => invoke<import("./releaseTypes").ReleaseSummary>("enable_releases", { enabled, path });
export const reconcileReleases = (historical: boolean, path?: string) => invoke<import("./releaseTypes").ReleaseSummary>("reconcile_releases", { historical, path });
export const editRelease = (action: string, release: string, task?: string, source?: import("./releaseTypes").SourceRef, evidence?: string, path?: string) => invoke<import("./releaseTypes").ReleaseRecord>("edit_release", { action, release, task, source, evidence, path });
export const prepareRelease = (release: string, evidence: string, path?: string) => invoke<import("./releaseTypes").ReleaseRecord>("prepare_release", { release, evidence, path });
export const outcomeRelease = (release: string, attempt: number, status: string, evidence: string, path?: string) => invoke<import("./releaseTypes").ReleaseRecord>("outcome_release", { release, attempt, status, evidence, path });

export const verifyReleaseMember = (release: string, attempt: number, task: string, passed: boolean, evidence: string, path?: string) =>
  invoke<import("./releaseTypes").ReleaseRecord>("verify_release_member", { release, attempt, task, passed, evidence, path });
export const newRelease = (path?: string) => invoke<import("./releaseTypes").ReleaseRecord>("new_release", { path });
