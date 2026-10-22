import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import "./App.css";
import { TASK_ICON_OPTIONS, TaskIconGlyph } from "./TaskIcons";
import { TitleMarkup } from "./TitleMarkup";
import { DependencyArrows } from "./DependencyArrows";
import { TaskLinkPicker } from "./TaskLinkPicker";
import { AutoTextarea, DescriptionTimeline } from "./TaskDescriptionPanel";
import { parseDescriptionTimeline, prepareTaskDraftForSave } from "./descriptionTimeline";
import { applyCardPlacements, orderCardsByDependencies } from "./dependencyLayout";
import { withEpicProxyRows } from "./epicProxyRows";
import { eligibleForAgeArchive } from "./archiveAge";
import { isHiddenByCollapsedEpic, toggleAllEpicCollapses } from "./epicCollapse";
import {
  archiveTask,
  archiveColumn,
  archiveColumnOlderThan,
  createTask,
  getDefaultBoardPath,
  highlightInFomni,
  loadBoard,
  openInObsidian,
  restoreSnapshot,
  saveBoardConfig,
  showMainWindow,
  updateTask,
} from "./api";
import type {
  BoardConfig,
  BoardSnapshot,
  Task,
  TaskAttention,
  TaskDraft,
} from "./types";

type EditorState = {
  mode: "create" | "edit";
  original?: Task;
  draft: TaskDraft;
};

type UndoEntry =
  | {
      kind: "board";
      label: string;
      before: BoardSnapshot;
      after: BoardSnapshot;
    }
  | {
      kind: "collapse";
      label: string;
      before: string[];
      after: string[];
    };

type TaskContextMenu = {
  taskId: string;
  x: number;
  y: number;
};

type ColumnContextMenu = {
  columnId: string;
  x: number;
  y: number;
};

type SnoozeSpan = "day" | "three-days" | "week" | "month";
type UserQueueReview = { ids: string[]; index: number };
type UserQueueSnoozes = Record<string, number>;
const isForUser = (task: Pick<Task, "kind" | "attention">) =>
  task.kind === "for_user" || (task.kind === "task" && task.attention.includes("needs-user"));

const USER_QUEUE_SNOOZES_KEY = "taskboard.userQueueSnoozes";
const USER_QUEUE_SNOOZE_MS = 24 * 60 * 60 * 1000;

const activeUserQueueSnoozes = (raw: unknown, now = Date.now()): UserQueueSnoozes => {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw).filter(([id, until]) =>
      /^WTE?-\d+$/.test(id) && typeof until === "number" &&
      Number.isFinite(until) && until > now),
  );
};

const loadUserQueueSnoozes = (): UserQueueSnoozes => {
  try {
    return activeUserQueueSnoozes(JSON.parse(window.localStorage.getItem(USER_QUEUE_SNOOZES_KEY) || "{}"));
  } catch {
    return {};
  }
};

const isUserQueueEligible = (
  task: Task, board: BoardSnapshot, snoozes: UserQueueSnoozes, now = Date.now(),
): boolean =>
  isForUser(task) &&
  !task.archived &&
  !task.completed &&
  !board.config.columns.some((column) => column.id === task.column && column.terminal) &&
  (snoozes[task.id] ?? 0) <= now;

const newestUserTasks = (tasks: Task[]): Task[] =>
  [...tasks].sort((a, b) =>
    (Date.parse(b.created) || 0) - (Date.parse(a.created) || 0) ||
    b.id.localeCompare(a.id));

const snoozeDeadline = (span: SnoozeSpan): string => {
  const now = new Date();
  if (span === "day") return new Date(now.getTime() + 86400000).toISOString();
  if (span === "three-days") return new Date(now.getTime() + 3 * 86400000).toISOString();
  if (span === "week") return new Date(now.getTime() + 7 * 86400000).toISOString();
  const month = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + 1, 1,
    now.getUTCHours(), now.getUTCMinutes(), now.getUTCSeconds(), now.getUTCMilliseconds()));
  const daysInMonth = new Date(Date.UTC(month.getUTCFullYear(), month.getUTCMonth() + 1, 0)).getUTCDate();
  month.setUTCDate(Math.min(now.getUTCDate(), daysInMonth));
  return month.toISOString();
};

const completedSortTime = (task: Task) => {
  const timestamp = Date.parse(task.completed_at || task.completed || "");
  return Number.isFinite(timestamp) ? timestamp : 0;
};

type TaskActivityKind = "created" | "moved" | "updated";
type RecentTaskActivity = { kind: TaskActivityKind; at: number };

const RECENT_ACTIVITY_STORAGE_KEY = "taskboard.recentTaskActivity";
const RECENT_ACTIVITY_MS = 10 * 60 * 1000;

const loadRecentTaskActivity = (): Record<string, RecentTaskActivity> => {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(RECENT_ACTIVITY_STORAGE_KEY) || "{}");
    if (!stored || typeof stored !== "object" || Array.isArray(stored)) return {};
    const now = Date.now();
    return Object.fromEntries(Object.entries(stored).filter((entry): entry is [string, RecentTaskActivity] => {
      const value = entry[1] as Partial<RecentTaskActivity> | null;
      return !!value && (value.kind === "created" || value.kind === "moved" || value.kind === "updated") &&
        typeof value.at === "number" && value.at <= now && now - value.at < RECENT_ACTIVITY_MS;
    }));
  } catch { return {}; }
};

const classifyTaskChange = (previous: Task | undefined, current: Task): TaskActivityKind | null => {
  if (!previous) return "created";
  if (previous.column !== current.column || previous.parent !== current.parent ||
      previous.position_after !== current.position_after || previous.positioned_at !== current.positioned_at) return "moved";
  return previous.revision !== current.revision ? "updated" : null;
};

const AUTO_REFRESH_MS = 3000;
const AUTO_REFRESH_IDLE_MS = 1600;
const TASK_ACTIVITY_MS = 2800;

// While a card or column is held near the visible board's left/right edge,
// keep scrolling each frame even when the mouse itself stops moving.
const startDragEdgeScroll = (
  board: HTMLElement | null,
  pointer: () => { x: number; y: number },
  onScroll?: () => void,
): (() => void) => {
  if (!board) return () => {};
  let frame = 0;
  let lastTime = 0;
  const tick = (time: number) => {
    const elapsed = lastTime ? Math.min(50, time - lastTime) : 16;
    lastTime = time;
    const { x, y } = pointer();
    const rect = board.getBoundingClientRect();
    if (y >= rect.top && y <= rect.bottom && x >= rect.left - 24 && x <= rect.right + 24 &&
        board.scrollWidth > board.clientWidth + 1) {
      const zone = 96;
      const left = Math.max(0, Math.min(1, (rect.left + zone - x) / zone));
      const right = Math.max(0, Math.min(1, (x - (rect.right - zone)) / zone));
      const velocity = (right * right - left * left) * 1450;
      if (velocity !== 0) {
        const before = board.scrollLeft;
        board.scrollLeft += velocity * elapsed / 1000;
        if (before !== board.scrollLeft) onScroll?.();
      }
    }
    frame = window.requestAnimationFrame(tick);
  };
  frame = window.requestAnimationFrame(tick);
  return () => window.cancelAnimationFrame(frame);
};

const emptyDraft = (
  column: string,
  parent: string | null = null,
  category: string | null = null,
): TaskDraft => ({
  title: "",
  column,
  parent,
  category,
  kind: "task",
  depends_on: [],
  related_to: [],
  icon: null,
  attention: [],
  question: null,
  description: "",
  acceptance_criteria: [],
  ai_notes: "",
  user_notes: "",
  notes: "",
});

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : String(error);

const columnPalette = [
  "#718f78",
  "#7388a8",
  "#8e7da8",
  "#a48669",
  "#6f8679",
  "#8c7d72",
  "#6f8b91",
  "#927b86",
];

const randomColumnColor = (used: string[]) => {
  const available = columnPalette.filter((color) => !used.includes(color));
  const pool = available.length ? available : columnPalette;
  return pool[Math.floor(Math.random() * pool.length)];
};

