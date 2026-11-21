import { useEffect, useRef, useState } from "react";
import "./ProjectTabs.css";
import { QueueIntelligence } from "./QueueIntelligence";
import { WindowControls } from "./WindowControls";
import { isInboxItemSnoozed, readInboxSnoozes } from "./inboxSnoozes";
import { open } from "@tauri-apps/plugin-dialog";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { QUICK_CREATE_WINDOW } from "./quickCreate";
import { activateProject, addProject, boardChangeTokens, listProjects, loadBoard, loadQuickBoard, moveTaskBetweenProjects, removeProject, reorderProjects, restoreProject, saveBoardConfig, setProjectFolders, showMainWindow, userQueueProjectIds, taskQueueWorkload } from "./api";
import type { ProjectCatalog, ProjectPriority, ProjectTab, QueueWorkload } from "./api";
import type { BoardConfig, BoardSnapshot, TaskDraft } from "./types";
import type { PendingClipboardImage } from "./TaskImageGallery";

type Props = {
  Board: (props: { project: ProjectTab; onInteractionChange: (locked: boolean) => void; requestedReviewId?: string; requestedTask?: {id: string; requestKey: number}; requestedCreate?: {draft: TaskDraft; images: PendingClipboardImage[]; sourcePath: string; requestKey: number}; requestedHotkeyCreate?: number; initialSnapshot?: BoardSnapshot; onSnapshot?: (path: string, snapshot: BoardSnapshot) => void; onInboxCount?: (path: string, count: number) => void }) => React.ReactNode;
};
type DialogMode = "create" | "folders" | null;
type TabDrop = { id: string; after: boolean };
type TabPointerDrag = { pointerId: number; fromId: string; x: number; y: number; started: boolean; target: TabDrop | null };
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
  // Keep snapshots in memory instead of mounting hidden boards and their timers.
  const boardCache = useRef(new Map<string, BoardSnapshot>());
  const [quickSnapshots, setQuickSnapshots] = useState<Record<string, BoardSnapshot>>({});
  useEffect(() => {
    if (!QUICK_CREATE_WINDOW || !catalog) return;
    const active = catalog.projects.find((item) => item.id === catalog.activeId);
    if (!active || quickSnapshots[active.path]) return;
    let cancelled = false;
    // Read just _board.md, not every task, to open the editor immediately.
    // ProjectBoard refreshes the full board in the background.
    void loadQuickBoard(active.path).catch(() => loadBoard(active.path))
      .then((snapshot) => {
        if (!cancelled) setQuickSnapshots((current) => ({ ...current, [active.path]: snapshot }));
      }).catch(() => undefined);
    return () => { cancelled = true; };
  }, [catalog?.activeId, catalog?.projects, quickSnapshots]);
  const activationWrites = useRef<Promise<unknown>>(Promise.resolve());
  const rememberBoard = (path: string, snapshot: BoardSnapshot) => { boardCache.current.set(path, snapshot); };
  useEffect(() => {
    if (QUICK_CREATE_WINDOW) return;
    const projects = catalog?.projects ?? [];
    if (!projects.length) return;
    let cancelled = false;
    const pending = projects.filter((project) => project.id !== catalog?.activeId && !boardCache.current.has(project.path));
    let index = 0;
    const warm = async () => {
      while (!cancelled && index < pending.length) {
        const project = pending[index++];
        try {
          const board = await loadBoard(project.path);
          if (!cancelled && !boardCache.current.has(project.path)) rememberBoard(project.path, board);
        } catch { /* A failed preload can retry on visit or refresh. */ }
      }
    };
    // Avoid flooding cloud storage while warming unopened tabs.
    const timer = window.setTimeout(() => { void warm(); void warm(); }, 250);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [catalog?.projects]);
  useEffect(() => {
    if (QUICK_CREATE_WINDOW || !catalog) return;
    const selected = catalog.activeId;
    // Save after painting, serialized so rapid switches cannot persist an older tab.
    const timer = window.setTimeout(() => {
      const job = activationWrites.current.catch(() => undefined).then(() => activateProject(selected));
      activationWrites.current = job;
      void job.catch((err) => setError(`Unable to remember selected project: ${String(err)}`));
    }, 150);
    return () => window.clearTimeout(timer);
  }, [catalog?.activeId]);
  useEffect(() => {
    // Show the shell promptly while the cloud-backed board loads.
    const timer = window.setTimeout(() => {
      if (QUICK_CREATE_WINDOW) {
        // Native placement must complete before showing the editor; otherwise
        // the WebView can briefly expose stale or half-sized restored bounds.
        void invoke("show_quick_create").catch(() => {
          // Keep the editor usable if native placement is temporarily unavailable.
          void getCurrentWindow().show().then(() => getCurrentWindow().setFocus()).catch(() => undefined);
        });
      } else {
        void showMainWindow().catch(() => undefined);
      }
    }, 120);
    return () => window.clearTimeout(timer);
  }, []);
  const [startupReview, setStartupReview] = useState<{ path: string; taskId: string } | null>(null);
  const [startupTask, setStartupTask] = useState<{ path: string; taskId: string; requestKey: number } | null>(null);
  const [hotkeyCreateRequest, setHotkeyCreateRequest] = useState(QUICK_CREATE_WINDOW ? 1 : 0);
  useEffect(() => {
    if (QUICK_CREATE_WINDOW) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // Legacy create requests to the main board (normal launches are unaffected).
    void listen("taskboard-create-task", () => setHotkeyCreateRequest((id) => id + 1))
      .then((remove) => { if (disposed) remove(); else unlisten = remove; })
      .catch(() => undefined);

    return () => { disposed = true; unlisten?.(); };
  }, []);
  // Handoff keeps an unsaved creation intact while the old board releases its ID.
  const [createHandoff, setCreateHandoff] = useState<{
    destinationId: string; draft: TaskDraft; images: PendingClipboardImage[];
    sourcePath: string; requestKey: number;
  } | null>(null);
  useEffect(() => {
    const switchEditor = (event: Event) => {
      const detail = (event as CustomEvent<{
        destinationId: string; draft: TaskDraft; images: PendingClipboardImage[]; sourcePath: string;
      }>).detail;
      if (!detail || !catalog?.projects.some((item) => item.id === detail.destinationId)) return;
      setCreateHandoff({ ...detail, requestKey: Date.now() });
      // The initial hotkey must not reopen the picker after choosing a project.
      if (QUICK_CREATE_WINDOW) setHotkeyCreateRequest(0);
      setCatalog((current) => current ? { ...current, activeId: detail.destinationId } : current);
    };
    const consumed = () => setCreateHandoff(null);
    window.addEventListener("taskboard:editor-switch-project", switchEditor);
    window.addEventListener("taskboard:create-handoff-consumed", consumed);
    return () => {
      window.removeEventListener("taskboard:editor-switch-project", switchEditor);
      window.removeEventListener("taskboard:create-handoff-consumed", consumed);
    };
  }, [catalog?.projects]);
  useEffect(() => {
    const onSearchResult = (event: Event) => {
      const detail = (event as CustomEvent<{ path: string; taskId: string }>).detail;
      if (detail?.path && detail.taskId) setStartupTask({ ...detail, requestKey: Date.now() });
    };
    window.addEventListener("taskboard:open-search-result", onSearchResult);
    return () => window.removeEventListener("taskboard:open-search-result", onSearchResult);
  }, []);
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
  const [priority, setPriority] = useState<ProjectPriority>("normal");
  const [taskPrefix, setTaskPrefix] = useState("WT");
  const [epicPrefix, setEpicPrefix] = useState("WTE");
  const [prefixEdited, setPrefixEdited] = useState(false);
  const [editingBoardConfig, setEditingBoardConfig] = useState<BoardConfig | null>(null);
  const recentProjects = useRef<string[]>([]);
  const [switchHint, setSwitchHint] = useState<string | null>(null);
  const switchHintTimer = useRef<number | null>(null);
  const tabStrip = useRef<HTMLDivElement>(null);
  const tabPointerDrag = useRef<TabPointerDrag | null>(null);
  const suppressTabClick = useRef(false);
  const [draggingTab, setDraggingTab] = useState<string | null>(null);
  const [dropTab, setDropTab] = useState<TabDrop | null>(null);
  const [changedProjects, setChangedProjects] = useState<Record<string, boolean>>({});
  const [inboxIdsByPath, setInboxIdsByPath] = useState<Record<string, { eligibleIds: string[]; reviewIds: string[] }>>({});
  // The mounted board owns the live Inbox count; unmounted tabs use the backend scan.
  const [liveInboxCounts, setLiveInboxCounts] = useState<Record<string, number>>({});
  const rememberInboxCount = (path: string, count: number) => {
    setLiveInboxCounts((current) => current[path] === count ? current : { ...current, [path]: count });
  };
  const knownBoardTokens = useRef<Record<string, string>>({});
  const manualRefreshPending = useRef(false);
  const [refreshingAll, setRefreshingAll] = useState(false);
  const [globalQueueWorkload, setGlobalQueueWorkload] = useState<QueueWorkload | null>(null);
  const [queueWorkloadError, setQueueWorkloadError] = useState(false);

  // T3 Keep Busy backlog is global, independent of the selected project tab.
  useEffect(() => {
    if (QUICK_CREATE_WINDOW) return;
    let canceled = false;
    let running = false;
    const sync = async () => {
      if (running) return;
      running = true;
      try {
        const value = await taskQueueWorkload();
        if (!canceled) { setGlobalQueueWorkload(value); setQueueWorkloadError(false); }
      } catch {
        if (!canceled) setQueueWorkloadError(true);
      } finally { running = false; }
    };
    void sync();
    const interval = window.setInterval(() => void sync(), 30000);
    window.addEventListener("focus", sync);
    window.addEventListener("taskboard:queue-changed", sync);
    window.addEventListener("taskboard:refresh-board", sync);
    return () => {
      canceled = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", sync);
      window.removeEventListener("taskboard:queue-changed", sync);
      window.removeEventListener("taskboard:refresh-board", sync);
    };
  }, []);

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
    const interval = window.setInterval(() => void refresh(), 10000);
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
    if (QUICK_CREATE_WINDOW || !catalog?.projects.length) return;
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
    const timer = window.setInterval(() => void scan(), 15000);
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
    if (QUICK_CREATE_WINDOW) return;
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
        setInboxIdsByPath((current) => {
          const next = { ...current };
          for (const row of rows) {
            // Offline boards have an unknown count, not an invented zero.
            if (!row.error) next[row.path] = { eligibleIds: row.eligibleIds, reviewIds: row.reviewIds };
            else delete next[row.path];
          }
          return next;
        });
      } catch { /* Retry on the next sync or focus event. */ }
      finally { inflight = false; }
    };
    void update();
    const timer = window.setInterval(() => void update(), 45000);
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

  const visibleInboxCount = (project: ProjectTab): number => {
    const { eligibleIds = [], reviewIds = [] } = inboxIdsByPath[project.path] ?? {};
    const key = project.id === "wartribes"
      ? "taskboard.userQueueSnoozes"
      : `taskboard.project.${project.id}.userQueueSnoozes`;
    let userSnoozes: Record<string, number> = {};
    const inboxSnoozes = readInboxSnoozes();
    try { userSnoozes = JSON.parse(window.localStorage.getItem(key) || "{}"); }
    catch { /* A corrupted local snooze entry must not hide tasks. */ }


    const now = Date.now();
    const notSnoozedInInbox = (id: string) => !isInboxItemSnoozed(inboxSnoozes, project.path, id, now);
    return reviewIds.filter(notSnoozedInInbox).length +
      eligibleIds.filter((id) => notSnoozedInInbox(id) && !(Number(userSnoozes[id]) > now)).length;
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
      results.forEach((result, index) => {
        if (result.status === "fulfilled") rememberBoard(inactive[index].path, result.value);
      });
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

  const switchTo = (id: string) => {
    if (working || locked || dialogMode || !catalog || catalog.activeId === id) return;
    setError("");
    // Render immediately; save selection independently and refresh cached data.
    setCatalog((current) => current ? { ...current, activeId: id } : current);
  };

  const reorder = async (fromId: string, toId: string, after: boolean) => {
    if (!catalog || working || locked || dialogMode || fromId === toId) return;
    const ids = catalog.projects.map((item) => item.id);
    if (!ids.includes(fromId) || !ids.includes(toId)) return;

    const next = ids.filter((id) => id !== fromId);
    next.splice(next.indexOf(toId) + Number(after), 0, fromId);
    if (next.every((id, index) => id === ids[index])) return;
    setWorking(true);
    setError("");
    try {
      setCatalog(await reorderProjects(next));
      setUndoProjectAction({ kind: "reorder", ids });
    } catch (err) {
      setError(String(err));
    } finally { setWorking(false); }
  };

  // Tab pointers use a movement threshold so ordinary clicks still switch boards.
  // Unlike native HTML drag/drop, this works consistently over WebView buttons.
  useEffect(() => {
    const clearDrag = () => {
      tabPointerDrag.current = null;
      setDraggingTab(null);
      setDropTab(null);
    };
    const onMove = (event: PointerEvent) => {
      const drag = tabPointerDrag.current;
      if (!drag || event.pointerId !== drag.pointerId) return;
      // A release outside the window may not send pointerup back to WebView.
      if (event.pointerType === "mouse" && !(event.buttons & 1)) { clearDrag(); return; }
      if (!drag.started) {
        if (Math.hypot(event.clientX - drag.x, event.clientY - drag.y) < 6) return;
        drag.started = true;
        setDraggingTab(drag.fromId);
      }
      if (event.cancelable) event.preventDefault();
      const strip = tabStrip.current;
      if (!strip || working || locked || dialogMode) return;
      const bounds = strip.getBoundingClientRect();
      if (event.clientY < bounds.top - 12 || event.clientY > bounds.bottom + 12) {
        drag.target = null;
        setDropTab(null);
        return;
      }
      const tabs = Array.from(strip.querySelectorAll<HTMLElement>("[data-project-tab-id]"));
      const target = tabs.find((tab) => event.clientX < tab.getBoundingClientRect().right) ?? tabs[tabs.length - 1];
      if (!target?.dataset.projectTabId) return;
      const tabBounds = target.getBoundingClientRect();
      const next = { id: target.dataset.projectTabId, after: event.clientX >= (tabBounds.left + tabBounds.right) / 2 };
      drag.target = next;
      setDropTab((previous) => previous?.id === next.id && previous.after === next.after ? previous : next);
    };
    const onUp = (event: PointerEvent) => {
      const drag = tabPointerDrag.current;
      if (!drag || event.pointerId !== drag.pointerId) return;
      if (drag.started) {
        suppressTabClick.current = true;
        window.setTimeout(() => { suppressTabClick.current = false; }, 0);
      }
      clearDrag();
      if (drag.started && drag.target && drag.fromId !== drag.target.id) {
        void reorder(drag.fromId, drag.target.id, drag.target.after);
      }
    };
    const onCancel = () => clearDrag();
    window.addEventListener("pointermove", onMove, { passive: false });
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onCancel);
    window.addEventListener("blur", onCancel);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onCancel);
      window.removeEventListener("blur", onCancel);
    };
  }, [catalog, working, locked, dialogMode]);

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
    setPriority("normal");
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
      setPriority(project.priority || "normal");
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
        updated = await setProjectFolders(editingId, selectedFolders, priority);
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
      <div className="project-titlebar">
      <div className="project-tabs" ref={tabStrip} role="tablist" aria-label="Project task boards">
        <QueueIntelligence data={globalQueueWorkload} error={queueWorkloadError}
          onOpenTask={(path, taskId) => setStartupTask({ path, taskId, requestKey: Date.now() })} />
        {catalog?.projects.map((project) => {
          const inboxCount = project.id === catalog.activeId && liveInboxCounts[project.path] !== undefined
            ? liveInboxCounts[project.path] : visibleInboxCount(project);
          return (
          <div className={`project-tab-wrapper ${project.id === catalog.activeId ? "selected" : ""} ${changedProjects[project.id] ? "changed" : ""} ${draggingTab === project.id ? "project-tab-being-dragged" : ""} ${dropTab?.id === project.id && draggingTab !== project.id ? (dropTab.after ? "project-tab-insert-after" : "project-tab-insert-before") : ""}`}
            key={project.id}
            data-project-tab-id={project.id}
            onPointerDown={(event) => {
              if (event.button !== 0 || !event.isPrimary || working || locked || dialogMode ||
                  !(event.target as Element).closest(".project-tab")) return;
              tabPointerDrag.current = {
                pointerId: event.pointerId, fromId: project.id, x: event.clientX,
                y: event.clientY, started: false, target: null,
              };
            }}
            onClickCapture={(event) => {
              if (!suppressTabClick.current) return;
              event.preventDefault();
              event.stopPropagation();
              suppressTabClick.current = false;
            }}
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
              {inboxCount > 0 && <span className="project-tab-user-count"
                aria-label={`${inboxCount} tasks in the user inbox`}
                title={`${inboxCount} tasks waiting for you`}>{inboxCount}</span>}
            </button>
            <button type="button" className="project-tab-settings"
              aria-label={`Edit ${project.name} settings`}
              title="Project priority, folders and ID prefixes"
              disabled={working || locked || !!dialogMode}
              onClick={() => void startEditFolders(project)}>⚙</button>

          </div>
          );
        })}
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
      {!QUICK_CREATE_WINDOW && <WindowControls />}
      </div>
      {(error || catalogError) && !dialogMode && <div className="project-catalog-error" role="alert">{error || catalogError}</div>}
      {active && (!QUICK_CREATE_WINDOW || quickSnapshots[active.path]) && <div className="project-workspace" id="project-board" role="tabpanel">
        <Board key={active.id} project={active} onInteractionChange={setLocked}
            requestedHotkeyCreate={hotkeyCreateRequest || undefined}
            {...(createHandoff?.destinationId === active.id ? { requestedCreate: createHandoff } : {})}
            initialSnapshot={QUICK_CREATE_WINDOW ? quickSnapshots[active.path] : boardCache.current.get(active.path)} onSnapshot={rememberBoard}
            onInboxCount={rememberInboxCount}
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
            {dialogMode === "folders" && (
              <label>Automatic work priority
                <select value={priority} disabled={working}
                  onChange={(event) => setPriority(event.currentTarget.value as ProjectPriority)}
                  style={{ padding: "10px 11px", border: "1px solid #496077", borderRadius: 7, background: "#101b28", color: "#f1f5fb", fontSize: 13 }}>
                  <option value="high">High — prefer when choosing new work</option>
                  <option value="normal">Normal — default</option>
                  <option value="low">Low — pick after other priorities</option>
                </select>
                <span style={{ color: "#96a9bc", fontWeight: 400 }}>T3 Code uses this to pick queued tasks, while spreading parallel work across different projects.</span>
              </label>
            )}
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
                {working ? "Saving…" : dialogMode === "create" ? "Add project" : "Save settings"}
              </button>
            </div>
          </section>
        </div>
      )}
    </div>
  );
}
