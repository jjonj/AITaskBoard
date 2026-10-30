import { useEffect, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { activateProject, addProject, boardChangeTokens, listProjects, loadBoard, moveTaskBetweenProjects, removeProject, reorderProjects, restoreProject, saveBoardConfig, setProjectFolders, showMainWindow, userQueueProjectIds } from "./api";
import type { ProjectCatalog, ProjectTab } from "./api";
import type { BoardConfig } from "./types";

type Props = {
  Board: (props: { project: ProjectTab; onInteractionChange: (locked: boolean) => void; requestedReviewId?: string; requestedTask?: {id: string; requestKey: number} }) => React.ReactNode;
};
type DialogMode = "create" | "folders" | null;
const OBSIDIAN_ROOT = "B:\\GDrive\\SharedWithPhone\\Obsidian\\jjj";

// Suggest an editable namespace for new projects; never infer a change for an
// existing _board.md. E.g. T3Code -> T3, Altraea -> ALT, Wartribes -> WT.
const suggestedPrefix = (name: string) => {
  if (name.trim().toLowerCase() === "wartribes") return "WT";
  const stem = name.trim().match(/^[A-Za-z]+[0-9]*/)?.[0]?.toUpperCase() ?? "";
  return (stem.length >= 2 ? stem.slice(0, 3) : "PRJ");
};
const validPrefixes = (task: string, epic: string) =>
  /^[A-Z][A-Z0-9]{1,11}$/.test(task) && /^[A-Z][A-Z0-9]{1,11}$/.test(epic) && task !== epic;

const suggestedName = (path: string) => {
  const parts = path.trim().replace(/[\\/]+$/, "").split(/[\\/]/).filter(Boolean);
  if (parts[parts.length - 1]?.toLowerCase() === "tasks") parts.pop();
  return parts[parts.length - 1] ?? "";
};

export function ProjectTabs({ Board }: Props) {
  const [catalog, setCatalog] = useState<ProjectCatalog | null>(null);
  const [startupReview, setStartupReview] = useState<{ path: string; taskId: string } | null>(null);
  const [startupTask, setStartupTask] = useState<{ path: string; taskId: string; requestKey: number } | null>(null);
  useEffect(() => {
    void invoke<{ path: string; taskId: string } | null>("startup_task")
      .then((request) => { if (request) setStartupTask({ ...request, requestKey: Date.now() }); })
      .catch(() => undefined);
  }, []);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{path: string; taskId: string}>("taskboard-open-task", (event) => {
      if (event.payload?.path && event.payload?.taskId)
        setStartupTask({ ...event.payload, requestKey: Date.now() });
    }).then((remove) => { if (disposed) remove(); else unlisten = remove; })
      .catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  }, []);
  const appliedStartupReview = useRef(false);
  useEffect(() => { void invoke<{ path: string; taskId: string } | null>("startup_review").then(setStartupReview).catch(() => undefined); }, []);
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void listen<{ path: string; taskId: string }>("taskboard-open-review", (event) => {
      if (!event.payload?.path || !event.payload?.taskId) return;
      appliedStartupReview.current = false;
      setStartupReview(event.payload);
    }).then((remove) => { if (disposed) remove(); else unlisten = remove; }).catch(() => undefined);
    return () => { disposed = true; unlisten?.(); };
  }, []);
  useEffect(() => {
    if (!startupReview || !catalog || appliedStartupReview.current) return;
    const same = (path: string) => path.replace(/\\/g, "/").replace(/\/+$/u, "").toLowerCase();
    const project = catalog.projects.find((item) => same(item.path) === same(startupReview.path));
    if (!project) return;
    appliedStartupReview.current = true;
    if (project.id !== catalog.activeId) setCatalog((current) => current ? { ...current, activeId: project.id } : current);
  }, [startupReview, catalog?.activeId, catalog?.projects]);
  useEffect(() => {
    if (!startupTask || !catalog) return;
    const same = (path: string) => path.replace(/\\/g, "/").replace(/\/+$/u, "").toLowerCase();
    const project = catalog.projects.find((item) => same(item.path) === same(startupTask.path));
    if (project && project.id !== catalog.activeId)
      setCatalog((current) => current ? { ...current, activeId: project.id } : current);
  }, [startupTask, catalog?.activeId, catalog?.projects]);
  const [locked, setLocked] = useState(false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [catalogError, setCatalogError] = useState("");
  const catalogLoaded = useRef(false);
  const [dialogMode, setDialogMode] = useState<DialogMode>(null);
  const [editingId, setEditingId] = useState("");
  const [confirmCloseTab, setConfirmCloseTab] = useState(false);
  const [undoProjectAction, setUndoProjectAction] = useState<
    { kind: "close"; project: ProjectTab; index: number } |
    { kind: "reorder"; ids: string[] } | null
  >(null);
  const [name, setName] = useState("");
  const [nameEdited, setNameEdited] = useState(false);
  const [folders, setFolders] = useState<string[]>([""]);
  const [taskPrefix, setTaskPrefix] = useState("WT");
  const [epicPrefix, setEpicPrefix] = useState("WTE");
  const [prefixEdited, setPrefixEdited] = useState(false);
  const [editingBoardConfig, setEditingBoardConfig] = useState<BoardConfig | null>(null);
  const recentProjects = useRef<string[]>([]);
  const [switchHint, setSwitchHint] = useState<string | null>(null);
  const switchHintTimer = useRef<number | null>(null);
  const draggedTab = useRef<string | null>(null);
  const [dropTab, setDropTab] = useState<string | null>(null);
  const [changedProjects, setChangedProjects] = useState<Record<string, boolean>>({});
  const [queueIdsByPath, setQueueIdsByPath] = useState<Record<string, string[]>>({});
  const knownBoardTokens = useRef<Record<string, string>>({});
  const manualRefreshPending = useRef(false);
  const [refreshingAll, setRefreshingAll] = useState(false);

  useEffect(() => {
    if (working || locked || dialogMode) return;
    let mounted = true;
    let pending = false;
    const refresh = async () => {
      if (pending) return;
      pending = true;
      try {
        const value = await listProjects();
        if (!mounted) return;
        setCatalog((current) => {
          // Another window may have selected a different project. Keep this
          // window on its current board unless that tab was actually removed.
          const next = current && value.projects.some((project) => project.id === current.activeId)
            ? { ...value, activeId: current.activeId } : value;
          return JSON.stringify(current) === JSON.stringify(next) ? current : next;
        });
        setCatalogError("");
        catalogLoaded.current = true;
      } catch (err: unknown) {
        if (mounted) {
          setCatalogError(String(err));
          if (!catalogLoaded.current) {
            catalogLoaded.current = true;
            void showMainWindow().catch(() => undefined);
          }
        }
      } finally {
        pending = false;
      }
    };
    void refresh();
    const interval = window.setInterval(() => void refresh(), 2000);
    window.addEventListener("focus", refresh);
    return () => {
      mounted = false;
      window.clearInterval(interval);
      window.removeEventListener("focus", refresh);
    };
  }, [working, locked, dialogMode]);

  // Agents and CLI tools write Markdown directly. A cheap metadata scan covers
  // every project, not just the mounted tab. A changed inactive board gets a
  // visible indicator; opening it always loads the latest snapshot.
  useEffect(() => {
    if (!catalog?.projects.length) return;
    let cancelled = false;
    let pending = false;
    const paths = catalog.projects.map((project) => project.path);
    const scan = async () => {
      if (pending) return;
      pending = true;
      try {
        const stamps = await boardChangeTokens(paths);
        if (cancelled) return;
        const changedPaths = new Set<string>();
        for (const { path, token } of stamps) {
          if (!token) continue; // Missing/offline boards must not clear known state.
          const old = knownBoardTokens.current[path];
          if (old && old !== token) changedPaths.add(path);
          knownBoardTokens.current[path] = token;
        }
        if (changedPaths.size) {
          setChangedProjects((current) => {
            const next = { ...current };
            for (const project of catalog.projects) {
              if (changedPaths.has(project.path)) next[project.id] = true;
            }
            return next;
          });
        }
      } catch { /* Cloud-mounted folders can be temporarily unavailable. Retry. */ }
      finally { pending = false; }
    };
    void scan();
    const timer = window.setInterval(() => void scan(), 8000);
    window.addEventListener("focus", scan);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", scan);
    };
  }, [catalog?.projects]);

  // The human-inbox badge is derived from the same eligibility criteria as the
  // board Inbox, not the number of cards tagged "needs-user" in all columns.
  // Backend reads stay off the UI thread and do not interfere with active editors.
  useEffect(() => {
    const paths = catalog?.projects.map((project) => project.path) ?? [];
    if (!paths.length) return;
    let cancelled = false;
    let inflight = false;
    const update = async () => {
      if (inflight) return;
      inflight = true;
      try {
        const rows = await userQueueProjectIds(paths);
        if (cancelled) return;
        setQueueIdsByPath((current) => {
          const next = { ...current };
          for (const row of rows) {
            // Offline boards have an unknown count, not an invented zero.
            if (!row.error) next[row.path] = row.eligibleIds;
            else delete next[row.path];
          }
          return next;
        });
      } catch { /* Retry on the next sync or focus event. */ }
      finally { inflight = false; }
    };
    void update();
    const timer = window.setInterval(() => void update(), 12000);
    window.addEventListener("focus", update);
    window.addEventListener("taskboard:refresh-board", update);
    window.addEventListener("taskboard:queue-changed", update);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
      window.removeEventListener("focus", update);
      window.removeEventListener("taskboard:refresh-board", update);
      window.removeEventListener("taskboard:queue-changed", update);
    };
  }, [catalog?.projects]);

  const visibleQueueCount = (project: ProjectTab): number => {
    const ids = queueIdsByPath[project.path] ?? [];
    const key = project.id === "wartribes"
      ? "taskboard.userQueueSnoozes"
      : `taskboard.project.${project.id}.userQueueSnoozes`;
    let snoozes: Record<string, number> = {};
    try { snoozes = JSON.parse(window.localStorage.getItem(key) || "{}"); }
    catch { /* A corrupted local snooze entry must not hide tasks. */ }
    const now = Date.now();
    return ids.filter((id) => !(Number(snoozes[id]) > now)).length;
  };

  // The active board owns its snapshot and mutation locks. Ask it to refresh
  // only when its editor/drag is closed; never replace an unsaved draft.
  useEffect(() => {
    const activeId = catalog?.activeId;
    if (!activeId || !changedProjects[activeId] || locked || working || dialogMode) return;
    window.dispatchEvent(new CustomEvent("taskboard:refresh-board", { detail: { silent: true } }));
    setChangedProjects((current) => {
      const next = { ...current };
      delete next[activeId];
      return next;
    });
  }, [catalog?.activeId, changedProjects, locked, working, dialogMode]);

  const refreshAllBoards = async () => {
    if (manualRefreshPending.current || !catalog) return;
    manualRefreshPending.current = true;
    setRefreshingAll(true);
    // Refresh the mounted board through its own concurrency-safe read path.
    window.dispatchEvent(new CustomEvent("taskboard:refresh-board", { detail: { silent: false } }));
    try {
      const latest = await listProjects();
      const next = latest.projects.some((project) => project.id === catalog.activeId)
        ? { ...latest, activeId: catalog.activeId } : latest;
      setCatalog(next);
      const inactive = next.projects.filter((project) => project.id !== next.activeId);
      const results = await Promise.allSettled(inactive.map((project) => loadBoard(project.path)));
      const failed = results.flatMap((result, index) => result.status === "rejected"
        ? [inactive[index].name] : []);
      if (failed.length) setError(`Unable to refresh: ${failed.join(", ")}`);
      else setError("");
      const stamps = await boardChangeTokens(next.projects.map((project) => project.path));
      for (const { path, token } of stamps) {
        if (token) knownBoardTokens.current[path] = token;
      }
      setChangedProjects((current) => {
        const updated = { ...current };
        for (const project of next.projects) {
          if (!failed.includes(project.name)) delete updated[project.id];
        }
        return updated;
      });
    } catch (reason) {
      setError(`Refresh all boards failed: ${String(reason)}`);
    } finally {
      manualRefreshPending.current = false;
      setRefreshingAll(false);
    }
  };

  // The Refresh button and F5 both reload all registered boards. Intercept
  // F5 before WebView2 can reload the whole app (which would discard drafts).
  useEffect(() => {
    const onRefresh = () => { void refreshAllBoards(); };
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "F5" || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.repeat) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void refreshAllBoards();
    };
    window.addEventListener("taskboard:refresh-all", onRefresh);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("taskboard:refresh-all", onRefresh);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [catalog]);

  // Keep an MRU order, like Windows task switching. Windows reserves Alt+Tab
  // globally, so Ctrl+Tab is the reliable in-app equivalent.
  useEffect(() => {
    if (!catalog) return;
    recentProjects.current = [catalog.activeId,
      ...recentProjects.current.filter((id) => id !== catalog.activeId && catalog.projects.some((p) => p.id === id)),
      ...catalog.projects.map((p) => p.id).filter((id) => id !== catalog.activeId && !recentProjects.current.includes(id))];
  }, [catalog?.activeId, catalog?.projects]);

  useEffect(() => {
    const onSwitch = (event: KeyboardEvent) => {
      if (event.key !== "Tab" || !(event.ctrlKey !== event.altKey) || event.metaKey ||
          working || locked || dialogMode || !catalog || catalog.projects.length < 2) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      const order = recentProjects.current.filter((id) => catalog.projects.some((p) => p.id === id));
      const nextId = event.shiftKey ? order[order.length - 1] : order.find((id) => id !== catalog.activeId);
      if (!nextId) return;
      const next = catalog.projects.find((p) => p.id === nextId);
      if (!next) return;
      setSwitchHint(next.name);
      if (switchHintTimer.current !== null) window.clearTimeout(switchHintTimer.current);
      switchHintTimer.current = window.setTimeout(() => setSwitchHint(null), 1050);
      void switchTo(nextId);
    };
    window.addEventListener("keydown", onSwitch, true);
    return () => window.removeEventListener("keydown", onSwitch, true);
  }, [catalog, working, locked, dialogMode]);
  // F1..F12 switch tabs, except F5 which is reserved for Refresh all.
  // Only intercept keys that actually map to a project.
  useEffect(() => {
    const handleFunctionKey = (event: KeyboardEvent) => {
      if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || event.repeat ||
          working || locked || dialogMode || !catalog) return;
      const match = /^F([1-9]|1[0-2])$/.exec(event.key);
      if (!match || event.key === "F5") return;
      const destination = catalog.projects[Number(match[1]) - 1];
      if (!destination) return;
      event.preventDefault();
      if (destination.id !== catalog.activeId) void switchTo(destination.id);
    };
    window.addEventListener("keydown", handleFunctionKey);
    return () => window.removeEventListener("keydown", handleFunctionKey);
  }, [working, locked, dialogMode, catalog]);

  const switchTo = async (id: string) => {
    if (working || locked || dialogMode || !catalog || catalog.activeId === id) return;
    setWorking(true);
    setError("");
    try {
      const updated = await activateProject(id);
      setLocked(false);
      setCatalog(updated);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const reorder = async (fromId: string, toId: string) => {
    if (!catalog || working || locked || dialogMode || fromId === toId) return;
    const ids = catalog.projects.map((item) => item.id);
    const source = ids.indexOf(fromId), destination = ids.indexOf(toId);
    if (source < 0 || destination < 0) return;
    const next = [...ids];
    next.splice(destination, 0, ...next.splice(source, 1));
    setWorking(true);
    setError("");
    try {
      setCatalog(await reorderProjects(next));
      setUndoProjectAction({ kind: "reorder", ids });
    } catch (err) {
      setError(String(err));
    } finally { setWorking(false); }
  };

  const undoProjectChange = async () => {
    if (!undoProjectAction || working || locked || dialogMode) return;
    setWorking(true);
    setError("");
    try {
      const updated = undoProjectAction.kind === "close"
        ? await restoreProject(undoProjectAction.project, undoProjectAction.index)
        : await reorderProjects(undoProjectAction.ids);
      setCatalog(updated);
      setUndoProjectAction(null);
    } catch (reason) {
      setError(`Unable to undo project change: ${String(reason)}`);
    } finally {
      setWorking(false);
    }
  };

  useEffect(() => {
    const onUndo = (event: KeyboardEvent) => {
      if (!event.ctrlKey || event.metaKey || event.altKey || event.shiftKey ||
          event.key.toLowerCase() !== "z" || event.repeat ||
          locked || working || dialogMode || !undoProjectAction) return;
      const target = event.target;
      if (target instanceof Element && target.closest("input,textarea,[contenteditable],.task-modal")) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      void undoProjectChange();
    };
    window.addEventListener("keydown", onUndo, true);
    return () => window.removeEventListener("keydown", onUndo, true);
  }, [undoProjectAction, locked, working, dialogMode]);

  const startCreate = () => {
    setConfirmCloseTab(false);
    setName("");
    setNameEdited(false);
    setFolders([""]);
    setTaskPrefix("PRJ");
    setEpicPrefix("PRJE");
    setPrefixEdited(false);
    setEditingBoardConfig(null);
    setEditingId("");
    setError("");
    setDialogMode("create");
  };

  const startEditFolders = async (project: ProjectTab) => {
    if (locked || working) return;
    setConfirmCloseTab(false);
    setWorking(true);
    setError("");
    try {
      // _board.md is the canonical prefix source for both CLI and desktop.
      const { config } = await loadBoard(project.path);
      setEditingBoardConfig(config);
      setTaskPrefix(config.task_prefix || "WT");
      setEpicPrefix(config.epic_prefix || "WTE");
      setEditingId(project.id);
      setName(project.name);
      setFolders(project.folders?.length ? [...project.folders] : [""]);
      setDialogMode("folders");
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const updateFolder = (index: number, value: string) => {
    setFolders((current) => current.map((old, i) => i === index ? value : old));
    if (dialogMode === "create" && index === 0 && !nameEdited) {
      const guessed = suggestedName(value);
      setName(guessed);
      if (!prefixEdited) {
        const prefix = suggestedPrefix(guessed);
        setTaskPrefix(prefix);
        setEpicPrefix(`${prefix}E`);
      }
    }
  };

  const browseFolder = async (index: number) => {
    if (working) return;
    setWorking(true);
    setError("");
    try {
      const path = await open({ directory: true, multiple: false, title: "Select a project folder" });
      if (typeof path === "string") updateFolder(index, path);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const saveDialog = async () => {
    if (working || locked) return;
    if (dialogMode === "create" && !name.trim()) return;
    if (!validPrefixes(taskPrefix.trim(), epicPrefix.trim())) {
      setError("Prefixes must be distinct: 2–12 uppercase letters/digits, starting with a letter.");
      return;
    }
    setWorking(true);
    setError("");
    try {
      const selectedFolders = folders.map((item) => item.trim()).filter(Boolean);
      let updated: ProjectCatalog;
      if (dialogMode === "create") {
        updated = await addProject(name, selectedFolders, taskPrefix.trim(), epicPrefix.trim());
      } else {
        if (editingBoardConfig && (taskPrefix !== (editingBoardConfig.task_prefix || "WT") ||
            epicPrefix !== (editingBoardConfig.epic_prefix || "WTE"))) {
          const boardPath = catalog?.projects.find((item) => item.id === editingId)?.path;
          if (!boardPath) throw new Error("Unknown project board");
          await saveBoardConfig({ ...editingBoardConfig, task_prefix: taskPrefix, epic_prefix: epicPrefix }, boardPath);
        }
        updated = await setProjectFolders(editingId, selectedFolders);
      }
      setCatalog(updated);
      setLocked(false);
      setDialogMode(null);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  const remove = async (project: ProjectTab) => {
    if (working || locked || !catalog || catalog.projects.length < 2 || !confirmCloseTab ||
        dialogMode !== "folders" || editingId !== project.id) return;
    setWorking(true);
    setError("");
    try {
      const previousIndex = catalog.projects.findIndex((item) => item.id === project.id);
      const updated = await removeProject(project.id);
      setUndoProjectAction({ kind: "close", project, index: previousIndex });
      setLocked(false);
      setCatalog(updated);
      setDialogMode(null);
      setConfirmCloseTab(false);
    } catch (err) {
      setError(String(err));
    } finally {
      setWorking(false);
    }
  };

  // Card drags originate inside Board (which is deliberately isolated per
  // project). The top strip receives the final drop through one custom event.
  useEffect(() => {
    const onTransfer = (event: Event) => {
      const detail = (event as CustomEvent<{
        taskId: string; expectedRevision: string;
        sourceProjectId: string; destinationProjectId: string;
      }>).detail;
      if (!catalog || working || dialogMode || !detail) return;
      if (detail.sourceProjectId === detail.destinationProjectId) return;
      const target = catalog.projects.find((project) => project.id === detail.destinationProjectId);
      const source = catalog.projects.find((project) => project.id === detail.sourceProjectId);
      if (!target || !source) return;
      void (async () => {
        setWorking(true);
        setError("");
        try {
          const transferred = await moveTaskBetweenProjects(
            source.id, target.id, detail.taskId, detail.expectedRevision,
          );
          setCatalog(await activateProject(target.id));
          setLocked(false);
          // Show the actual new board-specific ID; the source ID no longer exists.
          setError(`Moved ${detail.taskId} → ${target.name} / ${transferred.id}`);
        } catch (reason) {
          setError(`Could not move ${detail.taskId} to ${target.name}: ${String(reason)}`);
        } finally {
          setWorking(false);
        }
      })();
    };
    window.addEventListener("taskboard:project-transfer", onTransfer);
    return () => window.removeEventListener("taskboard:project-transfer", onTransfer);
  }, [catalog, working, dialogMode]);

  const active = catalog?.projects.find((project) => project.id === catalog.activeId);
  return (
    <div className="project-shell">
      {switchHint && <div className="project-switch-hint" role="status">Switching to {switchHint} · Ctrl+Tab</div>}
      <div className="project-tabs" role="tablist" aria-label="Project task boards">
        {catalog?.projects.map((project) => (
          <div className={`project-tab-wrapper ${project.id === catalog.activeId ? "selected" : ""} ${changedProjects[project.id] ? "changed" : ""} ${dropTab === project.id ? "project-tab-drop-target" : ""}`}
            key={project.id}
            data-project-tab-id={project.id}
            draggable={!working && !locked && !dialogMode}
            onDragStart={(event) => {
              draggedTab.current = project.id;
              event.dataTransfer.effectAllowed = "move";
              event.dataTransfer.setData("text/plain", project.id);
            }}
            onDragOver={(event) => {
              if (!draggedTab.current || draggedTab.current === project.id) return;
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              setDropTab(project.id);
            }}
            onDragLeave={() => setDropTab((current) => current === project.id ? null : current)}
            onDrop={(event) => {
              event.preventDefault();
              const source = draggedTab.current;
              draggedTab.current = null;
              setDropTab(null);
              if (source) void reorder(source, project.id);
            }}
            onDragEnd={() => { draggedTab.current = null; setDropTab(null); }}
          >
            <button
              type="button"
              role="tab"
              aria-selected={project.id === catalog.activeId}
              aria-controls="project-board"
              className="project-tab"
              title={`Drag to reorder · ${catalog.projects.indexOf(project) === 4 ? "F5 refreshes all boards" : `F${catalog.projects.indexOf(project) + 1}`} · Tasks: ${project.path}\nProject folders: ${(project.folders || []).join("; ") || "(none)"}${changedProjects[project.id] ? "\nNew external changes detected" : ""}`}
              disabled={working || locked || !!dialogMode}
              onClick={() => void switchTo(project.id)}
            >{project.name}
              {visibleQueueCount(project) > 0 && <span className="project-tab-user-count"
                aria-label={`${visibleQueueCount(project)} tasks in the user inbox`}
                title={`${visibleQueueCount(project)} tasks waiting for you`}>{visibleQueueCount(project)}</span>}
            </button>
            <button type="button" className="project-tab-settings"
              aria-label={`Edit ${project.name} settings`}
              title="Project folders and ID prefixes"
              disabled={working || locked || !!dialogMode}
              onClick={() => void startEditFolders(project)}>⚙</button>

          </div>
        ))}
        <button type="button" className="project-tab-add"
          title="Add a project with its own TaskBoard tasks"
          disabled={working || locked || !!dialogMode || !catalog}
          onClick={startCreate}>+ Project</button>
        {undoProjectAction && (
          <button type="button" className="project-undo-button"
            disabled={working || locked || !!dialogMode}
            title="Undo the last project tab closure or reorder (Ctrl+Z)"
            onClick={() => void undoProjectChange()}>
            ↶ Undo {undoProjectAction.kind === "close" ? "close" : "order"}
          </button>
        )}
        {refreshingAll && <span className="project-switch-note">Refreshing all boards…</span>}
        {locked && <span className="project-switch-note">Close the editor to change project</span>}
      </div>
      {(error || catalogError) && !dialogMode && <div className="project-catalog-error" role="alert">{error || catalogError}</div>}
      {active && <div className="project-workspace" id="project-board" role="tabpanel">
        <Board key={active.id} project={active} onInteractionChange={setLocked}
            {...(startupReview && startupReview.path.replace(/\\/g, "/").toLowerCase() === active.path.replace(/\\/g, "/").toLowerCase()
              ? { requestedReviewId: startupReview.taskId } : {})}
            {...(startupTask && startupTask.path.replace(/\\/g, "/").toLowerCase() === active.path.replace(/\\/g, "/").toLowerCase()
              ? { requestedTask: { id: startupTask.taskId, requestKey: startupTask.requestKey } } : {})} />
      </div>}
      {!active && <div className="project-loading">Loading TaskBoard project tabs…</div>}
      {dialogMode && (
        <div className="modal-backdrop project-add-backdrop"
          onMouseDown={() => { if (!working) setDialogMode(null); }}>
          <section className="project-add-dialog" role="dialog" aria-modal="true" aria-labelledby="project-add-heading"
            onMouseDown={(event) => event.stopPropagation()}>
            <h2 id="project-add-heading">{dialogMode === "create" ? "Add project" : `Project settings · ${name}`}</h2>
            <p>Associate one or more folders with this project. Your files stay where they are.</p>
            <div className="project-folder-entries">
              {folders.map((folder, index) => (
                <label key={index}>Project folder {folders.length > 1 ? index + 1 : ""}
                  <div className="project-folder-row">
                    <input autoFocus={index === 0} value={folder}
                      placeholder="Choose a source / working folder"
                      onChange={(event) => updateFolder(index, event.currentTarget.value)} />
                    <button type="button" className="button secondary"
                      disabled={working} onClick={() => void browseFolder(index)}>Browse…</button>
                    {folders.length > 1 && <button type="button" className="button secondary project-folder-remove"
                      aria-label={`Remove folder ${index + 1}`} disabled={working}
                      onClick={() => setFolders((current) => current.filter((_, i) => i !== index))}>×</button>}
                  </div>
                </label>
              ))}
            </div>
            <button className="project-add-folder" type="button" disabled={working}
              onClick={() => setFolders((current) => [...current, ""])}>+ Add folder</button>
            {dialogMode === "create" ? (
              <>
                <label>Project name
                  <input maxLength={80} value={name} placeholder="Suggested from the selected folder"
                    onChange={(event) => {
                      const next = event.currentTarget.value;
                      setNameEdited(true);
                      setName(next);
                      if (!prefixEdited) {
                        const prefix = suggestedPrefix(next);
                        setTaskPrefix(prefix);
                        setEpicPrefix(`${prefix}E`);
                      }
                    }} />
                </label>
                <p className="project-add-hint">
                  Tasks are stored automatically in your Obsidian vault:
                  <code className="project-task-path">{OBSIDIAN_ROOT}\{name.trim() || "[ProjectName]"}\Tasks</code>
                  Any existing TaskBoard there will be reused.
                </p>
              </>
            ) : (
              <p className="project-add-hint">TaskBoard tasks remain at
                <code className="project-task-path">{catalog?.projects.find((item) => item.id === editingId)?.path}</code>
                Removing a folder from this list never deletes it.
              </p>
            )}
            {dialogMode === "folders" && catalog && catalog.projects.length > 1 && (
              <div className="project-close-section">
                <div>
                  <strong>Close this project tab</strong>
                  <p>Removes ${name} from this TaskBoard tab strip. All project folders,
                    Markdown tasks and project data remain on disk.</p>
                </div>
                {confirmCloseTab ? (
                  <div className="project-close-confirm">
                    <span>Close ${name}?</span>
                    <button type="button" className="button secondary" disabled={working}
                      onClick={() => setConfirmCloseTab(false)}>Keep tab</button>
                    <button type="button" className="button danger" disabled={working}
                      onClick={() => {
                        const project = catalog.projects.find((item) => item.id === editingId);
                        if (project) void remove(project);
                      }}>Confirm close</button>
                  </div>
                ) : (
                  <button type="button" className="button secondary project-settings-close"
                    disabled={working} onClick={() => setConfirmCloseTab(true)}>
                    Close tab…
                  </button>
                )}
              </div>
            )}
            <div className="project-prefix-fields">
              <label>Task ID prefix
                <input maxLength={12} spellCheck={false} value={taskPrefix}
                  onChange={(event) => {
                    const next = event.currentTarget.value.toUpperCase();
                    setTaskPrefix(next);
                    setPrefixEdited(true);
                    if (epicPrefix === `${taskPrefix}E`) setEpicPrefix(`${next}E`);
                  }} />
              </label>
              <label>Epic ID prefix
                <input maxLength={12} spellCheck={false} value={epicPrefix}
                  onChange={(event) => { setEpicPrefix(event.currentTarget.value.toUpperCase()); setPrefixEdited(true); }} />
              </label>
            </div>
            <p className="project-add-hint">New IDs: {taskPrefix || "?"}-0001 / {epicPrefix || "?"}-0001. Existing task IDs, links and files stay unchanged. These settings are stored in the project's _board.md for CLI --root compatibility.</p>
            {error && <div className="project-form-error" role="alert">{error}</div>}
            <div className="project-dialog-actions">
              <button type="button" className="button secondary" disabled={working}
                onClick={() => { setDialogMode(null); setError(""); }}>Cancel</button>
              <button type="button" className="button primary"
                disabled={working || (dialogMode === "create" && !name.trim())}
                onClick={() => void saveDialog()}>
                {working ? "Saving…" : dialogMode === "create" ? "Add project" : "Save folders"}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
