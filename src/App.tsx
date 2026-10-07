import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import "./App.css";
import {
  archiveTask,
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

type TaskActivityKind = "created" | "moved";

const AUTO_REFRESH_MS = 1000;
const TASK_ACTIVITY_MS = 2800;

const emptyDraft = (
  column: string,
  parent: string | null = null,
  category: string | null = null,
): TaskDraft => ({
  title: "",
  column,
  parent,
  category,
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
  const snapshotRef = useRef<BoardSnapshot | null>(null);
  const activityTimersRef = useRef(new Map<string, number>());
  const [taskActivity, setTaskActivity] = useState<Record<string, TaskActivityKind>>({});
  const [hoverNotes, setHoverNotes] = useState<{
    title: string;
    aiNotes: string;
    userNotes: string;
    x: number;
    y: number;
  } | null>(null);
  const [questionAnswerText, setQuestionAnswerText] = useState("");
  const [contextMenu, setContextMenu] = useState<TaskContextMenu | null>(null);
  const undoStackRef = useRef<UndoEntry[]>([]);
  const [undoCount, setUndoCount] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const applySnapshot = (next: BoardSnapshot) => {
    snapshotRef.current = next;
    setSnapshot(next);
  };

  const flashTaskActivity = (
    changes: Array<{ id: string; kind: TaskActivityKind }>,
  ) => {
    if (!changes.length) return;

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
    if (!path || pendingTaskWrites.current.size > 0) return;
    const versionAtStart = localMutationVersion.current;
    const previous = snapshotRef.current;

    try {
      const next = await loadBoard(path);
      if (versionAtStart !== localMutationVersion.current) return;

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
          if (!oldTask) {
            changes.push({ id: task.id, kind: "created" });
          } else if (
            oldTask.column !== task.column ||
            oldTask.parent !== task.parent
          ) {
            changes.push({ id: task.id, kind: "moved" });
          }
        }

        flashTaskActivity(changes);
      }

      applySnapshot(next);
      if (!silent) setError("");
    } catch (err) {
      if (!silent) setError(errorText(err));
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

  useEffect(() => {
    if (!boardPath) return;
    const timer = window.setInterval(
      () => void refresh(boardPath, true),
      AUTO_REFRESH_MS,
    );
    return () => window.clearInterval(timer);
  }, [boardPath]);

  useEffect(
    () => () => {
      for (const timer of activityTimersRef.current.values()) {
        window.clearTimeout(timer);
      }
      activityTimersRef.current.clear();
    },
    [],
  );

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

    if (needle) return candidates;

    return candidates.filter((task) => {
      let current = task.parent;
      const seen = new Set<string>();
      while (current && !seen.has(current)) {
        if (collapsedTaskIds.has(current)) return false;
        seen.add(current);
        current = taskMap.get(current)?.parent ?? null;
      }
      return true;
    });
  }, [activeTasks, query, taskMap, collapsedTaskIds]);

  const columns = useMemo(
    () => [...(snapshot?.config.columns ?? [])].sort((a, b) => a.order - b.order),
    [snapshot],
  );

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
    setCollapsedTaskIds((current) => {
      const before = [...current];
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      pushUndo({
        kind: "collapse",
        label: "task collapse",
        before,
        after: [...next],
      });
      return next;
    });
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

  const showHoverAiNotes = (
    event: React.MouseEvent<HTMLElement>,
    task: Task,
  ) => {
    if (!task.ai_notes.trim() && !task.user_notes.trim()) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const width = 320;
    const x =
      rect.right + 10 + width < window.innerWidth
        ? rect.right + 10
        : Math.max(8, rect.left - width - 10);
    const y = Math.min(
      Math.max(8, rect.top),
      Math.max(8, window.innerHeight - 220),
    );
    setHoverNotes({
      title: task.title,
      aiNotes: task.ai_notes,
      userNotes: task.user_notes,
      x,
      y,
    });
  };

  const openEdit = (task: Task) => {
    setQuestionAnswerText("");
    setHoverNotes(null);
    setEditor({
      mode: "edit",
      original: task,
      draft: taskToDraft(task),
    });
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
    setEditor({ mode: "create", draft: emptyDraft(target, parent, category) });
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

  const saveEditor = () =>
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
      let draft = editor.draft;
      let after = before;

      if (editor.mode === "create") {
        if (draft.user_notes.trim() && !draft.attention.includes("user-note")) {
          draft = {
            ...draft,
            attention: [...draft.attention, "user-note"],
          };
        }
        const saved = await createTask(draft, boardPath);
        after = snapshotWithTask(before, saved);
        pushUndo({ kind: "board", label: "create task", before, after });
      } else if (editor.original) {
        const userNoteChanged =
          draft.user_notes !== editor.original.user_notes;
        if (userNoteChanged) {
          const attention: TaskAttention[] = draft.attention.filter(
            (flag) => flag !== "user-note",
          );
          if (draft.user_notes.trim()) attention.push("user-note");
          draft = { ...draft, attention };
        }
        const saved = await updateTask({ ...editor.original, ...draft }, boardPath);
        after = snapshotWithTask(before, saved);
        pushUndo({ kind: "board", label: "edit task", before, after });
      }

      applySnapshot(after);
      setEditor(null);
    });

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
        attention: editor.draft.attention.filter((item) => item !== "question"),
        question: {
          ...editor.draft.question,
          answer: answer.trim(),
        },
      };
      const saved = await updateTask(nextTask, boardPath);
      const after = snapshotWithTask(before, saved);
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
      const saved = await updateTask(optimistic, boardPath);
      const after = snapshotWithTask(before, saved);
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

  const moveTaskTo = async (id: string, column: string) => {
    const task = taskMap.get(id);
    if (!task || task.column === column) return;
    await persistTaskMutation(id, "move task", (current) => ({
      ...current,
      column,
    }));
  };

  const makeSubtaskOf = async (id: string, parentId: string) => {
    const task = taskMap.get(id);
    const parent = taskMap.get(parentId);
    if (!task || !parent || id === parentId) return;

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
      column: parent.column,
    }));
  };

  const beginCardDrag = (
    event: React.MouseEvent<HTMLElement>,
    task: Task,
  ) => {
    if (event.button !== 0) return;
    event.preventDefault();

    const startX = event.clientX;
    const startY = event.clientY;
    const rect = event.currentTarget.getBoundingClientRect();
    const grabOffsetX = startX - rect.left;
    const grabOffsetY = startY - rect.top;
    let dragging = false;

    const cleanup = () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
      setDragId(null);
      setCardDragPreview(null);
      setCardDropTargetId(null);
      document.body.classList.remove("card-dragging");
    };

    const validParentTargetAt = (x: number, y: number) => {
      const candidate = document
        .elementFromPoint(x, y)
        ?.closest<HTMLElement>("[data-task-id]")
        ?.dataset.taskId;
      if (!candidate || candidate === task.id) return null;

      let current: string | null = candidate;
      const seen = new Set<string>();
      while (current && !seen.has(current)) {
        if (current === task.id) return null;
        seen.add(current);
        current = taskMap.get(current)?.parent ?? null;
      }
      return candidate;
    };

    const handleMove = (moveEvent: MouseEvent) => {
      if (
        !dragging &&
        Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) >= 6
      ) {
        dragging = true;
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
          validParentTargetAt(moveEvent.clientX, moveEvent.clientY),
        );
      }
    };

    const handleUp = (upEvent: MouseEvent) => {
      const parentTarget = dragging
        ? validParentTargetAt(upEvent.clientX, upEvent.clientY)
        : null;
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

      if (parentTarget) {
        void makeSubtaskOf(task.id, parentTarget);
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

    const cleanup = () => {
      window.removeEventListener("mousemove", handleMove);
      window.removeEventListener("mouseup", handleUp);
      setDragColumnId(null);
      setColumnDragPreview(null);
      document.body.classList.remove("column-dragging");
    };

    const handleMove = (moveEvent: MouseEvent) => {
      if (
        !dragging &&
        Math.hypot(moveEvent.clientX - startX, moveEvent.clientY - startY) >= 6
      ) {
        dragging = true;
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
          column.id === id ? { ...column, hidden: !column.hidden } : column,
        ),
      },
      "toggle column visibility",
    );
  };

  const startBoardPan = (event: React.PointerEvent<HTMLElement>) => {
    if (event.button !== 2 || !boardRef.current) return;
    const target = event.target as HTMLElement;
    if (
      target.closest(
        ".task-card, .column-header, button, input, textarea, select, .task-context-menu",
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
      if (editor?.original?.id === task.id) setEditor(null);
    });

  const archiveSelected = () => {
    if (editor?.original) void archiveTaskById(editor.original);
  };

  const contextTask = contextMenu
    ? taskMap.get(contextMenu.taskId)
    : undefined;

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const key = event.key.toLowerCase();

      if (event.key === "Escape") {
        if (contextMenu) {
          event.preventDefault();
          setContextMenu(null);
          return;
        }
        if (editor && !busy) {
          event.preventDefault();
          setEditor(null);
          return;
        }
      }

      if (!event.ctrlKey || event.altKey) return;

      if (key === "t") {
        event.preventDefault();
        if (!editor) openCreate();
        return;
      }

      if (key === "s") {
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
  }, [editor, busy, contextMenu, snapshot, boardPath, undoCount]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => setContextMenu(null);
    window.addEventListener("blur", close);
    return () => window.removeEventListener("blur", close);
  }, [contextMenu]);

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
        if (contextMenu) setContextMenu(null);
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
            className="button secondary"
            onClick={() => void undoLast()}
            disabled={busy || undoCount === 0 || !!editor}
            title="Undo last TaskBoard action (Ctrl+Z)"
          >
            Undo{undoCount ? ` (${undoCount})` : ""}
          </button>
          <button className="button secondary" onClick={() => void refresh()} disabled={busy}>
            Refresh
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
          const columnTasks = visibleTasks
            .filter((task) => task.column === column.id)
            .sort(
              (a, b) =>
                attentionRank(a) - attentionRank(b) ||
                hierarchyKey(a).localeCompare(hierarchyKey(b)),
            );
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

          if (column.hidden) {
            return (
              <button
                className={`hidden-column${columnActivityClass}`}
                key={column.id}
                data-column-id={column.id}
                style={columnStyle}
                title={`Show ${column.name}`}
                onClick={() => toggleColumnHidden(column.id)}
              >
                |
              </button>
            );
          }

          return (
            <div
              className={`column ${column.terminal ? "terminal" : ""} ${
                dragColumnId === column.id ? "column-drag-source" : ""
              }${columnActivityClass}`}
              key={column.id}
              data-column-id={column.id}
              style={columnStyle}
            >
              <div
                className="column-header"
                onMouseDown={(event) => beginColumnDrag(event, column)}
                title="Drag to reorder"
              >
                <div className="column-title">
                  <span>{column.name}</span>
                  <small>{columnTasks.length}</small>
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
                {columnTasks.map((task) => {
                  const depth = depthOf(task);
                  return (
                    <article
                      className={`task-card ${dragId === task.id ? "drag-source" : ""} ${
                        cardDropTargetId === task.id ? "drop-parent-target" : ""
                      } ${
                        taskActivity[task.id]
                          ? `activity-${taskActivity[task.id]}`
                          : ""
                      }`}
                      key={task.id}
                      data-task-id={task.id}
                      style={{ marginLeft: Math.min(depth, 4) * 12 }}
                      onMouseDown={(event) => beginCardDrag(event, task)}
                      onContextMenu={(event) => {
                        event.preventDefault();
                        event.stopPropagation();
                        setHoverNotes(null);
                        setContextMenu({
                          taskId: task.id,
                          x: event.clientX,
                          y: event.clientY,
                        });
                      }}
                      onMouseEnter={(event) => showHoverAiNotes(event, task)}
                      onMouseLeave={() => setHoverNotes(null)}
                      title={
                        task.ai_notes.trim() || task.user_notes.trim()
                          ? undefined
                          : task.title
                      }
                    >
                      <div className="task-title-row">
                        {childCounts.has(task.id) && (
                          <button
                            className="task-collapse"
                            draggable={false}
                            title={
                              collapsedTaskIds.has(task.id)
                                ? "Expand subtasks"
                                : "Collapse subtasks"
                            }
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
                        <h3>{task.title}</h3>
                        {!!task.attention.length && (
                          <div className="task-attention-icons">
                            {task.attention.includes("question") && (
                              <span className="attention-icon question" title="Question">?</span>
                            )}
                            {task.attention.includes("needs-user") && (
                              <span className="attention-icon needs-user" title="Needs user">◆</span>
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
            <h3>{cardDragPreview.title}</h3>
          </div>
        </div>
      )}

      {hoverNotes && !cardDragPreview && (
        <div
          className="ai-notes-hover"
          style={{ left: hoverNotes.x, top: hoverNotes.y }}
        >
          <span>{hoverNotes.title}</span>
          {hoverNotes.userNotes.trim() && (
            <section className="hover-note-section user">
              <strong>User Notes</strong>
              <p>{hoverNotes.userNotes}</p>
            </section>
          )}
          {hoverNotes.aiNotes.trim() && (
            <section className="hover-note-section ai">
              <strong>AI Notes</strong>
              <p>{hoverNotes.aiNotes}</p>
            </section>
          )}
        </div>
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
          <button
            onClick={() => {
              setContextMenu(null);
              openCreate(
                contextTask.column,
                contextTask.id,
                contextTask.category,
              );
            }}
          >
            New subtask
          </button>
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

      {editor && (
        <div className="modal-backdrop" onMouseDown={() => !busy && setEditor(null)}>
          <section className="task-modal" onMouseDown={(event) => event.stopPropagation()}>
            <div className="modal-header">
              <div>
                <div className="eyebrow">
                  {editor.mode === "edit" ? editor.original?.id : "NEW TASK"}
                </div>
                <h2>{editor.mode === "edit" ? "Edit task" : "Create task"}</h2>
              </div>
              <button className="icon-button" onClick={() => setEditor(null)}>×</button>
            </div>

            {editor.mode === "edit" &&
              editor.original &&
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

            {editor.draft.question?.answer && (
              <section className="answered-question">
                <span>Answered question</span>
                <strong>{editor.draft.question.prompt}</strong>
                <p>{editor.draft.question.answer}</p>
              </section>
            )}

            <label>
              Title
              <input
                autoFocus
                value={editor.draft.title}
                onChange={(event) =>
                  setEditor({ ...editor, draft: { ...editor.draft, title: event.currentTarget.value } })
                }
              />
            </label>

            <div className="form-grid">
              <label>
                Column
                <select
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
              <label>
                Parent
                <select
                  value={editor.draft.parent ?? ""}
                  onChange={(event) =>
                    setEditor({
                      ...editor,
                      draft: { ...editor.draft, parent: event.currentTarget.value || null },
                    })
                  }
                >
                  <option value="">No parent</option>
                  {activeTasks
                    .filter((task) => task.id !== editor.original?.id)
                    .map((task) => (
                      <option value={task.id} key={task.id}>{task.id} — {task.title}</option>
                    ))}
                </select>
              </label>
            </div>

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

            <div className="attention-editor">
              <span>Attention</span>
              <div className="attention-editor-buttons">
                {(["question", "needs-user", "blocked"] as TaskAttention[]).map(
                  (flag) => (
                    <button
                      type="button"
                      key={flag}
                      className={`attention-toggle ${flag} ${
                        editor.draft.attention.includes(flag) ? "active" : ""
                      }`}
                      onClick={() => toggleAttentionDraft(flag)}
                    >
                      {flag === "question"
                        ? "? Question"
                        : flag === "needs-user"
                          ? "◆ NeedsUser"
                          : "× Blocked"}
                    </button>
                  ),
                )}
              </div>
            </div>

            {editor.draft.attention.includes("question") &&
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
                    <textarea
                      rows={3}
                      placeholder={"Optional — one option per line"}
                      value={editor.draft.question.options.join("\n")}
                      onChange={(event) =>
                        setEditor({
                          ...editor,
                          draft: {
                            ...editor.draft,
                            question: {
                              ...editor.draft.question!,
                              options: event.currentTarget.value
                                .split("\n")
                                .map((item) => item.trim())
                                .filter(Boolean),
                            },
                          },
                        })
                      }
                    />
                  </label>
                </section>
              )}

            <label className="user-note-editor">
              Note to AI
              <textarea
                rows={4}
                placeholder="Tell the AI what to do with this task, e.g. make it clearer, propose a design, or incorporate your feedback."
                value={editor.draft.user_notes}
                onChange={(event) =>
                  setEditor({
                    ...editor,
                    draft: {
                      ...editor.draft,
                      user_notes: event.currentTarget.value,
                    },
                  })
                }
              />
              <span className="field-hint">
                Changing this note marks the task for AI attention until an agent handles it.
              </span>
            </label>

            <label>
              AI Notes
              <textarea
                rows={4}
                placeholder="Agent-facing context, current state, blocker details, what the user needs to know…"
                value={editor.draft.ai_notes}
                onChange={(event) =>
                  setEditor({
                    ...editor,
                    draft: {
                      ...editor.draft,
                      ai_notes: event.currentTarget.value,
                    },
                  })
                }
              />
            </label>

            <label>
              Description
              <textarea
                rows={5}
                value={editor.draft.description}
                onChange={(event) =>
                  setEditor({
                    ...editor,
                    draft: { ...editor.draft, description: event.currentTarget.value },
                  })
                }
              />
            </label>

            <label>
              Acceptance criteria
              <textarea
                rows={5}
                placeholder="One criterion per line"
                value={editor.draft.acceptance_criteria.join("\n")}
                onChange={(event) =>
                  setEditor({
                    ...editor,
                    draft: {
                      ...editor.draft,
                      acceptance_criteria: event.currentTarget.value
                        .split("\n")
                        .map((item) => item.trim())
                        .filter(Boolean),
                    },
                  })
                }
              />
            </label>

            <div className="modal-actions">
              {editor.mode === "edit" && editor.original && (
                <>
                  <button
                    className="button secondary"
                    onClick={() =>
                      openCreate(
                        editor.draft.column,
                        editor.original!.id,
                        editor.draft.category,
                      )
                    }
                    disabled={busy}
                  >
                    + Subtask
                  </button>
                  <button className="button danger" onClick={archiveSelected} disabled={busy}>
                    Archive
                  </button>
                </>
              )}
              <span className="spacer" />
              <button className="button secondary" onClick={() => setEditor(null)} disabled={busy}>
                Cancel
              </button>
              <button
                className="button primary"
                onClick={saveEditor}
                disabled={
                  busy ||
                  !editor.draft.title.trim() ||
                  (editor.draft.attention.includes("question") &&
                    !editor.draft.question?.prompt.trim())
                }
              >
                {busy ? "Saving…" : "Save"}
              </button>
            </div>
          </section>
        </div>
      )}
    </main>
  );
}

export default App;