function App() {
  const [boardPath, setBoardPath] = useState("");
  const [snapshot, setSnapshot] = useState<BoardSnapshot | null>(null);
  const [query, setQuery] = useState("");
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [descriptionUpdateInput, setDescriptionUpdateInput] = useState("");
  const [userQueueReview, setUserQueueReview] = useState<UserQueueReview | null>(null);
  const [followupQueueReturn, setFollowupQueueReturn] = useState<UserQueueReview | null>(null);
  const [userQueueSnoozes, setUserQueueSnoozes] = useState<UserQueueSnoozes>(loadUserQueueSnoozes);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dragColumnId, setDragColumnId] = useState<string | null>(null);
  const [columnDragPreview, setColumnDragPreview] = useState<{
    id: string;
    name: string;
    x: number;
    y: number;
    width: number;
  } | null>(null);
  const [cardDropTargetId, setCardDropTargetId] = useState<string | null>(null);
  const [cardDragPreview, setCardDragPreview] = useState<{
    id: string;
    title: string;
    x: number;
    y: number;
    width: number;
  } | null>(null);
  const [collapsedTaskIds, setCollapsedTaskIds] = useState<Set<string>>(() => {
    try {
      const stored = window.localStorage.getItem("taskboard.collapsedTasks");
      return new Set(stored ? (JSON.parse(stored) as string[]) : []);
    } catch {
      return new Set();
    }
  });
  const boardRef = useRef<HTMLElement | null>(null);
  const panRef = useRef({ active: false, startX: 0, scrollLeft: 0 });
  const localMutationVersion = useRef(0);
  const pendingTaskWrites = useRef(new Set<string>());
  const refreshJobRef = useRef<Promise<BoardSnapshot> | null>(null);
  const lastUserInputAtRef = useRef(Date.now());
  const pointerIsDownRef = useRef(false);
  const refreshFeedbackTimerRef = useRef<number | null>(null);
  const snapshotRef = useRef<BoardSnapshot | null>(null);
  const activityTimersRef = useRef(new Map<string, number>());
  const [taskActivity, setTaskActivity] = useState<Record<string, TaskActivityKind>>({});
  const [recentTaskActivity, setRecentTaskActivity] = useState<Record<string, RecentTaskActivity>>(loadRecentTaskActivity);
  const [hoverTask, setHoverTask] = useState<{
    task: Task;
    x: number;
    y: number;
  } | null>(null);
  const [highlightedEpicId, setHighlightedEpicId] = useState<string | null>(null);
  const [questionAnswerText, setQuestionAnswerText] = useState("");
  const [contextMenu, setContextMenu] = useState<TaskContextMenu | null>(null);
  const [moveSubmenu, setMoveSubmenu] = useState<{ x: number; y: number } | null>(null);
  const [columnContextMenu, setColumnContextMenu] = useState<ColumnContextMenu | null>(null);
  const lastTerminalFirstIds = useRef(new Map<string, string>());
  const undoStackRef = useRef<UndoEntry[]>([]);
  const [undoCount, setUndoCount] = useState(0);
  const [busy, setBusy] = useState(false);
  const [refreshFeedback, setRefreshFeedback] = useState<"idle" | "working" | "done">("idle");
  const [error, setError] = useState("");

  const applySnapshot = (next: BoardSnapshot) => {
    snapshotRef.current = next;
    setSnapshot(next);
  };

  const flashTaskActivity = (
    changes: Array<{ id: string; kind: TaskActivityKind }>,
  ) => {
    if (!changes.length) return;

    const at = Date.now();
    setRecentTaskActivity((current) => {
      const next = { ...current };
      for (const change of changes) next[change.id] = { kind: change.kind, at };
      return next;
    });
    setTaskActivity((current) => {
      const next = { ...current };
      for (const change of changes) next[change.id] = change.kind;
      return next;
    });

    for (const change of changes) {
      const currentTimer = activityTimersRef.current.get(change.id);
      if (currentTimer) window.clearTimeout(currentTimer);

      const timer = window.setTimeout(() => {
        setTaskActivity((current) => {
          if (!(change.id in current)) return current;
          const next = { ...current };
          delete next[change.id];
          return next;
        });
        activityTimersRef.current.delete(change.id);
      }, TASK_ACTIVITY_MS);

      activityTimersRef.current.set(change.id, timer);
    }
  };

  const refresh = async (path = boardPath, silent = false) => {
    if (!path) return;
    if (!silent) {
      if (refreshFeedbackTimerRef.current !== null) {
        window.clearTimeout(refreshFeedbackTimerRef.current);
        refreshFeedbackTimerRef.current = null;
      }
      setRefreshFeedback("working");
    }

    try {
      // A manual refresh always gets a new read after any in-flight poll or
      // local write. Automatic refreshes skip instead of piling up requests.
      if (silent && (refreshJobRef.current || pendingTaskWrites.current.size)) return;
      if (!silent) {
        const started = Date.now();
        while (refreshJobRef.current || pendingTaskWrites.current.size) {
          if (Date.now() - started > 10000) {
            throw new Error("Timed out waiting for an ongoing task operation");
          }
          if (refreshJobRef.current) {
            try {
              await refreshJobRef.current;
            } catch {
              // The manual read below can recover from a failed prior poll.
            }
          } else {
            await new Promise((resolve) => window.setTimeout(resolve, 80));
          }
        }
      }

      const versionAtStart = localMutationVersion.current;
      const job = loadBoard(path);
      refreshJobRef.current = job;
      let next: BoardSnapshot;
      try {
        next = await job;
      } finally {
        if (refreshJobRef.current === job) refreshJobRef.current = null;
      }
      if (versionAtStart !== localMutationVersion.current) {
        if (!silent) throw new Error("A task changed during refresh; try again");
        return;
      }

      const previous = snapshotRef.current;
      if (previous) {
        const previousTasks = new Map(
          previous.tasks
            .filter((task) => !task.archived)
            .map((task) => [task.id, task]),
        );
        const changes: Array<{ id: string; kind: TaskActivityKind }> = [];

        for (const task of next.tasks) {
          if (task.archived) continue;
          const oldTask = previousTasks.get(task.id);
          const kind = classifyTaskChange(oldTask, task);
          if (kind) changes.push({ id: task.id, kind });
        }

        flashTaskActivity(changes);
      }

      applySnapshot(next);
      setHoverTask((current) => {
        if (!current) return null;
        const updated = next.tasks.find((task) => task.id === current.task.id);
        return updated ? { ...current, task: updated } : null;
      });
      if (!silent) {
        setError("");
        setRefreshFeedback("done");
        refreshFeedbackTimerRef.current = window.setTimeout(() => {
          setRefreshFeedback("idle");
          refreshFeedbackTimerRef.current = null;
        }, 1600);
      }
    } catch (err) {
      if (!silent) {
        setError(`Refresh failed: ${errorText(err)}`);
        setRefreshFeedback("idle");
      }
    }
  };

  useEffect(() => {
    void (async () => {
      try {
        const path = await getDefaultBoardPath();
        setBoardPath(path);
        const next = await loadBoard(path);
        applySnapshot(next);
        setError("");
      } catch (err) {
        setError(errorText(err));
      } finally {
        // The native window starts hidden. Give React/WebView2 time to commit
        // and paint the loaded board (or dark error state) before revealing it.
        await new Promise((resolve) => window.setTimeout(resolve, 160));
        void document.documentElement.getBoundingClientRect();
        try {
          await showMainWindow();
        } catch (err) {
          setError(errorText(err));
        }
      }
    })();
  }, []);

  // UI-only activity markers survive a window restart but never change the
  // underlying task Markdown or create synthetic timestamps for old tasks.
  useEffect(() => {
    try {
      window.localStorage.setItem(RECENT_ACTIVITY_STORAGE_KEY, JSON.stringify(recentTaskActivity));
    } catch { /* Storage may be disabled; badges still work for this session. */ }
  }, [recentTaskActivity]);

  // Persist snoozes independently of Markdown task state. Wake the counter at
  // the exact expiry even if automatic board refresh is suspended by an editor.
  useEffect(() => {
    try {
      window.localStorage.setItem(USER_QUEUE_SNOOZES_KEY, JSON.stringify(userQueueSnoozes));
    } catch { /* Snooze continues for the current window if storage is unavailable. */ }
    const nextExpiry = Math.min(...Object.values(userQueueSnoozes).filter((until) => until > Date.now()));
    if (!Number.isFinite(nextExpiry)) return;
    const timer = window.setTimeout(() => {
      setUserQueueSnoozes((current) => activeUserQueueSnoozes(current));
    }, Math.max(1, Math.min(2147483647, nextExpiry - Date.now() + 20)));
    return () => window.clearTimeout(timer);
  }, [userQueueSnoozes]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setRecentTaskActivity((current) => {
        const now = Date.now();
        const active = Object.entries(current).filter(([, value]) => now - value.at < RECENT_ACTIVITY_MS);
        return active.length === Object.keys(current).length ? current : Object.fromEntries(active);
      });
    }, 15000);
    return () => window.clearInterval(timer);
  }, []);

  useEffect(() => {
    const markInput = () => { lastUserInputAtRef.current = Date.now(); };
    const pointerDown = () => { pointerIsDownRef.current = true; markInput(); };
    const pointerUp = () => { pointerIsDownRef.current = false; markInput(); };
    const windowBlur = () => { pointerIsDownRef.current = false; };
    window.addEventListener("pointerdown", pointerDown);
    window.addEventListener("pointerup", pointerUp);
    window.addEventListener("pointercancel", pointerUp);
    window.addEventListener("keydown", markInput);
    window.addEventListener("wheel", markInput);
    window.addEventListener("blur", windowBlur);
    return () => {
      window.removeEventListener("pointerdown", pointerDown);
      window.removeEventListener("pointerup", pointerUp);
      window.removeEventListener("pointercancel", pointerUp);
      window.removeEventListener("keydown", markInput);
      window.removeEventListener("wheel", markInput);
      window.removeEventListener("blur", windowBlur);
    };
  }, []);

  useEffect(() => {
    if (!boardPath || editor || busy || contextMenu || columnContextMenu || dragId || dragColumnId) return;
    const timer = window.setInterval(() => {
      if (pointerIsDownRef.current || panRef.current.active) return;
      if (Date.now() - lastUserInputAtRef.current < AUTO_REFRESH_IDLE_MS) return;
      void refresh(boardPath, true);
    }, AUTO_REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [boardPath, editor, busy, contextMenu, columnContextMenu, dragId, dragColumnId]);

  // Timed wake-up must not depend on the idle polling loop, which pauses
  // while an editor/context menu is open or the user is actively interacting.
  useEffect(() => {
    if (!boardPath) return;
    const timer = window.setInterval(() => {
      if (busy || pendingTaskWrites.current.size) return;
      const due = snapshotRef.current?.config.columns.some((column) =>
        column.snoozed_until && Date.parse(column.snoozed_until) <= Date.now());
      if (due) void refresh(boardPath, true);
    }, 10000);
    return () => window.clearInterval(timer);
  }, [boardPath, busy]);

  useEffect(
    () => () => {
      for (const timer of activityTimersRef.current.values()) {
        window.clearTimeout(timer);
      }
      activityTimersRef.current.clear();
      if (refreshFeedbackTimerRef.current !== null) window.clearTimeout(refreshFeedbackTimerRef.current);
    },
    [],
  );

  // Wheel over open board space scrolls across columns. Preserve normal
  // vertical scrolling inside card lists and hover previews. A native
  // non-passive listener is required: React's wheel listener is passive.
  useEffect(() => {
    const board = boardRef.current;
    if (!board) return;
    const handleBoardWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.shiftKey || Math.abs(event.deltaY) < 0.1 ||
          Math.abs(event.deltaX) > Math.abs(event.deltaY) ||
          board.scrollWidth <= board.clientWidth + 1) return;

      const target = event.target;
      if (!(target instanceof Element)) return;
      if (target.closest(".task-card, .task-context-menu, .column-actions, .quick-add, button, input, textarea, select")) return;

      const cardList = target.closest<HTMLElement>(".cards");
      if (cardList && cardList.scrollHeight > cardList.clientHeight + 1) return;

      event.preventDefault();
      const factor = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 :
        event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? board.clientWidth : 1;
      board.scrollLeft += event.deltaY * factor;
      setHoverTask(null);
    };
    board.addEventListener("wheel", handleBoardWheel, { passive: false });
    return () => board.removeEventListener("wheel", handleBoardWheel);
  }, [!!snapshot]);

  useEffect(() => {
    window.localStorage.setItem(
      "taskboard.collapsedTasks",
      JSON.stringify([...collapsedTaskIds]),
    );
  }, [collapsedTaskIds]);

  const activeTasks = useMemo(
    () => snapshot?.tasks.filter((task) => !task.archived) ?? [],
    [snapshot],
  );

  const taskMap = useMemo(
    () => new Map(activeTasks.map((task) => [task.id, task])),
    [activeTasks],
  );

  const isComplete = (id: string) => {
    const item = snapshot?.tasks.find((t) => t.id === id);
    return !!item && (item.archived || !!item.completed || columns.some((col) => col.id === item.column && col.terminal));
  };
  const waitingFor = (task: Task): string[] => (task.depends_on || []).filter((id) => !isComplete(id));
  const readinessOf = (task: Task) => task.kind === "epic" ? "epic"
    : isForUser(task) ? "for_user"
    : task.attention.includes("blocked") || task.column === "blocked" ? "blocked"
    : waitingFor(task).length ? "waiting" : "ready";
  const epicProgress = (epic: Task) => {
    const visited = new Set<string>();
    const leaves: Task[] = [];
    const stack = [epic.id];
    while (stack.length) {
      const parent = stack.pop()!;
      for (const task of snapshot?.tasks ?? []) {
        if (task.parent !== parent || visited.has(task.id)) continue;
        visited.add(task.id);
        if (task.kind === "epic") stack.push(task.id);
        else leaves.push(task);
      }
    }
    return { total: leaves.length, done: leaves.filter((task) => task.archived || isComplete(task.id)).length,
      inProgress: leaves.filter((task) => !isComplete(task.id) && task.column === "in-progress").length,
      blocked: leaves.filter((task) => !isComplete(task.id) && readinessOf(task) !== "ready").length };
  };

  const dependencyLayoutKey = `${query}|${dragId ?? ""}|${dragColumnId ?? ""}|${[...collapsedTaskIds].sort().join(",")}`;

  const childCounts = useMemo(() => {
    const counts = new Map<string, number>();
    for (const task of activeTasks) {
      if (!task.parent) continue;
      counts.set(task.parent, (counts.get(task.parent) ?? 0) + 1);
    }
    return counts;
  }, [activeTasks]);

  const visibleTasks = useMemo(() => {
    const needle = query.trim().toLowerCase();
    const candidates = needle
      ? activeTasks.filter((task) =>
          [
            task.id,
            task.title,
            task.category ?? "",
            task.description,
            task.ai_notes,
            task.user_notes,
            task.question?.prompt ?? "",
            task.question?.answer ?? "",
            task.notes,
          ].join("\n")
            .toLowerCase()
            .includes(needle),
        )
      : activeTasks;

    // Collapsing an epic hides its executable descendants in every column,
    // including while searching. Orange For user cards are always visible.
    return candidates.filter((task) => !isHiddenByCollapsedEpic(task, collapsedTaskIds, taskMap));
  }, [activeTasks, query, taskMap, collapsedTaskIds]);

  const columns = useMemo(
    () => [...(snapshot?.config.columns ?? [])].sort((a, b) => a.order - b.order),
    [snapshot],
  );
  const userQueueTasks = useMemo(
    () => snapshot
      ? newestUserTasks(activeTasks.filter((task) => isUserQueueEligible(task, snapshot, userQueueSnoozes)))
      : [],
    [activeTasks, snapshot, userQueueSnoozes],
  );

  // A newly completed card should remain visible even when Done was scrolled down.
  useEffect(() => {
    if (!snapshot) return;
    const terminalColumns = snapshot.config.columns.filter((column) => column.terminal);
    for (const column of terminalColumns) {
      const first = snapshot.tasks.filter((task) => !task.archived && task.column === column.id)
        .sort((a, b) => completedSortTime(b) - completedSortTime(a) || b.id.localeCompare(a.id))[0]?.id;
      if (!first) continue;
      const previous = lastTerminalFirstIds.current.get(column.id);
      lastTerminalFirstIds.current.set(column.id, first);
      if (previous && previous !== first) {
        document.querySelector<HTMLElement>(`[data-column-id="${CSS.escape(column.id)}"] .cards`)
          ?.scrollTo({ top: 0 });
      }
    }
  }, [snapshot]);

  const categories = useMemo(
    () =>
      [...new Set(activeTasks.map((task) => task.category).filter((value): value is string => !!value))]
        .sort((a, b) => a.localeCompare(b)),
    [activeTasks],
  );

  const depthOf = (task: Task) => {
    let depth = 0;
    let current = task.parent;
    const seen = new Set<string>();
    while (current && depth < 8 && !seen.has(current)) {
      seen.add(current);
      const parent = taskMap.get(current);
      if (!parent) break;
      depth += 1;
      current = parent.parent;
    }
    return depth;
  };

  const hierarchyKey = (task: Task) => {
    const parts = [task.id];
    let current = task.parent;
    const seen = new Set<string>();
    while (current && !seen.has(current)) {
      seen.add(current);
      parts.unshift(current);
      current = taskMap.get(current)?.parent ?? null;
    }
    return parts.join("/");
  };

  const pushUndo = (entry: UndoEntry) => {
    undoStackRef.current.push(entry);
    if (undoStackRef.current.length > 80) {
      undoStackRef.current.shift();
    }
    setUndoCount(undoStackRef.current.length);
  };

  const snapshotWithTask = (base: BoardSnapshot, task: Task) => ({
    ...base,
    tasks: base.tasks.some((item) => item.id === task.id)
      ? base.tasks.map((item) => (item.id === task.id ? task : item))
      : [...base.tasks, task],
  });

  const toggleTaskCollapsed = (id: string) => {
    if (taskMap.get(id)?.kind !== "epic") return;
    const next = new Set(collapsedTaskIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    pushUndo({
      kind: "collapse",
      label: "epic collapse",
      before: [...collapsedTaskIds],
      after: [...next],
    });
    setCollapsedTaskIds(next);
  };

  const epicIdsWithChildren = activeTasks
    .filter((task) => task.kind === "epic" && childCounts.has(task.id))
    .map((task) => task.id);
  const allEpicsCollapsed = epicIdsWithChildren.length > 0 &&
    epicIdsWithChildren.every((id) => collapsedTaskIds.has(id));

  const toggleAllEpicsCollapsed = () => {
    if (!epicIdsWithChildren.length) return;
    const next = toggleAllEpicCollapses(collapsedTaskIds, epicIdsWithChildren);
    pushUndo({
      kind: "collapse",
      label: allEpicsCollapsed ? "expand all epics" : "collapse all epics",
      before: [...collapsedTaskIds],
      after: [...next],
    });
    setCollapsedTaskIds(next);
    setHoverTask(null);
  };

  const undoLast = async () => {
    if (busy) return;
    const entry = undoStackRef.current.pop();
    if (!entry) return;
    setUndoCount(undoStackRef.current.length);

    if (entry.kind === "collapse") {
      setCollapsedTaskIds(new Set(entry.before));
      return;
    }

    setBusy(true);
    try {
      const restored = await restoreSnapshot(entry.before, entry.after, boardPath);
      applySnapshot(restored);
      setError("");
    } catch (err) {
      // Put it back: the user can resolve/reload the conflicting external edit
      // and decide whether to retry.
      undoStackRef.current.push(entry);
      setUndoCount(undoStackRef.current.length);
      setError(errorText(err));
      await refresh(boardPath, true);
    } finally {
      setBusy(false);
    }
  };

  const taskToDraft = (task: Task): TaskDraft => ({
    title: task.title,
    column: task.column,
    parent: task.parent,
    category: task.category,
    kind: task.kind,
    depends_on: [...task.depends_on],
    related_to: [...task.related_to],
    icon: task.icon ?? null,
    attention: [...task.attention],
    question: task.question
      ? {
          prompt: task.question.prompt,
          options: [...task.question.options],
          answer: task.question.answer,
        }
      : null,
    description: task.description,
    acceptance_criteria: [...task.acceptance_criteria],
    ai_notes: task.ai_notes,
    user_notes: task.user_notes,
    notes: task.notes,
  });

  const attentionRank = (task: Task) => {
    if (isForUser(task)) return 0;
    if (task.attention.includes("blocked")) return 2;
    if (
      task.attention.includes("question") ||
      task.attention.includes("needs-user") ||
      task.attention.includes("user-note")
    ) {
      return 0;
    }
    return 1;
  };

  const showHoverDetails = (
    event: React.MouseEvent<HTMLElement>,
    task: Task,
  ) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const width = Math.min(390, window.innerWidth - 16);
    const x = rect.right + width + 10 <= window.innerWidth
      ? rect.right + 10
      : Math.max(8, rect.left - width - 10);
    const height = Math.min(520, window.innerHeight * 0.7);
    const y = Math.max(8, Math.min(rect.top, window.innerHeight - height - 8));
    setHoverTask({ task, x, y });
  };

  const jumpToOriginalEpic = (epic: Task) => {
    setHoverTask(null);
    setHighlightedEpicId(epic.id);
    setQuery("");
    const column = columns.find((item) => item.id === epic.column);
    // Respect manually collapsed/snoozed columns; their original epic
    // can still be inspected without silently changing column settings.
    if (column?.hidden) {
      openEdit(epic);
      return;
    }
    setCollapsedTaskIds((current) => {
      const next = new Set(current);
      const visited = new Set<string>();
      let ancestor = epic.parent;
      while (ancestor && !visited.has(ancestor)) {
        visited.add(ancestor);
        next.delete(ancestor);
        ancestor = taskMap.get(ancestor)?.parent ?? null;
      }
      return next;
    });
    window.setTimeout(() => {
      const target = document.querySelector<HTMLElement>(
        `[data-task-id="${CSS.escape(epic.id)}"].task-card`,
      );
      if (target) {
        target.scrollIntoView({ behavior: "smooth", inline: "center", block: "center" });
      } else {
        openEdit(epic);
      }
    }, 90);
  };

  const closeEditor = (board = snapshot) => {
    // A follow-up created during queue review is edited outside the queue.
    // Closing or saving it resumes the queued tasks where the user left off.
    if (followupQueueReturn && board) {
      const review = followupQueueReturn;
      setFollowupQueueReturn(null);
      advanceUserQueue(board, userQueueSnoozes, review);
      return;
    }
    setUserQueueReview(null);
    setEditor(null);
  };

  useEffect(() => {
    if (editor) document.querySelector<HTMLElement>(".task-modal")?.scrollTo({ top: 0 });
  }, [editor?.original?.id]);

  const openEdit = (task: Task, fromQueue = false) => {
    if (!fromQueue) setUserQueueReview(null);
    setQuestionAnswerText("");
    setDescriptionUpdateInput("");
    setHoverTask(null);
    setEditor({
      mode: "edit",
      original: task,
      draft: taskToDraft(task),
    });
  };

  const startUserQueue = () => {
    if (busy || !userQueueTasks.length) return;
    const ids = userQueueTasks.map((task) => task.id);
    setFollowupQueueReturn(null);
    setUserQueueReview({ ids, index: 0 });
    openEdit(userQueueTasks[0], true);
  };

  const advanceUserQueue = (board: BoardSnapshot, snoozes = userQueueSnoozes, review = userQueueReview) => {
    if (!review) return;
    for (let index = review.index + 1; index < review.ids.length; index += 1) {
      const task = board.tasks.find((entry) => entry.id === review.ids[index]);
      if (task && isUserQueueEligible(task, board, snoozes)) {
        setUserQueueReview({ ...review, index });
        openEdit(task, true);
        return;
      }
    }
    // No banner: return directly to the board, leaving skipped tasks queued.
    setUserQueueReview(null);
    setFollowupQueueReturn(null);
    setEditor(null);
  };

  const skipUserQueueTask = () => {
    if (busy || !userQueueReview || !snapshot) return;
    advanceUserQueue(snapshot);
  };

  const snoozeUserQueueTask = () => {
    if (busy || !userQueueReview || !editor?.original || !snapshot) return;
    const until = Date.now() + USER_QUEUE_SNOOZE_MS;
    const snoozes = { ...userQueueSnoozes, [editor.original.id]: until };
    setUserQueueSnoozes(snoozes);
    advanceUserQueue(snapshot, snoozes);
  };

  const defaultCreateColumn = () =>
    columns.find((item) => item.id === "ai-ready")?.id ??
    columns.find((item) => item.ai_ready)?.id ??
    columns.find((item) => !item.terminal && item.id !== "draft")?.id ??
    columns[0]?.id;

  const openCreate = (
    column?: string,
    parent: string | null = null,
    category: string | null = null,
  ) => {
    const target = column ?? defaultCreateColumn();
    if (!target) return;
    setContextMenu(null);
    setColumnContextMenu(null);
    setUserQueueReview(null);
    setFollowupQueueReturn(null);
    setDescriptionUpdateInput("");
    setEditor({ mode: "create", draft: emptyDraft(target,
      parent && taskMap.get(parent)?.kind === "epic" ? parent : null, category) });
  };

  const run = async (operation: () => Promise<void>) => {
    setBusy(true);
    try {
      await operation();
      setError("");
    } catch (err) {
      setError(errorText(err));
      await refresh(boardPath, true);
    } finally {
      setBusy(false);
    }
  };

  const saveEditor = (markDone = false) =>
    run(async () => {
      if (!editor || !snapshot) return;
      if (!editor.draft.title.trim()) return;
      if (
        editor.draft.attention.includes("question") &&
        !editor.draft.question?.prompt.trim()
      ) {
        return;
      }

      const before = snapshot;
      const prepared = prepareTaskDraftForSave(editor.draft, descriptionUpdateInput, editor.original?.claimed_by);
      let draft = { ...prepared,
        column: markDone && editor.mode === "edit" && doneColumn ? doneColumn.id : prepared.column,
        depends_on: editor.draft.kind === "epic" ? [] : [...editor.draft.depends_on],
        related_to: [...editor.draft.related_to],
        parent: editor.draft.kind === "epic" ? null : editor.draft.parent,
        icon: editor.draft.kind === "epic" ? null : editor.draft.icon,
        attention: editor.draft.kind === "epic" ? [] : [...editor.draft.attention],
        question: editor.draft.kind === "epic" ? null : prepared.question,
        acceptance_criteria: editor.draft.kind === "epic" ? [] : prepared.acceptance_criteria,
      };
      if (draft.kind === "epic" && editor.original?.kind !== "epic") {
        const retained = [
          draft.notes,
          ...(prepared.acceptance_criteria.length
            ? ["Previous acceptance criteria (retained as planning notes):\n" +
                prepared.acceptance_criteria.map((item) => "- " + item).join("\n")] : []),
        ].filter(Boolean);
        draft = { ...draft, notes: retained.join("\n\n") };
      }
      let after = before;

      if (editor.mode === "create") {
        const saved = await createTask(draft, boardPath);
        after = snapshotWithTask(before, saved);
        flashTaskActivity([{ id: saved.id, kind: "created" }]);
        pushUndo({ kind: "board", label: "create task", before, after });
      } else if (editor.original) {
        const saved = await updateTask({ ...editor.original, ...draft }, boardPath);
        // Completing a prerequisite can release other tasks from Blocked in
        // the shared core. Reload those automatic changes as part of this save.
        after = doneColumn && saved.column === doneColumn.id
          ? await loadBoard(boardPath) : snapshotWithTask(before, saved);
        const kind = classifyTaskChange(editor.original, saved);
        if (kind) flashTaskActivity([{ id: saved.id, kind }]);
        pushUndo({ kind: "board", label: "edit task", before, after });
      }

      applySnapshot(after);
      if (userQueueReview && editor.original?.id === userQueueReview.ids[userQueueReview.index]) {
        advanceUserQueue(after);
      } else {
        closeEditor(after);
      }
    });

  // Closing a task preserves edited fields; the follow-up starts in the same
  // work column/epic/category and links back to the completed task.
  const closeAndFollowup = () =>
    run(async () => {
      if (!editor || !editor.original || editor.mode !== "edit" ||
          editor.draft.kind === "epic" || !snapshot || !doneColumn) return;
      if (!editor.draft.title.trim() ||
          (editor.draft.attention.includes("question") &&
           !editor.draft.question?.prompt.trim())) return;
      const original = editor.original;
      const before = snapshot;
      const sourceColumn = editor.draft.column === doneColumn.id
        ? original.column : editor.draft.column;
      const prepared = prepareTaskDraftForSave(editor.draft, descriptionUpdateInput, original.claimed_by);
      const closed = await updateTask({
        ...original,
        ...prepared,
        column: doneColumn.id,
        position_after: null,
        positioned_at: null,
      }, boardPath);
      let followup: Task;
      try {
        followup = await createTask({
          ...emptyDraft(sourceColumn, editor.draft.parent, editor.draft.category),
          title: `RE: ${original.id}`,
          related_to: [original.id],
        }, boardPath);
      } catch (error) {
        // The first save has committed: do not leave a stale editor open or
        // suggest the close was rolled back by a failed second operation.
        const after = await loadBoard(boardPath);
        applySnapshot(after);
        if (userQueueReview) advanceUserQueue(after);
        else closeEditor(after);
        flashTaskActivity([{ id: original.id, kind: "moved" }]);
        throw new Error(`${original.id} was saved and moved to Done, but the follow-up could not be created: ${errorText(error)}`);
      }
      const after = await loadBoard(boardPath);
      applySnapshot(after);
      flashTaskActivity([
        { id: closed.id, kind: "moved" },
        { id: followup.id, kind: "created" },
      ]);
      pushUndo({ kind: "board", label: "close and create follow-up", before, after });
      if (userQueueReview) setFollowupQueueReturn(userQueueReview);
      openEdit(followup);
    });

  const chooseDraftKind = (kind: TaskDraft["kind"]) => {
    if (!editor) return;
    setEditor({
      ...editor,
      draft: {
        ...editor.draft,
        kind,
        parent: kind === "epic" ? null : editor.draft.parent,
        column: editor.mode === "create"
          ? kind === "epic" ? "draft" : kind === "for_user" ? "in-progress" : defaultCreateColumn() ?? editor.draft.column
          : editor.draft.column,
        depends_on: kind === "epic" ? [] : editor.draft.depends_on,
        icon: kind === "epic" ? null : editor.draft.icon,
        attention: kind === "epic" ? [] : editor.draft.attention.filter((flag) => flag !== "needs-user"),
        question: kind === "epic" ? null : editor.draft.question,
      },
    });
  };

  const toggleAttentionDraft = (flag: TaskAttention) => {
    if (!editor) return;
    const hasFlag = editor.draft.attention.includes(flag);
    let question = editor.draft.question;
    if (flag === "question") {
      if (hasFlag && question && !question.answer) {
        question = null;
      } else if (!hasFlag && (!question || question.answer)) {
        question = { prompt: "", options: [], answer: null };
      }
    }
    setEditor({
      ...editor,
      draft: {
        ...editor.draft,
        attention: hasFlag
          ? editor.draft.attention.filter((item) => item !== flag)
          : [...editor.draft.attention, flag],
        question,
      },
    });
  };

  const answerQuestion = (answer: string) =>
    run(async () => {
      if (
        !editor?.original ||
        !editor.draft.question ||
        !answer.trim() ||
        !snapshot
      ) {
        return;
      }
      const before = snapshot;
      const nextTask: Task = {
        ...editor.original,
        ...editor.draft,
        acceptance_criteria: editor.draft.acceptance_criteria.map((item) => item.trim()).filter(Boolean),
        attention: editor.draft.attention.filter((item) => item !== "question"),
        question: {
          ...editor.draft.question,
          options: editor.draft.question.options.map((item) => item.trim()).filter(Boolean),
          answer: answer.trim(),
        },
      };
      const saved = await updateTask(nextTask, boardPath);
      const after = snapshotWithTask(before, saved);
      const kind = classifyTaskChange(editor.original, saved);
      if (kind) flashTaskActivity([{ id: saved.id, kind }]);
      pushUndo({ kind: "board", label: "answer question", before, after });
      applySnapshot(after);
      setEditor({
        mode: "edit",
        original: saved,
        draft: taskToDraft(saved),
      });
      setQuestionAnswerText("");
    });

  const persistTaskMutation = async (
    id: string,
    label: string,
    mutate: (task: Task) => Task,
  ) => {
    const task = taskMap.get(id);
    if (!task || !snapshot) return;

    const before = snapshot;
    const optimistic = mutate(task);
    localMutationVersion.current += 1;
    pendingTaskWrites.current.add(id);
    applySnapshot(snapshotWithTask(before, optimistic));
    setError("");

    try {
      await updateTask(optimistic, boardPath);
      const after = await loadBoard(boardPath);
      const saved = after.tasks.find((item) => item.id === id);
      if (saved) {
        const kind = classifyTaskChange(task, saved);
        if (kind) flashTaskActivity([{ id, kind }]);
      }
      pushUndo({ kind: "board", label, before, after });
      applySnapshot(after);
    } catch (err) {
      localMutationVersion.current += 1;
      pendingTaskWrites.current.delete(id);
      applySnapshot(before);
      setError(errorText(err));
      await refresh(boardPath, true);
    } finally {
      pendingTaskWrites.current.delete(id);
    }
  };

  const moveTaskTo = async (id: string, column: string, afterTaskId: string | null = null) => {
    const task = taskMap.get(id);
    if (!task || task.kind === "epic" || (task.column === column && !afterTaskId)) return;
    const terminal = columns.find((item) => item.id === column)?.terminal;
    await persistTaskMutation(id, afterTaskId ? "place task below another" : "move task", (current) => ({
      ...current,
      column,
      position_after: afterTaskId,
      positioned_at: afterTaskId ? new Date().toISOString() : null,
      ...(terminal ? {
        kind: current.kind === "for_user" ? "task" as const : current.kind,
        attention: current.attention.filter((flag) => flag !== "needs-user"),
      } : {}),
      ...(terminal && !current.completed
        ? { completed_at: new Date().toISOString() }
        : {}),
    }));
  };

  const makeSubtaskOf = async (id: string, parentId: string) => {
    const task = taskMap.get(id);
    const parent = taskMap.get(parentId);
    if (!task || !parent || parent.kind !== "epic" || id === parentId) return;

    let ancestor: string | null = parent.id;
    const seen = new Set<string>();
    while (ancestor && !seen.has(ancestor)) {
      if (ancestor === id) {
        setError("That would create a task hierarchy cycle.");
        return;
      }
      seen.add(ancestor);
      ancestor = taskMap.get(ancestor)?.parent ?? null;
    }

    setCollapsedTaskIds((current) => {
      const next = new Set(current);
      next.delete(parentId);
      return next;
    });

    await persistTaskMutation(id, "make subtask", (current) => ({
      ...current,
      parent: parentId,
      position_after: null,
      positioned_at: null,
    }));
  };

  const beginCardDrag = (
    event: React.MouseEvent<HTMLElement>,
    task: Task,
  ) => {
    if (event.button !== 0) return;
    if (task.kind === "epic") {
      openEdit(task); // Epics can be inspected, but never dragged/moved.
      return;
    }
    event.preventDefault();

    const startX = event.clientX;
    const startY = event.clientY;
    const rect = event.currentTarget.getBoundingClientRect();
    const grabOffsetX = startX - rect.left;
    const grabOffsetY = startY - rect.top;
    let dragging = false;
    let pointerX = startX;
    let pointerY = startY;
    let stopAutoScroll: (() => void) | null = null;

    const cleanup = () => {
      stopAutoScroll?.();
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
      setDragId(null);
      setCardDragPreview(null);
      setCardDropTargetId(null);
      document.body.classList.remove("card-dragging");
    };

    const validDropTaskAt = (x: number, y: number) => {
      const candidate = document
        .elementFromPoint(x, y)
        ?.closest<HTMLElement>("[data-task-id]")
        ?.dataset.taskId;
      if (!candidate || candidate === task.id) return null;
      // Normal cards are placement targets, not parents. Only epic cards
      // establish hierarchy; moving onto another card merely places below it.
      return taskMap.has(candidate) ? candidate : null;
    };

    const handleMove = (moveEvent: MouseEvent) => {
      pointerX = moveEvent.clientX;
      pointerY = moveEvent.clientY;
      if (
        !dragging &&
        Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) >= 6
      ) {
        dragging = true;
        stopAutoScroll = startDragEdgeScroll(boardRef.current, () => ({ x: pointerX, y: pointerY }), () => {
          setCardDropTargetId(validDropTaskAt(pointerX, pointerY));
          setHoverTask(null);
        });
        setDragId(task.id);
        setCardDragPreview({
          id: task.id,
          title: task.title,
          x: moveEvent.clientX - grabOffsetX,
          y: moveEvent.clientY - grabOffsetY,
          width: rect.width,
        });
        document.body.classList.add("card-dragging");
      }

      if (dragging) {
        moveEvent.preventDefault();
        setCardDragPreview({
          id: task.id,
          title: task.title,
          x: moveEvent.clientX - grabOffsetX,
          y: moveEvent.clientY - grabOffsetY,
          width: rect.width,
        });
        setCardDropTargetId(
          validDropTaskAt(moveEvent.clientX, moveEvent.clientY),
        );
      }
    };

    const handleUp = (upEvent: MouseEvent) => {
      const taskTargetId = dragging
        ? validDropTaskAt(upEvent.clientX, upEvent.clientY)
        : null;
      const taskTarget = taskTargetId ? taskMap.get(taskTargetId) : null;
      const columnTarget = dragging
        ? document
            .elementFromPoint(upEvent.clientX, upEvent.clientY)
            ?.closest<HTMLElement>("[data-column-id]")
            ?.dataset.columnId
        : null;

      cleanup();

      if (!dragging) {
        openEdit(task);
        return;
      }

      if (taskTarget?.kind === "epic") {
        void makeSubtaskOf(task.id, taskTarget.id);
      } else if (taskTarget) {
        void moveTaskTo(task.id, taskTarget.column, taskTarget.id);
      } else if (columnTarget) {
        void moveTaskTo(task.id, columnTarget);
      }
    };

    window.addEventListener("mousemove", handleMove, { passive: false });
    window.addEventListener("mouseup", handleUp, { once: true });
  };

  const saveConfig = (config: BoardConfig, label = "edit columns") =>
    run(async () => {
      if (!snapshot) return;
      const before = snapshot;
      const saved = await saveBoardConfig(config, boardPath);
      const after = { ...before, config: saved };
      pushUndo({ kind: "board", label, before, after });
      applySnapshot(after);
    });

  const addColumn = () => {
    if (!snapshot) return;
    const name = window.prompt("Column name");
    if (!name?.trim()) return;
    const base =
      name.toLowerCase().trim().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") ||
      "column";
    let id = base;
    let suffix = 2;
    while (snapshot.config.columns.some((column) => column.id === id)) {
      id = `${base}-${suffix++}`;
    }
    const config = {
      ...snapshot.config,
      columns: [
        ...snapshot.config.columns,
        {
          id,
          name: name.trim(),
          order: snapshot.config.columns.length,
          terminal: false,
          hidden: false,
          snoozed_until: null,
          awake_until: null,
          ai_ready: false,
          color: randomColumnColor(snapshot.config.columns.map((column) => column.color)),
        },
      ],
    };
    void saveConfig(config, "create column");
  };

  const renameColumn = (id: string) => {
    if (!snapshot) return;
    const column = snapshot.config.columns.find((item) => item.id === id);
    if (!column) return;
    const name = window.prompt("Rename column", column.name);
    if (!name?.trim() || name.trim() === column.name) return;
    void saveConfig(
      {
        ...snapshot.config,
        columns: snapshot.config.columns.map((item) =>
          item.id === id ? { ...item, name: name.trim() } : item,
        ),
      },
      "rename column",
    );
  };

  const reorderColumn = (sourceId: string, targetId: string, after: boolean) => {
    if (!snapshot || sourceId === targetId) return;
    const ordered = [...columns];
    const sourceIndex = ordered.findIndex((column) => column.id === sourceId);
    if (sourceIndex < 0) return;
    const [source] = ordered.splice(sourceIndex, 1);
    const targetIndex = ordered.findIndex((column) => column.id === targetId);
    if (targetIndex < 0) return;
    ordered.splice(targetIndex + (after ? 1 : 0), 0, source);
    const reordered = ordered.map((column, order) => ({ ...column, order }));
    void saveConfig(
      { ...snapshot.config, columns: reordered },
      "reorder columns",
    );
  };

  const beginColumnDrag = (
    event: React.MouseEvent<HTMLElement>,
    column: BoardConfig["columns"][number],
  ) => {
    if (event.button !== 0) return;
    const target = event.target as HTMLElement;
    if (target.closest("button")) return;

    event.preventDefault();
    const startX = event.clientX;
    const startY = event.clientY;
    const rect = event.currentTarget.getBoundingClientRect();
    const grabOffsetX = startX - rect.left;
    const grabOffsetY = startY - rect.top;
    let dragging = false;
    let pointerX = startX;
    let pointerY = startY;
    let stopAutoScroll: (() => void) | null = null;

    const cleanup = () => {
      stopAutoScroll?.();
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
      setDragColumnId(null);
      setColumnDragPreview(null);
      document.body.classList.remove("column-dragging");
    };

    const handleMove = (moveEvent: MouseEvent) => {
      pointerX = moveEvent.clientX;
      pointerY = moveEvent.clientY;
      if (
        !dragging &&
        Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) >= 6
      ) {
        dragging = true;
        stopAutoScroll = startDragEdgeScroll(boardRef.current, () => ({ x: pointerX, y: pointerY }));
        setDragColumnId(column.id);
        document.body.classList.add("column-dragging");
      }

      if (dragging) {
        moveEvent.preventDefault();
        setColumnDragPreview({
          id: column.id,
          name: column.name,
          x: moveEvent.clientX - grabOffsetX,
          y: moveEvent.clientY - grabOffsetY,
          width: rect.width,
        });
      }
    };

    const handleUp = (upEvent: MouseEvent) => {
      const targetColumn = dragging
        ? document
            .elementFromPoint(upEvent.clientX, upEvent.clientY)
            ?.closest<HTMLElement>("[data-column-id]")
        : null;
      const targetId = targetColumn?.dataset.columnId;
      const targetRect = targetColumn?.getBoundingClientRect();
      cleanup();

      if (
        dragging &&
        targetId &&
        targetId !== column.id &&
        targetRect
      ) {
        reorderColumn(
          column.id,
          targetId,
          upEvent.clientX > targetRect.left + targetRect.width / 2,
        );
      }
    };

    window.addEventListener("mousemove", handleMove, { passive: false });
    window.addEventListener("mouseup", handleUp, { once: true });
  };

  const toggleColumnAiReady = (id: string) => {
    if (!snapshot) return;
    const column = snapshot.config.columns.find((item) => item.id === id);
    if (
      !column ||
      column.id === "draft" ||
      column.id === "ai-ready" ||
      column.terminal
    ) return;
    void saveConfig(
      {
        ...snapshot.config,
        columns: snapshot.config.columns.map((item) =>
          item.id === id ? { ...item, ai_ready: !item.ai_ready } : item,
        ),
      },
      "toggle AI-ready column",
    );
  };

  const toggleColumnHidden = (id: string) => {
    if (!snapshot) return;
    void saveConfig(
      {
        ...snapshot.config,
        columns: snapshot.config.columns.map((column) =>
          column.id === id
            ? { ...column, hidden: !(column.hidden && (!column.snoozed_until || Date.parse(column.snoozed_until) > Date.now())), snoozed_until: null, awake_until: null }
            : column,
        ),
      },
      "toggle column visibility",
    );
  };

  const snoozeColumn = (id: string, span: SnoozeSpan) => {
    if (!snapshot || busy) return;
    setColumnContextMenu(null);
    const until = snoozeDeadline(span);
    void saveConfig(
      {
        ...snapshot.config,
        columns: snapshot.config.columns.map((column) =>
          column.id === id
            ? { ...column, hidden: true, snoozed_until: until, awake_until: null }
            : column,
        ),
      },
      `snooze column for one ${span}`,
    );
  };

  const startBoardPan = (event: React.PointerEvent<HTMLElement>) => {
    if (event.button !== 2 || !boardRef.current) return;
    const target = event.target as HTMLElement;
    if (
      target.closest(
        ".task-card, .column, .hidden-column, .column-header, button, input, textarea, select, .task-context-menu",
      )
    ) {
      return;
    }
    event.preventDefault();
    panRef.current = {
      active: true,
      startX: event.clientX,
      scrollLeft: boardRef.current.scrollLeft,
    };
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const moveBoardPan = (event: React.PointerEvent<HTMLElement>) => {
    if (!panRef.current.active || !boardRef.current) return;
    event.preventDefault();
    boardRef.current.scrollLeft =
      panRef.current.scrollLeft - (event.clientX - panRef.current.startX);
  };

  const endBoardPan = (event: React.PointerEvent<HTMLElement>) => {
    if (!panRef.current.active) return;
    panRef.current.active = false;
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
  };

  const deleteColumn = (id: string) => {
    if (!snapshot || snapshot.config.columns.length <= 1) return;
    const occupied = activeTasks.filter((task) => task.column === id);
    const remaining = columns.filter((column) => column.id !== id);
    let destination: string | null = null;

    if (occupied.length) {
      const answer = window.prompt(
        `This column contains ${occupied.length} task(s). Move them to which column?\n${remaining
          .map((column) => column.name)
          .join(", ")}`,
        remaining[0]?.name,
      );
      if (!answer) return;
      destination =
        remaining.find(
          (column) =>
            column.id.toLowerCase() === answer.toLowerCase() ||
            column.name.toLowerCase() === answer.toLowerCase(),
        )?.id ?? null;
      if (!destination) {
        setError("Destination column not found.");
        return;
      }
    } else if (!window.confirm("Delete this empty column?")) {
      return;
    }

    void run(async () => {
      const before = snapshot;
      if (destination) {
        for (const task of occupied) {
          await updateTask({ ...task, column: destination }, boardPath);
        }
      }
      const nextColumns = remaining.map((column, order) => ({ ...column, order }));
      await saveBoardConfig({ ...snapshot.config, columns: nextColumns }, boardPath);
      const after = await loadBoard(boardPath);
      pushUndo({ kind: "board", label: "delete column", before, after });
      applySnapshot(after);
    });
  };

  const archiveTaskById = (task: Task) =>
    run(async () => {
      if (!snapshot) return;
      const before = snapshot;
      const archived = await archiveTask(task.id, boardPath);
      const after = snapshotWithTask(before, archived);
      pushUndo({ kind: "board", label: "archive task", before, after });
      applySnapshot(after);
      setContextMenu(null);
      if (editor?.original?.id === task.id) {
        if (userQueueReview) advanceUserQueue(after);
        else closeEditor(after);
      }
    });

  const archiveAllInColumn = (id: string) => {
    setColumnContextMenu(null);
    if (!snapshot || busy) return;
    const tasks = activeTasks.filter((task) => task.column === id);
    const column = columns.find((item) => item.id === id);
    if (!tasks.length || !column) return;
    const selected = new Set(tasks.map((task) => task.id));
    const orphanCount = activeTasks.filter((task) => task.parent && selected.has(task.parent) && !selected.has(task.id)).length;
    const details = orphanCount ? ` ${orphanCount} active subtask(s) outside this column will be unparented.` : "";
    if (!window.confirm(`Archive all ${tasks.length} active task(s) in "${column.name}"?${details} You can undo this action.`)) return;

    void run(async () => {
      const before = snapshot;
      // One core operation holds the board's write lock and checks every
      // task revision before making changes, rather than racing agent edits.
      const after = await archiveColumn(id, tasks.map((task) => [task.id, task.revision]), boardPath);
      pushUndo({ kind: "board", label: "archive all column tasks", before, after });
      applySnapshot(after);
    });
  };

  const archiveDoneOlderThanThreeDays = (id: string) => {
    setColumnContextMenu(null);
    if (!snapshot || busy) return;
    const column = columns.find((item) => item.id === id);
    if (!column?.terminal) return;
    const cutoffIso = new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString();
    const eligible = eligibleForAgeArchive(activeTasks, id, cutoffIso);
    if (!eligible.length) {
      setError("No tasks in Done have a verified completion timestamp older than 3 days.");
      return;
    }
    const skipped = activeTasks.filter((task) => task.column === id).length - eligible.length;
    if (!window.confirm(
      `Archive ${eligible.length} task(s) moved into "${column.name}" more than 3 days ago?\n\n${skipped} newer, undated, or parent epic task(s) will remain. No newer work or active epic children will be detached. You can undo this.`,
    )) return;

    void run(async () => {
      const before = snapshot;
      // The backend rechecks the fixed cutoff and every revision under one
      // board lock. Any concurrent task change aborts the entire operation.
      const after = await archiveColumnOlderThan(
        id, cutoffIso, eligible.map((task) => [task.id, task.revision]), boardPath,
      );
      pushUndo({ kind: "board", label: "archive Done older than 3 days", before, after });
      applySnapshot(after);
    });
  };

  const openColumnMenu = (event: React.MouseEvent<HTMLElement>, id: string) => {
    event.preventDefault();
    event.stopPropagation();
    setContextMenu(null);
    setColumnContextMenu({
      columnId: id,
      x: Math.max(8, Math.min(event.clientX, window.innerWidth - 252)),
      y: Math.max(8, Math.min(event.clientY, window.innerHeight - 410)),
    });
  };

  const archiveSelected = () => {
    if (editor?.original) void archiveTaskById(editor.original);
  };

  const copyTaskText = async (value: string) => {
    setContextMenu(null);
    try {
      let copied = false;
      if (navigator.clipboard?.writeText) {
        try {
          await navigator.clipboard.writeText(value);
          copied = true;
        } catch {
          // Use the legacy selection fallback if clipboard permission is unavailable.
        }
      }
      if (!copied) {
        const input = document.createElement("textarea");
        input.value = value;
        input.style.position = "fixed";
        input.style.opacity = "0";
        document.body.appendChild(input);
        input.select();
        try {
          copied = document.execCommand("copy");
        } finally {
          input.remove();
        }
      }
      if (!copied) throw new Error("Clipboard access was denied");
      setError("");
    } catch (err) {
      setError(`Could not copy to clipboard: ${errorText(err)}`);
    }
  };

  const contextTask = contextMenu
    ? taskMap.get(contextMenu.taskId)
    : undefined;
  const contextColumn = columnContextMenu
    ? columns.find((column) => column.id === columnContextMenu.columnId)
    : undefined;
  const doneColumn = columns.find((column) => column.id === "done" && column.terminal)
    ?? columns.find((column) => column.terminal);
  const openMoveSubmenu = (event: React.MouseEvent<HTMLButtonElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const width = 240;
    const height = Math.min(window.innerHeight - 16, columns.length * 33 + 14);
    setMoveSubmenu({
      x: rect.right + width + 4 > window.innerWidth - 8
        ? Math.max(8, rect.left - width - 4) : rect.right + 4,
      y: Math.max(8, Math.min(rect.top, window.innerHeight - height - 8)),
    });
  };

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();

      if (event.key === "Escape") {
        if (contextMenu || columnContextMenu) {
          event.preventDefault();
          setContextMenu(null);
          setMoveSubmenu(null);
          setColumnContextMenu(null);
          return;
        }
        if (editor && !busy) {
          event.preventDefault();
          closeEditor();
          return;
        }
      }

      if (!event.ctrlKey || event.altKey) return;

      if (key === "t") {
        event.preventDefault();
        if (!editor) openCreate();
        return;
      }

      if (key === "s" || (event.key === "Enter" && editor)) {
        event.preventDefault();
        if (editor) void saveEditor();
        return;
      }

      // While editing text, leave Ctrl+Z to the native input/textarea undo.
      // Otherwise it reverses the most recent persisted TaskBoard mutation.
      if (key === "z" && !editor) {
        event.preventDefault();
        void undoLast();
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [editor, busy, contextMenu, columnContextMenu, snapshot, boardPath, undoCount]);

  useEffect(() => {
    if (!contextMenu && !columnContextMenu) return;
    const close = () => { setContextMenu(null); setColumnContextMenu(null); };
    window.addEventListener("blur", close);
    return () => window.removeEventListener("blur", close);
  }, [contextMenu, columnContextMenu]);

  if (!snapshot) {
    return (
      <main className="loading-shell">
        <div className="loading-card">
          <strong>TaskBoard</strong>
          <span>{error || "Loading Wartribes tasks…"}</span>
        </div>
      </main>
    );
  }

  return (
    <main
      className="app-shell"
      onMouseDown={() => {
        if (contextMenu) { setContextMenu(null); setMoveSubmenu(null); }
        if (columnContextMenu) setColumnContextMenu(null);
      }}
    >
      <header className="topbar">
        <div>
          <div className="eyebrow">WARTRIBES</div>
          <h1>{snapshot.config.name}</h1>
        </div>
        <div className="toolbar">
          <div className="search-wrap">
            <span>⌕</span>
            <input
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              placeholder="Search tasks"
            />
          </div>
          <button
            type="button"
            className="button user-queue-button"
            onClick={startUserQueue}
            disabled={busy || userQueueTasks.length === 0}
            title={userQueueTasks.length
              ? "Review pending For user tasks, newest first. Snoozed tasks return after 24 hours."
              : "No pending For user tasks (snoozed tasks will return automatically)."}
          >
            User queue <span className="user-queue-count">{userQueueTasks.length}</span>
          </button>
          <button
            className="button secondary epic-collapse-all"
            onClick={toggleAllEpicsCollapsed}
            disabled={busy || !epicIdsWithChildren.length}
            title={allEpicsCollapsed
              ? "Expand all epics to show their tasks again"
              : "Collapse every epic across all columns. Orange For user tasks stay visible."}
            aria-label={allEpicsCollapsed ? "Expand all epics" : "Collapse all epics"}
          >
            <span aria-hidden="true">{allEpicsCollapsed ? "▾" : "▸"}</span>
            {allEpicsCollapsed ? "Expand all epics" : "Collapse all epics"}
          </button>
          <button
            className="button secondary"
            onClick={() => void undoLast()}
            disabled={busy || undoCount === 0 || !!editor}
            title="Undo last TaskBoard action (Ctrl+Z)"
          >
            Undo{undoCount ? ` (${undoCount})` : ""}
          </button>
          <button
            className="button secondary"
            onClick={() => void refresh(boardPath, false)}
            disabled={busy || refreshFeedback === "working"}
            title="Reload the latest tasks from Markdown files"
          >
            {refreshFeedback === "working" ? "Refreshing…" : refreshFeedback === "done" ? "✓ Updated" : "Refresh"}
          </button>
          <button className="button primary" onClick={() => openCreate()} disabled={busy}>
            + Task
          </button>
          <button className="button secondary" onClick={addColumn} disabled={busy}>
            + Column
          </button>
        </div>
      </header>

      <div className="pathbar">
        <span className="status-dot" />
        <span>{boardPath}</span>
        <span className="spacer" />
        <span>{activeTasks.length} active</span>
        <span>{snapshot.tasks.filter((task) => task.archived).length} archived</span>
      </div>

      {error && <div className="error-banner">{error}</div>}
      {!!snapshot.warnings.length && (
        <div className="warning-banner">{snapshot.warnings.join(" • ")}</div>
      )}

      <section
        className="board"
        ref={boardRef}
        onPointerDown={startBoardPan}
        onPointerMove={moveBoardPan}
        onPointerUp={endBoardPan}
        onPointerCancel={endBoardPan}
        onContextMenu={(event) => event.preventDefault()}
      >
        {columns.map((column) => {
          const columnTasks = orderCardsByDependencies(applyCardPlacements(visibleTasks
            .filter((task) => task.column === column.id)
            .sort((a, b) => column.terminal
              ? completedSortTime(b) - completedSortTime(a) || b.id.localeCompare(a.id)
              : attentionRank(a) - attentionRank(b) ||
                hierarchyKey(a).localeCompare(hierarchyKey(b)))));
          const columnStyle = {
            "--column-color": column.color || "#77828f",
          } as CSSProperties;
          const columnActivityKind: TaskActivityKind | undefined =
            activeTasks.some(
              (task) =>
                task.column === column.id && taskActivity[task.id] === "created",
            )
              ? "created"
              : activeTasks.some(
                    (task) =>
                      task.column === column.id &&
                      taskActivity[task.id] === "moved",
                  )
                ? "moved"
                : undefined;
          const columnActivityClass = columnActivityKind
            ? ` activity-${columnActivityKind}`
            : "";

          if (column.hidden && (!column.snoozed_until || Date.parse(column.snoozed_until) > Date.now())) {
            return (
              <button
                className={`hidden-column${column.snoozed_until ? " snoozed" : ""}${columnActivityClass}`}
                key={column.id}
                data-column-id={column.id}
                style={columnStyle}
                title={column.snoozed_until
                  ? `${column.name} — snoozed until ${new Date(column.snoozed_until).toLocaleString()}; click to wake now`
                  : `Show ${column.name}`}
                aria-label={column.snoozed_until ? `Wake ${column.name} column` : `Expand ${column.name} column`}
                onContextMenu={(event) => openColumnMenu(event, column.id)}
                onClick={() => toggleColumnHidden(column.id)}
              >
                {column.snoozed_until && <span className="hidden-column-snooze-icon" aria-hidden="true">☾</span>}
                <span className="hidden-column-label" aria-hidden="true">{column.name}</span>
              </button>
            );
          }

          return (
            <div
              className={`column ${column.terminal ? "terminal" : ""} ${highlightedEpicId && taskMap.get(highlightedEpicId)?.column === column.id ? "epic-proxy-origin-column" : ""} ${
                dragColumnId === column.id ? "column-drag-source" : ""
              }${columnActivityClass}`}
              key={column.id}
              data-column-id={column.id}
              style={columnStyle}
              onContextMenu={(event) => openColumnMenu(event, column.id)}
            >
              <div
                className="column-header"
                onMouseDown={(event) => beginColumnDrag(event, column)}
                title="Drag to reorder"
              >
                <div className="column-title">
                  <span>{column.name}</span>
                  <small>{columnTasks.length}</small>
                  {column.awake_until && Date.parse(column.awake_until) > Date.now() && (
                    <span className="column-awake" title="This column recently woke from snooze">☀</span>
                  )}
                </div>
                <div
                  className="column-actions"
                  onPointerDown={(event) => event.stopPropagation()}
                  onMouseDown={(event) => event.stopPropagation()}
                >
                  {column.id !== "draft" && !column.terminal && (
                    <button
                      className={`column-ai-ready ${column.ai_ready ? "active" : ""}`}
                      title={
                        column.id === "ai-ready"
                          ? "Canonical AI Ready column"
                          : column.ai_ready
                            ? "Tasks in this column are AI-ready"
                            : "Mark this column as AI-ready"
                      }
                      disabled={column.id === "ai-ready"}
                      onClick={() => toggleColumnAiReady(column.id)}
                    >
                      ✓
                    </button>
                  )}
                  <button title="Collapse column" onClick={() => toggleColumnHidden(column.id)}>│</button>
                  <button title="Rename" onClick={() => renameColumn(column.id)}>✎</button>
                  <button title="Delete" onClick={() => deleteColumn(column.id)}>×</button>
                </div>
              </div>

              <div className="cards">
                {withEpicProxyRows(columnTasks, taskMap).map((row, rowIndex) => {
                  if (row.type === "proxy") {
                    const epic = row.epic;
                    const realColumn = columns.find((c) => c.id === epic.column);
                    return (
                      <button
                        type="button"
                        key={`epic-proxy-${epic.id}-${rowIndex}`}
                        className={`epic-proxy${highlightedEpicId === epic.id ? " epic-proxy-active" : ""}`}
                        data-epic-proxy={epic.id}
                        title={`Proxy for ${epic.id}: ${epic.title}. Original in ${realColumn?.name ?? epic.column}. Hover to highlight the original; click to jump to it.`}
                        aria-label={`Epic proxy ${epic.id}: ${epic.title}. Click to jump to the real epic.`}
                        onPointerDown={(event) => event.stopPropagation()}
                        onMouseDown={(event) => event.stopPropagation()}
                        onMouseEnter={() => { setHoverTask(null); setHighlightedEpicId(epic.id); }}
                        onMouseLeave={() => setHighlightedEpicId((current) => current === epic.id ? null : current)}
                        onFocus={() => setHighlightedEpicId(epic.id)}
                        onBlur={() => setHighlightedEpicId((current) => current === epic.id ? null : current)}
                        onClick={() => jumpToOriginalEpic(epic)}
                      >
                        <span className="epic-proxy-id">{epic.id}</span>
                        <span className="epic-proxy-title"><TitleMarkup title={epic.title} /></span>
                        <span className="epic-proxy-jump" aria-hidden="true">↗</span>
                      </button>
                    );
                  }
                  const task = row.task;
                  const depth = depthOf(task);
                  return (
                    <article
                      className={`task-card ${task.kind === "epic" ? "epic-card" : isForUser(task) ? "for-user-card" : ""} ${highlightedEpicId === task.id ? "epic-proxy-highlight" : ""} ${dragId === task.id ? "drag-source" : ""} ${
                        cardDropTargetId === task.id ? (task.kind === "epic" ? "drop-parent-target" : "drop-after-target") : ""
                      } ${
                        taskActivity[task.id]
                          ? `activity-${taskActivity[task.id]}`
                          : ""
                      } ${recentTaskActivity[task.id] ? "recently-changed" : ""}`}
                      key={task.id}
                      data-task-id={task.id}
                      title={recentTaskActivity[task.id]
                        ? `Recently ${recentTaskActivity[task.id].kind} at ${new Date(recentTaskActivity[task.id].at).toLocaleTimeString()} (highlighted for 10 minutes)`
                        : undefined}
                      style={{ marginLeft: Math.min(depth, 4) * 12 }}
                      onMouseDown={(event) => beginCardDrag(event, task)}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        setHoverTask(null);
                        setMoveSubmenu(null);
                        setContextMenu({
                          taskId: task.id,
                          x: Math.min(event.clientX, Math.max(8, window.innerWidth - 238)),
                          y: Math.min(event.clientY, Math.max(8, window.innerHeight - 408)),
                        });
                      }}
                      onMouseEnter={(event) => showHoverDetails(event, task)}
                      onMouseLeave={() => setHoverTask(null)}
                    >
                      <div className="task-title-row">
                        {isForUser(task) && <span className="for-user-badge" title="Requires user input. Legacy NeedsUser flags are displayed here too.">FOR USER</span>}
                        <span className="task-card-id" title={task.kind === "epic" ? "Epic (non-executable)" : isForUser(task) ? "For user (human input)" : "Executable task"}>{task.id}</span>
                        {task.kind === "epic" && childCounts.has(task.id) && (
                          <button
                            className="task-collapse epic-collapse-toggle"
                            draggable={false}
                            title={
                              collapsedTaskIds.has(task.id)
                                ? "Expand epic to show its tasks"
                                : "Collapse epic tasks; orange For user tasks remain visible"
                            }
                            aria-label={`${collapsedTaskIds.has(task.id) ? "Expand" : "Collapse"} epic ${task.id}`}
                            aria-expanded={!collapsedTaskIds.has(task.id)}
                            onPointerDown={(event) => event.stopPropagation()}
                            onMouseDown={(event) => event.stopPropagation()}
                            onDragStart={(event) => event.preventDefault()}
                            onClick={(event) => {
                              event.stopPropagation();
                              toggleTaskCollapsed(task.id);
                            }}
                          >
                            {collapsedTaskIds.has(task.id) ? "▸" : "▾"}
                          </button>
                        )}
                        {task.icon && (
                          <span className={`task-icon ${task.icon}`} title={TASK_ICON_OPTIONS.find((item) => item.value === task.icon)?.label}>
                            <TaskIconGlyph icon={task.icon} />
                          </span>
                        )}
                        <h3><TitleMarkup title={task.title} /></h3>
                        {task.kind === "task" && !isForUser(task) && (
                          <span className={`readiness-badge ${readinessOf(task)}`}
                            title={readinessOf(task) === "waiting" ? `Waiting for ${waitingFor(task).join(", ")}` : readinessOf(task) === "ready" ? "Ready to start" : "Blocked"}
                            aria-label={readinessOf(task) === "ready" ? "Ready to start" : readinessOf(task) === "waiting" ? `Waiting for ${waitingFor(task).join(", ")}` : "Blocked"}>
                            {readinessOf(task) === "waiting" ? `WAIT ${waitingFor(task).length}` : readinessOf(task) === "blocked" ? "BLOCKED" : "\u25B6"}
                          </span>
                        )}
                        {!!task.attention.length && (
                          <div className="task-attention-icons">
                            {task.attention.includes("question") && (
                              <span className="attention-icon question" title="Question">?</span>
                            )}
                            {task.attention.includes("user-note") && (
                              <span className="attention-icon user-note" title="User note for AI">✎</span>
                            )}
                            {task.attention.includes("blocked") && (
                              <span className="attention-icon blocked" title="Blocked">×</span>
                            )}
                          </div>
                        )}
                      </div>
                      {task.kind === "epic" && (() => {
                        const status = epicProgress(task);
                        return <div className="epic-progress" title={`${status.done}/${status.total} completed  /  ${status.inProgress} in progress  /  ${status.blocked} waiting/blocked`}>
                          <div className="epic-progress-track"><span style={{ width: status.total ? `${100 * status.done / status.total}%` : "0%" }} /></div>
                          <small>{status.done}/{status.total} done  /  {status.inProgress} active  /  {status.blocked} waiting</small>
                        </div>;
                      })()}
                    </article>
                  );
                })}
              </div>

              <button className="quick-add" onClick={() => openCreate(column.id)}>
                + Add task
              </button>
            </div>
          );
        })}
      </section>

      {!editor && <DependencyArrows boardRef={boardRef} visibleTasks={visibleTasks}
        columns={columns} layoutKey={dependencyLayoutKey} />}

      {columnDragPreview && (
        <div
          className="column-drag-preview"
          style={{
            left: columnDragPreview.x,
            top: columnDragPreview.y,
            width: columnDragPreview.width,
          }}
        >
          <strong>{columnDragPreview.name}</strong>
        </div>
      )}

      {cardDragPreview && (
        <div
          className="card-drag-preview"
          style={{
            left: cardDragPreview.x,
            top: cardDragPreview.y,
            width: cardDragPreview.width,
          }}
        >
          <div className="task-title-row">
            <h3><TitleMarkup title={cardDragPreview.title} /></h3>
          </div>
        </div>
      )}

      {hoverTask && !cardDragPreview && !contextMenu && (
        <aside
          className="task-hover-preview"
          style={{ left: hoverTask.x, top: hoverTask.y }}
          aria-label={`Task details: ${hoverTask.task.id}`}
        >
          <div className="hover-preview-heading">
            {hoverTask.task.icon && (
              <span className={`task-icon ${hoverTask.task.icon}`}>
                <TaskIconGlyph icon={hoverTask.task.icon} size={22} />
              </span>
            )}
            <div>
              <span className="hover-task-id">{hoverTask.task.kind === "epic" ? "EPIC  /  " : isForUser(hoverTask.task) ? "FOR USER  /  " : ""}{hoverTask.task.id}</span>
              <h3><TitleMarkup title={hoverTask.task.title} /></h3>
            </div>
          </div>
          <div className="hover-preview-meta">
            <span>{columns.find((column) => column.id === hoverTask.task.column)?.name ?? hoverTask.task.column}</span>
            {hoverTask.task.category && <span>{hoverTask.task.category}</span>}
            {hoverTask.task.parent && <span>Subtask of {hoverTask.task.parent}</span>}
            {hoverTask.task.claimed_by && <span>Claimed: {hoverTask.task.claimed_by}</span>}
            {hoverTask.task.kind === "epic" && <span>Coordination only  -  cannot be claimed or moved</span>}
            {hoverTask.task.kind === "task" && !isForUser(hoverTask.task) && <span>Readiness: {readinessOf(hoverTask.task)}</span>}
            {isForUser(hoverTask.task) && <span>FOR USER — awaiting user input.</span>}
          </div>
          {!!hoverTask.task.depends_on.length && <section className="hover-preview-section"><strong>Hard prerequisites</strong>
            <div className="task-links">{hoverTask.task.depends_on.map((id) =>
              <button key={id} className={isComplete(id) ? "satisfied" : "waiting"} title="Open prerequisite task"
                onClick={() => { const match = taskMap.get(id); if (match) openEdit(match); }}>
                {id} {isComplete(id) ? "(done)" : "(waiting)"}
              </button>)}</div></section>}
          {!!hoverTask.task.related_to.length && <section className="hover-preview-section"><strong>Related tasks</strong>
            <div className="task-links">{hoverTask.task.related_to.map((id) =>
              <button key={id} title="Open related task"
                onClick={() => { const match = taskMap.get(id); if (match) openEdit(match); }}>{id}</button>)}</div></section>}
          {hoverTask.task.kind === "epic" && <section className="hover-preview-section"><strong>Epic progress</strong>
            <p>{epicProgress(hoverTask.task).done}/{epicProgress(hoverTask.task).total} executable tasks complete</p></section>}
          {hoverTask.task.description.trim() && (
            <section className="hover-preview-section">
              <strong>Task description</strong>
              <p>{parseDescriptionTimeline(hoverTask.task.description).initial}</p>
              {parseDescriptionTimeline(hoverTask.task.description).updates.map((update, index) => (
                <div className="hover-description-update" key={index}>
                  <small>{update.kind === "user-note" ? "Note to AI" :
                    update.kind === "legacy-ai" ? "Imported AI notes" : "Task update"} · {new Date(update.at).toLocaleString()}</small>
                  <p>{update.text}</p>
                </div>
              ))}
            </section>
          )}
          {!!hoverTask.task.acceptance_criteria.length && (
            <section className="hover-preview-section">
              <strong>Acceptance criteria</strong>
              <ul>{hoverTask.task.acceptance_criteria.map((criterion, index) => <li key={index}>{criterion}</li>)}</ul>
            </section>
          )}
          {hoverTask.task.question && (
            <section className="hover-preview-section">
              <strong>Question {hoverTask.task.question.answer ? "(answered)" : "(open)"}</strong>
              <p>{hoverTask.task.question.prompt}</p>
            </section>
          )}
          {hoverTask.task.user_notes.trim() && (
            <section className="hover-preview-section user">
              <strong>User notes</strong>
              <p>{hoverTask.task.user_notes}</p>
            </section>
          )}
          {hoverTask.task.ai_notes.trim() && (
            <section className="hover-preview-section ai">
              <strong>Historical AI notes (moved to timeline on next save)</strong>
              <p>{hoverTask.task.ai_notes}</p>
            </section>
          )}
          {hoverTask.task.notes.trim() && (
            <section className="hover-preview-section">
              <strong>Notes</strong>
              <p>{hoverTask.task.notes}</p>
            </section>
          )}
        </aside>
      )}

      {contextMenu && contextTask && (
        <div
          className="task-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <button
            onClick={() => {
              setContextMenu(null);
              openEdit(contextTask);
            }}
          >
            Edit task
          </button>
          {contextTask.kind === "epic" && <button
            onClick={() => {
              setContextMenu(null);
              openCreate(defaultCreateColumn(), contextTask.id, contextTask.category);
            }}
          >
            New task under epic
          </button>}
          {contextTask.parent && (
            <button
              onClick={() => {
                setContextMenu(null);
                void persistTaskMutation(
                  contextTask.id,
                  "remove parent",
                  (task) => ({ ...task, parent: null }),
                );
              }}
            >
              Remove from parent
            </button>
          )}
          {contextTask.kind !== "epic" && (
            <>
              <div className="context-separator" />
              <button
                disabled={busy || !doneColumn || contextTask.column === doneColumn.id}
                onClick={() => {
                  if (!doneColumn) return;
                  setContextMenu(null);
                  setMoveSubmenu(null);
                  void moveTaskTo(contextTask.id, doneColumn.id);
                }}
              >Move to done</button>
              <button
                className="context-submenu-trigger"
                aria-haspopup="menu"
                aria-expanded={!!moveSubmenu}
                disabled={busy}
                onMouseEnter={openMoveSubmenu}
                onClick={openMoveSubmenu}
              >Move to... <span aria-hidden="true">▸</span></button>
            </>
          )}
          <div className="context-separator" />
          <button onClick={() => void copyTaskText(contextTask.id)}>Copy task ID</button>
          <button onClick={() => void copyTaskText(`${contextTask.id} ${contextTask.title}`)}>Copy task ID + title</button>
          <button onClick={() => void copyTaskText(contextTask.path)}>Copy full file path</button>
          <button
            onClick={() => void copyTaskText(contextTask.kind === "epic"
                ? `In Wartribes, review epic ${contextTask.id} and choose a ready executable subtask. Do not implement or claim the epic itself.`
                : contextTask.kind === "for_user"
                  ? `Review human-only task ${contextTask.id} in TaskBoard, prepare useful context, then ask the user to take the required action. Do not claim, complete, or archive it on their behalf.`
                  : `In Wartribes, implement ${contextTask.id} from the TaskBoard.`)}
            title="Copy a ready-to-paste implementation request for an agent"
          >
            Copy agent execute message
          </button>
          <div className="context-separator" />
          <button
            onClick={() => {
              setContextMenu(null);
              void run(async () => {
                await highlightInFomni(contextTask.path);
              });
            }}
          >
            Highlight in Fomni
          </button>
          <button
            onClick={() => {
              setContextMenu(null);
              void run(async () => {
                await openInObsidian(contextTask.path);
              });
            }}
          >
            Open file in Obsidian
          </button>
          <div className="context-separator" />
          <button
            className="danger"
            onClick={() => void archiveTaskById(contextTask)}
          >
            Archive task
          </button>
        </div>
      )}

      {contextMenu && contextTask && contextTask.kind !== "epic" && moveSubmenu && (
        <div
          className="task-context-menu task-move-submenu"
          role="menu"
          aria-label="Move to column"
          style={{ left: moveSubmenu.x, top: moveSubmenu.y }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <div className="context-menu-label">Move {contextTask.id} to</div>
          {columns.map((column) => (
            <button
              key={column.id}
              role="menuitem"
              disabled={busy || contextTask.column === column.id}
              onClick={() => {
                setContextMenu(null);
                setMoveSubmenu(null);
                void moveTaskTo(contextTask.id, column.id);
              }}
            >{column.name}{contextTask.column === column.id ? " ✓" : ""}</button>
          ))}
        </div>
      )}

      {columnContextMenu && contextColumn && (
        <div
          className="task-context-menu column-context-menu"
          style={{ left: columnContextMenu.x, top: columnContextMenu.y }}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <div className="context-menu-label">{contextColumn.name}</div>
          <button onClick={() => {
            setColumnContextMenu(null);
            toggleColumnHidden(contextColumn.id);
          }}>
            {contextColumn.hidden ? "Expand column" : "Collapse column"}
          </button>
          <button onClick={() => {
            setColumnContextMenu(null);
            renameColumn(contextColumn.id);
          }}>Rename column</button>
          <div className="context-separator" />
          <div className="context-menu-label">Snooze column</div>
          <button onClick={() => snoozeColumn(contextColumn.id, "day")}>For 1 day</button>
          <button onClick={() => snoozeColumn(contextColumn.id, "three-days")}>For 3 days</button>
          <button onClick={() => snoozeColumn(contextColumn.id, "week")}>For 1 week</button>
          <button onClick={() => snoozeColumn(contextColumn.id, "month")}>For 1 month</button>
          {contextColumn.snoozed_until && (
            <button onClick={() => {
              setColumnContextMenu(null);
              toggleColumnHidden(contextColumn.id);
            }}>Cancel snooze / wake now</button>
          )}
          <div className="context-separator" />
          {contextColumn.terminal && (
            <button
              className="archive-age-option"
              disabled={busy}
              title="Archive only tasks that entered Done more than 72 hours ago. Recent or undated tasks remain, and younger child tasks are not detached from their epic."
              onClick={() => archiveDoneOlderThanThreeDays(contextColumn.id)}
            >
              Archive older than 3 days
            </button>
          )}
          <button
            className="danger"
            disabled={!activeTasks.some((task) => task.column === contextColumn.id) || busy}
            onClick={() => archiveAllInColumn(contextColumn.id)}
          >
            Archive all ({activeTasks.filter((task) => task.column === contextColumn.id).length})
          </button>
        </div>
      )}

      {editor && (
        <div className="modal-backdrop" onMouseDown={() => !busy && closeEditor()}>
          <section className="task-modal" onMouseDown={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <div className="eyebrow">
                {editor.mode === "edit" ? editor.original?.id : "NEW TASK"}
                {userQueueReview && editor.original?.id === userQueueReview.ids[userQueueReview.index] && (
                  <span className="user-queue-progress">
                    USER QUEUE · {userQueueReview.index + 1} / {userQueueReview.ids.length}
                  </span>
                )}
              </div>
              <div className="task-kind-switch" role="group" aria-label="Card type">
                <div className="task-kind-switch-options">
                  <button type="button"
                    className={`kind-option${editor.draft.kind !== "epic" ? " active" : ""}`}
                    aria-pressed={editor.draft.kind !== "epic"}
                    disabled={editor.mode === "edit" && editor.original?.kind === "epic" && childCounts.has(editor.original.id)}
                    onClick={() => chooseDraftKind("task")}>Task</button>
                  <button type="button"
                    className={`kind-option${editor.draft.kind === "epic" ? " active epic" : ""}`}
                    aria-pressed={editor.draft.kind === "epic"}
                    onClick={() => chooseDraftKind("epic")}>Epic</button>
                </div>
                {editor.draft.kind !== "epic" && <button type="button"
                  className={`kind-user-option${isForUser(editor.draft) ? " active" : ""}`}
                  aria-pressed={isForUser(editor.draft)}
                  onClick={() => chooseDraftKind(editor.draft.kind === "for_user" || editor.draft.attention.includes("needs-user") ? "task" : "for_user")}>
                    {isForUser(editor.draft) ? "●" : "○"} For user
                </button>}
              </div>
              <button className="icon-button" aria-label="Close editor" onClick={() => closeEditor()} disabled={busy}>×</button>
            </div>

            <input
              className="task-title-input"
              autoFocus
              aria-label="Title"
              placeholder={editor.draft.kind === "epic" ? "Epic title" : "Task title"}
              value={editor.draft.title}
              onChange={(event) =>
                setEditor({ ...editor, draft: { ...editor.draft, title: event.currentTarget.value } })
              }
            />

            {editor.mode === "edit" &&
              editor.original &&
              editor.draft.kind !== "epic" &&
              editor.draft.attention.includes("question") &&
              editor.draft.question &&
              !editor.draft.question.answer && (
                <section className="question-panel">
                  <div className="question-panel-label">Question for you</div>
                  <h3>{editor.draft.question.prompt || "Question not yet written"}</h3>
                  {!!editor.draft.question.options.length && (
                    <div className="question-options">
                      {editor.draft.question.options.map((option) => (
                        <button
                          className="question-option"
                          key={option}
                          disabled={busy}
                          onClick={() => void answerQuestion(option)}
                        >
                          {option}
                        </button>
                      ))}
                    </div>
                  )}
                  <div className="question-freeform">
                    <input
                      placeholder="Or type an answer…"
                      value={questionAnswerText}
                      onChange={(event) =>
                        setQuestionAnswerText(event.currentTarget.value)
                      }
                      onKeyDown={(event) => {
                        if (event.key === "Enter" && questionAnswerText.trim()) {
                          event.preventDefault();
                          void answerQuestion(questionAnswerText);
                        }
                      }}
                    />
                    <button
                      className="button primary"
                      disabled={busy || !questionAnswerText.trim()}
                      onClick={() => void answerQuestion(questionAnswerText)}
                    >
                      Answer
                    </button>
                  </div>
                </section>
              )}

            {editor.draft.kind !== "epic" && editor.draft.question?.answer && (
              <section className="answered-question">
                <span>Answered question</span>
                <strong>{editor.draft.question.prompt}</strong>
                <p>{editor.draft.question.answer}</p>
              </section>
            )}

            <div className={`form-grid ${editor.draft.kind === "epic" ? "two" : "three"}`}>
              <label>
                Column
                <select
                  disabled={editor.draft.kind === "epic"}
                  title={editor.draft.kind === "epic" ? "Epic status follows its subtasks automatically" : undefined}
                  value={editor.draft.column}
                  onChange={(event) =>
                    setEditor({ ...editor, draft: { ...editor.draft, column: event.currentTarget.value } })
                  }
                >
                  {columns.map((column) => (
                    <option value={column.id} key={column.id}>{column.name}</option>
                  ))}
                </select>
              </label>
              {editor.draft.kind !== "epic" && <label>
                Epic
                <select
                  value={editor.draft.parent ?? ""}
                  onChange={(event) =>
                    setEditor({
                      ...editor,
                      draft: { ...editor.draft, parent: event.currentTarget.value || null },
                    })
                  }
                >
                  <option value="">No epic</option>
                  {activeTasks
                    .filter((task) => task.kind === "epic" && task.id !== editor.original?.id)
                    .sort((a, b) => a.title.localeCompare(b.title))
                    .map((task) => (
                      <option value={task.id} key={task.id}>{task.id} — {task.title}</option>
                    ))}
                </select>
              </label>}
              <label>
                Category
                <input
                  list="task-categories"
                  placeholder="Optional"
                  value={editor.draft.category ?? ""}
                  onChange={(event) =>
                    setEditor({
                      ...editor,
                      draft: {
                        ...editor.draft,
                        category: event.currentTarget.value.trim() || null,
                      },
                    })
                  }
                />
                <datalist id="task-categories">
                  {categories.map((category) => (
                    <option value={category} key={category} />
                  ))}
                </datalist>
              </label>
            </div>

            {editor.draft.kind === "epic" && (
              <div className="epic-editor-note">EPIC — not executable or assignable. Its board column is automatically derived from its subtasks.</div>
            )}
            <div className="form-grid">
              {editor.draft.kind !== "epic" && <TaskLinkPicker
                key={`${editor.original?.id ?? "new"}-depends`}
                label="Depends on (hard prerequisites)"
                tasks={snapshot?.tasks ?? []}
                excludeId={editor.original?.id}
                allowEpics={false}
                values={editor.draft.depends_on}
                onChange={(depends_on) => setEditor((current) => current
                  ? { ...current, draft: { ...current.draft, depends_on } } : current)}
              />}
              <TaskLinkPicker
                key={`${editor.original?.id ?? "new"}-related`}
                label="Related to (informational)"
                tasks={snapshot?.tasks ?? []}
                excludeId={editor.original?.id}
                values={editor.draft.related_to}
                onChange={(related_to) => setEditor((current) => current
                  ? { ...current, draft: { ...current.draft, related_to } } : current)}
              />
            </div>

            {editor.draft.kind !== "epic" && <div className="task-meta-row">
              <div className="task-icon-editor">
                <span>Icon</span>
                <div className="task-icon-picker">
                  <button
                    type="button"
                    title="No icon"
                    aria-label="No icon"
                    className={`icon-choice ${!editor.draft.icon ? "selected" : ""}`}
                    aria-pressed={!editor.draft.icon}
                    onClick={() => setEditor({ ...editor, draft: { ...editor.draft, icon: null } })}
                  >
                    <span className="no-icon-mark">—</span>
                  </button>
                  {TASK_ICON_OPTIONS.map(({ value, label }) => (
                    <button
                      type="button"
                      key={value}
                      title={label}
                      aria-label={label}
                      className={`icon-choice ${value} ${editor.draft.icon === value ? "selected" : ""}`}
                      aria-pressed={editor.draft.icon === value}
                      onClick={() => setEditor({ ...editor, draft: { ...editor.draft, icon: value } })}
                    >
                      <TaskIconGlyph icon={value} size={17} />
                    </button>
                  ))}
                </div>
              </div>
              <div className="attention-editor">
                <span>Attention</span>
                <div className="attention-editor-buttons">
                  {(["question", "blocked"] as TaskAttention[]).map((flag) => (
                    <button
                      type="button"
                      key={flag}
                      className={`attention-toggle ${flag} ${
                        editor.draft.attention.includes(flag) ? "active" : ""
                      }`}
                      onClick={() => toggleAttentionDraft(flag)}
                    >
                      {flag === "question" ? "? Question" : "× Blocked"}
                    </button>
                  ))}
                </div>
              </div>
            </div>}

            {editor.draft.kind !== "epic" && editor.draft.attention.includes("question") &&
              editor.draft.question &&
              !editor.draft.question.answer && (
                <section className="question-editor">
                  <label>
                    Question
                    <input
                      value={editor.draft.question.prompt}
                      placeholder="What do you need the user to answer?"
                      onChange={(event) =>
                        setEditor({
                          ...editor,
                          draft: {
                            ...editor.draft,
                            question: {
                              ...editor.draft.question!,
                              prompt: event.currentTarget.value,
                            },
                          },
                        })
                      }
                    />
                  </label>
                  <label>
                    Multiple-choice options
                    <AutoTextarea
                      minRows={2}
                      placeholder={"Optional — one option per line"}
                      value={editor.draft.question.options.join("\n")}
                      onChange={(event) =>
                        setEditor({
                          ...editor,
                          draft: {
                            ...editor.draft,
                            question: {
                              ...editor.draft.question!,
                              // Raw lines while typing; trimmed on save.
                              options: event.currentTarget.value.split("\n"),
                            },
                          },
                        })
                      }
                    />
                  </label>
                </section>
              )}

            <DescriptionTimeline
              key={editor.original?.id ?? "new-task"}
              description={editor.draft.description}
              legacyAiNotes={editor.draft.ai_notes}
              pendingUpdate={descriptionUpdateInput}
              onPendingUpdateChange={setDescriptionUpdateInput}
              onChange={(description) =>
                setEditor((current) => current
                  ? { ...current, draft: { ...current.draft, description } }
                  : current)}
              {...(editor.draft.kind !== "epic" && {
                noteToAi: editor.draft.user_notes,
                onNoteToAiChange: (note: string) => setEditor((current) => {
                  if (!current) return current;
                  const releaseToAI = note.trim().length > 0;
                  return {
                    ...current,
                    draft: {
                      ...current.draft,
                      user_notes: note,
                      kind: releaseToAI && current.draft.kind === "for_user"
                        ? "task" : current.draft.kind,
                      attention: releaseToAI
                        ? current.draft.attention.filter((flag) => flag !== "needs-user")
                        : current.draft.attention,
                    },
                  };
                }),
              })}
            />

            {editor.draft.kind === "epic" && <label>
              User design and coordination notes
              <AutoTextarea
                minRows={2}
                placeholder="User-authored scope decisions, design priorities, coordination guidance…"
                value={editor.draft.user_notes}
                onChange={(event) => setEditor({ ...editor, draft: { ...editor.draft, user_notes: event.currentTarget.value } })}
              />
            </label>}

            {editor.draft.kind === "epic" && <label>
              Epic planning and coordination notes
              <AutoTextarea
                minRows={4}
                placeholder="Design decisions, scope, references and coordination notes. No implementation checklist or AI work log."
                value={editor.draft.notes}
                onChange={(event) => setEditor({ ...editor, draft: { ...editor.draft, notes: event.currentTarget.value } })}
              />
            </label>}

            {editor.draft.kind !== "epic" && <label>
              Acceptance criteria
              <AutoTextarea
                minRows={2}
                placeholder="One criterion per line"
                value={editor.draft.acceptance_criteria.join("\n")}
                onChange={(event) =>
                  setEditor({
                    ...editor,
                    draft: {
                      ...editor.draft,
                      // Raw lines while typing; trimmed on save.
                      acceptance_criteria: event.currentTarget.value.split("\n"),
                    },
                  })
                }
              />
            </label>}

            <div className="modal-actions">
              {editor.mode === "edit" && editor.original && (
                <>
                  {editor.original.kind === "epic" && <button
                    className="button secondary"
                    onClick={() => openCreate(
                      defaultCreateColumn(),
                      editor.original!.id,
                      editor.draft.category,
                    )}
                    disabled={busy}
                  >
                    + Task under epic
                  </button>}
                  <button className="button danger" onClick={archiveSelected} disabled={busy}>
                    Archive
                  </button>
                  {editor.original.kind !== "epic" && editor.draft.kind !== "epic" && (
                    <button type="button" className="button done-button"
                      title="Save this task and move it to Done"
                      onClick={() => saveEditor(true)}
                      disabled={busy || !doneColumn || editor.original.column === doneColumn.id ||
                        !editor.draft.title.trim() ||
                        (editor.draft.attention.includes("question") && !editor.draft.question?.prompt.trim())}>
                      Done
                    </button>
                  )}
                </>
              )}
              <span className="spacer" />
              {userQueueReview && editor.original?.id === userQueueReview.ids[userQueueReview.index] && (
                <>
                  <button type="button" className="button secondary queue-skip-button"
                    title="Leave this task unchanged and review the next task"
                    onClick={skipUserQueueTask} disabled={busy}>
                    Skip
                  </button>
                  <button type="button" className="button queue-snooze-button"
                    title="Leave this task unchanged, hide it from User queue for 24 hours, and review the next task"
                    onClick={snoozeUserQueueTask} disabled={busy}>
                    Snooze 24h
                  </button>
                </>
              )}
              <button className="button secondary" onClick={() => closeEditor()} disabled={busy}>
                Cancel
              </button>
              <button
                className="button primary"
                title="Save (Ctrl+S or Ctrl+Enter)"
                onClick={() => saveEditor()}
                disabled={
                  busy ||
                  !editor.draft.title.trim() ||
                  (editor.draft.attention.includes("question") &&
                    !editor.draft.question?.prompt.trim())
                }
              >
                {busy ? "Saving…" : "Save"}
              </button>
              {editor.mode === "edit" && editor.original?.kind !== "epic" &&
                editor.draft.kind !== "epic" && (
                <button
                  className="button followup-button"
                  title="Save this task, move it to Done, and open a new related follow-up in the original column."
                  onClick={closeAndFollowup}
                  disabled={busy || !doneColumn || editor.original?.column === doneColumn.id ||
                    !editor.draft.title.trim() ||
                    (editor.draft.attention.includes("question") && !editor.draft.question?.prompt.trim())}
                >
                  Close &amp; Followup
                </button>
              )}
            </div>
          </section>
        </div>
      )}
    </main>
  );
}

export default App;
