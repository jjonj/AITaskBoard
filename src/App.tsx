import { ToolbarIcon } from "./ToolbarIcons";
import { useLoggedError } from "./errorLog";
import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import "./App.css";
import { TASK_ICON_OPTIONS, TaskIconGlyph } from "./TaskIcons";
import { TitleMarkup } from "./TitleMarkup";
import { DependencyArrows } from "./DependencyArrows";
import { TaskLinkPicker } from "./TaskLinkPicker";
import { AutoTextarea, DescriptionTimeline } from "./TaskDescriptionPanel";
import { MarkdownText, MarkdownPreviewToggle } from "./MarkdownText";
import { TaskImageGallery, type PendingClipboardImage } from "./TaskImageGallery";
import { ReviewView } from "./ReviewView";
import { isUserQueueEligible } from "./userQueueEligibility";
import { attachImagesToDescription, parseDescriptionTimeline, prepareTaskDraftForSave, updateInitialDescription } from "./descriptionTimeline";
import { applyCardPlacements, orderCardsByDependencies } from "./dependencyLayout";
import { withEpicProxyRows } from "./epicProxyRows";
import { eligibleForAgeArchive } from "./archiveAge";
import { isHiddenByCollapsedEpic, toggleAllEpicCollapses } from "./epicCollapse";
import { ProjectTabs } from "./ProjectTabs";
import { ReleaseCenter } from "./ReleaseCenter";
import { QueueOverviewPanel } from "./QueueOverviewPanel";
import { TaskTracePanel } from "./TaskTracePanel";
import { StaleBlockerPanel } from "./StaleBlockerPanel";
import { AgentRecoverySettings } from "./AgentRecoverySettings";
import { TaskFilterPanel, type TaskFilterSelection } from "./TaskFilterPanel";
import { mergeTaskEdits } from "./taskMerge";
import type { ProjectTab, TaskIdReservation, ExecutionMode, QueueOverview, QueueCapacity, ClaimHealth, StaleBlocker } from "./api";
import {
  archiveTask,
  archiveColumn,
  archiveColumnOlderThan,
  createTask,
  reserveTaskId,
  releaseTaskId,
  readLocalImage,
  saveTaskImage,
  highlightInFomni,
  loadBoard,
  listProjects,
  openInObsidian,
  openTaskSession,
  startTaskPreparation,
  notifyTaskPreparationSaved,
  taskLiveWithMe,
  restoreSnapshot,
  saveBoardConfig,
  showMainWindow,
  taskExecutionIntent,
  taskExecutionModes,
  taskSetExecutionMode,
  taskSyncQueue,
  taskQueueOverview,
  taskQueueCapacity,
  taskClaimHealth,
  taskOpenOriginSession,
  taskHandoffBundle,
  taskEnqueueAgentNotice,
  taskReconcileAgentNotices,
  taskStaleBlockers,
  taskAcknowledgeStaleBlocker,
  taskQueueReconcile,
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

const QueueArrows = () => <svg viewBox="0 0 24 24" width="17" height="17"
  aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.9"
  strokeLinecap="round" strokeLinejoin="round">
  <path d="M4 10a8 8 0 0 1 13-5M17 2v4h-4" />
  <path d="M20 14a8 8 0 0 1-13 5M7 22v-4h4" />
</svg>;

const USER_QUEUE_SNOOZES_KEY = "taskboard.userQueueSnoozes";
const USER_QUEUE_SNOOZE_MS = 24 * 60 * 60 * 1000;

const activeUserQueueSnoozes = (raw: unknown, now = Date.now()): UserQueueSnoozes => {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw).filter(([id, until]) =>
      /^[A-Z][A-Z0-9]{1,11}-\d+$/.test(id) && typeof until === "number" &&
      Number.isFinite(until) && until > now),
  );
};

const loadUserQueueSnoozes = (key = USER_QUEUE_SNOOZES_KEY): UserQueueSnoozes => {
  try {
    return activeUserQueueSnoozes(JSON.parse(window.localStorage.getItem(key) || "{}"));
  } catch {
    return {};
  }
};

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

