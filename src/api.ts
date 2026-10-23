import { invoke } from "@tauri-apps/api/core";
import type { BoardConfig, BoardSnapshot, Task, TaskDraft } from "./types";

export type ProjectTab = { id: string; name: string; path: string; folders: string[] };
export type ProjectCatalog = { schema: number; activeId: string; projects: ProjectTab[] };
export const listProjects = () => invoke<ProjectCatalog>("list_projects");
export const addProject = (name: string, folders: string[]) =>
  invoke<ProjectCatalog>("add_project", { name, folders });
export const setProjectFolders = (id: string, folders: string[]) =>
  invoke<ProjectCatalog>("set_project_folders", { id, folders });
export const activateProject = (id: string) =>
  invoke<ProjectCatalog>("activate_project", { id });
export const removeProject = (id: string) =>
  invoke<ProjectCatalog>("remove_project", { id });

export const getDefaultBoardPath = () =>
  invoke<string>("default_board_path");

export const loadBoard = (path?: string) =>
  invoke<BoardSnapshot>("load_board", { path });

export const createTask = (draft: TaskDraft, path?: string) =>
  invoke<Task>("create_task", { path, draft });

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

export const showMainWindow = () =>
  invoke<void>("show_main_window");
