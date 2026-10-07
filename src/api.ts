import { invoke } from "@tauri-apps/api/core";
import type { BoardConfig, BoardSnapshot, Task, TaskDraft } from "./types";

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