const loadRecentTaskActivity = (key = RECENT_ACTIVITY_STORAGE_KEY): Record<string, RecentTaskActivity> => {
  try {
    const stored: unknown = JSON.parse(window.localStorage.getItem(key) || "{}");
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
  deployable: null,
  repeat_every_days: null,
  next_due: null,
  blocked_reason: null,
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

type TaskDumpRow = { line: number; text: string };
// Keep every source line intact. The optional description contains only the
// overflow, never a paraphrase that might lose a requirement.
const splitTaskDumpText = (source: string): { title: string; description: string } => {
  const text = source.trim().replace(/\s+/g, " ");
  if (text.length <= 105) return { title: text, description: "" };
  const sentence = text.slice(40, 110).search(/[.!?]\s/);
  const sentenceEnd = sentence >= 0 ? sentence + 41 : -1;
  const wordEnd = text.lastIndexOf(" ", 105);
  const cut = sentenceEnd >= 45 ? sentenceEnd : wordEnd >= 45 ? wordEnd : 105;
  return {
    title: text.slice(0, cut).trim(),
    description: text.slice(cut).trim(),
  };
};

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

function ProjectBoard({ project, onInteractionChange, requestedReviewId, requestedTask }: {
  project: ProjectTab;
  onInteractionChange: (locked: boolean) => void;
  requestedReviewId?: string;
  requestedTask?: { id: string; requestKey: number };
}) {
  const boardPath = project.path;
  // Preserve existing Wartribes UI settings while isolating all other boards.
  // IDs such as WT-0020 are allowed to repeat between unrelated projects.
  const storageKey = (name: string) =>
    project.id === "wartribes" ? `taskboard.${name}` : `taskboard.project.${project.id}.${name}`;
  const recentKey = storageKey("recentTaskActivity");
  const snoozesKey = storageKey("userQueueSnoozes");
  const collapsedKey = storageKey("collapsedTasks");
  const blockedInboxFilterKey = storageKey("hideBlockedInboxTasks");
  const hideWaitingKey = storageKey("hideDependencyWaitingTasks");
  const [snapshot, setSnapshot] = useState<BoardSnapshot | null>(null);
  const [query, setQuery] = useState("");
  const [searchResultsOpen, setSearchResultsOpen] = useState(false);
  const [taskFilters, setTaskFilters] = useState<TaskFilterSelection>({
    icons: [], noIcon: false, columns: [], readiness: [], kinds: [], attention: [],
  });
  const [filterPanelOpen, setFilterPanelOpen] = useState(false);
  const [moreMenuOpen, setMoreMenuOpen] = useState(false);
  const [remoteSearchMatches, setRemoteSearchMatches] = useState<Array<{ id: string; title: string; project: string; path: string }>>([]);
  useEffect(() => {
    const needle = query.trim().toLowerCase();
    if (needle.length < 3) { setRemoteSearchMatches([]); return; }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const catalog = await listProjects();
          const otherBoards = await Promise.all(catalog.projects.filter((item) => item.path !== boardPath)
            .map(async (project) => {
              try {
                const board = await loadBoard(project.path);
                return board.tasks.filter((task) => !task.archived &&
                  `${task.id} ${task.title}`.toLowerCase().includes(needle))
                  .map((task) => ({ id: task.id, title: task.title, project: project.name, path: project.path }));
              } catch { return []; }
            }));
          if (!cancelled) setRemoteSearchMatches(otherBoards.flat().slice(0, 20));
        } catch { if (!cancelled) setRemoteSearchMatches([]); }
      })();
    }, 250);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [query, boardPath]);
  // MISC-0019: a search filter is temporary, not a persistent board state.
  // Restart the idle countdown with each keystroke so active editing is unaffected.
  useEffect(() => {
    if (!query.trim()) return;
    const timer = window.setTimeout(() => setQuery(""), 90_000);
    return () => window.clearTimeout(timer);
  }, [query]);
  const [reviewOpen, setReviewOpen] = useState(false);
  const [queueDrawerOpen, setQueueDrawerOpen] = useState(false);
  const [queueTelemetry, setQueueTelemetry] = useState<QueueOverview | null>(null);
  const [capacityTelemetry, setCapacityTelemetry] = useState<QueueCapacity | null>(null);
  useEffect(() => {
    let alive = true;
    const observe = () => {
      void taskQueueOverview(boardPath).then(value => {
        if (alive) setQueueTelemetry(value);
      }).catch(() => { if (alive) setQueueTelemetry(null); });
      void taskQueueCapacity().then(value => {
        if (alive) setCapacityTelemetry(value);
      }).catch(() => { if (alive) setCapacityTelemetry(null); });
    };
    observe();
    const poll = window.setInterval(observe, 20000);
    return () => { alive = false; window.clearInterval(poll); };
  }, [boardPath]);
  const [queueRecoveryWarning, setQueueRecoveryWarning] = useState("");
  const [queueRecoveryAgentBusy, setQueueRecoveryAgentBusy] = useState(false);
  const [queueSyncStatuses, setQueueSyncStatuses] = useState<Record<string, { status: string; reason: string | null }>>({});
  const [queueReconciliationOffline, setQueueReconciliationOffline] = useState(false);
  useEffect(() => {
    let active = true;
    let busy = false;
    let last = 0;
    const reconcile = () => {
      // A bounded activation/focus pass, not a second dispatcher or polling
      // service. Pre-send UNKNOWN IDs remain lookup-only in the native bridge.
      const now = Date.now();
      if (busy || now - last < 45_000) return;
      last = now;
      busy = true;
      void taskQueueReconcile(boardPath).then(result => {
        if (!active) return;
        setQueueReconciliationOffline(false);
        setQueueSyncStatuses(Object.fromEntries(result.items.map(item =>
          [item.id, { status: item.status, reason: item.reason ?? null }])));
        const unresolved = result.items.filter(item =>
          ["unknown", "conflict", "rejected"].includes(item.status));
        setQueueRecoveryWarning([
          ...unresolved.slice(0, 3).map(item =>
            `${item.id}: ${item.status} — ${item.reason ?? "inspect exact OpenRig receipt"}`),
          ...(unresolved.length > 3 ? [`${unresolved.length - 3} additional unreconciled Queue intents`] : []),
          // Truncated batches and temporarily ineligible queued work are normal.
        ].join(" · "));
      }).catch(() => {
        if (active) {
          // A transient lock or unavailable queue bridge does not invalidate
          // durable Queue intent. Reconcile on the next focus without a banner.
          setQueueReconciliationOffline(true);
          setQueueRecoveryWarning("");
        }
      }).finally(() => { busy = false; });
    };
    reconcile();
    window.addEventListener("focus", reconcile);
    return () => { active = false; window.removeEventListener("focus", reconcile); };
  }, [boardPath]);
  const [reviewNotice, setReviewNotice] = useState("");
  useEffect(() => { if (requestedReviewId) setReviewOpen(true); }, [requestedReviewId]);
  const [hideBlockedInboxTasks, setHideBlockedInboxTasks] = useState(() => {
    try { return window.localStorage.getItem(blockedInboxFilterKey) === "true"; }
    catch { return false; }
  });
  useEffect(() => {
    try { window.localStorage.setItem(blockedInboxFilterKey, String(hideBlockedInboxTasks)); }
    catch { /* This filter remains usable when local storage is unavailable. */ }
  }, [blockedInboxFilterKey, hideBlockedInboxTasks]);
  const [hideWaitingTasks, setHideWaitingTasks] = useState(() => {
    try { return window.localStorage.getItem(hideWaitingKey) !== "false"; }
    catch { return true; }
  });
  useEffect(() => {
    try { window.localStorage.setItem(hideWaitingKey, String(hideWaitingTasks)); }
    catch { /* The board filter remains functional without persistence. */ }
  }, [hideWaitingKey, hideWaitingTasks]);
  const [staleBlockers, setStaleBlockers] = useState<StaleBlocker[]>([]);
  const [staleDrawerOpen, setStaleDrawerOpen] = useState(false);
  const [agentSettingsOpen, setAgentSettingsOpen] = useState(false);
  const [staleAckBusy, setStaleAckBusy] = useState(false);
  useEffect(() => {
    let live = true;
    const refresh = () => void taskStaleBlockers(boardPath)
      .then(items => { if (live) setStaleBlockers(items); })
      .catch(() => { if (live) setStaleBlockers([]); });
    refresh();
    const timer = window.setInterval(refresh, 5 * 60_000);
    return () => { live = false; window.clearInterval(timer); };
  }, [boardPath]);
  const acknowledgeBlocker = async (item: StaleBlocker) => {
    setStaleAckBusy(true);
    try {
      await taskAcknowledgeStaleBlocker(boardPath, item.id, item.signature);
      setStaleBlockers(items => items.filter(candidate => candidate.id !== item.id));
    } catch (error) { setError(`Cannot acknowledge blocker: ${String(error)}`); }
    finally { setStaleAckBusy(false); }
  };
  const [claimHealth, setClaimHealth] = useState<Record<string, ClaimHealth>>({});
  const claimTooltip = (task: Task): string => {
    const h = claimHealth[task.id];
    if (!h) return `Claimed by ${task.claimed_by} · session/task telemetry not verified`;
    const age = (time: string | number | null) => {
      if (!time) return "never verified";
      const ms = typeof time === "number" ? time : Date.parse(time);
      if (!Number.isFinite(ms)) return "invalid timestamp";
      const minutes = Math.max(0, Math.floor((Date.now() - ms) / 60_000));
      return minutes < 60 ? `${minutes}m ago` : `${Math.floor(minutes / 60)}h ${minutes % 60}m ago`;
    };
    return `Claimed: ${h.owner} · ${h.agentState} · task update ${age(h.taskActivityAt)} · T3 last working ${age(h.agentActivityAt)} · T3 observed ${age(h.verifiedAt)}`;
  };
  const [editor, setEditor] = useState<EditorState | null>(null);
  const [prepRequestId, setPrepRequestId] = useState<string | null>(null);
  const prepLaunchRef = useRef<Promise<unknown> | null>(null);
  const prepNotified = useRef(new Set<string>());
  const prepNoticeLastAttempt = useRef(new Map<string, number>());
  const snapshotRef = useRef<BoardSnapshot | null>(null);
  // One throttled refresh per board: backend shares a 30s T3 inventory
  // between claim telemetry and durable notification reconciliation.
  useEffect(() => {
    // Background maintenance should not compete with a task editor's writes.
    // Durable notices resume automatically when the editor closes.
    if (editor) return;
    let active = true;
    let inFlight = false;
    let lastStarted = 0;
    const reconcileSavedPreparation = async () => {
      const now = Date.now();
      const candidate = snapshotRef.current?.tasks.find(task => {
        if (task.archived || !task.attention.includes("prepare") ||
            !task.notes.includes("taskboard:preparation-saved")) return false;
        const requestId = task.notes.match(/taskboard:preparation-request:([a-f0-9-]{36})/)?.[1];
        if (!requestId) return false;
        const key = `${boardPath}#${task.id}#${requestId}`;
        return !prepNotified.current.has(key) &&
          now - (prepNoticeLastAttempt.current.get(key) ?? 0) >= 120_000;
      });
      if (!candidate) return;
      const requestId = candidate.notes.match(/taskboard:preparation-request:([a-f0-9-]{36})/)?.[1];
      if (!requestId) return;
      const key = `${boardPath}#${candidate.id}#${requestId}`;
      prepNoticeLastAttempt.current.set(key, now);
      try {
        await notifyTaskPreparationSaved(boardPath, candidate.id, requestId);
        prepNotified.current.add(key);
      } catch { /* T3 may be temporarily unavailable; retry only the saved notification. */ }
    };
    const refresh = () => {
      if (inFlight || Date.now() - lastStarted < 30_000) return;
      lastStarted = Date.now();
      inFlight = true;
      void taskReconcileAgentNotices(boardPath)
        // Agent notices are durable. A transient board lock is not a UI error;
        // the next background refresh can reconcile them without interrupting edits.
        .catch(() => undefined)
        .then(() => taskClaimHealth(boardPath))
        .then(data => { if (active) setClaimHealth(data); })
        .catch(() => { if (active) setClaimHealth({}); })
        .then(() => reconcileSavedPreparation())
        .finally(() => { inFlight = false; });
    };
    refresh();
    const timer = window.setInterval(refresh, 30_000);
    window.addEventListener("focus", refresh);
    return () => { active = false; window.clearInterval(timer); window.removeEventListener("focus", refresh); };
  }, [boardPath, !!editor]);
  const [notifyAgentOverride, setNotifyAgentOverride] = useState<boolean | null>(null);
  const substantiveClaimedEdit = editor?.mode === "edit" && !!editor.original?.claimed_by &&
    editor.original.column === "in-progress" && (
      editor.draft.description !== editor.original.description ||
      editor.draft.ai_notes !== editor.original.ai_notes ||
      JSON.stringify(editor.draft.acceptance_criteria) !== JSON.stringify(editor.original.acceptance_criteria));
  const notifyAgent = notifyAgentOverride ?? !!substantiveClaimedEdit;
  const showNotifyAgent = editor?.mode === "edit" && !!editor.original?.claimed_by &&
    editor.original.column === "in-progress" && editor.draft.column === "in-progress";
  const [remoteRelatedTasks, setRemoteRelatedTasks] = useState<Task[]>([]);
  useEffect(() => {
    if (!editor) { setRemoteRelatedTasks([]); return; }
    let cancelled = false;
    void (async () => {
      try {
        const catalog = await listProjects();
        const boards = await Promise.all(catalog.projects.filter((item) => item.path !== boardPath)
          .map(async (item) => {
            try {
              const snapshot = await loadBoard(item.path);
              return snapshot.tasks.map((task) => ({
                ...task,
                id: `${item.path}#${task.id}`,
                title: `${item.name} · ${task.title}`,
              }));
            } catch { return [] as Task[]; }
          }));
        if (!cancelled) setRemoteRelatedTasks(boards.flat());
      } catch { if (!cancelled) setRemoteRelatedTasks([]); }
    })();
    return () => { cancelled = true; };
  }, [!!editor, boardPath]);
  const remotePrerequisiteKey = (snapshot?.tasks ?? [])
    .flatMap((task) => task.depends_on).filter((id) => id.includes("#")).sort().join("|");
  const [remotePrerequisites, setRemotePrerequisites] = useState<Map<string, Task>>(() => new Map());
  useEffect(() => {
    let cancelled = false;
    const references = remotePrerequisiteKey ? remotePrerequisiteKey.split("|") : [];
    const refreshRemote = async () => {
      const paths = [...new Set(references.map((id) => id.slice(0, id.lastIndexOf("#"))))];
      const boards = await Promise.all(paths.map(async (path) => {
        try { return { path, board: await loadBoard(path) }; }
        catch { return null; }
      }));
      if (cancelled) return;
      const next = new Map<string, Task>();
      for (const entry of boards) {
        if (!entry) continue;
        for (const task of entry.board.tasks) next.set(`${entry.path}#${task.id}`, task);
      }
      setRemotePrerequisites(next);
    };
    void refreshRemote();
    const timer = window.setInterval(() => void refreshRemote(), 15_000);
    return () => { cancelled = true; window.clearInterval(timer); };
  }, [remotePrerequisiteKey]);
  const [taskDumpOpen, setTaskDumpOpen] = useState(false);
  const [taskDumpText, setTaskDumpText] = useState("");
  const [taskDumpPriorities, setTaskDumpPriorities] = useState<Record<number, boolean>>({});
  const [taskDumpColumn, setTaskDumpColumn] = useState("");
  const [taskDumpEpic, setTaskDumpEpic] = useState("");
  const [taskDumpNewEpicTitle, setTaskDumpNewEpicTitle] = useState("");
  const taskDumpRows = useMemo<TaskDumpRow[]>(() =>
    taskDumpText.split(/\r?\n/).map((text, line) => ({ line, text: text.trim() }))
      .filter((row) => row.text.length > 0), [taskDumpText]);
  const [reservedTaskId, setReservedTaskId] = useState<TaskIdReservation | null>(null);
  const [pendingClipboardImages, setPendingClipboardImages] = useState<PendingClipboardImage[]>([]);
  const imageDialogGeneration = useRef(0);
  const editorModalRef = useRef<HTMLElement | null>(null);
  const autosavingDraft = useRef(false);
  const [autoSavedDraft, setAutoSavedDraft] = useState<{ id: string; title: string } | null>(null);
  useEffect(() => {
    if (!autoSavedDraft) return;
    const timeout = window.setTimeout(() => setAutoSavedDraft(null), 9000);
    return () => window.clearTimeout(timeout);
  }, [autoSavedDraft]);
  const epicSingleClickTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const suppressEpicClickUntil = useRef(0); // swallow the click that ends an epic drag
  const [fileDropActive, setFileDropActive] = useState(false);
  const [doneMenuOpen, setDoneMenuOpen] = useState(false);
  const [copiedTaskId, setCopiedTaskId] = useState<string | null>(null);
  const [executionChoice, setExecutionChoice] = useState<ExecutionMode | "start_now">("backlog");
  const [executionTouched, setExecutionTouched] = useState(false);
  const [executionModes, setExecutionModes] = useState<Record<string, import("./api").ExecutionIntent>>({});
  const pendingTaskStarts = useRef(new Set<string>());
  const [startingTaskIds, setStartingTaskIds] = useState<string[]>([]);
  useEffect(() => {
    if (!snapshot) return;
    let cancelled = false;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let attempts = 0;
    const load = () => {
      void taskExecutionModes(boardPath).then(modes => {
        if (!cancelled) setExecutionModes(modes);
      }).catch(() => {
        // Markdown writers briefly hold the lock. Retain last-known modes
        // and retry; this is not an actionable error for the user.
        if (!cancelled && attempts++ < 4)
          retry = setTimeout(load, Math.min(2000 * attempts, 8000));
      });
    };
    load();
    return () => { cancelled = true; if (retry) clearTimeout(retry); };
  }, [boardPath, snapshot]);
  const [withMeSelected, setWithMeSelected] = useState(false);
  const [executionLoading, setExecutionLoading] = useState(false);
  useEffect(() => {
    setExecutionTouched(false);
    if (!editor || editor.mode === "create" || !editor.original) {
      setWithMeSelected(false);
      setExecutionChoice("backlog");
      setExecutionLoading(false);
      return;
    }
    let cancelled = false;
    setExecutionLoading(true);
    setWithMeSelected(editor.original.live?.with_me === true);
    setExecutionChoice("backlog");
    void taskExecutionIntent(editor.original.id, boardPath).then((intent) => {
      if (!cancelled) setExecutionChoice(intent.mode);
    }).catch((err) => {
      if (!cancelled) setError(`Cannot read execution mode: ${errorText(err)}`);
    }).finally(() => { if (!cancelled) setExecutionLoading(false); });
    return () => { cancelled = true; };
  }, [boardPath, editor?.mode, editor?.original?.id]);
  // Creating in an auto-queue column cannot opt out into Backlog. Keep Start now
  // available, and fall back to Queue if the column changes after a Backlog choice.
  const creatingInAutoQueueColumn = editor?.mode === "create" &&
    snapshot?.config.columns.some(col => col.id === editor.draft.column && col.ai_ready && col.auto_queue) === true;
  const editorQueueChoice = (creatingInAutoQueueColumn && executionChoice === "backlog") ||
    (!executionTouched && editor && editor.draft.attention.includes("priority") &&
      !editor.original?.attention.includes("priority")) ? "queue" : executionChoice;
  const [releasesOpen, setReleasesOpen] = useState(false);
  const [userQueueReview, setUserQueueReview] = useState<UserQueueReview | null>(null);
  // A full editor opened from Inbox returns to the same feedback queue on Cancel or Save.
  const [inboxEditorTaskId, setInboxEditorTaskId] = useState<string | null>(null);
  const [inboxResumeId, setInboxResumeId] = useState<string | undefined>(undefined);
  const [queueExpanded, setQueueExpanded] = useState(false);
  const [followupQueueReturn, setFollowupQueueReturn] = useState<UserQueueReview | null>(null);
  const [userQueueSnoozes, setUserQueueSnoozes] = useState<UserQueueSnoozes>(() => loadUserQueueSnoozes(snoozesKey));
  const [keyboardSelectedTaskId, setKeyboardSelectedTaskId] = useState<string | null>(null);
  const [dragId, setDragId] = useState<string | null>(null);
  const [dragSourceColumn, setDragSourceColumn] = useState<string | null>(null);
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
      const stored = window.localStorage.getItem(collapsedKey);
      return new Set(stored ? (JSON.parse(stored) as string[]) : []);
    } catch {
      return new Set();
    }
  });
  const boardRef = useRef<HTMLElement | null>(null);
  const searchWrapRef = useRef<HTMLDivElement | null>(null);
  const filterControlRef = useRef<HTMLDivElement | null>(null);
  const moreMenuRef = useRef<HTMLDivElement | null>(null);
  const panRef = useRef({ active: false, startX: 0, scrollLeft: 0 });
  const localMutationVersion = useRef(0);
  const pendingTaskWrites = useRef(new Set<string>());
  const refreshJobRef = useRef<Promise<BoardSnapshot> | null>(null);
  const deferredExternalRefresh = useRef(false);
  const lastUserInputAtRef = useRef(Date.now());
  const pointerIsDownRef = useRef(false);
  const refreshFeedbackTimerRef = useRef<number | null>(null);
  const activityTimersRef = useRef(new Map<string, number>());
  const [taskActivity, setTaskActivity] = useState<Record<string, TaskActivityKind>>({});
  const [recentTaskActivity, setRecentTaskActivity] = useState<Record<string, RecentTaskActivity>>(() => loadRecentTaskActivity(recentKey));
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
  const [error, setError] = useLoggedError(boardPath);

  // Windows Explorer file drops are handled by Tauri rather than HTML5 DnD.
  // The native event includes absolute paths; no file contents are imported.
  useEffect(() => {
    if (!editor || busy || !("__TAURI_INTERNALS__" in window)) {
      setFileDropActive(false);
      return;
    }
    let active = true;
    let unlisten: (() => void) | undefined;
    const overEditor = (position: { x: number; y: number }) => {
      const bounds = editorModalRef.current?.getBoundingClientRect();
      if (!bounds) return false;
      // Tauri reports physical webview pixels; DOM bounds use CSS pixels.
      const scale = window.devicePixelRatio || 1;
      const x = position.x / scale;
      const y = position.y / scale;
      return x >= bounds.left && x <= bounds.right && y >= bounds.top && y <= bounds.bottom;
    };
    void getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (!active) return;
      if (payload.type === "leave") {
        setFileDropActive(false);
        return;
      }
      const inside = overEditor(payload.position);
      if (payload.type !== "drop") {
        setFileDropActive(inside);
        return;
      }
      setFileDropActive(false);
      if (!inside) return;
      const paths = [...new Set(payload.paths.filter((path) => path.trim()))];
      if (!paths.length) return;
      void addEditorFilePaths(paths);
    }).then((dispose) => {
      if (active) unlisten = dispose;
      else dispose();
    }).catch(() => { /* Browser preview has no native file-drop bridge. */ });
    return () => { active = false; unlisten?.(); setFileDropActive(false); };
  }, [!!editor, busy]);

  // The reservation is held only while this creation dialog is open.
  // Switching Task/Epic releases the old namespace and reserves a new ID.
  useEffect(() => {
    if (editor?.mode !== "create") {
      setReservedTaskId(null);
      return;
    }
    let closed = false;
    let owned: TaskIdReservation | null = null;
    setReservedTaskId(null);
    void reserveTaskId(editor.draft.kind, boardPath).then((reservation) => {
      if (closed) {
        void releaseTaskId(reservation, boardPath);
      } else {
        owned = reservation;
        setReservedTaskId(reservation);
      }
    }).catch((reason) => { if (!closed) setError(`Could not reserve task ID: ${errorText(reason)}`); });
    return () => {
      closed = true;
      if (owned) void releaseTaskId(owned, boardPath);
    };
  }, [boardPath, editor?.mode, editor?.draft.kind]);

  useEffect(() => {
    onInteractionChange(!!editor || busy || !!dragId || !!dragColumnId || !!columnDragPreview);
    return () => onInteractionChange(false);
  }, [editor, busy, dragId, dragColumnId, columnDragPreview, onInteractionChange]);

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
        const next = await loadBoard(boardPath);
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
      window.localStorage.setItem(recentKey, JSON.stringify(recentTaskActivity));
    } catch { /* Storage may be disabled; badges still work for this session. */ }
  }, [recentTaskActivity]);

  // Persist snoozes independently of Markdown task state. Wake the counter at
  // the exact expiry even if automatic board refresh is suspended by an editor.
  useEffect(() => {
    try {
      window.localStorage.setItem(snoozesKey, JSON.stringify(userQueueSnoozes));
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

  // Parent project tabs detect out-of-band edits across all boards. Never
  // overwrite a draft or drag; replay the pending refresh after interaction.
  useEffect(() => {
    const request = (event: Event) => {
      if (editor || busy || contextMenu || columnContextMenu || dragId || dragColumnId ||
          pendingTaskWrites.current.size) {
        deferredExternalRefresh.current = true;
        return;
      }
      const silent = (event as CustomEvent<{ silent?: boolean }>).detail?.silent ?? true;
      void refresh(boardPath, silent);
    };
    window.addEventListener("taskboard:refresh-board", request);
    return () => window.removeEventListener("taskboard:refresh-board", request);
  }, [boardPath, editor, busy, contextMenu, columnContextMenu, dragId, dragColumnId]);

  useEffect(() => {
    if (!deferredExternalRefresh.current || editor || busy || contextMenu ||
        columnContextMenu || dragId || dragColumnId || pendingTaskWrites.current.size) return;
    deferredExternalRefresh.current = false;
    void refresh(boardPath, true);
  }, [boardPath, editor, busy, contextMenu, columnContextMenu, dragId, dragColumnId]);

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
      collapsedKey,
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
    const item = id.includes("#") ? remotePrerequisites.get(id) : snapshot?.tasks.find((t) => t.id === id);
    return !!item && (item.archived || !!item.completed || (!id.includes("#") && columns.some((col) => col.id === item.column && col.terminal)));
  };
  const waitingFor = (task: Task): string[] => (task.depends_on || []).filter((id) => !isComplete(id));
  const awaitingUserAnswer = (task: Task) => task.column === "blocked" && task.attention.includes("question") && !!task.question && !task.question.answer;
  const readinessOf = (task: Task) => task.kind === "epic" ? "epic"
    : isForUser(task) ? "for_user"
    : awaitingUserAnswer(task) ? "waiting"
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

  const columns = useMemo(
    () => [...(snapshot?.config.columns ?? [])].sort((a, b) => a.order - b.order),
    [snapshot],
  );

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

    const hasIconFilter = taskFilters.icons.length > 0 || taskFilters.noIcon;
    return candidates.filter((task) => {
      if (hasIconFilter && !(task.icon
        ? taskFilters.icons.includes(task.icon)
        : taskFilters.noIcon)) return false;
      if (taskFilters.columns.length && !taskFilters.columns.includes(task.column)) return false;
      if (taskFilters.readiness.length && !taskFilters.readiness.includes(readinessOf(task))) return false;
      if (taskFilters.attention.length && !taskFilters.attention.some(flag => task.attention.includes(flag))) return false;
      if (taskFilters.kinds.length) {
        const kind = isForUser(task) ? "for_user" : task.kind;
        if (!taskFilters.kinds.includes(kind)) return false;
      }

      // Collapsing an epic hides its executable descendants in every column,
      // including while searching. Orange For user cards are always visible.
      return !isHiddenByCollapsedEpic(task, collapsedTaskIds, taskMap);
    });
  }, [activeTasks, query, taskMap, collapsedTaskIds, taskFilters, remotePrerequisites, columns]);

  const activeFilterCount = taskFilters.icons.length + Number(taskFilters.noIcon) +
    taskFilters.columns.length + taskFilters.readiness.length +
    taskFilters.kinds.length + taskFilters.attention.length;
  const userQueueTasks = useMemo(
    () => snapshot
      ? newestUserTasks(activeTasks.filter((task) => isUserQueueEligible(task, snapshot, userQueueSnoozes)))
      : [],
    [activeTasks, snapshot, userQueueSnoozes],
  );
  // The Inbox contains open visual reviews and actionable For user cards.
  // Filtering the Blocked column never changes their underlying task state.
  const isInReviewInbox = (task: Task) => task.review_status === "open" ||
    (!!snapshot && isUserQueueEligible(task, snapshot, userQueueSnoozes));

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
    deployable: task.deployable ?? null,
    repeat_every_days: task.repeat_every_days ?? null,
    next_due: task.next_due ?? null,
    blocked_reason: task.blocked_reason ?? null,
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
    if (task.kind === "task" && readinessOf(task) === "ready" && task.attention.includes("priority")) return -1;
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

  const preparationNotes = (notes: string, requestId: string, target: string, saved = true) => [
    notes.replace(/\s*<!-- taskboard:preparation-(?:request|target|saved)(?::[^>]*)?-->/g, "").trim(),
    `<!-- taskboard:preparation-request:${requestId} -->`,
    `<!-- taskboard:preparation-target:${target} -->`,
    ...(saved ? ["<!-- taskboard:preparation-saved -->"] : []),
  ].filter(Boolean).join("\n\n");

  // T3 owns delivery and reconciliation; never repeat an uncertain agent spawn.
  const linkPreparationAfterSave = (id: string, path: string, requestId: string) => {
    prepNoticeLastAttempt.current.set(`${path}#${id}#${requestId}`, Date.now());
    const launch = prepLaunchRef.current;
    void (async () => {
      try {
        await launch?.catch(() => undefined);
        await notifyTaskPreparationSaved(path, id, requestId);
        prepNotified.current.add(`${path}#${id}#${requestId}`);
      } catch (error) {
        setError(`${id} saved in Draft, but the preparation session could not be notified: ${errorText(error)}. Reconcile the existing T3 session; do not launch a second agent.`);
      }
    })();
  };

  const closeEditor = (board = snapshot, persisted = false) => {
    // A dismissed creation with meaningful contents becomes a short-lived
    // human draft, rather than vanishing without a Save click.
    if (!persisted && editor?.mode === "create" && !autosavingDraft.current &&
        (editor.draft.title.trim() || editor.draft.description.trim() ||
         editor.draft.user_notes.trim() || editor.draft.notes.trim() ||
         editor.draft.acceptance_criteria.some((item) => item.trim()) ||
         editor.draft.attention.includes("prepare"))) {
      if (!reservedTaskId) { setError("Cannot save draft yet: task ID reservation is not ready."); return; }
      autosavingDraft.current = true;
      setBusy(true);
      const stamp = new Date().toISOString();
      const preparing = editor.draft.attention.includes("prepare") && prepRequestId;
      const draft = {
        ...editor.draft,
        title: editor.draft.title.trim() || "Untitled draft",
        column: "draft",
        notes: [preparing
          ? preparationNotes(editor.draft.notes, preparing, editor.draft.column, false)
          : editor.draft.notes, `<!-- taskboard:auto-draft:${stamp} -->`].filter(Boolean).join("\n"),
        reserved_id: reservedTaskId.id,
        reservation_token: reservedTaskId.token,
      };
      void createTask(draft, boardPath).then(async (saved) => {
        const latest = await loadBoard(boardPath);
        applySnapshot(latest);
        flashTaskActivity([{ id: saved.id, kind: "created" }]);
        closeEditor(latest, true);
        setAutoSavedDraft({ id: saved.id, title: saved.title });
        // Closing is only an autosave, not permission to release the preparation task.
      }).catch((error) => setError(`Could not auto-save draft: ${errorText(error)}`))
        .finally(() => { autosavingDraft.current = false; setBusy(false); });
      return;
    }
    if (autosavingDraft.current && !persisted) return;
    imageDialogGeneration.current += 1;
    setPendingClipboardImages([]);
    // A follow-up created during queue review is edited outside the queue.
    // Closing or saving it resumes the queued tasks where the user left off.
    if (followupQueueReturn && board) {
      const review = followupQueueReturn;
      setFollowupQueueReturn(null);
      advanceUserQueue(board, userQueueSnoozes, review);
      return;
    }
    if (inboxEditorTaskId) {
      setInboxResumeId(persisted ? "" : inboxEditorTaskId);
      setInboxEditorTaskId(null);
      setEditor(null);
      setReviewOpen(true);
      return;
    }
    setUserQueueReview(null);
    setEditor(null);
  };

  useEffect(() => {
    if (editor) document.querySelector<HTMLElement>(".task-modal")?.scrollTo({ top: 0 });
  }, [editor?.original?.id]);

  const openEdit = (task: Task, fromQueue = false) => {
    setPrepRequestId(null);
    prepLaunchRef.current = null;
    setNotifyAgentOverride(null);
    setQueueExpanded(false);
    imageDialogGeneration.current += 1;
    setPendingClipboardImages([]);
    if (!fromQueue) setUserQueueReview(null);
    setQuestionAnswerText("");
    setDoneMenuOpen(false);
    setHoverTask(null);
    setEditor({
      mode: "edit",
      original: task,
      draft: taskToDraft(task),
    });
  };

  const handledDeepLink = useRef<number | null>(null);
  useEffect(() => {
    if (!requestedTask || !snapshot || handledDeepLink.current === requestedTask.requestKey) return;
    const task = snapshot.tasks.find((candidate) => candidate.id === requestedTask.id);
    if (!task) { setError(`Deep-linked task ${requestedTask.id} is not on this board.`); return; }
    handledDeepLink.current = requestedTask.requestKey;
    openEdit(task);
  }, [requestedTask?.id, requestedTask?.requestKey, snapshot]);

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
    setPrepRequestId(null);
    prepLaunchRef.current = null;
    imageDialogGeneration.current += 1;
    setPendingClipboardImages([]);
    const target = column ?? defaultCreateColumn();
    if (!target) return;
    setContextMenu(null);
    setColumnContextMenu(null);
    setUserQueueReview(null);
    setFollowupQueueReturn(null);
    setDoneMenuOpen(false);
    setEditor({ mode: "create", draft: emptyDraft(target,
      parent && taskMap.get(parent)?.kind === "epic" ? parent : null, category) });
  };

  const openTaskDump = (text = "") => {
    if (busy || editor) return;
    setContextMenu(null);
    setColumnContextMenu(null);
    setReviewOpen(false);
    setTaskDumpText(text);
    setTaskDumpPriorities({});
    setTaskDumpColumn(defaultCreateColumn() ?? "");
    setTaskDumpEpic("");
    setTaskDumpNewEpicTitle("");
    setError("");
    setTaskDumpOpen(true);
  };

  // Only intercept a board-level paste. Pasting in search, notes or an editor
  // continues to work exactly as before, including clipboard images.
  useEffect(() => {
    const handleBoardPaste = (event: ClipboardEvent) => {
      if (editor || taskDumpOpen || busy || reviewOpen || event.defaultPrevented) return;
      if (event.target instanceof Element &&
          event.target.closest("input, textarea, [contenteditable], [role='textbox']")) return;
      const text = event.clipboardData?.getData("text/plain");
      if (!text?.trim()) return;
      event.preventDefault();
      openTaskDump(text);
    };
    window.addEventListener("paste", handleBoardPaste);
    return () => window.removeEventListener("paste", handleBoardPaste);
  }, [editor, taskDumpOpen, busy, reviewOpen]);

  const generateTaskDump = async () => {
    if (busy || !snapshot || !taskDumpRows.length) return;
    if (!columns.some((column) => column.id === taskDumpColumn)) {
      setError("Select a valid destination column before generating tasks.");
      return;
    }
    if (taskDumpEpic === "__new__" && !taskDumpNewEpicTitle.trim()) {
      setError("Enter a title for the shared epic.");
      return;
    }
    if (taskDumpEpic && taskDumpEpic !== "__new__" &&
        !activeTasks.some((task) => task.id === taskDumpEpic && task.kind === "epic")) {
      setError("The selected epic is no longer available.");
      return;
    }
    const rows = [...taskDumpRows];
    const before = snapshot;
    const created: Task[] = [];
    let createdEpic: Task | null = null;
    let epicId = taskDumpEpic || null;
    setBusy(true);
    setError("");
    try {
      if (taskDumpEpic === "__new__") {
        // Create the container once, then retain its ID if any child fails.
        // Retrying a partial dump must never create a duplicate epic.
        createdEpic = await createTask({
          ...emptyDraft(taskDumpColumn), kind: "epic", title: taskDumpNewEpicTitle.trim(),
        }, boardPath);
        epicId = createdEpic.id;
        setTaskDumpEpic(epicId);
        setTaskDumpNewEpicTitle("");
      }
      for (const row of rows) {
        const parts = splitTaskDumpText(row.text);
        const draft = {
          ...emptyDraft(taskDumpColumn, epicId),
          title: parts.title,
          description: parts.description,
          attention: taskDumpPriorities[row.line] ? ["priority" as const] : [],
        };
        created.push(await createTask(draft, boardPath));
      }
      const after = await loadBoard(boardPath);
      applySnapshot(after);
      flashTaskActivity([...(createdEpic ? [createdEpic] : []), ...created]
        .map((task) => ({ id: task.id, kind: "created" as const })));
      pushUndo({ kind: "board", label: "generate task dump", before, after });
      setTaskDumpOpen(false);
      setTaskDumpText("");
      setTaskDumpPriorities({});
    } catch (reason) {
      // A partial batch is real work, not a rollback. Keep only uncreated
      // lines in the dialog so retry cannot silently generate duplicates.
      const remaining = rows.slice(created.length);
      setTaskDumpText(remaining.map((row) => row.text).join("\n"));
      setTaskDumpPriorities(Object.fromEntries(remaining.map((row, index) =>
        [index, !!taskDumpPriorities[row.line]])));
      setError(`${created.length} of ${rows.length} tasks created. ${errorText(reason)}`);
      try {
        const after = await loadBoard(boardPath);
        applySnapshot(after);
        if (created.length || createdEpic) {
          flashTaskActivity([...(createdEpic ? [createdEpic] : []), ...created]
            .map((task) => ({ id: task.id, kind: "created" as const })));
          pushUndo({ kind: "board", label: "partial task dump", before, after });
        }
      } catch { /* Keep the original error and remaining lines. */ }
    } finally {
      setBusy(false);
    }
  };

  const run = async (operation: () => Promise<void>) => {
    setBusy(true);
    // Clear the previous error BEFORE the operation. A successful local save
    // can still report a separate OpenRig registration/UNKNOWN warning; clearing
    // after operation erased that truthful warning from the UI.
    setError("");
    try {
      await operation();
    } catch (err) {
      setError(errorText(err));
      await refresh(boardPath, true);
    } finally {
      setBusy(false);
    }
  };

  // T3 owns the task-bound spawn journal and deduplicates concurrent clicks.
  const hasTaskSession = (task?: Task) => /^(t3-browser:|t3:|[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}$)/i.test(task?.claimed_by ?? "");
  const canSpawnTask = (task?: Task) => !!task && !task.claimed_by && !task.archived &&
    task.kind !== "epic" && task.kind !== "for_user" && !task.completed &&
    !snapshot?.config.columns.some((column) => column.id === task.column && column.terminal);
  const openForTask = (task: Task) => void run(async () => {
    await openTaskSession(task.id, boardPath, !hasTaskSession(task) && !task.live?.with_me);
    const after = await loadBoard(boardPath);
    applySnapshot(after);
  });

  const spawnQueueRecoveryAgent = async () => {
    if (queueRecoveryAgentBusy || !queueRecoveryWarning) return;
    setQueueRecoveryAgentBusy(true);
    const miscRoot = "B:\\GDrive\\SharedWithPhone\\Obsidian\\jjj\\Misc\\Tasks";
    try {
      const misc = await loadBoard(miscRoot);
      const existing = misc.tasks.find(item => item.id === "MISC-0119" && !item.archived);
      const diagnostic = `Queue integrity issue (${new Date().toISOString()})
Board: ${boardPath}
Details: ${queueRecoveryWarning}`;
      // Reuse the user's original diagnostic task. If it was already marked
      // Done, the explicit Spawn click reopens it before requesting a worker.
      // If the old task has been archived, create a fresh repair assignment.
      const recovery = existing ? await updateTask({
        ...existing,
        ...(misc.config.columns.some(column => column.id === existing.column && column.terminal)
          ? { column: "ai-ready", completed: null, completed_at: null, claimed_by: null } : {}),
        user_notes: [existing.user_notes, diagnostic].filter(Boolean).join("\n\n"),
      }, miscRoot) : await createTask({
        kind: "task", title: "Investigate TaskBoard queue integrity failure",
        column: "ai-ready", parent: null, category: "TaskBoard",
        icon: "bug", attention: ["priority"], question: null,
        depends_on: [], related_to: [], acceptance_criteria: [],
        ai_notes: "", user_notes: diagnostic, notes: "",
        description: "Investigate the queue receipt conflict/UNKNOWN outcome and repair it without duplicating a dispatch.",
      }, miscRoot);
      await openTaskSession(recovery.id, miscRoot, !hasTaskSession(recovery));
      setQueueRecoveryWarning("");
    } catch (reason) {
      setError(`Could not start queue-recovery agent: ${errorText(reason)}`);
    } finally {
      setQueueRecoveryAgentBusy(false);
    }
  };

  const addEditorFilePaths = async (rawPaths: string[]) => {
    const paths = [...new Set(rawPaths.map(path => path.trim()).filter(Boolean))];
    if (!paths.length) return;
    const generation = imageDialogGeneration.current;
    const images = paths.filter(path => /\.(png|jpe?g|webp|gif)$/i.test(path));
    const nonImages = paths.filter(path => !images.includes(path));
    if (nonImages.length) {
      setEditor(current => current && generation === imageDialogGeneration.current
        ? { ...current, draft: { ...current.draft,
            user_notes: [current.draft.user_notes.trim(), ...nonImages].filter(Boolean).join("\n") } }
        : current);
    }
    for (const path of images) {
      try {
        const preview = await readLocalImage(path);
        if (generation === imageDialogGeneration.current) {
          setPendingClipboardImages(current => [...current, {
            dataUrl: preview.dataUrl, mime: preview.mime, name: preview.name,
          }]);
        }
      } catch (reason) {
        setError(`Unable to attach ${path}: ${errorText(reason)}`);
      }
    }
  };
  const browseEditorFiles = async () => {
    try {
      const selection = await open({ multiple: true, directory: false });
      if (selection) await addEditorFilePaths(Array.isArray(selection) ? selection : [selection]);
    } catch (reason) { setError(`File picker failed: ${errorText(reason)}`); }
  };

  const pasteClipboardImages = (event: React.ClipboardEvent<HTMLElement>) => {
    if (busy) return;
    const files = [...event.clipboardData.items]
      .filter((item) => item.kind === "file" && /^image\/(png|jpeg|webp|gif)$/.test(item.type))
      .map((item) => item.getAsFile()).filter((file): file is File => !!file);
    if (!files.length) return; // Text Ctrl+V must keep its normal editing behavior.
    event.preventDefault();
    const generation = imageDialogGeneration.current;
    for (const file of files) {
      if (file.size > 20 * 1024 * 1024) {
        setError("A pasted image exceeds the 20 MB attachment limit.");
        continue;
      }
      const reader = new FileReader();
      reader.onload = () => {
        if (generation !== imageDialogGeneration.current || typeof reader.result !== "string") return;
        setPendingClipboardImages((current) => [...current, {
          dataUrl: reader.result as string, mime: file.type,
          name: file.name || "Pasted image",
        }]);
      };
      reader.onerror = () => { if (generation === imageDialogGeneration.current) setError("Could not read the clipboard image."); };
      reader.readAsDataURL(file);
    }
  };

  const withClipboardImages = async (draft: TaskDraft, id: string): Promise<TaskDraft> => {
    if (!pendingClipboardImages.length) return draft;
    const links: string[] = [];
    for (const image of pendingClipboardImages) {
      const encoded = image.dataUrl.split(",", 2)[1];
      if (!encoded) throw new Error("Invalid clipboard image data.");
      const savedPath = await saveTaskImage(id, encoded, image.mime, boardPath);
      links.push(`![Pasted image](../${savedPath})`);
    }
    const locked = editor?.original?.id === id && editor?.original?.agent_interacted === true;
    return {
      ...draft,
      description: attachImagesToDescription(draft.description, links, locked),
      attention: locked && !draft.attention.includes("user-note")
        ? [...draft.attention, "user-note"] : draft.attention,
    };
  };

  // Release a user-queue item only when Save persists an actual human edit.
  const queueEdited = !!(editor?.original && userQueueReview &&
    editor.original.id === userQueueReview.ids[userQueueReview.index] &&
    isForUser(editor.original) &&
    (editor.draft.title !== editor.original.title ||
      editor.draft.description !== editor.original.description ||
      editor.draft.user_notes !== editor.original.user_notes ||
      editor.draft.notes !== editor.original.notes ||
      editor.draft.category !== editor.original.category ||
      editor.draft.column !== editor.original.column ||
      editor.draft.parent !== editor.original.parent ||
      editor.draft.icon !== editor.original.icon ||
      editor.draft.kind !== editor.original.kind ||
      editor.draft.repeat_every_days !== (editor.original.repeat_every_days ?? null) ||
      editor.draft.next_due !== (editor.original.next_due ?? null) ||
      !!pendingClipboardImages.length ||
      JSON.stringify(editor.draft.related_to) !== JSON.stringify(editor.original.related_to) ||
      JSON.stringify(editor.draft.depends_on) !== JSON.stringify(editor.original.depends_on) ||
      JSON.stringify(editor.draft.attention) !== JSON.stringify(editor.original.attention) ||
      JSON.stringify(editor.draft.acceptance_criteria) !== JSON.stringify(editor.original.acceptance_criteria) ||
      JSON.stringify(editor.draft.question) !== JSON.stringify(editor.original.question)));

  const syncQueueWarning = async (id: string): Promise<string | null> => {
    // Local task updates are authoritative. Only uncertain receipts or
    // identity conflicts require investigation, not normal offline retries.
    try {
      const result = await taskSyncQueue(id, boardPath);
      if (["unknown", "conflict", "rejected"].includes(result.status))
        setQueueRecoveryWarning(`${id}: ${result.status} — ${result.reason || "receipt needs investigation"}`);
    } catch { /* The durable queue intent survives an unavailable bridge. */ }
    return null;
  };

  // Saving a task and waiting for T3 to start it are separate operations. The
  // launch remains one-shot in T3; UNKNOWN delivery must be reconciled, not retried.
  const startTaskSessionDetached = (id: string, path: string, implement: boolean) => {
    const key = `${path}:${id}`;
    if (pendingTaskStarts.current.has(key)) return;
    pendingTaskStarts.current.add(key);
    setStartingTaskIds(current => [...current, id]);
    void (async () => {
      try {
        const receipt = await openTaskSession(id, path, implement);
        if (implement && ["prompt_unknown", "unknown", "in_progress"].includes(String(receipt.status))) {
          setError(`${id} T3 start: ${receipt.status}. Inspect its existing request before trying again.`);
        } else if (implement && receipt.status !== "opened" && receipt.status !== "created") {
          throw new Error(`T3 did not confirm opening the task session (${String(receipt.status)}).`);
        }
      } catch (err) {
        setError(`${id} was saved, but its T3 launch needs attention: ${errorText(err)}`);
      } finally {
        pendingTaskStarts.current.delete(key);
        setStartingTaskIds(current => current.filter(item => item !== id));
        await refresh(path, true);
      }
    })();
  };

  const queueableColumn = (columnId: string, config: BoardConfig | undefined = snapshot?.config) =>
    !!config && config.columns.some(column => column.id === columnId && column.ai_ready &&
      !column.advisory && !column.terminal && column.id !== "draft");
  const queueEligible = (task: Task) =>
    !!snapshot && queueableColumn(task.column) &&
    (task.kind === "task" || task.kind === "recurring") &&
    !task.archived && !task.completed && !task.claimed_by &&
    readinessOf(task) === "ready" &&
    task.review_status !== "open" &&
    !(task.attention.includes("question") && task.question?.answer == null);

  const toggleTaskQueue = (task: Task) => run(async () => {
    if (!queueEligible(task)) throw new Error(`${task.id} is not currently eligible for Queue.`);
    const current = await taskExecutionIntent(task.id, boardPath);
    const next = current.mode === "queue" ? "backlog" : "queue";
    await taskSetExecutionMode(task.id, current.revision, next, boardPath);
    const after = await loadBoard(boardPath);
    applySnapshot(after);
    const warning = await syncQueueWarning(task.id);
    if (warning) setError(warning);
  });

  const toggleColumnQueue = (columnId: string) => run(async () => {
    if (!snapshot) return;
    const current = await loadBoard(boardPath);
    const column = current.config.columns.find(item => item.id === columnId);
    if (!column || !queueableColumn(columnId, current.config)) return;
    const next = !column.auto_queue;
    await saveBoardConfig({ ...current.config, columns: current.config.columns.map(item =>
      item.id === columnId ? { ...item, auto_queue: next } : item) }, boardPath);
    // Saving the column preference first makes newly created tasks inherit it,
    // even if a later individual queue CAS encounters an uncertain receipt.
    const afterPolicy = await loadBoard(boardPath);
    const conflicts: string[] = [];
    for (const task of afterPolicy.tasks.filter(task =>
      task.column === columnId && queueEligible(task))) {
      try {
        const mode = await taskExecutionIntent(task.id, boardPath);
        if (mode.mode !== (next ? "queue" : "backlog")) {
          await taskSetExecutionMode(task.id, mode.revision, next ? "queue" : "backlog", boardPath);
        }
      } catch (err) { conflicts.push(`${task.id}: ${errorText(err)}`); }
    }
    applySnapshot(await loadBoard(boardPath));
    // An agent may update one of these tasks concurrently; column auto-queue
    // policy is already persisted, and subsequent refreshes reconcile it.
    if (!conflicts.length) {
      void taskQueueReconcile(boardPath).then(() => refresh(boardPath, true)).catch(() =>
        setQueueReconciliationOffline(true));
    } else {
      setError(`${column.name} auto-queue saved, but ${conflicts.length} task(s) could not be updated: ${conflicts.join("; ")}`);
    }
  });

  const showLinkedReviewOutcome = (before: BoardSnapshot, after: BoardSnapshot, reviewId: string) => {
    const review = after.tasks.find((task) => task.id === reviewId);
    const previous = before.tasks.find((task) => task.id === reviewId);
    if (review?.kind !== "for_user" || previous?.completed || !review.completed ||
        review.related_to.length !== 1) return;
    const parent = after.tasks.find((task) => task.id === review.related_to[0]);
    if (!parent) return;
    const audit = parent.notes.split(`[review:${reviewId}]`).slice(-1)[0] ?? "";
    const disposition = audit.trim().split(" ")[0];
    if (disposition === "approved_parent_done") {
      setReviewNotice(`Review approved — ${parent.id} moved to Done.`);
    } else if (disposition === "needs_changes") {
      setReviewNotice(`Review requested changes — ${parent.id} remains actionable.`);
    } else if (disposition === "partial_or_blocked") {
      setReviewNotice(`Review accepted its scope — ${parent.id} remains ${parent.column} with remaining work.`);
    } else if (disposition === "ambiguous_feedback") {
      setReviewNotice(`Review closed — ${parent.id} needs a clear disposition.`);
    }
  };
  /** Merge independent on-disk changes instead of discarding the user's draft.
   * Overlapping edits remain explicit conflicts rather than silently taking a side. */
  const updateWithDiskMerge = async (original: Task, edited: Task): Promise<Task> => {
    try { return await updateTask(edited, boardPath); }
    catch (error) {
      if (!errorText(error).includes("changed on disk")) throw error;
      const current = (await loadBoard(boardPath)).tasks.find((task) => task.id === original.id);
      if (!current) throw new Error(`${original.id} no longer exists on disk.`);
      const { merged, conflicts } = mergeTaskEdits(original, edited, current);
      if (conflicts.length) {
        throw new Error(`Concurrent changes to ${conflicts.join(", ")}. Latest task loaded from disk; review before saving.`);
      }
      return await updateTask(merged, boardPath);
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
      const prepared = prepareTaskDraftForSave(editor.draft, "", editor.original?.claimed_by);
      const savedPrepRequest = editor.mode === "edit" && editor.original?.column === "draft" &&
        editor.original.attention.includes("prepare")
        ? editor.original.notes.match(/taskboard:preparation-request:([a-f0-9-]{36})/)?.[1] ?? null : null;
      const preparing = editor.draft.attention.includes("prepare")
        ? editor.mode === "create" ? prepRequestId : savedPrepRequest : null;
      const priorTarget = editor.original?.notes.match(/taskboard:preparation-target:([^\s>]+)/)?.[1];
      const preparationTarget = prepared.column === "draft" ? priorTarget ?? defaultCreateColumn() ?? "draft" : prepared.column;
      let draft = { ...prepared,
        column: preparing ? "draft" : markDone && editor.mode === "edit" && doneColumn ? doneColumn.id : prepared.column,
        notes: preparing ? preparationNotes(prepared.notes, preparing, preparationTarget) : prepared.notes,
        depends_on: editor.draft.kind === "epic" ? [] : [...editor.draft.depends_on],
        related_to: [...editor.draft.related_to],
        parent: editor.draft.kind === "epic" ? null : editor.draft.parent,
        icon: editor.draft.kind === "epic" ? null : editor.draft.icon,
        attention: editor.draft.kind === "epic" ? [] : [...prepared.attention],
        question: editor.draft.kind === "epic" ? null : prepared.question,
        acceptance_criteria: editor.draft.kind === "epic" ? [] : prepared.acceptance_criteria,
      };
      if (queueEdited) {
        draft = {
          ...draft,
          kind: draft.kind === "for_user" ? "task" : draft.kind,
          attention: draft.attention.filter((flag) => flag !== "needs-user"),
        };
      }
      if (pendingClipboardImages.length) {
        const id = editor.original?.id ?? reservedTaskId?.id;
        if (!id) throw new Error("Task ID must be reserved before saving clipboard images.");
        draft = await withClipboardImages(draft, id);
      }
      if (draft.kind === "epic" && editor.original?.kind !== "epic") {
        const retained = [
          draft.notes,
          ...(prepared.acceptance_criteria.length
            ? ["Previous acceptance criteria (retained as planning notes):\n" +
                prepared.acceptance_criteria.map((item) => "- " + item).join("\n")] : []),
        ].filter(Boolean);
        draft = { ...draft, notes: retained.join("\n\n") };
      }
      if (executionLoading) throw new Error("Still reading the existing execution state; save after it loads.");
      let after = before;
      let savedTask: Task | null = null;
      let undoAction: string | null = null;
      let notifyWarning: string | null = null;
      let startAfterSave: { id: string; implement: boolean } | null = null;

      if (editor.mode === "create") {
        if (!reservedTaskId) throw new Error("Task ID reservation is not ready yet.");
        const saved = await createTask({ ...draft, reserved_id: reservedTaskId.id, reservation_token: reservedTaskId.token }, boardPath);
        savedTask = saved;
        after = snapshotWithTask(before, saved);
        flashTaskActivity([{ id: saved.id, kind: "created" }]);
        undoAction = "create task";
        if (preparing) linkPreparationAfterSave(saved.id, boardPath, preparing);
      } else if (editor.original) {
        const saved = await updateWithDiskMerge(editor.original, { ...editor.original, ...draft });
        savedTask = saved;
        if (showNotifyAgent && notifyAgent && saved.claimed_by && saved.column === "in-progress") {
          try {
            await taskEnqueueAgentNotice(boardPath, saved.id, saved.claimed_by, saved.revision, "requirements-change");
            // The durable outbox is picked up by the background reconciler.
            // Do not race the rest of this save with an eager board-wide refresh.
          } catch (error) {
            // The task edit is already durable. Do not roll it back or pretend no change happened.
            notifyWarning = `Task saved, but notification was not recorded: ${String(error)}`;
          }
        }
        // Completing a prerequisite can release other tasks from Blocked in
        // the shared core. Reload those automatic changes as part of this save.
        after = doneColumn && saved.column === doneColumn.id
          ? await loadBoard(boardPath) : snapshotWithTask(before, saved);
        const kind = classifyTaskChange(editor.original, saved);
        if (kind) flashTaskActivity([{ id: saved.id, kind }]);
        undoAction = "edit task";
        if (preparing) linkPreparationAfterSave(saved.id, boardPath, preparing);
      }

      // Persist execution intent only after the task exists. Each write is revision-checked;
      // a pending Start now ID is never retried or replaced after an uncertain result.
      if (savedTask && !markDone && !preparing && (savedTask.kind === "task" || savedTask.kind === "recurring")) {
        if (withMeSelected !== (savedTask.live?.with_me === true)) {
          savedTask = await taskLiveWithMe(savedTask.id, savedTask.revision, withMeSelected, boardPath);
        }
        const selectedMode = editor.mode === "create" ? editorQueueChoice : executionChoice;
        if (selectedMode === "start_now") {
          // The saved task is durable; do not hold the editor open while T3
          // creates/opens a browser session or delivers its prompt.
          startAfterSave = { id: savedTask.id, implement: !withMeSelected };
        } else if (executionTouched) {
          const current = await taskExecutionIntent(savedTask.id, boardPath);
          if (selectedMode !== current.mode)
            await taskSetExecutionMode(savedTask.id, current.revision, selectedMode, boardPath);
        }
      }
      if (savedTask) showLinkedReviewOutcome(before, after, savedTask.id);
      if (undoAction) pushUndo({ kind: "board", label: undoAction, before, after });
      const syncWarning = savedTask && !startAfterSave && !preparing ? await syncQueueWarning(savedTask.id) : null;
      // The sync CAS may add a receipt, changing the revision: reload before
      // delivering the local board view, rather than leaving a stale edit draft.
      if (savedTask) after = await loadBoard(boardPath);
      applySnapshot(after);
      if (userQueueReview && editor.original?.id === userQueueReview.ids[userQueueReview.index]) {
        advanceUserQueue(after);
      } else {
        closeEditor(after, true);
      }
      if (notifyWarning || syncWarning) setError([notifyWarning, syncWarning].filter(Boolean).join(" · "));
      if (startAfterSave) startTaskSessionDetached(startAfterSave.id, boardPath, startAfterSave.implement);
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
      const prepared = await withClipboardImages(
        prepareTaskDraftForSave(editor.draft, "", original.claimed_by), original.id);
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
        else closeEditor(after, true);
        flashTaskActivity([{ id: original.id, kind: "moved" }]);
        throw new Error(`${original.id} was saved and moved to Done, but the follow-up could not be created: ${errorText(error)}`);
      }
      const syncWarning = await syncQueueWarning(closed.id);
      const after = await loadBoard(boardPath);
      showLinkedReviewOutcome(before, after, closed.id);
      applySnapshot(after);
      if (syncWarning) setError(syncWarning);
      flashTaskActivity([
        { id: closed.id, kind: "moved" },
        { id: followup.id, kind: "created" },
      ]);
      pushUndo({ kind: "board", label: "close and create follow-up", before, after });
      if (userQueueReview) setFollowupQueueReturn(userQueueReview);
      openEdit(followup);
    });

  // Reopen a blocked task to AI Ready through the normal
  // update path. The core clears blocked_reason once the task is unblocked.
  const reopenBlockedTask = () =>
    run(async () => {
      if (!editor?.original || editor.mode !== "edit" || editor.draft.kind === "epic" || !snapshot) return;
      if (!editor.draft.title.trim()) return;
      const original = editor.original;
      const before = snapshot;
      const readyColumn = snapshot.config.columns.find((column) => column.id === "ai-ready" && !column.terminal)
        ?? snapshot.config.columns.find((column) => column.ai_ready && !column.terminal && column.id !== "blocked");
      if (!readyColumn) throw new Error("This board has no AI Ready column to unblock into.");
      const prepared = await withClipboardImages(
        prepareTaskDraftForSave(editor.draft, "", original.claimed_by), original.id);
      const saved = await updateTask({
        ...original,
        ...prepared,
        column: readyColumn.id,
        attention: prepared.attention.filter((flag) => flag !== "blocked"),
        blocked_reason: null,
        position_after: null,
        positioned_at: null,
        description: prepared.description,
      }, boardPath);
      const after = snapshotWithTask(before, saved);
      flashTaskActivity([{ id: saved.id, kind: "moved" }]);
      pushUndo({ kind: "board", label: "reopen blocked task", before, after });
      applySnapshot(after);
      if (userQueueReview && original.id === userQueueReview.ids[userQueueReview.index]) {
        advanceUserQueue(after);
      } else {
        closeEditor(after, true);
      }
    });

  const chooseDraftKind = (kind: TaskDraft["kind"]) => {
    if (!editor) return;
    if (prepRequestId) { setError("Preparation has started for this task ID. Save or close this draft before changing the task type."); return; }
    setEditor({
      ...editor,
      draft: {
        ...editor.draft,
        kind,
        repeat_every_days: kind === "recurring" ? editor.draft.repeat_every_days : null,
        next_due: kind === "recurring" ? editor.draft.next_due : null,
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

  const enableTaskPreparation = () => {
    if (!editor || editor.mode !== "create" || prepRequestId) return;
    if (!reservedTaskId) { setError("Task ID is being reserved; try Preparation again."); return; }
    const requestId = crypto.randomUUID();
    setPrepRequestId(requestId);
    setEditor({ ...editor, draft: { ...editor.draft,
      attention: [...editor.draft.attention, "prepare"] } });
    const launch = startTaskPreparation(boardPath, reservedTaskId.id, requestId,
      editor.draft.title, [editor.draft.description, editor.draft.user_notes].filter(Boolean).join("\n\n"), false);
    prepLaunchRef.current = launch;
    void launch.catch((error) => {
      setError(`Preparation launch not confirmed: ${errorText(error)}. Inspect the existing session before retrying.`);
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
      // A review verdict may atomically update its linked implementation task.
      const after = await loadBoard(boardPath);
      showLinkedReviewOutcome(before, after, saved.id);
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
      const syncWarning = await syncQueueWarning(id);
      const after = await loadBoard(boardPath);
      showLinkedReviewOutcome(before, after, id);
      if (syncWarning) setError(syncWarning);
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

  const moveTaskTo = async (id: string, column: string, afterTaskId: string | null = null, detachFromEpic = false) => {
    const task = taskMap.get(id);
    if (!task || task.kind === "epic" || (task.column === column && !afterTaskId && !(detachFromEpic && task.parent))) return;
    const terminal = columns.find((item) => item.id === column)?.terminal;
    await persistTaskMutation(id, afterTaskId ? "place task below another" : "move task", (current) => ({
      ...current,
      column,
      ...(detachFromEpic ? { parent: null } : {}),
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

  // An epic can span multiple columns. Dragging its real card or a proxy
  // moves only descendant cards in the column where the drag began, leaving
  // other columns intact. Claimed cards stay put; one Undo restores the batch.
  const moveEpicTo = async (epicId: string, sourceColumn: string, column: string) => {
    if (!snapshot || sourceColumn === column) return;
    const leaves: Task[] = [];
    const queue = [epicId];
    const seen = new Set<string>();
    while (queue.length) {
      const parent = queue.pop()!;
      for (const child of snapshot.tasks) {
        if (child.parent !== parent || seen.has(child.id)) continue;
        seen.add(child.id);
        if (child.kind === "epic") queue.push(child.id);
        else if (!child.archived && child.column === sourceColumn) leaves.push(child);
      }
    }
    const target = columns.find((item) => item.id === column);
    const movable = leaves.filter((task) => !task.claimed_by);
    const skipped = leaves.filter((task) => !!task.claimed_by);
    if (!leaves.length) {
      setError(`${epicId} has no tasks in this column to move.`);
      return;
    }
    if (!movable.length) {
      if (skipped.length) setError(`${epicId}: its open tasks are claimed by agents and were left in place.`);
      return;
    }
    if (target?.terminal && !window.confirm(`Move ${epicId} to ${target.name}? This completes ${movable.length} open task(s). You can undo this.`)) return;
    const before = snapshot;
    const now = new Date().toISOString();
    localMutationVersion.current += 1;
    setError("");
    try {
      for (const task of movable) {
        pendingTaskWrites.current.add(task.id);
        await updateTask({
          ...task,
          column,
          position_after: null,
          positioned_at: null,
          ...(target?.terminal ? {
            kind: task.kind === "for_user" ? "task" as const : task.kind,
            attention: task.attention.filter((flag) => flag !== "needs-user"),
            completed_at: now,
          } : {}),
        }, boardPath);
      }
      const after = await loadBoard(boardPath);
      pushUndo({ kind: "board", label: `move ${epicId} from ${sourceColumn} to ${column}`, before, after });
      applySnapshot(after);
      flashTaskActivity([epicId, ...movable.map((task) => task.id)].map((id) => ({ id, kind: "moved" as const })));
      if (skipped.length) setError(`Moved ${movable.length} task(s). Left ${skipped.map((task) => task.id).join(", ")} in place (claimed by an agent).`);
    } catch (err) {
      setError(errorText(err));
      await refresh(boardPath, true);
    } finally {
      for (const task of movable) pendingTaskWrites.current.delete(task.id);
    }
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

  // Column under the pointer; below a short column, its whole vertical lane
  // still counts so cards can be dropped without aiming at the column body.
  const columnAtPoint = (x: number, y: number) => {
    const direct = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-column-id]")?.dataset.columnId;
    if (direct) return direct;
    const lane = [...document.querySelectorAll<HTMLElement>(".board [data-column-id]")].find((element) => {
      const rect = element.getBoundingClientRect();
      return x >= rect.left && x <= rect.right && y >= rect.top;
    });
    return lane?.dataset.columnId ?? null;
  };

  const beginCardDrag = (
    event: React.MouseEvent<HTMLElement>,
    task: Task,
    sourceColumn: string,
  ) => {
    if (event.button !== 0) return;
    // Both the real epic card and each cross-column proxy drag only their
    // own column's tasks; a plain click keeps its inspect/jump behavior.
    const isEpic = task.kind === "epic";
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
      window.removeEventListener("pointerup", handlePointerUp, true);
      window.removeEventListener("blur", cleanup);
      setDragId(null);
      setDragSourceColumn(null);
      setCardDragPreview(null);
      setCardDropTargetId(null);
      document.body.classList.remove("card-dragging");
    };

    const validDropTaskAt = (x: number, y: number) => {
      const candidate = document
        .elementFromPoint(x, y)
        ?.closest<HTMLElement>("[data-task-id]")
        ?.dataset.taskId;
      if (isEpic || !candidate || candidate === task.id) return null; // epics only target columns
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
        setDragSourceColumn(sourceColumn);
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

    let finished = false;
    const handleUp = (upEvent: MouseEvent | PointerEvent) => {
      if (finished) return;
      finished = true;
      const droppedProjectId = dragging
        ? document.elementFromPoint(upEvent.clientX, upEvent.clientY)
            ?.closest<HTMLElement>("[data-project-tab-id]")?.dataset.projectTabId
        : null;
      const taskTargetId = dragging
        ? validDropTaskAt(upEvent.clientX, upEvent.clientY)
        : null;
      const taskTarget = taskTargetId ? taskMap.get(taskTargetId) : null;
      const columnHeaderTarget = dragging
        ? document.elementFromPoint(upEvent.clientX, upEvent.clientY)
            ?.closest<HTMLElement>(".column-header")
        : null;
      const columnTarget = dragging ? columnAtPoint(upEvent.clientX, upEvent.clientY) : null;

      cleanup();

      if (!dragging) {
        if (!isEpic) openEdit(task);
        return;
      }
      if (isEpic) {
        suppressEpicClickUntil.current = Date.now() + 300;
        const column = taskTarget?.column ?? columnTarget;
        if (column) void moveEpicTo(task.id, sourceColumn, column);
        return;
      }

      if (droppedProjectId && droppedProjectId !== project.id) {
        window.dispatchEvent(new CustomEvent("taskboard:project-transfer", {
          detail: {
            taskId: task.id, expectedRevision: task.revision,
            sourceProjectId: project.id, destinationProjectId: droppedProjectId,
          },
        }));
        return;
      }
      if (taskTarget?.kind === "epic") {
        void makeSubtaskOf(task.id, taskTarget.id);
      } else if (taskTarget) {
        void moveTaskTo(task.id, taskTarget.column, taskTarget.id);
      } else if (columnTarget) {
        void moveTaskTo(task.id, columnTarget, null, !!columnHeaderTarget);
      }
    };

    // Pointer release in capture phase also reaches us when a draggable tab
    // or nested control consumes the normal mouseup during cross-board drops.
    const handlePointerUp = (event: PointerEvent) => {
      if (event.pointerType === "mouse") handleUp(event);
    };
    window.addEventListener("pointerup", handlePointerUp, true);
    window.addEventListener("blur", cleanup);
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
          ai_ready: !["draft", "done", "review", "blocked"].includes(id.toLowerCase()) &&
            !["draft", "done", "review", "blocked"].includes(name.trim().toLowerCase()),
          advisory: false,
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
      const board = boardRef.current;
      const boardRect = board?.getBoundingClientRect();
      // Use insertion slots rather than elementFromPoint: the gaps between columns
      // are valid drop locations, and the dragged column itself is not a target.
      const withinBoard = boardRect &&
        upEvent.clientX >= boardRect.left && upEvent.clientX <= boardRect.right &&
        upEvent.clientY >= boardRect.top && upEvent.clientY <= boardRect.bottom;
      const destinations = withinBoard
        ? Array.from(board!.querySelectorAll<HTMLElement>("[data-column-id]"))
            .filter((element) => element.dataset.columnId !== column.id)
            .map((element) => ({ id: element.dataset.columnId!, rect: element.getBoundingClientRect() }))
            .sort((a, b) => a.rect.left - b.rect.left)
        : [];
      const nextColumn = destinations.find(
        ({ rect }) => upEvent.clientX < rect.left + rect.width / 2,
      );
      const destination = nextColumn ?? destinations[destinations.length - 1];
      cleanup();

      if (dragging && destination) {
        reorderColumn(column.id, destination.id, !nextColumn);
      }
    };

    window.addEventListener("mousemove", handleMove, { passive: false });
    window.addEventListener("mouseup", handleUp, { once: true });
  };

  const cycleColumnMode = (id: string) => {
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
          item.id === id ? {
            ...item,
            ai_ready: !item.ai_ready && !item.advisory,
            advisory: item.ai_ready ? true : false,
            auto_queue: item.ai_ready ? false : item.auto_queue,
          } : item,
        ),
      },
      "cycle column Off / AI Ready / Advisory",
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
      let after = snapshotWithTask(before, archived);
      const syncWarning = await syncQueueWarning(archived.id);
      after = await loadBoard(boardPath);
      pushUndo({ kind: "board", label: "archive task", before, after });
      applySnapshot(after);
      if (syncWarning) setError(syncWarning);
      setContextMenu(null);
      if (editor?.original?.id === task.id) {
        if (userQueueReview) advanceUserQueue(after);
        else closeEditor(after, true);
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
      setCopiedTaskId(value);
      window.setTimeout(() => setCopiedTaskId((current) => current === value ? null : current), 1600);
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
  const isAdvisoryEditor = !!editor && columns.some((column) =>
    column.id === editor.draft.column && column.advisory);
  const isQueueCompact = !!editor && !queueExpanded && !!userQueueReview &&
    editor.original?.id === userQueueReview.ids[userQueueReview.index];
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

  const navigateTaskCards = (direction: "up" | "down" | "left" | "right") => {
    const board = boardRef.current;
    if (!board) return;
    const cards = [...board.querySelectorAll<HTMLElement>(".task-card[data-task-id]")];
    if (!cards.length) return;
    const current = cards.find((card) => card.dataset.taskId === keyboardSelectedTaskId);
    let target = current ?? cards[0];
    if (current) {
      const column = current.closest<HTMLElement>(".column[data-column-id]");
      const siblings = column
        ? [...column.querySelectorAll<HTMLElement>(".task-card[data-task-id]")] : [];
      if (direction === "up" || direction === "down") {
        const index = siblings.indexOf(current);
        target = siblings[Math.max(0, Math.min(siblings.length - 1,
          index + (direction === "down" ? 1 : -1)))] ?? current;
      } else {
        const columnEls = [...board.querySelectorAll<HTMLElement>(".column[data-column-id]")];
        const index = column ? columnEls.indexOf(column) : -1;
        const delta = direction === "right" ? 1 : -1;
        const midY = current.getBoundingClientRect().top + current.offsetHeight / 2;
        for (let nextIndex = index + delta; nextIndex >= 0 && nextIndex < columnEls.length; nextIndex += delta) {
          const candidates = [...columnEls[nextIndex].querySelectorAll<HTMLElement>(".task-card[data-task-id]")];
          if (!candidates.length) continue;
          target = candidates.reduce((best, candidate) => {
            const distance = (item: HTMLElement) =>
              Math.abs(item.getBoundingClientRect().top + item.offsetHeight / 2 - midY);
            return distance(candidate) < distance(best) ? candidate : best;
          });
          break;
        }
      }
    }
    setKeyboardSelectedTaskId(target.dataset.taskId ?? null);
    setHoverTask(null);
    target.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  };

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();
      const editingTarget = event.target instanceof Element &&
        !!event.target.closest("input, textarea, select, [contenteditable], [role='textbox']");
      const controlTarget = event.target instanceof Element &&
        !!event.target.closest("button, [role='button']");

      if (event.key === "Escape") {
        if (taskDumpOpen && !busy) {
          event.preventDefault();
          setTaskDumpOpen(false);
          return;
        }
        if (contextMenu || columnContextMenu) {
          event.preventDefault();
          setContextMenu(null);
          setMoveSubmenu(null);
          setColumnContextMenu(null);
          return;
        }
        if (editor && !busy) {
          event.preventDefault();
          if (doneMenuOpen) setDoneMenuOpen(false);
          else closeEditor();
          return;
        }
      }

      if (taskDumpOpen || busy) return;
      if (editor) {
        if (event.ctrlKey && !event.altKey && key === "tab") {
          event.preventDefault();
          if (!(editor.mode === "edit" && editor.original?.kind === "epic" && childCounts.has(editor.original.id))) {
            chooseDraftKind(editor.draft.kind === "epic" ? "task" : "epic");
          }
          return;
        }
        if (event.ctrlKey && !event.altKey && key === "u" && editor.draft.kind !== "epic") {
          event.preventDefault();
          chooseDraftKind(isForUser(editor.draft) ? "task" : "for_user");
          return;
        }
        if (event.ctrlKey && !event.altKey && key === "enter") {
          event.preventDefault();
          void saveEditor(editor.mode === "edit" && editor.draft.kind !== "epic");
          return;
        }
        if ((!event.ctrlKey && !event.altKey && key === "enter" && !event.shiftKey && !event.isComposing &&
            !controlTarget && !(event.target instanceof Element && event.target.closest("textarea, [contenteditable]"))) ||
            (event.ctrlKey && !event.altKey && key === "s")) {
          event.preventDefault();
          void saveEditor();
        }
        return;
      }

      if (!event.ctrlKey && !event.metaKey && !event.altKey && !editingTarget && !controlTarget &&
          !reviewOpen) {
        const directions: Record<string, "up" | "down" | "left" | "right"> = {
          arrowup: "up", w: "up", arrowdown: "down", s: "down",
          arrowleft: "left", a: "left", arrowright: "right", d: "right",
        };
        if (directions[key]) {
          event.preventDefault();
          navigateTaskCards(directions[key]);
          return;
        }
        if ((key === " " || key === "spacebar") && keyboardSelectedTaskId) {
          const selected = taskMap.get(keyboardSelectedTaskId);
          if (selected) { event.preventDefault(); openEdit(selected); }
          return;
        }
      }

      if (!event.ctrlKey || event.altKey) return;
      if (key === " " || key === "spacebar") {
        event.preventDefault();
        openCreate();
        return;
      }
      if (key === "r") {
        event.preventDefault();
        setReviewOpen((current) => !current);
        return;
      }
      if (key === "k") {
        event.preventDefault();
        document.querySelector<HTMLInputElement>(".search-wrap input")?.focus();
        return;
      }
      if (key === "q") {
        event.preventDefault();
        startUserQueue();
        return;
      }
      if (key === "t") {
        event.preventDefault();
        openCreate();
        return;
      }
      if (key === "s") {
        event.preventDefault();
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
  }, [editor, taskDumpOpen, busy, contextMenu, columnContextMenu, snapshot, boardPath,
      undoCount, reviewOpen, keyboardSelectedTaskId, query, userQueueTasks, childCounts, doneMenuOpen]);

  useEffect(() => {
    if (reviewOpen) setFilterPanelOpen(false);
  }, [reviewOpen]);

  // The More menu closes on outside click or Escape, like the filter panel.
  useEffect(() => {
    if (!moreMenuOpen) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !moreMenuRef.current?.contains(event.target)) setMoreMenuOpen(false);
    };
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") setMoreMenuOpen(false); };
    document.addEventListener("pointerdown", closeOutside);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOutside);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [moreMenuOpen]);

  useEffect(() => {
    if (!searchResultsOpen) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !searchWrapRef.current?.contains(event.target)) {
        setSearchResultsOpen(false);
      }
    };
    document.addEventListener("pointerdown", closeOutside);
    return () => document.removeEventListener("pointerdown", closeOutside);
  }, [searchResultsOpen]);

  useEffect(() => {
    if (!filterPanelOpen) return;
    const closeOutside = (event: PointerEvent) => {
      if (event.target instanceof Node && !filterControlRef.current?.contains(event.target)) {
        setFilterPanelOpen(false);
      }
    };
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setFilterPanelOpen(false);
    };
    document.addEventListener("pointerdown", closeOutside);
    window.addEventListener("keydown", closeOnEscape);
    return () => {
      document.removeEventListener("pointerdown", closeOutside);
      window.removeEventListener("keydown", closeOnEscape);
    };
  }, [filterPanelOpen]);

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
          <span>{error || `Loading ${project.name} tasks…`}</span>
        </div>
      </main>
    );
  }

  const inboxCount = activeTasks.filter((task) => task.review_status === "open").length + userQueueTasks.length;

  return (
    <main
      className="app-shell"
      onMouseDown={() => {
        if (contextMenu) { setContextMenu(null); setMoveSubmenu(null); }
        if (columnContextMenu) setColumnContextMenu(null);
      }}
    >
      {agentSettingsOpen && <AgentRecoverySettings boardPath={boardPath}
        onClose={() => setAgentSettingsOpen(false)} />}
      {staleDrawerOpen && <StaleBlockerPanel items={staleBlockers} busy={staleAckBusy}
        onClose={() => setStaleDrawerOpen(false)}
        onAcknowledge={(item) => void acknowledgeBlocker(item)}
        onOpenTask={id => {
          setStaleDrawerOpen(false);
          const task = taskMap.get(id);
          if (task) openEdit(task);
        }} />}
      {releasesOpen && <ReleaseCenter boardPath={boardPath} enabled={snapshot.config.releases_enabled ?? false} onClose={() => { setReleasesOpen(false); void refresh(boardPath, false); }} />}
      {queueDrawerOpen && <QueueOverviewPanel boardPath={boardPath} capacity={capacityTelemetry}
        onClose={() => setQueueDrawerOpen(false)}
        onOpenTask={(id) => {
          const task = snapshot.tasks.find(item => item.id === id && !item.archived);
          if (task) { setQueueDrawerOpen(false); openEdit(task); }
          else setError(`Task ${id} is not an active task on this board`);
        }}
        onChanged={() => void refresh(boardPath, true)} />}
      {reviewNotice ? <div className="review-reconciliation-notice" role="status">{reviewNotice}</div> : null}
      <header className="topbar">
        <div className="topbar-title" title={boardPath}>
          <h1>{snapshot.config.name}</h1>
          <span className="topbar-meta"><span className="status-dot" />
            {activeTasks.length} active · {snapshot.tasks.filter((task) => task.archived).length} archived</span>
        </div>
        <div className="review-mode-switch" role="group" aria-label="TaskBoard view">
          <button type="button" className={reviewOpen ? "" : "active"}
            onClick={() => setReviewOpen(false)}>Board</button>
          <button type="button" className={reviewOpen ? "active" : ""}
            onClick={() => setReviewOpen(true)} disabled={!!editor}
            title="Open unified feedback inbox: visual reviews and For user quick replies (Ctrl+R)">
            Inbox {inboxCount > 0 && <span className="review-mode-count attention">{inboxCount}</span>}
          </button>
          <button type="button" className={queueDrawerOpen ? "active" : ""}
            onClick={() => setQueueDrawerOpen(open => !open)} disabled={!!editor}
            aria-expanded={queueDrawerOpen} title="Open exact OpenRig queue overview">
            Queue <span className="review-mode-count">{queueTelemetry?.hasMore ? `${queueTelemetry.limit}+`
              : queueTelemetry?.activeCount ?? "?"}</span>
          </button>
        </div>
        <span className="queue-capacity-chip" role="status"
          title={capacityTelemetry?.reason ?? "Current verified T3/OpenRig capacity"}>
          <span className={`capacity-dot${capacityTelemetry && capacityTelemetry.working > 0 ? " busy" : ""}`} />
          {capacityTelemetry?.reason === null && capacityTelemetry.unknown === 0
            ? capacityTelemetry.working : "?"}/{capacityTelemetry?.limit ?? "?"} agents
        </span>
        <div className="toolbar">
          <div className="search-wrap" ref={searchWrapRef}>
            <span>⌕</span>
            <input
              value={query}
              onFocus={() => setSearchResultsOpen(true)}
              onChange={(event) => { setQuery(event.currentTarget.value); setSearchResultsOpen(true); }}
              placeholder="Search tasks"
              aria-label="Search tasks"
              title="Focus search (Ctrl+K)"
              onKeyDown={(event) => { if (event.key === "Escape") setQuery(""); }}
            />
            {query && <button type="button" className="search-clear" aria-label="Clear task search"
              title="Clear filter (Esc)" onClick={() => setQuery("")}>×</button>}
            {searchResultsOpen && query.trim().length >= 3 && (visibleTasks.length > 0 || remoteSearchMatches.length > 0) &&
              <div className="search-results" role="group" aria-label="Search results">
                {visibleTasks.slice(0, 5).map((task) => <button type="button" key={task.id}
                  onClick={() => { setQuery(""); openEdit(task); }}>
                  <strong>{task.id}</strong> {task.title}
                </button>)}
                {remoteSearchMatches.length > 0 && <small>Other project boards</small>}
                {remoteSearchMatches.map((task) => <button type="button" className="remote-search-result"
                  key={`${task.path}#${task.id}`}
                  onClick={() => {
                    setQuery("");
                    window.dispatchEvent(new CustomEvent("taskboard:open-search-result", {
                      detail: { path: task.path, taskId: task.id },
                    }));
                  }}>
                  <strong>{task.id}</strong> {task.title} <em>{task.project}</em>
                </button>)}
              </div>}
          </div>
          {!reviewOpen && <div className="task-filter-control" ref={filterControlRef}>
            <button type="button"
              className={`button secondary task-filter-trigger${activeFilterCount ? " has-filters" : ""}`}
              onClick={() => setFilterPanelOpen(open => !open)}
              aria-expanded={filterPanelOpen}
              aria-haspopup="dialog"
              title="Filter cards by icon, column, readiness, type and attention states">
              <svg viewBox="0 0 20 20" width="15" height="15" aria-hidden="true"
                fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round">
                <path d="M2.5 4h15l-5.8 6.4v5.1l-3.4 1.8v-6.9z" />
              </svg>
              Filters{activeFilterCount > 0 && <span className="task-filter-count">{activeFilterCount}</span>}
            </button>
            {filterPanelOpen && <TaskFilterPanel
              filters={taskFilters}
              columns={columns.map(column => ({ id: column.id, name: column.name }))}
              matchingCount={visibleTasks.length}
              onChange={setTaskFilters}
              onClear={() => setTaskFilters({
                icons: [], noIcon: false, columns: [], readiness: [], kinds: [], attention: [],
              })}
              onClose={() => setFilterPanelOpen(false)}
            />}
          </div>}
          {!reviewOpen && <button type="button"
            className={`toolbar-toggle${hideWaitingTasks ? " on" : ""}`}
            aria-pressed={hideWaitingTasks}
            onClick={() => setHideWaitingTasks((value) => !value)}
            title="Hide tasks that cannot start until prerequisites finish. Does not change their status.">
            Hide waiting
          </button>}
          <div className="toolbar-icons" role="group" aria-label="Board actions">
            <button type="button" className="icon-tool" onClick={() => void undoLast()}
              disabled={busy || undoCount === 0 || !!editor}
              title={undoCount ? `Undo last action (Ctrl+Z) · ${undoCount} available` : "Nothing to undo"}
              aria-label="Undo">
              <ToolbarIcon name="undo" />
            </button>
            <button type="button" className={`icon-tool${refreshFeedback === "working" ? " spinning" : refreshFeedback === "done" ? " confirmed" : ""}`}
              onClick={() => window.dispatchEvent(new Event("taskboard:refresh-all"))}
              disabled={busy || refreshFeedback === "working"}
              title={refreshFeedback === "done" ? "Updated" : "Refresh all project boards from Markdown files (F5)"}
              aria-label="Refresh">
              <ToolbarIcon name="refresh" />
            </button>
            <button type="button" className="icon-tool" onClick={toggleAllEpicsCollapsed}
              disabled={busy || !epicIdsWithChildren.length}
              title={allEpicsCollapsed ? "Expand all epics" : "Collapse all epics (orange For user tasks stay visible)"}
              aria-label={allEpicsCollapsed ? "Expand all epics" : "Collapse all epics"}>
              <ToolbarIcon name={allEpicsCollapsed ? "expand" : "collapse"} />
            </button>
          </div>
          <div className="toolbar-more" ref={moreMenuRef}>
            <button type="button" className={`icon-tool${moreMenuOpen ? " open" : ""}`}
              aria-haspopup="menu" aria-expanded={moreMenuOpen} title="More TaskBoard actions"
              onClick={() => setMoreMenuOpen((open) => !open)}>
              <ToolbarIcon name="more" />
              {(staleBlockers.length > 0 || userQueueTasks.length > 0) && <span className="more-dot" />}
            </button>
            {moreMenuOpen && <div className="toolbar-more-menu" role="menu">
              <div className="menu-section">Board</div>
              <button type="button" role="menuitem" onClick={() => { setMoreMenuOpen(false); addColumn(); }} disabled={busy}>
                <ToolbarIcon name="column" /><span>Add column</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreMenuOpen(false); setReleasesOpen(true); }} disabled={busy}>
                <ToolbarIcon name="releases" /><span>Releases</span>
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreMenuOpen(false); openTaskDump(); }} disabled={busy || !!editor}
                title="Paste one task per line (Ctrl+V on the board also opens Task Dump)">
                <ToolbarIcon name="dump" /><span>Task dump</span><kbd>Ctrl+V</kbd>
              </button>
              <div className="menu-section">Agents &amp; people</div>
              <button type="button" role="menuitem" onClick={() => { setMoreMenuOpen(false); startUserQueue(); }}
                disabled={busy || userQueueTasks.length === 0} title="Direct For user quick-reply queue">
                <ToolbarIcon name="user" /><span>For user queue</span>
                {userQueueTasks.length > 0 ? <em className="menu-count warm">{userQueueTasks.length}</em> : <kbd>Ctrl+Q</kbd>}
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreMenuOpen(false); setStaleDrawerOpen(true); }}
                title="Show genuine stale technical blockers, excluding dependency waiting. No workers are spawned.">
                <ToolbarIcon name="blockers" /><span>Recheck blockers</span>
                {staleBlockers.length > 0 && <em className="menu-count danger">{staleBlockers.length}</em>}
              </button>
              <button type="button" role="menuitem" onClick={() => { setMoreMenuOpen(false); setAgentSettingsOpen(true); }}
                title="Configure resume window, idle wake, missing-session grace and external leases">
                <ToolbarIcon name="agents" /><span>Agent recovery settings</span>
              </button>
              <div className="menu-footer" title={boardPath}><ToolbarIcon name="folder" size={13} /><span>{boardPath}</span></div>
            </div>}
          </div>
          <button className="button primary new-task-button" onClick={() => openCreate()} disabled={busy}
            title="Create a task (Ctrl+Space or Ctrl+T)">
            <ToolbarIcon name="plus" size={14} /> Task
          </button>
        </div>
      </header>

      {error && <div className="error-banner">{error}</div>}
      {startingTaskIds.length > 0 && <div className="warning-banner" role="status">
        Starting {startingTaskIds.join(", ")} in T3 in the background. TaskBoard remains usable.
      </div>}
      {autoSavedDraft && <div className="draft-saved-toast" role="status">
        <span className="draft-saved-toast-icon" aria-hidden="true">✓</span>
        <div className="draft-saved-toast-copy">
          <strong>Saved to Drafts</strong>
          <span title={autoSavedDraft.title}>{autoSavedDraft.id} · {autoSavedDraft.title}</span>
          <small>Auto-deletes in 24 hours unless edited.</small>
        </div>
        <button type="button" className="draft-saved-toast-show" onClick={() => {
          const task = snapshot.tasks.find((item) => item.id === autoSavedDraft.id);
          setAutoSavedDraft(null);
          if (!task) return;
          setQuery("");
          window.setTimeout(() => {
            const card = document.querySelector<HTMLElement>(`[data-task-id="${CSS.escape(task.id)}"].task-card`);
            if (card) {
              card.scrollIntoView({ behavior: "smooth", block: "center", inline: "center" });
              flashTaskActivity([{ id: task.id, kind: "created" }]);
            } else {
              openEdit(task);
            }
          }, 90);
        }}>Show</button>
        <button type="button" className="draft-saved-toast-dismiss" aria-label="Dismiss draft notification"
          onClick={() => setAutoSavedDraft(null)}>×</button>
      </div>}
      {queueRecoveryWarning && <div className="queue-attention-banner" role="alert">
        <span><strong>Queue needs attention</strong> <span title={queueRecoveryWarning}>{queueRecoveryWarning}</span></span>
        <button type="button" className="button secondary" disabled={queueRecoveryAgentBusy}
          onClick={() => void spawnQueueRecoveryAgent()}
          title="Open a task-bound worker to investigate this genuine Queue integrity issue">
          {queueRecoveryAgentBusy ? "Opening agent…" : "Spawn agent"}
        </button>
      </div>}
      {!!snapshot.warnings.length && (
        <div className="warning-banner">{snapshot.warnings.join(" • ")}</div>
      )}

      <section
        className="board"
        style={reviewOpen ? { display: "none" } : undefined}
        ref={boardRef}
        onPointerDown={startBoardPan}
        onPointerMove={moveBoardPan}
        onPointerUp={endBoardPan}
        onPointerCancel={endBoardPan}
        onContextMenu={(event) => event.preventDefault()}
      >
        {columns.map((column) => {
          const allColumnTasks = visibleTasks.filter((task) => task.column === column.id);
          const inReviewCount = column.id === "blocked"
            ? allColumnTasks.filter(isInReviewInbox).length : 0;
          const eligibleColumnQueue = activeTasks.filter(task => task.column === column.id && queueEligible(task));
          const queuedColumnCount = eligibleColumnQueue.filter(task => executionModes[task.id]?.mode === "queue").length;
          const columnQueueMixed = queuedColumnCount > 0 && queuedColumnCount < eligibleColumnQueue.length;
          const columnTasks = orderCardsByDependencies(applyCardPlacements(allColumnTasks
            .filter((task) => !(column.id === "blocked" && hideBlockedInboxTasks && isInReviewInbox(task)))
            .filter((task) => !hideWaitingTasks || taskFilters.readiness.includes("waiting") || isComplete(task.id) || task.column === "in-progress" || readinessOf(task) !== "waiting")
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
              {column.id === "blocked" && (
                <label className="blocked-review-filter" title="Hide blocked tasks currently awaiting your attention in Inbox. Tasks are not modified.">
                  <input type="checkbox" checked={hideBlockedInboxTasks}
                    onChange={(event) => setHideBlockedInboxTasks(event.currentTarget.checked)}
                    onPointerDown={(event) => event.stopPropagation()} />
                  <span>Hide tasks in Inbox</span>
                  <small>{inReviewCount}</small>
                </label>
              )}
              <div
                className="column-header"
                onMouseDown={(event) => beginColumnDrag(event, column)}
                title="Drag to reorder"
              >
                <div className="column-title">
                  <span className="column-name" title={column.id === "blocked" ? "Waiting" : column.name}>{column.id === "blocked" ? "Waiting" : column.name}</span>
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
                  {queueableColumn(column.id) && <button type="button"
                    className={`queue-intent-toggle column-queue-toggle${column.auto_queue ? " queued" : ""}${columnQueueMixed ? " mixed" : ""}`}
                    title={`${column.auto_queue ? "Disable" : "Enable"} ${column.name} auto-queue for current and future eligible tasks · ${queuedColumnCount}/${eligibleColumnQueue.length} queued`}
                    aria-label={`${column.auto_queue ? "Disable" : "Enable"} ${column.name} auto-queue`}
                    aria-pressed={!!column.auto_queue}
                    disabled={busy}
                    onClick={() => void toggleColumnQueue(column.id)}><QueueArrows /></button>}
                  <button type="button" className="column-quick-add" title={`Add task to ${column.name}`}
                    aria-label={`Add task to ${column.name}`} onClick={() => openCreate(column.id)}>+</button>
                  {column.id !== "draft" && !column.terminal && (
                    <button
                      className={`column-ai-ready ${column.advisory ? "advisory" : column.ai_ready ? "active" : ""}`}
                      title={
                        column.id === "ai-ready"
                          ? "Canonical AI Ready column"
                          : column.ai_ready
                            ? "AI Ready (green) — click for Advisory (yellow)"
                            : column.advisory
                              ? "Advisory: temporary instructions shown to all board agents — click for Off"
                              : "Off — click for AI Ready (green)"
                      }
                      disabled={column.id === "ai-ready"}
                      onClick={() => cycleColumnMode(column.id)}
                    >
                      {column.advisory ? "!" : "✓"}
                    </button>
                  )}
                  <button className="column-secondary-action" title="Collapse column" onClick={() => toggleColumnHidden(column.id)}>│</button>
                  <button className="column-secondary-action" title="Rename" onClick={() => renameColumn(column.id)}>✎</button>
                  <button className="column-secondary-action" title="Delete" onClick={() => deleteColumn(column.id)}>×</button>
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
                        className={`epic-proxy${highlightedEpicId === epic.id ? " epic-proxy-active" : ""}${dragId === epic.id && dragSourceColumn === column.id ? " drag-source" : ""}`}
                        data-epic-proxy={epic.id}
                        title={`${epic.id}: ${epic.title}. Drag to move only this column's cards. Click to jump to the original in ${realColumn?.name ?? epic.column}.`}
                        aria-label={`Epic proxy ${epic.id}: ${epic.title}. Click to jump to the real epic.`}
                        onPointerDown={(event) => event.stopPropagation()}
                        onMouseDown={(event) => { event.stopPropagation(); beginCardDrag(event, epic, column.id); }}
                        onMouseEnter={() => { setHoverTask(null); setHighlightedEpicId(epic.id); }}
                        onMouseLeave={() => setHighlightedEpicId((current) => current === epic.id ? null : current)}
                        onFocus={() => setHighlightedEpicId(epic.id)}
                        onBlur={() => setHighlightedEpicId((current) => current === epic.id ? null : current)}
                        onClick={() => { if (Date.now() >= suppressEpicClickUntil.current) jumpToOriginalEpic(epic); }}
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
                      className={`task-card ${keyboardSelectedTaskId === task.id ? "keyboard-selected" : ""} ${task.kind === "epic" ? "epic-card" : isForUser(task) ? "for-user-card" : task.kind === "recurring" ? "recurring-card" : ""} ${task.attention.includes("priority") ? "priority-card" : ""} ${highlightedEpicId === task.id ? "epic-proxy-highlight" : ""} ${dragId === task.id && dragSourceColumn === column.id ? "drag-source" : ""} ${
                        cardDropTargetId === task.id ? (task.kind === "epic" ? "drop-parent-target" : "drop-after-target") : ""
                      } ${
                        taskActivity[task.id]
                          ? `activity-${taskActivity[task.id]}`
                          : ""
                      } ${recentTaskActivity[task.id] ? "recently-changed" : ""} ${task.claimed_by ? `claim-${(claimHealth[task.id]?.agentState ?? "unknown").toLowerCase().replace(/[^a-z]/g, "")}` : ""}`}
                      key={task.id}
                      data-task-id={task.id}
                      style={{ marginLeft: Math.min(depth, 4) * 12 }}
                      onMouseDown={(event) => { setKeyboardSelectedTaskId(task.id); beginCardDrag(event, task, column.id); }}
                      onClick={() => {
                        if (task.kind !== "epic" || Date.now() < suppressEpicClickUntil.current) return;
                        if (epicSingleClickTimer.current) clearTimeout(epicSingleClickTimer.current);
                        epicSingleClickTimer.current = setTimeout(() => {
                          epicSingleClickTimer.current = null;
                          openEdit(task);
                        }, 230);
                      }}
                      onDoubleClick={(event) => {
                        if (task.kind !== "epic") return;
                        event.preventDefault();
                        if (epicSingleClickTimer.current) clearTimeout(epicSingleClickTimer.current);
                        epicSingleClickTimer.current = null;
                        toggleTaskCollapsed(task.id);
                      }}
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
                        {task.kind === "recurring" && <span className="recurring-badge"
                          title={task.next_due ? `Next run ${task.next_due}; ${task.run_count ?? 0} completed` : "Recurring: available after each recorded run"}>↻ {task.next_due || "RECURRING"}</span>}
                        {task.claimed_by && <span className={`claim-indicator ${claimHealth[task.id]?.agentState === "Working" ? "working" : ""}`}
                          title={claimTooltip(task)} aria-label={claimTooltip(task)} />}
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
                        <span className="task-meta-spacer" />
                        {queueEligible(task) && <button type="button"
                          className={`queue-intent-toggle card-queue-toggle${executionModes[task.id]?.mode === "queue" ? " queued" : ""}`}
                          title={executionModes[task.id]?.mode === "queue"
                            ? `Queued · ${queueReconciliationOffline ? "OpenRig unavailable, pending sync" :
                                queueSyncStatuses[task.id] ? `${queueSyncStatuses[task.id].status}: ${queueSyncStatuses[task.id].reason ?? "exact receipt tracked"}` :
                                  "durable intent, awaiting reconciliation"} · click to return to Backlog`
                            : "Backlog · click to Queue"}
                          aria-label={`${executionModes[task.id]?.mode === "queue" ? "Unqueue" : "Queue"} ${task.id}`}
                          aria-pressed={executionModes[task.id]?.mode === "queue"}
                          disabled={busy || !executionModes[task.id]}
                          onPointerDown={event => event.stopPropagation()}
                          onMouseDown={event => event.stopPropagation()}
                          onClick={event => { event.stopPropagation(); void toggleTaskQueue(task); }}><QueueArrows /></button>}
                        {task.kind === "task" && !isForUser(task) && readinessOf(task) !== "ready" && (
                          <span className={`readiness-badge ${readinessOf(task)}`}
                            title={readinessOf(task) === "waiting" ? awaitingUserAnswer(task) ? "Waiting for your answer" : `Waiting for ${waitingFor(task).join(", ")}` : readinessOf(task) === "ready" ? "Ready to start" : "Blocked"}
                            aria-label={readinessOf(task) === "ready" ? "Ready to start" : readinessOf(task) === "waiting" ? awaitingUserAnswer(task) ? "Waiting for your answer" : `Waiting for ${waitingFor(task).join(", ")}` : "Blocked"}>
                            {readinessOf(task) === "waiting" ? awaitingUserAnswer(task) ? "WAITING" : `WAIT ${waitingFor(task).length}` : readinessOf(task) === "blocked" ? "BLOCKED" : "\u25B6"}
                          </span>
                        )}
                        {!!task.attention.length && (
                          <div className="task-attention-icons">
                            {task.attention.includes("question") && (
                              <span className="attention-icon question" title="Question">?</span>
                            )}
                            {task.attention.includes("blocked") && (
                              <span className="attention-icon blocked" title="Blocked">×</span>
                            )}
                            {task.attention.includes("priority") && (
                              <span className="attention-icon priority" title="Priority — handle before ordinary ready tasks" aria-label="Priority">!</span>
                            )}
                            {task.attention.includes("prepare") && (
                              <span className="attention-icon prepare" title="Preparation requested — implementation waits until clarified" aria-label="Preparation">✦</span>
                            )}
                          </div>
                        )}
                        {task.claimed_by && (
                          <button type="button" className="task-more-button"
                            aria-label={`More actions for ${task.id}`}
                            title="Task actions, including open claimed T3 session"
                            onPointerDown={(event) => event.stopPropagation()}
                            onMouseDown={(event) => event.stopPropagation()}
                            onClick={(event) => {
                              event.stopPropagation();
                              const rect = event.currentTarget.getBoundingClientRect();
                              setContextMenu({
                                taskId: task.id,
                                x: Math.min(rect.right, window.innerWidth - 240),
                                y: Math.min(rect.bottom, window.innerHeight - 410),
                              });
                            }}>···</button>
                        )}
                      </div>
                      <h3><TitleMarkup title={task.title} /></h3>
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

            </div>
          );
        })}
      </section>

      {reviewOpen && <div className="review-unified-backdrop" role="dialog" aria-modal="true"
          aria-label="Unified human feedback queue">
          <ReviewView project={project}
            onClose={() => { setReviewOpen(false); setInboxResumeId(undefined); }}
            initialTaskId={inboxResumeId === undefined ? requestedReviewId : inboxResumeId}
            userTasks={userQueueTasks}
            onUserTaskUpdated={(notice) => {
              if (notice) setReviewNotice(notice);
               window.dispatchEvent(new Event("taskboard:queue-changed"));
              void loadBoard(boardPath).then(applySnapshot).catch((cause) => setError(String(cause)));
            }}
            onSnoozeUserTask={(task, path, until) => {
              if (path === boardPath) {
                setUserQueueSnoozes((current) => ({
                  ...current, [task.id]: until,
                }));
              }
            }}
            onOpenUserTask={(task) => {
              setInboxEditorTaskId(task.id);
              setReviewOpen(false);
              openEdit(task);
            }} />
        </div>}

      {!reviewOpen && !editor && <DependencyArrows boardRef={boardRef} visibleTasks={visibleTasks}
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
            {hoverTask.task.claimed_by && <span title={claimTooltip(hoverTask.task)}>{claimTooltip(hoverTask.task)}</span>}
            {hoverTask.task.kind === "epic" && <span>Coordination only  -  drag to move tasks in this column</span>}
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
              <MarkdownText text={parseDescriptionTimeline(hoverTask.task.description).initial} />
              {parseDescriptionTimeline(hoverTask.task.description).updates.map((update, index) => (
                <div className={`hover-description-update ${update.kind}`} key={index}>
                  <small>{update.kind === "user-note" ? "Note to AI" :
                    update.kind === "legacy-ai" ? "Imported AI notes" :
                    update.kind === "blocked" ? "Blocked" : "Task update"} · {new Date(update.at).toLocaleString()}</small>
                  <MarkdownText text={update.text} />
                </div>
              ))}
            </section>
          )}
          {!!hoverTask.task.acceptance_criteria.length && (
            <section className="hover-preview-section">
              <strong>Acceptance criteria</strong>
              <ul>{hoverTask.task.acceptance_criteria.map((criterion, index) =>
                <li key={index}><MarkdownText text={criterion} /></li>)}</ul>
            </section>
          )}
          {hoverTask.task.question && (
            <section className="hover-preview-section">
              <strong>Question {hoverTask.task.question.answer ? "(answered)" : "(open)"}</strong>
              <MarkdownText text={hoverTask.task.question.prompt} />
            </section>
          )}
          {hoverTask.task.user_notes.trim() && (
            <section className="hover-preview-section user">
              <strong>User notes</strong>
              <MarkdownText text={hoverTask.task.user_notes} />
            </section>
          )}
          {hoverTask.task.ai_notes.trim() && (
            <section className="hover-preview-section ai">
              <strong>Historical AI notes (moved to timeline on next save)</strong>
              <MarkdownText text={hoverTask.task.ai_notes} />
            </section>
          )}
          {hoverTask.task.notes.trim() && (
            <section className="hover-preview-section">
              <strong>Notes</strong>
              <MarkdownText text={hoverTask.task.notes} />
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
          {contextTask.kind !== "epic" && (
            <button disabled={busy || (!hasTaskSession(contextTask) && !canSpawnTask(contextTask))} onClick={() => {
              setContextMenu(null);
              openForTask(contextTask);
            }} title={hasTaskSession(contextTask) ? "Open the linked T3 session" : contextTask.live?.with_me ? "Start a conversation with me" : "Spawn a T3 implementation worker for this task"}>
              {hasTaskSession(contextTask) ? "Open Session" : contextTask.live?.with_me ? "Start session with me" : "Spawn"}
            </button>
          )}
          {contextTask.kind !== "epic" && <button type="button"
            title="Navigate to a previously linked T3 browser session, even if hidden or unloaded"
            onClick={() => {
              setContextMenu(null);
              void taskOpenOriginSession(boardPath, contextTask.id).catch(error =>
                setError(`Could not open previous session: ${String(error)}`));
            }}>Open previous session</button>}
          {contextTask.kind !== "epic" && <button type="button"
            title="Copy verified task provenance, user decisions, artifacts and possible uncommitted work to clipboard"
            onClick={() => {
              setContextMenu(null);
              void taskHandoffBundle(boardPath,contextTask.id)
                .then(packet => navigator.clipboard.writeText(JSON.stringify(packet,null,2)))
                .catch(error=>setError(`Handoff unavailable: ${String(error)}`));
            }}>Copy worker handoff</button>}
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
                disabled={busy}
                className="priority-context-action"
                aria-pressed={contextTask.attention.includes("priority")}
                onClick={() => {
                  const id = contextTask.id;
                  setContextMenu(null);
                  void persistTaskMutation(id, "toggle priority", (task) => ({
                    ...task,
                    attention: task.attention.includes("priority")
                      ? task.attention.filter((flag) => flag !== "priority")
                      : [...task.attention, "priority"],
                  }));
                }}
              >{contextTask.attention.includes("priority") ? "✓ Remove priority" : "! Mark priority"}</button>
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
          {contextTask.claimed_by && contextTask.column === "in-progress" && <button
            title="Release the current claim and return the task to AI Ready"
            disabled={busy} onClick={() => {
              setContextMenu(null);
              void persistTaskMutation(contextTask.id, "release claim", task => ({
                ...task, column: "ai-ready", claimed_by: null,
              }));
            }}>Release claim</button>}
          <button onClick={() => void copyTaskText(contextTask.id)}>Copy task ID</button>
          <button onClick={() => void copyTaskText(`${contextTask.id} ${contextTask.title}`)}>Copy task ID + title</button>
          <button onClick={() => void copyTaskText(contextTask.path)}>Copy full file path</button>
          <button
            onClick={() => void copyTaskText(contextTask.kind === "epic"
                ? `In ${project.name}, review epic ${contextTask.id} and choose a ready executable subtask. Do not implement or claim the epic itself.`
                : contextTask.kind === "for_user"
                  ? `Review human-only task ${contextTask.id} in TaskBoard, prepare useful context, then ask the user to take the required action. Do not claim, complete, or archive it on their behalf.`
                  : `In ${project.name}, implement ${contextTask.id} from the TaskBoard.`)}
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

      {taskDumpOpen && (
        <div className="modal-backdrop" onMouseDown={() => { if (!busy) setTaskDumpOpen(false); }}>
          <section className="task-modal task-dump-modal" onMouseDown={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <div>
                <div className="eyebrow">BATCH CREATE · {project.name.toUpperCase()}</div>
                <h2>Task Dump</h2>
              </div>
              <button type="button" className="icon-button" aria-label="Close Task Dump"
                onClick={() => setTaskDumpOpen(false)} disabled={busy}>×</button>
            </div>
            <p className="task-dump-help">Enter one task per line. Check priority beside any line before generating.</p>
            <div className="task-dump-options">
              <label>Column
                <select value={taskDumpColumn} disabled={busy}
                  onChange={(event) => setTaskDumpColumn(event.currentTarget.value)}>
                  {columns.map((column) => (
                    <option key={column.id} value={column.id}>{column.name}</option>
                  ))}
                </select>
              </label>
              <label>Shared epic (optional)
                <select value={taskDumpEpic} disabled={busy}
                  onChange={(event) => setTaskDumpEpic(event.currentTarget.value)}>
                  <option value="">No epic</option>
                  <option value="__new__">+ Create new epic</option>
                  {activeTasks.filter((task) => task.kind === "epic")
                    .sort((a, b) => a.title.localeCompare(b.title))
                    .map((task) => (
                      <option key={task.id} value={task.id}>{task.id} — {task.title}</option>
                    ))}
                </select>
              </label>
              {taskDumpEpic === "__new__" && <label>New epic title
                <input type="text" value={taskDumpNewEpicTitle} disabled={busy}
                  placeholder="Name this group"
                  onChange={(event) => setTaskDumpNewEpicTitle(event.currentTarget.value)} />
              </label>}
            </div>
            <textarea className="task-dump-input" aria-label="Task Dump input" autoFocus
              value={taskDumpText} disabled={busy}
              placeholder={"Fix pathfinding on rivers\nImprove minimap readability\nAdd more detailed combat logs"}
              onChange={(event) => setTaskDumpText(event.currentTarget.value)} />
            <div className="task-dump-preview" aria-label="Tasks to create">
              {taskDumpRows.length ? taskDumpRows.map((row) => (
                <div className="task-dump-row" key={row.line}>
                  <label className="task-dump-priority" title="Mark this generated task as priority">
                    <input type="checkbox" disabled={busy}
                      checked={!!taskDumpPriorities[row.line]}
                      onChange={(event) => setTaskDumpPriorities((current) => ({
                        ...current, [row.line]: event.currentTarget.checked,
                      }))} />
                    Priority
                  </label>
                  <span>{row.text}</span>
                </div>
              )) : <span className="task-dump-empty">Paste or type tasks above to preview them here.</span>}
            </div>
            {error && <div className="error-banner">{error}</div>}
            <div className="task-dump-footer">
              <span>{taskDumpRows.length} task{taskDumpRows.length === 1 ? "" : "s"} · {taskDumpRows.filter((row) => taskDumpPriorities[row.line]).length} priority</span>
              <div className="spacer" />
              <button type="button" className="button secondary" disabled={busy}
                onClick={() => setTaskDumpOpen(false)}>Cancel</button>
              <button type="button" className="button primary"
                disabled={busy || !taskDumpRows.length || !columns.some((column) => column.id === taskDumpColumn) ||
                  (taskDumpEpic === "__new__" && !taskDumpNewEpicTitle.trim())}
                onClick={() => void generateTaskDump()}>{busy ? "Generating…" : "Generate"}</button>
            </div>
          </section>
        </div>
      )}

      {editor && (
        <div className="modal-backdrop task-editor-backdrop" onMouseDown={() => !busy && closeEditor()}>
          <section ref={editorModalRef}
            className={`task-modal ${isAdvisoryEditor ? "advisory-editor" : ""} ${isQueueCompact ? "queue-compact-modal" : ""} ${fileDropActive ? "file-drop-active" : ""}`}
            onMouseDown={(event) => {
              event.stopPropagation();
              if (!(event.target as HTMLElement).closest(".editor-finish-menu")) setDoneMenuOpen(false);
            }} onPaste={pasteClipboardImages}>
            {fileDropActive && <div className="editor-file-drop-hint">Drop images to attach or files to link in Note to AI</div>}
            <div className="modal-header">
              <div className="eyebrow">
                {editor.mode === "edit" && editor.original ? (
                  <button type="button" className={`reserved-id-copy${copiedTaskId === editor.original.id ? " copied" : ""}`} title="Copy task ID" aria-label={`Copy task ID ${editor.original.id}`} onClick={() => void copyTaskText(editor.original!.id)}>{editor.original.id} <span aria-hidden="true">{copiedTaskId === editor.original.id ? "✓ Copied" : "⧉"}</span></button>
                ) : reservedTaskId ? (
                  <button type="button" className={`reserved-id-copy${copiedTaskId === reservedTaskId.id ? " copied" : ""}`} title="Copy task ID" onClick={() => void copyTaskText(reservedTaskId.id)} onContextMenu={(event) => { event.preventDefault(); void copyTaskText(reservedTaskId.id); }}>{reservedTaskId.id} <span aria-hidden="true">{copiedTaskId === reservedTaskId.id ? "✓ Copied" : "⧉"}</span></button>
                ) : "RESERVING ID…"}
                {userQueueReview && editor.original?.id === userQueueReview.ids[userQueueReview.index] && (
                  <span className="user-queue-progress">
                    USER QUEUE · {userQueueReview.index + 1} / {userQueueReview.ids.length}
                  </span>
                )}
              </div>
              {!isAdvisoryEditor && <div className="task-kind-switch" role="group" aria-label="Card type">
                {!isQueueCompact && <div className="task-kind-switch-options">
                  <button type="button"
                    className={`kind-option${editor.draft.kind === "task" || editor.draft.kind === "for_user" ? " active" : ""}`}
                    aria-pressed={editor.draft.kind === "task" || editor.draft.kind === "for_user"}
                    disabled={editor.mode === "edit" && editor.original?.kind === "epic" && childCounts.has(editor.original.id)}
                    title="Toggle Task/Epic (Ctrl+Tab)"
                    onClick={() => chooseDraftKind("task")}>Task</button>
                  <button type="button"
                    className={`kind-option${editor.draft.kind === "epic" ? " active epic" : ""}`}
                    aria-pressed={editor.draft.kind === "epic"}
                    title="Toggle Task/Epic (Ctrl+Tab)"
                    onClick={() => chooseDraftKind("epic")}>Epic</button>
                  <button type="button"
                    className={`kind-option${editor.draft.kind === "recurring" ? " active recurring" : ""}`}
                    aria-pressed={editor.draft.kind === "recurring"}
                    title="Repeating work item: completion records a run and reopens it"
                    onClick={() => chooseDraftKind("recurring")}>↻ Recurring</button>
                </div>}
                {editor.draft.kind !== "epic" && <button type="button"
                  className={`kind-user-option${isForUser(editor.draft) ? " active" : ""}`}
                  aria-pressed={isForUser(editor.draft)}
                  title="Toggle For user (Ctrl+U)"
                  onClick={() => chooseDraftKind(editor.draft.kind === "for_user" || editor.draft.attention.includes("needs-user") ? "task" : "for_user")}>
                    {isForUser(editor.draft) ? "●" : "○"} For user
                </button>}
              </div>}
              {editor.mode === "edit" && !!editor.original && hasTaskSession(editor.original) && (
                <button type="button" className="button secondary" disabled={busy}
                  title="Open the linked T3 session"
                  onClick={() => openForTask(editor.original!)}>
                  Open Session
                </button>
              )}
              <button className="icon-button" aria-label="Close editor" onClick={() => closeEditor()} disabled={busy}>×</button>
            </div>

            {editor.mode === "edit" && editor.original && editor.original.kind !== "epic" &&
              <TaskTracePanel path={boardPath} taskId={editor.original.id} />}
            {isQueueCompact ? (
              <h2 className="queue-compact-title">{editor.draft.title}</h2>
            ) : <input
              className="task-title-input"
              autoFocus
              aria-label="Title"
              onKeyDown={(event) => {
                if (event.key !== "Tab" || event.shiftKey || event.ctrlKey || event.metaKey) return;
                const description = event.currentTarget.closest(".task-modal")
                  ?.querySelector<HTMLTextAreaElement>("textarea.description-initial");
                if (!description) return;
                event.preventDefault();
                description.focus();
              }}
              placeholder={editor.draft.kind === "epic" ? "Epic title" : "Task title"}
              value={editor.draft.title}
              onChange={(event) =>
                setEditor({ ...editor, draft: { ...editor.draft, title: event.currentTarget.value } })
              }
            />}

            {isQueueCompact ? (
              <div className="queue-compact-fields">
                <div className="queue-compact-summary">
                  {parseDescriptionTimeline(editor.draft.description).initial.slice(0, 550)
                    || "Your response will be recorded on this task."}
                </div>
                {editor.draft.question?.prompt && !editor.draft.question.answer && (
                  <div className="question-panel">
                    <strong>{editor.draft.question.prompt}</strong>
                    <div className="question-options">
                      {editor.draft.question.options.map((option) =>
                        <button className="question-option" key={option} onClick={() => void answerQuestion(option)}
                          disabled={busy}>{option}</button>)}
                    </div>
                  </div>
                )}
                <label>Note to AI
                  <AutoTextarea minRows={5} placeholder="Your instructions or decision..."
                    value={editor.draft.user_notes} onChange={(event) => setEditor((current) => {
                      if (!current) return null;
                      const note = event.currentTarget.value;
                      return { ...current, draft: {
                        ...current.draft,
                        user_notes: note,

                      } };
                    })} />
                </label>
                <MarkdownPreviewToggle text={editor.draft.user_notes} />
                <label>Acceptance criteria
                  <AutoTextarea minRows={3} placeholder="One per line"
                    value={editor.draft.acceptance_criteria.join("\n")}
                    onChange={(event) => setEditor((current) => current ? {
                      ...current, draft: {
                        ...current.draft,
                        acceptance_criteria: event.currentTarget.value.split("\n"),
                      },
                    } : current)} />
                </label>
                <MarkdownPreviewToggle text={editor.draft.acceptance_criteria.join("\n\n")} />
                <div className="queue-compact-actions">
                  <button className="button secondary" type="button"
                    onClick={() => setQueueExpanded(true)}>Full task editor ↗</button>
                  <button className="button secondary" type="button"
                    onClick={() => { closeEditor(); setReviewOpen(true); }}>Visual reviews ↗</button>
                </div>
              </div>
            ) : isAdvisoryEditor ? (
              <div className="advisory-fields">
                <p>Advisory — a temporary notice shown to all agents working on this board. Moving it to Done deletes the notice; Draft hides it from agent discovery.</p>
                <label>Column
                  <select value={editor.draft.column} onChange={(event) =>
                    setEditor({ ...editor, draft: { ...editor.draft, column: event.currentTarget.value } })}>
                    {columns.map((column) => <option key={column.id} value={column.id}>{column.name}</option>)}
                  </select>
                </label>
                <label>Advisory content
                  <AutoTextarea minRows={6} placeholder="Temporary instructions, deadlines or pause notices…"
                    value={parseDescriptionTimeline(editor.draft.description).initial}
                    onChange={(event) => setEditor((current) => current ? {
                      ...current,
                      draft: { ...current.draft,
                        description: updateInitialDescription(current.draft.description, event.currentTarget.value),
                      },
                    } : current)} />
                </label>
              </div>
            ) : (<>
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

            {editor.draft.kind === "recurring" && <div className="recurring-fields">
              <label>Repeat every (days, optional)
                <input type="number" min="1" max="3650" step="1"
                  placeholder="Immediately after completion"
                  value={editor.draft.repeat_every_days ?? ""}
                  onChange={(event) => setEditor({ ...editor, draft: {
                    ...editor.draft, repeat_every_days: event.currentTarget.value === ""
                      ? null : Number(event.currentTarget.value),
                  } })} />
              </label>
              <label>Next eligible date (optional)
                <input type="date"
                  value={editor.draft.next_due ?? ""}
                  onChange={(event) => setEditor({ ...editor, draft: {
                    ...editor.draft, next_due: event.currentTarget.value || null,
                  } })} />
              </label>
              <span className="recurring-history">Recorded runs: {editor.original?.run_count ?? 0}
                {editor.original?.last_run_at ? ` · Last: ${editor.original.last_run_at.slice(0, 10)}` : ""}
                <br />Completing records a run and returns this task to AI Ready.</span>
            </div>}

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

            {snapshot.config.releases_enabled && editor.draft.kind === "task" && (
              <label className="release-deployable">
                <input type="checkbox" checked={editor.draft.deployable !== false}
                  onChange={(event) => setEditor({ ...editor, draft: { ...editor.draft, deployable: event.currentTarget.checked } })} />
                Enroll automatically in the rolling release when this task is Done
              </label>
            )}
            {editor.draft.kind === "epic" && (
              <div className="epic-editor-note">EPIC — not executable or assignable. Its board column is automatically derived from its subtasks.</div>
            )}
            <div className="form-grid">
              {editor.draft.kind !== "epic" && <TaskLinkPicker
                key={`${editor.original?.id ?? "new"}-depends`}
                label="Depends on (hard prerequisites · all projects)"
                tasks={[...(snapshot?.tasks ?? []), ...remoteRelatedTasks]}
                excludeId={editor.original?.id}
                allowEpics={false}
                values={editor.draft.depends_on}
                onChange={(depends_on) => setEditor((current) => current
                  ? { ...current, draft: { ...current.draft, depends_on } } : current)}
              />}
              <TaskLinkPicker
                key={`${editor.original?.id ?? "new"}-related`}
                label="Related to (informational · all projects)"
                tasks={[...(snapshot?.tasks ?? []), ...remoteRelatedTasks]}
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
                  {(["question", "blocked", "priority", "prepare"] as TaskAttention[]).map((flag) => (
                    (flag !== "prepare" || editor.mode === "create" || editor.draft.attention.includes("prepare")) && (
                    <button
                      type="button"
                      key={flag}
                      className={`attention-toggle ${flag} ${
                        editor.draft.attention.includes(flag) ? "active" : ""
                      }`}
                      disabled={flag === "prepare" && editor.mode === "create" && (!!prepRequestId || !reservedTaskId)}
                      title={flag === "prepare" ? "Immediately open a research-only T3 agent. The saved task stays in Draft until clarified." : undefined}
                      onClick={() => flag === "prepare" && editor.mode === "create"
                        ? enableTaskPreparation() : toggleAttentionDraft(flag)}
                    >
                      {flag === "question" ? "? Question" : flag === "blocked" ? "× Blocked"
                        : flag === "priority" ? "! Priority" : "✦ Prepare"}
                    </button>)
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
              readOnlyDescription={editor.mode === "edit" && !!editor.original?.agent_interacted}
              legacyAiNotes={editor.draft.ai_notes}
              descriptionMinRows={editor.mode === "create" ? 7 : 3}
              descriptionMaxHeight={editor.mode === "create" ? 480 : 360}
              onChange={(description) =>
                setEditor((current) => current
                  ? { ...current, draft: { ...current.draft, description } }
                  : current)}
              {...(editor.mode === "edit" && editor.draft.kind !== "epic" && {
                noteToAi: editor.draft.user_notes,
                onNoteToAiChange: (note: string) => setEditor((current) => {
                  if (!current) return current;
                  const releaseToAI = !userQueueReview && note.trim().length > 0;
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
            {editor.mode === "create" && editor.draft.kind !== "epic" && <label>
              Note to AI
              <AutoTextarea minRows={2} value={editor.draft.user_notes}
                placeholder="Instructions and local file paths…"
                onChange={event => {
                  const value = event.currentTarget.value;
                  setEditor(current => current
                    ? { ...current, draft: { ...current.draft, user_notes: value } }
                    : current);
                }} />
            </label>}
            <button type="button" className="button secondary editor-local-file-picker" disabled={busy}
              onClick={() => void browseEditorFiles()} title="Attach image previews; add other local paths to Note to AI without uploading">
              + Images / local files…</button>
            <TaskImageGallery
              description={editor.draft.description}
              boardPath={boardPath}
              pending={pendingClipboardImages}
              onRemovePending={(index) =>
                setPendingClipboardImages((current) => current.filter((_, position) => position !== index))}
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
            {editor.draft.kind === "epic" && <MarkdownPreviewToggle text={editor.draft.user_notes} />}

            {editor.draft.kind === "epic" && <label>
              Epic planning and coordination notes
              <AutoTextarea
                minRows={4}
                placeholder="Design decisions, scope, references and coordination notes. No implementation checklist or AI work log."
                value={editor.draft.notes}
                onChange={(event) => setEditor({ ...editor, draft: { ...editor.draft, notes: event.currentTarget.value } })}
              />
            </label>}
            {editor.draft.kind === "epic" && <MarkdownPreviewToggle text={editor.draft.notes} />}

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
            {editor.draft.kind !== "epic" &&
              <MarkdownPreviewToggle text={editor.draft.acceptance_criteria.join("\n\n")} />}

            {editor.mode === "edit" && editor.original && editor.original.kind !== "epic" &&
              (editor.original.column === "blocked" || editor.original.attention.includes("blocked")) && (
              <div className="blocked-authorize">
                <div className="blocked-authorize-reason">
                  <strong>Blocked</strong>
                  <span>{editor.original.blocked_reason?.trim()
                    || "No blocked reason recorded (blocked before reasons were required)."}</span>
                </div>
                <button type="button" className="button blocked-authorize-button"
                  title="Clear the block and move this task to AI Ready"
                  onClick={reopenBlockedTask} disabled={busy}>
                  Reopen
                </button>
              </div>
            )}
            </>)}

            <div className="modal-actions">
              {!isAdvisoryEditor && !!snapshot && queueableColumn(editor.draft.column) && !editor.original?.claimed_by && !editor.original?.completed &&
                (editor.draft.kind === "task" || editor.draft.kind === "recurring") &&
                !editor.draft.attention.some(flag => ["blocked", "needs-user", "question"].includes(flag)) &&
                editor.draft.depends_on.every(id => isComplete(id)) &&
                <div className="execution-mode-picker" role="group" aria-label="Execution mode">
                {(["backlog", "queue", "start_now"] as const)
                  .filter(mode => !creatingInAutoQueueColumn || mode !== "backlog")
                  .map((mode) =>
                  <button type="button" key={mode} disabled={busy || executionLoading}
                    aria-pressed={editorQueueChoice === mode}
                    className={`execution-mode-option${editorQueueChoice === mode ? " active" : ""}`}
                    title={mode === "start_now" ? "Record a durable foreground start request; no duplicate dispatch on uncertain delivery" : mode === "queue" ? "Register this task for background scheduling when the scheduler is installed" : "Keep this task in the backlog"}
                    onClick={() => { setExecutionChoice(mode); setExecutionTouched(true); }}>
                    {mode === "start_now" ? "Start now" : mode === "queue" ? "Queue" : "Backlog"}
                  </button>)}
              </div>}
              {editor.mode === "edit" && editor.original?.kind === "epic" && <button type="button"
                className="button secondary" onClick={() => openCreate(defaultCreateColumn(), editor.original!.id, editor.draft.category)} disabled={busy}>
                + Task under epic
              </button>}
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
              {editor.mode === "edit" && editor.original && <div className="editor-finish-menu">
                <button type="button" className="button secondary editor-finish-trigger" aria-expanded={doneMenuOpen}
                  title="Archive or finish this task" onClick={() => setDoneMenuOpen((value) => !value)} disabled={busy}>
                  ☰ ▴
                </button>
                {doneMenuOpen && <div className="editor-finish-options">
                  <button type="button" className="editor-archive-option" onClick={() => { setDoneMenuOpen(false); archiveSelected(); }} disabled={busy}>Archive</button>
                  {editor.original.kind !== "epic" && editor.draft.kind !== "epic" && <>
                    <button type="button" className="editor-complete-option" onClick={() => { setDoneMenuOpen(false); void saveEditor(true); }} disabled={busy || !doneColumn || editor.original.column === doneColumn.id || !editor.draft.title.trim() ||
                      (editor.draft.attention.includes("question") && !editor.draft.question?.prompt.trim())}>Mark Done</button>
                    {!isAdvisoryEditor && <button type="button" className="editor-followup-option" onClick={() => { setDoneMenuOpen(false); void closeAndFollowup(); }} disabled={busy || !doneColumn || editor.original.column === doneColumn.id || !editor.draft.title.trim() ||
                      (editor.draft.attention.includes("question") && !editor.draft.question?.prompt.trim())}>Mark Done and Followup</button>}
                  </>}
                </div>}
              </div>}
              {showNotifyAgent && <label className="agent-notify-toggle" title="Queue this task update for the claimed agent; delivery does not interrupt active work">
                <input type="checkbox" checked={notifyAgent}
                  onChange={(event) => setNotifyAgentOverride(event.currentTarget.checked)} />
                <span className="agent-notify-switch" aria-hidden="true" />
                Notify agent
              </label>}
              <button className="button editor-cancel-button" onClick={() => closeEditor()} disabled={busy}>Cancel</button>
              <button
                className={`button primary ${queueEdited ? "queue-save-release" : ""}`}
                title={queueEdited ? "Save your response and remove For user from this task" : "Save (Enter or Ctrl+S). Textareas keep Enter for new lines."}
                onClick={() => saveEditor()}
                disabled={
                  busy || executionLoading ||
                  !editor.draft.title.trim() ||
                  (editor.mode === "create" && !reservedTaskId) ||
                  (editor.draft.attention.includes("question") &&
                    !editor.draft.question?.prompt.trim())
                }
              >
                {busy ? "Saving…" : queueEdited ? "Save & release" : "Save"}
              </button>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}

export default function App() {
  return <ProjectTabs Board={ProjectBoard} />;
}
