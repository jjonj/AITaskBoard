import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type DragEvent, type MouseEvent } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { archiveTask, listProjects, loadBoard, openTaskSession, readLocalImage, saveTaskImage, taskSyncQueue, updateTask, type ProjectTab } from "./api";
import { reviewAsset, reviewReconsider, reviewResolve, reviewShow,
  type ReviewArtifact, type ReviewManifest, type ReviewPin } from "./reviewApi";
import type { Task } from "./types";
import { isUserQueueEligible } from "./userQueueEligibility";
import { VisualReviewInspector } from "./VisualReviewInspector";
import { InboxResponseFooter, reviewOutcomes } from "./InboxResponseFooter";

type QueueItem = { path: string; project: string; task: Task };
type Props = {
  project: ProjectTab;
  onClose: () => void;
  initialTaskId?: string;
  userTasks?: Task[];
  onOpenUserTask?: (task: Task) => void;
  onUserTaskUpdated?: (notice?: string) => void;
  onSnoozeUserTask?: (task: Task, path: string, until: number) => void;
};
const keyOf = (item: QueueItem) => `${item.path}|${item.task.id}`;
const REVIEW_SNOOZES_KEY = "taskboard.inboxSnoozes";
const loadReviewSnoozes = (): Record<string, number> => {
  try {
    const raw = JSON.parse(window.localStorage.getItem(REVIEW_SNOOZES_KEY) || "{}");
    return raw && typeof raw === "object" && !Array.isArray(raw) ? raw : {};
  } catch { return {}; }
};

export function ReviewView({ project, onClose, initialTaskId, userTasks = [], onOpenUserTask, onUserTaskUpdated, onSnoozeUserTask }: Props) {
  const [allBoards, setAllBoards] = useState(false);
  const [showResolved, setShowResolved] = useState(false);
  const [items, setItems] = useState<QueueItem[]>([]);
  const [selectedKey, setSelectedKey] = useState("");
  const initialSelectionApplied = useRef(false);
  // Skips last for this Inbox pass; manual selection can still revisit the item.
  const skippedThisPass = useRef<Set<string>>(new Set());
  const [manifest, setManifest] = useState<ReviewManifest | null>(null);
  const [assets, setAssets] = useState<Record<string, string>>({});
  const [htmlDocuments, setHtmlDocuments] = useState<Record<string, string>>({});
  const [queueThumbnails, setQueueThumbnails] = useState<Record<string, string>>({});
  const [queueModes, setQueueModes] = useState<Record<string, string>>({});
  const [queueCounts, setQueueCounts] = useState<Record<string, number>>({});
  const [iframeWidth, setIframeWidth] = useState(0);
  const [showPoster, setShowPoster] = useState(false);
  const scrollOffset = useRef({ x: 0, y: 0 });
  const embedFrameRef = useRef<HTMLIFrameElement>(null);
  const embedTimeRef = useRef<number | null>(null);
  const [embedPlayback, setEmbedPlayback] = useState<{ duration: number; playing: boolean; t: number } | null>(null);
  const [choice, setChoice] = useState("");
  const [ranking, setRanking] = useState<string[]>([]);
  const [pins, setPins] = useState<ReviewPin[]>([]);
  const [note, setNote] = useState("");
  const [pinMode, setPinMode] = useState(false);
  const [gameZoom, setGameZoom] = useState(false);
  const [onion, setOnion] = useState(false);
  const [slider, setSlider] = useState(50);
  const [mediaIndex, setMediaIndex] = useState(0);
  const [saving, setSaving] = useState(false);
  const [openingTaskSession, setOpeningTaskSession] = useState(false);
  const [error, setError] = useState("");
  const [userNote, setUserNote] = useState("");
  const [userCriteria, setUserCriteria] = useState("");
  const [keepForUser, setKeepForUser] = useState(false);
  const [userAnswer, setUserAnswer] = useState("");
  const [snoozedLocally, setSnoozedLocally] = useState<Record<string, number>>(loadReviewSnoozes);
  const [responseOutcome, setResponseOutcome] = useState("");
  const [moreOpen, setMoreOpen] = useState(false);
  const [inboxImages, setInboxImages] = useState<Array<{dataUrl:string;mime:string;name:string}>>([]);
  useEffect(() => { setInboxImages([]); }, [selectedKey]);
  const videoRef = useRef<HTMLVideoElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const reviewTask = items.find((item) => keyOf(item) === selectedKey);
  const openReviewTaskSession = async (item: QueueItem) => {
    if (openingTaskSession) return;
    setOpeningTaskSession(true);
    try {
      await openTaskSession(item.task.id, item.path);
    } catch (cause) {
      setError("Could not open the linked T3 session: " + String(cause));
    } finally {
      setOpeningTaskSession(false);
    }
  };
  const isVisualReview = reviewTask?.task.review_status === "open" ||
    ((showResolved || reviewTask?.task.id === initialTaskId) &&
      (reviewTask?.task.review_status === "resolved" ||
       reviewTask?.task.review_status === "awaiting_revision"));
  const round = manifest?.rounds[manifest.rounds.length - 1];
  const activeInboxKey = useRef(selectedKey);
  activeInboxKey.current = selectedKey;
  const addInboxPaths = async (raw: string[]) => {
    const expected = selectedKey;
    const paths = [...new Set(raw.map(path => path.trim()).filter(Boolean))];
    const images = paths.filter(path => /\.(png|jpe?g|webp|gif)$/i.test(path));
    const references = paths.filter(path => !images.includes(path));
    if (references.length) {
      const add = (text: string) => [text.trim(), ...references].filter(Boolean).join("\n");
      if (isVisualReview) setNote(add);
      else setUserNote(add);
    }
    for (const path of images) {
      try {
        const preview = await readLocalImage(path);
        if (activeInboxKey.current === expected) setInboxImages(current => [...current, preview]);
      } catch (cause) {
        if (activeInboxKey.current === expected) setError(`Cannot preview ${path}: ${String(cause)}`);
      }
    }
  };
  const browseInboxFiles = async () => {
    try {
      const chosen = await open({ multiple: true, directory: false });
      if (chosen) await addInboxPaths(Array.isArray(chosen) ? chosen : [chosen]);
    } catch (cause) { setError(`File picker failed: ${String(cause)}`); }
  };
  const addBrowserImages = (files: File[]) => {
    const key = selectedKey;
    for (const file of files.filter(f => /^image\/(png|jpeg|webp|gif)$/.test(f.type))) {
      if (file.size > 20*1024*1024) { setError("Image exceeds 20 MB."); continue; }
      const reader = new FileReader();
      reader.onload = () => {
        if (activeInboxKey.current === key && typeof reader.result === "string")
          setInboxImages(current => [...current, { dataUrl: reader.result as string,
            mime: file.type, name: file.name || "Pasted image" }]);
      };
      reader.readAsDataURL(file);
    }
  };
  const onInboxPaste = (event: ClipboardEvent<HTMLElement>) => {
    const files = [...event.clipboardData.items].filter(item => item.kind === "file" &&
      /^image\/(png|jpeg|webp|gif)$/.test(item.type))
      .map(item => item.getAsFile()).filter((item): item is File => !!item);
    if (!files.length) return;
    event.preventDefault();
    addBrowserImages(files);
  };
  const onInboxDrop = (event: DragEvent<HTMLElement>) => {
    if ("__TAURI_INTERNALS__" in window) return; // Native event provides authoritative absolute paths.
    const files = [...event.dataTransfer.files];
    if (!files.length) return;
    event.preventDefault();
    addBrowserImages(files);
    if (files.some(file => !/^image\//.test(file.type)))
      setError("Non-image files require native Explorer drag/drop or the file picker to preserve full paths.");
  };
  const attachInboxImages = async (id: string, boardPath: string) => {
    const paths: string[] = [];
    for (const image of inboxImages) {
      const encoded = image.dataUrl.split(",", 2)[1];
      if (!encoded) throw new Error("Image preview encoding missing");
      const saved = await saveTaskImage(id, encoded, image.mime, boardPath);
      paths.push(`![${image.name.replace(/[\[\]<>]/g, "")}](../${saved})`);
    }
    return paths;
  };
  useEffect(() => {
    if (!("__TAURI_INTERNALS__" in window)) return;
    let mounted = true;
    let unlisten: (() => void) | undefined;
    void getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (!mounted || payload.type !== "drop" || !selectedKey) return;
      void addInboxPaths(payload.paths);
    }).then(dispose => { if (mounted) unlisten = dispose; else dispose(); })
      .catch(() => { /* Non-native previews still support Ctrl+V images. */ });
    return () => { mounted = false; unlisten?.(); };
  }, [selectedKey, isVisualReview]);
  const inboxImagePreview = <div className="inbox-image-previews" aria-label="Pending image attachments">
    {inboxImages.map((image, index) => <figure key={index}>
      <img src={image.dataUrl} alt={image.name}/>
      <figcaption title={image.name}>{image.name}</figcaption>
      <button type="button" title="Remove attachment" onClick={() =>
        setInboxImages(current => current.filter((_, i) => i !== index))}>×</button>
    </figure>)}
  </div>;


  const queueKey = items.filter((item) => !!item.task.review_status)
    .map((item) => keyOf(item) + ":" + item.task.revision).join("|");

  useEffect(() => {
    let cancelled = false;
    void Promise.all(items.filter((item) => !!item.task.review_status).slice(0, 60).map(async (item) => {
      try {
        const manifest = await reviewShow(item.task.id, item.path);
        const latest = manifest.rounds[manifest.rounds.length - 1];
        const artifact = latest?.artifacts[0];
        const path = artifact?.poster || (artifact?.kind === "image" ? artifact.path : "");
        const full = path && !(/^[a-z]:[\\/]/i.test(path) || path.startsWith("/")) ? item.path + "/" + path : path;
        const thumbnail = full ? await reviewAsset(full, item.path).catch(() => "") : "";
        return [keyOf(item), thumbnail, latest?.mode || "review", latest?.artifacts.length ?? 0] as const;
      } catch { return [keyOf(item), "", "review", 0] as const; }
    })).then((values) => {
      if (!cancelled) {
        setQueueThumbnails(Object.fromEntries(values.map(([key, src]) => [key, src])));
        setQueueModes(Object.fromEntries(values.map(([key, , mode]) => [key, mode])));
        setQueueCounts(Object.fromEntries(values.map(([key, , , count]) => [key, count])));
      }
    });
    return () => { cancelled = true; };
  }, [queueKey]);

  const refresh = useCallback(async () => {
    try {
      const boards = allBoards ? (await listProjects()).projects : [project];
      const snapshots = await Promise.all(boards.map(async (board) => ({
        project: board,
        snapshot: await loadBoard(board.path),
      })));
      const pending = snapshots.flatMap(({ project: board, snapshot }) =>
        snapshot.tasks.filter((task) => !task.archived &&
          (task.review_status === "open" ||
            (showResolved && (task.review_status === "resolved" || task.review_status === "awaiting_revision")) ||
            (task.id === initialTaskId && board.path === project.path) ||
            (isUserQueueEligible(task, snapshot, {}) && (allBoards || board.id !== project.id ||
              userTasks.some((candidate) => candidate.id === task.id)))))
          .map((task) => ({ project: board.name, path: board.path, task })))
        .filter((item) => !(Number(snoozedLocally[keyOf(item)]) > Date.now()));
      pending.sort((a, b) => b.task.created.localeCompare(a.task.created) || a.task.id.localeCompare(b.task.id));
      setItems(pending);
      if (initialTaskId && !initialSelectionApplied.current) {
        initialSelectionApplied.current = true;
        const target = pending.find((item) => item.task.id === initialTaskId && item.path === project.path);
        setSelectedKey(target ? keyOf(target) : "");
        if (!target) setError(`The requested ${initialTaskId} is no longer pending in this board.`);
      } else {
        setSelectedKey((current) => {
          if (pending.some((item) => keyOf(item) === current)) return current;
          const first = pending.find((item) => !skippedThisPass.current.has(keyOf(item)));
          return first ? keyOf(first) : "";
        });
      }
      // Do not erase a submission error on every three-second queue refresh.
    } catch (cause) {
      setError(`Unable to load review queue: ${String(cause)}`);
    }
  }, [project, allBoards, showResolved, userTasks, snoozedLocally, initialTaskId]);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(timer);
  }, [refresh]);

  useEffect(() => {
    setError("");
    setManifest(null);
    setAssets({});
    setHtmlDocuments({});
    setShowPoster(false);
    setChoice("");
    setRanking([]);
    setPins([]);
    setNote("");
    setMediaIndex(0);
    setPinMode(false);
    setUserNote(reviewTask?.task.user_notes ?? "");
    setUserCriteria(reviewTask?.task.acceptance_criteria.join("\n") ?? "");
    setKeepForUser(reviewTask?.task.kind === "for_user" ||
      !!reviewTask?.task.attention.includes("needs-user"));
    setUserAnswer(reviewTask?.task.question?.answer ?? "");
    setResponseOutcome("");
    if (!reviewTask || !reviewTask.task.review_status) return;
    let cancelled = false;
    void reviewShow(reviewTask.task.id, reviewTask.path).then((value) => {
      if (cancelled) return;
      setManifest(value);
      const verdict = value.rounds[value.rounds.length - 1]?.result;
      setPins(verdict?.pins || []);
      setNote(verdict?.note || "");
      setChoice(verdict?.choices[0] || "");
      setRanking(verdict?.ranking || []);
    }).catch((cause) => { if (!cancelled) setError(String(cause)); });
    return () => { cancelled = true; };
  }, [selectedKey]);

  useEffect(() => {
    if (!round || !reviewTask) return;
    let cancelled = false;
    const list = [...round.artifacts, ...round.references, ...(round.previous ? [round.previous] : [])];
    const paths = [...new Set(list.flatMap((artifact) => [artifact.path, artifact.poster].filter(Boolean) as string[]))];
    void Promise.all(paths.map(async (path) => {
      if (/^https?:\/\//i.test(path)) return [path, path] as const;
      const source = /^[a-z]:[\\/]/i.test(path) || path.startsWith("/") ? path : reviewTask.path + "/" + path;
      try { return [path, await reviewAsset(source, reviewTask.path)] as const; }
      catch { return [path, ""] as const; }
    })).then((pairs) => { if (!cancelled) setAssets(Object.fromEntries(pairs)); });
    return () => { cancelled = true; };
  }, [manifest?.revision, selectedKey]);

  useEffect(() => {
    if (!round || !reviewTask) return;
    let cancelled = false;
    // Parse inert HTML, replace local relative assets with data URIs, then load
    // the resulting document ONLY inside a sandboxed opaque-origin iframe.
    const compile = async (artifact: ReviewArtifact): Promise<[string, string]> => {
      const absolute = artifact.path.split(String.fromCharCode(92)).join("/");
      const index = /\.html?$/i.test(absolute) ? absolute : (absolute.endsWith("/") ? absolute.slice(0, -1) : absolute) + "/index.html";
      const source = await reviewAsset(index, reviewTask.path);
      const raw = source.split(",")[1] ?? "";
      const html = new TextDecoder().decode(Uint8Array.from(atob(raw), (ch) => ch.charCodeAt(0)));
      const doc = new DOMParser().parseFromString(html, "text/html");
      const folder = index.substring(0, index.lastIndexOf("/"));
      const isRelative = (ref: string) => !!ref && !/^(?:[a-z][a-z0-9+.-]*:|\/|#)/i.test(ref);
      for (const element of [...doc.querySelectorAll<HTMLElement>("[src],[href]")]) {
        for (const attr of ["src", "href"]) {
          const ref = element.getAttribute(attr);
          if (!ref || !isRelative(ref)) continue;
          try {
            const src = await reviewAsset(folder + "/" + ref.split(/[?#]/)[0], reviewTask.path);
            if (element.tagName === "LINK" && attr === "href" &&
                element.getAttribute("rel")?.toLowerCase() === "stylesheet") {
              const css = new TextDecoder().decode(Uint8Array.from(atob(src.split(",")[1] || ""), (ch) => ch.charCodeAt(0)));
              const style = doc.createElement("style");
              style.textContent = css;
              element.replaceWith(style);
            } else element.setAttribute(attr, src);
          } catch {
            // Missing references stay visible as browser errors within the isolated frame.
          }
        }
      }
      const bridge = doc.createElement("script");
      bridge.textContent = 'addEventListener("scroll",function(){parent.postMessage({type:"taskboard-review-scroll",x:scrollX,y:scrollY},"*")},true);';
      doc.body.appendChild(bridge);
      return [artifact.path, "<!doctype html>\n" + doc.documentElement.outerHTML];
    };
    void Promise.all(round.artifacts.filter((artifact) => artifact.kind === "html")
      .map(async (artifact) => {
        try { return await compile(artifact); }
        catch { return [artifact.path, ""] as [string, string]; }
      })).then((pairs) => { if (!cancelled) setHtmlDocuments(Object.fromEntries(pairs)); });
    return () => { cancelled = true; };
  }, [manifest?.revision, selectedKey]);

  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      if (event.data?.type === "taskboard-review-scroll") {
        const x = Number(event.data.x), y = Number(event.data.y);
        if (Number.isFinite(x) && Number.isFinite(y))
          scrollOffset.current = { x: Math.max(0, x), y: Math.max(0, y) };
        return;
      }
      // Ignore arbitrary page messages. A live Animation Lab frame must be the
      // actual current iframe, not another tab or a stale artifact.
      if (event.source !== embedFrameRef.current?.contentWindow ||
          event.data?.source !== "animation-lab-embed" || event.data?.version !== 1 ||
          !["ready", "time", "state"].includes(event.data?.type)) return;
      const time = Number(event.data.t), duration = Number(event.data.duration);
      if (!Number.isFinite(time) || !Number.isFinite(duration) || duration < 0) return;
      embedTimeRef.current = Math.max(0, Math.min(duration, time));
      // Continuous 10Hz updates only need a timestamp ref. Render when paused
      // or loaded so the reviewing user gets a stable lightweight control row.
      if (event.data.type !== "time" || !event.data.playing)
        setEmbedPlayback({ duration, t: embedTimeRef.current, playing: !!event.data.playing });
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, []);

  const playable = round?.artifacts ?? [];
  const activeArtifact = playable[mediaIndex] ?? playable[0];
  useEffect(() => {
    embedTimeRef.current = null;
    setEmbedPlayback(null);
  }, [selectedKey, activeArtifact?.path]);
  const isAnimationLabEmbed = (artifact?: ReviewArtifact) => {
    if (!artifact || artifact.kind !== "url") return false;
    try {
      const url = new URL(artifact.path);
      return /^(localhost|127\\.0\\.0\\.1|\\[::1\\])$/.test(url.hostname) &&
        url.searchParams.get("embed") === "1";
    } catch { return false; }
  };
  const reviewEmbedUrl = (url: string) => {
    if (!isAnimationLabEmbed({ kind: "url", path: url } as ReviewArtifact)) return url;
    const updated = new URL(url);
    updated.searchParams.append("parentOrigin", window.location.origin);
    return updated.toString();
  };
  const sendEmbedCommand = (type: "time" | "play" | "pause", t?: number) => {
    if (!embedFrameRef.current?.contentWindow || !isAnimationLabEmbed(activeArtifact)) return;
    embedFrameRef.current.contentWindow.postMessage({
      source: "taskboard-review", type, ...(t == null ? {} : { t }),
    }, new URL(activeArtifact!.path).origin);
  };
  useEffect(() => {
    if (activeArtifact?.kind !== "url" || !activeArtifact.poster) return;
    let cancelled = false;
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), 5000);
    void fetch(activeArtifact.path, { mode: "no-cors", signal: controller.signal })
      .catch(() => { if (!cancelled) setShowPoster(true); })
      .finally(() => window.clearTimeout(timer));
    return () => { cancelled = true; controller.abort(); window.clearTimeout(timer); };
  }, [activeArtifact?.path]);
  const activeSrc = (artifact?: ReviewArtifact) => artifact &&
    (assets[artifact.path] || (artifact.poster && assets[artifact.poster]) || "");
  const advance = (excludingKey?: string, skipCurrent = false) => {
    if (skipCurrent && selectedKey) skippedThisPass.current.add(selectedKey);
    const index = items.findIndex((item) => keyOf(item) === selectedKey);
    const ordered = index < 0 ? items : [...items.slice(index + 1), ...items.slice(0, index)];
    const next = ordered.find((item) => keyOf(item) !== excludingKey &&
      !skippedThisPass.current.has(keyOf(item)) &&
      !(Number(snoozedLocally[keyOf(item)]) > Date.now()));
    if (next) setSelectedKey(keyOf(next));
    else onClose();
  };

  const hideLocally = (item: QueueItem, milliseconds: number, persist = false) => {
    const until = Date.now() + milliseconds;
    const key = keyOf(item);
    setSnoozedLocally((current) => {
      const updated = { ...current, [key]: until };
      if (persist) {
        try { window.localStorage.setItem(REVIEW_SNOOZES_KEY, JSON.stringify(updated)); }
        catch { /* An in-memory snooze still works if browser storage is unavailable. */ }
      }
      return updated;
    });
  };

  const submitUser = async (markDone = false, reopen = false) => {
    if (!reviewTask || isVisualReview || saving) return;
    setSaving(true);
    setError("");
    try {
      const old = reviewTask.task;
      const aiQuestion = old.kind === "task" && old.attention.includes("question") &&
        !!old.question && !old.question.answer;
      // The green Done action is an explicit acceptance/dismissal, even if
      // another option was selected before it was clicked.
      const answer = reopen ? "Needs changes" : aiQuestion && markDone ? "Done"
        : userAnswer.trim() || (aiQuestion ? userNote.trim() : "");
      if (aiQuestion && !answer) throw new Error("Choose a response or enter feedback before submitting.");
      const needsChanges = reopen || (aiQuestion && !markDone && answer.toLowerCase() === "needs changes");
      // An Inbox Done response closes an AI request; it does not turn the
      // implementation task into Done. For user tasks remain directly completable.
      const attention = (aiQuestion ? old.attention.filter((flag) => flag !== "question" && flag !== "needs-user")
        : markDone ? old.attention.filter((flag) => flag !== "needs-user")
        : keepForUser ? [...new Set([...old.attention, "needs-user" as const])]
        : old.attention.filter((flag) => flag !== "needs-user"))
        .filter((flag) => flag !== "question" || !answer);
      if (aiQuestion && !attention.includes("user-note")) attention.push("user-note");
      let doneColumn: string | undefined;
      if (markDone) {
        const board = await loadBoard(reviewTask.path);
        doneColumn = board.config.columns.find((col) => col.id === "done" && col.terminal)?.id
          ?? board.config.columns.find((col) => col.terminal)?.id;
        if (!doneColumn) throw new Error("This board has no Done column.");
      }
      const imageLinks = await attachInboxImages(old.id, reviewTask.path);
      const updated: Task = {
        ...old,
        ...(needsChanges ? { column: "ai-ready", claimed_by: null, blocked_reason: null } :
          doneColumn ? { column: doneColumn, claimed_by: null } : {}),
        user_notes: [userNote, ...imageLinks].filter(Boolean).join("\n\n"),
        acceptance_criteria: userCriteria.split(/\r?\n/).filter(Boolean),
        kind: needsChanges ? "task" : aiQuestion ? old.kind : markDone ? "task" : keepForUser ? (old.kind === "epic" ? "epic" : "for_user") : "task",
        attention: needsChanges ? attention.filter(flag => flag !== "blocked" && flag !== "needs-user") : attention,
        question: old.question ? { ...old.question, answer: answer || null } : null,
      };
      const saved = await updateTask(updated, reviewTask.path);
      let notice: string | undefined;
      if (saved.completed && saved.category === "Implementation Review" && saved.related_to.length === 1) {
        const board = await loadBoard(reviewTask.path);
        const parent = board.tasks.find((task) => task.id === saved.related_to[0]);
        const entry = parent?.notes.split("\n").find((line) => line.includes(`[review:${saved.id}]`));
        if (parent && entry) notice = `Review ${saved.id} — ${parent.id}: ` +
          (entry.includes("approved_parent_done") ? "approved, original moved to Done"
            : entry.includes("needs_changes") ? "needs changes, original returned to work"
            : entry.includes("partial_or_blocked") ? "partial approval or other work remains"
            : "review closed; original disposition unresolved");
      }
      onUserTaskUpdated?.(notice);
      setInboxImages([]);
      // A response may deliberately leave "For user" enabled. Do not ask for
      // the same answer again in this Inbox pass.
      hideLocally(reviewTask, 60 * 60 * 1000);
      advance(keyOf(reviewTask));
      void refresh();
    } catch (cause) {
      setError(`Could not save this task: ${String(cause)}`);
    } finally {
      setSaving(false);
    }
  };
  const moveSelectedToDraft = async () => {
    if (!reviewTask || saving || reviewTask.task.kind === "epic") return;
    setSaving(true);
    setMoreOpen(false);
    setError("");
    try {
      const board = await loadBoard(reviewTask.path);
      const current = board.tasks.find(t => t.id === reviewTask.task.id);
      if (!current) throw new Error("Task no longer exists");
      await updateTask({
        ...current, column: "draft", claimed_by: null,
        blocked_reason: null, review_status: null,
        attention: current.attention.filter(flag =>
          !["question", "needs-user", "blocked"].includes(flag)),
        question: current.question ? { ...current.question,
          answer: current.question.answer || "Moved to Draft" } : null,
      }, reviewTask.path);
      await taskSyncQueue(current.id, reviewTask.path).catch(() => null);
      advance(keyOf(reviewTask));
      await refresh();
    } catch (cause) {
      setError(`Move to Draft failed: ${String(cause)}`);
    } finally { setSaving(false); }
  };
  const archiveSelected = async () => {
    if (!reviewTask || saving) return;
    if (!window.confirm(`Archive ${reviewTask.task.id}? This retains its history but clears active ownership and Queue intent.`)) return;
    setSaving(true);
    setMoreOpen(false);
    setError("");
    try {
      await archiveTask(reviewTask.task.id, reviewTask.path);
      await taskSyncQueue(reviewTask.task.id, reviewTask.path).catch(() => null);
      advance(keyOf(reviewTask));
      await refresh();
    } catch (cause) {
      setError(`Archive failed: ${String(cause)}`);
    } finally { setSaving(false); }
  };

  const snoozeSelected = (milliseconds: number) => {
    if (!reviewTask || saving) return;
    const until = Date.now() + milliseconds;
    hideLocally(reviewTask, milliseconds, true);
    onSnoozeUserTask?.(reviewTask.task, reviewTask.path, until);
    advance(keyOf(reviewTask));
  };

  const reconsider = async () => {
    if (!reviewTask || !manifest || !round?.result || manifest.status !== "open" || saving) return;
    setSaving(true);
    setError("");
    try {
      const reopened = await reviewReconsider(reviewTask.task.id, reviewTask.task.revision,
        manifest.revision, reviewTask.path);
      setPins([]);
      setNote("");
      setChoice("");
      setRanking([]);
      setResponseOutcome("");
      setMediaIndex(0);
      await refresh(); // Refresh the task revision before allowing the new verdict.
      setManifest(reopened);
    } catch (cause) {
      setError(`Unable to reconsider review: ${String(cause)}`);
      await refresh();
    } finally { setSaving(false); }
  };

  const submit = async (verdict: string) => {
    // A rejected/more-variants round stays in an open conversation, but its
    // verdict is immutable. Only a NEW requested round can accept another send.
    if (!reviewTask || !manifest || !round || saving || manifest.status === "resolved" || round.result) return;
    if (!reviewOutcomes(round.mode).some((option) => option.value === verdict)) {
      setError("Select a valid response outcome before submitting.");
      return;
    }
    if (verdict === "picked" && !choice ||
        verdict === "ranked" && ranking.length !== playable.length) {
      setError("Choose the required variant or finish ranking before submitting.");
      return;
    }
    setSaving(true);
    try {
      const resolved = await reviewResolve({
        task_id: reviewTask.task.id, verdict,
        choices: verdict === "picked" ? [choice] : verdict === "approved" ? [playable[0]?.id ?? "A"] : [],
        ranking: verdict === "ranked" ? ranking : [],
        pins, note: [note, ...await attachInboxImages(reviewTask.task.id, reviewTask.path)].filter(Boolean).join("\n\n"),
        expected_task_revision: reviewTask.task.revision,
        expected_manifest_revision: manifest.revision,
      }, reviewTask.path);
      // Always advance the unified Inbox after a successful decision, even
      // when a rejected round remains open pending the agent's next variant.
      hideLocally(reviewTask, 60 * 60 * 1000);
      setInboxImages([]);
      advance(keyOf(reviewTask));
      setManifest(verdict === "rejected" || verdict === "more-variants" ? resolved : null);
      setError("");
      void refresh();
    } catch (cause) {
      setError(`Review submission failed: ${String(cause)}`);
      await refresh();
    } finally { setSaving(false); }
  };
  const submitNoteOnly = async () => {
    if (!reviewTask || !isVisualReview || !(note.trim() || inboxImages.length) || saving || resolved) return;
    setSaving(true);
    setError("");
    try {
      const board = await loadBoard(reviewTask.path);
      const latest = board.tasks.find(task => task.id === reviewTask.task.id);
      if (!latest || latest.archived) throw new Error("Original task is unavailable");
      const old = latest.user_notes.trim();
      const message = [note.trim(), ...await attachInboxImages(latest.id, reviewTask.path)].filter(Boolean).join("\n\n");
      await updateTask({ ...latest,
        user_notes: [old, message].filter(Boolean).join("\n\n"),
        attention: [...new Set([...latest.attention, "user-note" as const])],
      }, reviewTask.path);
      setNote("");
      setInboxImages([]);
      await refresh(); // The review remains open; the next verdict uses the updated task revision.
    } catch (cause) {
      setError(`Unable to submit note: ${String(cause)}`);
    } finally { setSaving(false); }
  };

  const primaryVerdict = round?.mode === "pick" ? "picked"
    : round?.mode === "approve" ? "approved"
    : round?.mode === "rank" ? "ranked" : "marked";
  const primaryReady = round?.mode === "pick" ? !!choice
    : round?.mode === "rank" ? ranking.length === playable.length : true;
  const primaryLabel = round?.mode === "pick" ? (choice ? `Pick ${choice}` : "Pick")
    : round?.mode === "approve" ? "Approve"
    : round?.mode === "rank" ? "Submit ranking" : "Send markup";
  const labelOf = (id: string) => playable.find((artifact) => artifact.id === id)?.label || id;
  const awaitingNextRound = manifest?.status === "open" && !!round?.result;
  const resolved = manifest?.status === "resolved" || !!round?.result;
  const pinning = (pinMode || round?.mode === "markup") && !resolved;
  const isWeb = (artifact?: ReviewArtifact) => artifact?.kind === "html" || artifact?.kind === "url";
  const showStrip = playable.length > 1 || round?.mode === "pick" || round?.mode === "rank";

  const choose = (id: string) => {
    // Focusing a candidate is always allowed (one stage); changing the verdict is not once resolved.
    const index = playable.findIndex((value) => value.id === id);
    if (index >= 0) setMediaIndex(index);
    if (manifest?.status === "resolved") return;
    if (round?.mode === "pick") setChoice(id);
    if (round?.mode === "rank") setRanking((current) => current.includes(id)
      ? current.filter((value) => value !== id) : [...current, id]);
  };

  const addPin = (event: MouseEvent<HTMLDivElement>, artifact = activeArtifact) => {
    if (!(pinMode || round?.mode === "markup") || !artifact || manifest?.status === "resolved") return;
    const rect = event.currentTarget.getBoundingClientRect();
    const x = (event.clientX - rect.left) / rect.width;
    const y = (event.clientY - rect.top) / rect.height;
    setPins((current) => [...current, {
      x: Math.max(0, Math.min(1, x)), y: Math.max(0, Math.min(1, y)),
      artifact: artifact.id,
      ...(artifact.kind === "video" && videoRef.current ? { timestamp: videoRef.current.currentTime } : {}),
      ...(isAnimationLabEmbed(artifact) && embedTimeRef.current != null
        ? { timestamp: embedTimeRef.current } : {}),
      ...(artifact.kind === "html" || artifact.kind === "url" ? {
        viewport_width: iframeWidth || Math.round(rect.width), viewport_height: Math.round(rect.height),
        scroll_x: scrollOffset.current.x, scroll_y: scrollOffset.current.y,
      } : {}),
      note: "",
    }]);
  };

  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        onClose();
        return;
      }
      if (event.ctrlKey || event.altKey || event.metaKey || !isVisualReview) return;
      const target = event.target;
      // Buttons and links own Enter/Space. Otherwise pressing Enter on an
      // inspector control could ALSO submit the whole review.
      if (target instanceof HTMLElement &&
          (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT|BUTTON|A)$/.test(target.tagName))) return;
      const key = event.key.toLowerCase();
      if (event.key >= "1" && event.key <= "4") {
        const artifact = playable[Number(event.key) - 1];
        if (artifact) choose(artifact.id);
      } else if (key === "p") setPinMode((current) => !current);
      else if (key === "v") setOnion((current) => !current);
      else if (key === "z") setGameZoom((current) => !current);
      else if (key === "enter") { if (responseOutcome) void submit(responseOutcome); }
      else if (key === "r") setResponseOutcome("rejected");
      else if (key === "m") {
        if (round && reviewOutcomes(round.mode).some((option) => option.value === "more-variants"))
          setResponseOutcome("more-variants");
      }
      else if (key === "j" || key === "k") {
        const index = items.findIndex((item) => keyOf(item) === selectedKey);
        const next = items[index + (key === "j" ? 1 : -1)];
        if (next) setSelectedKey(keyOf(next));
      } else return;
      event.preventDefault();
    };
    window.addEventListener("keydown", keydown);
    return () => window.removeEventListener("keydown", keydown);
  });

  const pinsFor = (artifact: ReviewArtifact) => pins.filter((pin) => pin.artifact === artifact.id);
  const image = (artifact: ReviewArtifact, className = "") => {
    const src = activeSrc(artifact);
    if (artifact.kind === "video" && src) return (
      <video ref={videoRef} controls src={src} className={className} />
    );
    if (artifact.kind === "html" || artifact.kind === "url") {
      const posterSrc = artifact.poster ? assets[artifact.poster] : "";
      const html = htmlDocuments[artifact.path];
      const iframe = artifact.kind === "html"
        ? (html ? <iframe title={artifact.label} srcDoc={html}
            style={{ width: iframeWidth || "100%", maxWidth: "none" }}
            sandbox="allow-scripts allow-forms" referrerPolicy="no-referrer" />
          : null)
        : (showPoster ? null : <iframe ref={isAnimationLabEmbed(artifact) ? embedFrameRef : undefined}
            title={artifact.label} src={reviewEmbedUrl(artifact.path)}
            sandbox="allow-scripts allow-forms" referrerPolicy="no-referrer"
            onError={() => setShowPoster(true)} />);
      return (
        <div className="review-web-artifact">
          {iframe || (posterSrc ? <img src={posterSrc} alt={artifact.label + " poster"} /> :
            <p>HTML preview unavailable · {artifact.label}</p>)}
          {isAnimationLabEmbed(artifact) && embedPlayback && (
            <div className="review-animation-controls" onClick={(event) => event.stopPropagation()}>
              <button className="button secondary" type="button" onClick={() => sendEmbedCommand("play")}>Play</button>
              <button className="button secondary" type="button" onClick={() => sendEmbedCommand("pause")}>Pause</button>
              <input aria-label="Animation time" type="range" min={0} max={embedPlayback.duration || 1}
                step={0.01} value={embedPlayback.t}
                onChange={(event) => {
                  const t = Number(event.currentTarget.value);
                  setEmbedPlayback((current) => current ? { ...current, t } : null);
                  sendEmbedCommand("time", t);
                }} />
              <span>{embedPlayback.t.toFixed(2)}s / {embedPlayback.duration.toFixed(2)}s</span>
            </div>
          )}
          {artifact.kind === "url" && (
            <button className="review-web-fallback" type="button"
              onClick={(event) => { event.stopPropagation(); setShowPoster((value) => !value); }}>
              {showPoster ? "Try live URL" : "Show poster"}
            </button>
          )}
          {(pinMode || round?.mode === "markup") && <div className="review-web-pin-surface" aria-label="Place review pin" />}
        </div>
      );
    }
    return src ? <img src={src} alt={artifact.label} className={className} />
      : <div className="review-media-missing">Media unavailable{artifact.storage === "local" ? " (stored on the PC)" : ""}</div>;
  };

  // Any visual review gets the same maximized inspection workspace. Human
  // text requests continue to use the compact unified Inbox below.
  if (isVisualReview && reviewTask && round) {
    return <VisualReviewInspector
      review={reviewTask}
      round={round}
      history={manifest?.rounds.slice(0, -1) ?? []}
      items={items}
      selectedKey={selectedKey}
      onSelectTask={setSelectedKey}
      onClose={onClose}
      onOpenSession={() => void openReviewTaskSession(reviewTask)}
      sessionOpening={openingTaskSession}
      allBoards={allBoards}
      showResolved={showResolved}
      onAllBoards={setAllBoards}
      onShowResolved={setShowResolved}
      playable={playable}
      activeIndex={Math.min(mediaIndex, Math.max(0, playable.length - 1))}
      onFocusArtifact={setMediaIndex}
      onChooseArtifact={choose}
      choice={choice}
      ranking={ranking}
      pins={pins}
      setPins={setPins}
      pinning={pinning}
      onTogglePin={() => setPinMode((current) => !current)}
      addPin={addPin}
      renderArtifact={image}
      posterOf={(artifact) => artifact.kind === "image"
        ? activeSrc(artifact) || "" : artifact.poster ? assets[artifact.poster] || "" : ""}
      iframeWidth={iframeWidth}
      onIframeWidth={setIframeWidth}
      note={note}
      setNote={setNote}
      saving={saving}
      outcome={responseOutcome}
      onOutcome={setResponseOutcome}
      onSkip={() => advance(undefined, true)}
      onSnooze={snoozeSelected}
      resolved={resolved}
      awaitingNextRound={awaitingNextRound}
      error={error}
      primaryVerdict={primaryVerdict}
      primaryReady={primaryReady}
      submit={submit}
      onNoteOnly={submitNoteOnly}
      onBrowseFiles={() => void browseInboxFiles()}
      imagePreviews={inboxImagePreview}
      imageCount={inboxImages.length}
      onPaste={onInboxPaste}
      onDrop={onInboxDrop}
      onMoveToDraft={moveSelectedToDraft}
      onArchive={archiveSelected}
      onReconsider={reconsider}
    />;
  }

  return (
    <section className="review-shell" onPaste={onInboxPaste}
      onDragOver={event => event.preventDefault()} onDrop={onInboxDrop}>
      <button className="button secondary review-close review-close-corner" type="button"
        title="Close Inbox (Esc)" aria-label="Close Inbox" onClick={onClose}>×</button>
      <aside className="review-queue">
        <header>
          <strong>Feedback inbox <span>{items.length}</span></strong>

        </header>
        <div className="review-queue-filters">
          <label className="review-all-boards">
            <input type="checkbox" checked={allBoards} onChange={(event) => setAllBoards(event.target.checked)} />
            All boards
          </label>
          <label className="review-all-boards">
            <input type="checkbox" checked={showResolved} onChange={(event) => setShowResolved(event.target.checked)} />
            Include past decisions
          </label>
        </div>
        <nav className="review-queue-list" aria-label="Unified human feedback queue">
        {items.map((item) => (
          <button key={keyOf(item)} type="button" title={item.task.title}
            aria-current={keyOf(item) === selectedKey ? "true" : undefined}
            className={["review-queue-item", queueThumbnails[keyOf(item)] && "has-preview",
              keyOf(item) === selectedKey && "selected",
              item.task.review_status !== "open" && !!item.task.review_status && "resolved",
              !item.task.review_status && "review-user-item"
            ].filter(Boolean).join(" ")}
            onClick={() => setSelectedKey(keyOf(item))}>
            {queueThumbnails[keyOf(item)] && (
              <span className="review-queue-media" aria-hidden="true">
                <img className="review-queue-preview" src={queueThumbnails[keyOf(item)]} alt="" />
              </span>
            )}
            <span className="review-queue-copy">
              <small>{item.project} · {item.task.id}</small>
              <strong>{item.task.title}</strong>
              <span className="review-queue-meta">
                <em className={`review-queue-mode review-queue-mode-${queueModes[keyOf(item)] ||
                  (item.task.review_status ? "review" : "user")}`}>
                  {queueModes[keyOf(item)] || (item.task.review_status ? "review" : "response")}
                </em>
                {item.task.review_status === "awaiting_revision" ? "Rejected · awaiting AI revision" :
                  item.task.review_status === "resolved" ? "Resolved" :
                  item.task.review_round ? `Round ${item.task.review_round}` : "Awaiting response"}
                {(queueCounts[keyOf(item)] ?? 0) > 1 && ` · ${queueCounts[keyOf(item)]} candidates`}
              </span>
            </span>
          </button>
        ))}
        </nav>
        {!items.length && <p className="review-empty">No pending feedback. Agents can request reviews with <code>task review request</code>.</p>}
      </aside>
      <section className="review-main">
        <div className="review-scroll">
        {error && <div className="error-banner" role="alert">{error}</div>}
        {!reviewTask ? (
          <div className="review-placeholder">No pending feedback. Open the board to continue work.</div>
        ) : !isVisualReview ? (
          <div className="review-human-stage">
            <header className="review-heading">
              <div>
                <small>{reviewTask.project} · {reviewTask.task.id}</small>
                <h2>{reviewTask.task.title}</h2>
                <p>{reviewTask.task.description || "Provide your instructions for this task."}</p>
              </div>
              <div className="flex items-center gap-2">
                {(reviewTask.task.claimed_by || reviewTask.task.live?.with_me) && (
                  <button type="button" className="button secondary"
                    aria-label={`Open session for ${reviewTask.task.id}`}
                    disabled={openingTaskSession}
                    onClick={() => void openReviewTaskSession(reviewTask)}>
                    {openingTaskSession ? "Opening…" : "Open Session"}
                  </button>
                )}
                <span className={`inbox-task-column-chip column-${reviewTask.task.column}`}>{reviewTask.task.column}</span>
                <div className="inbox-more-wrap">
                  <button className="button secondary" type="button" aria-label="More Inbox actions"
                    aria-expanded={moreOpen} onClick={() => setMoreOpen(value => !value)}>⋯</button>
                  {moreOpen && <div className="inbox-more-menu">
                    <button type="button" disabled={saving} onClick={() => void moveSelectedToDraft()}>Move to Draft</button>
                    <button type="button" disabled={saving} onClick={() => void archiveSelected()}>Archive</button>
                  </div>}
                </div>
              </div>
            </header>
            <div className="review-workspace">
              <div className="review-stage-column">
                <div className="review-user-fields">
                  {reviewTask.task.question?.prompt && (
                    <div className="review-user-question">
                      <strong>{reviewTask.task.question.prompt}</strong>
                      <div className="review-user-choice">
                        {(reviewTask.task.question.options || []).map((value) =>
                          <button type="button" key={value}
                            className={userAnswer === value ? "button primary" : "button secondary"}
                            onClick={() => { setUserAnswer(value); setKeepForUser(false); }}>{value}</button>)}
                      </div>
                    </div>
                  )}
                  <label>Your response
                    <textarea onKeyDown={(event) => {
                      if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey) {
                        event.preventDefault();
                        void submitUser();
                      }
                    }} value={userNote} onChange={(event) => {
                      setUserNote(event.currentTarget.value);
                      setKeepForUser(false);
                    }}
                      rows={7} placeholder="Your feedback or instructions…" />
                  </label>
                  <button type="button" className="button secondary inbox-attach-button"
                    onClick={() => void browseInboxFiles()}>+ Images / local files…</button>
                  {inboxImages.length > 0 && inboxImagePreview}
                  <label>Acceptance criteria
                    <textarea value={userCriteria} onChange={(event) => {
                      setUserCriteria(event.currentTarget.value);
                      setKeepForUser(false);
                    }}
                      rows={4} placeholder="One criterion per line" />
                  </label>
                  {(reviewTask.task.kind === "for_user" || reviewTask.task.attention.includes("needs-user")) &&
                    <label className="review-user-checkbox">
                      <input type="checkbox" checked={keepForUser}
                        onChange={(event) => setKeepForUser(event.target.checked)} />
                      Keep marked For user
                    </label>}
                </div>
              </div>
              <div className="review-feedback-rail">
                <h3>Task details</h3>
                <p>{reviewTask.task.category || "Uncategorized"} · {reviewTask.task.column}</p>
                <p>Respond without leaving this inbox. All messages, visual files and
                  ordinary feedback are handled through the same queue.</p>
                {reviewTask.path === project.path && (
                  <button className="button secondary" type="button"
                    onClick={() => onOpenUserTask?.(reviewTask.task)}>Full editor ↗</button>
                )}
              </div>
            </div>
            <InboxResponseFooter
              mode="reply"
              outcome="respond"
              onOutcome={() => {}}
              onSkip={() => advance(undefined, true)}
              onSnooze={snoozeSelected}
              onSubmit={() => void submitUser()}
              onComplete={() => void submitUser(true)}
              onReopen={() => void submitUser(false, true)}
              notePresent={!!userNote.trim() || !!userAnswer.trim() || inboxImages.length > 0}
              busy={saving}
              summary={reviewTask.task.kind === "for_user"
                ? "Respond or complete this human-owned task"
                : "Done closes the feedback request; Needs changes returns work to AI Ready"}
            />
          </div>
        ) : !round ? (
          <div className="review-placeholder">Loading feedback details…</div>
        ) : (
          <>
            <header className="review-heading">
              <div><small>{reviewTask.project} · {reviewTask.task.id} · Round {round.number}</small>
                <h2>{reviewTask.task.title}</h2><p>{round.judge}</p></div>
              <span className="review-mode-label">{round.mode.toUpperCase()}</span>
            </header>
            {round.criteria.length > 0 &&
              <div className="review-criteria" aria-label="Review criteria">{round.criteria.map((text, i) => <span key={i}>{text}</span>)}</div>}
            <div className="review-workspace">
              <div className="review-stage-column">
            <div className="review-toolbar" role="toolbar" aria-label="Review tools">
              <button type="button" className={pinMode ? "button primary review-pin-toggle" : "button secondary review-pin-toggle"}
                disabled={resolved}
                onClick={() => setPinMode((current) => !current)} aria-pressed={pinMode}>
                Pin (P){pins.length > 0 && <span className="review-pin-count" aria-label={`${pins.length} pins`}>{pins.length}</span>}</button>
              <button type="button" className={gameZoom ? "button primary" : "button secondary"}
                onClick={() => setGameZoom((current) => !current)} aria-pressed={gameZoom}>Game zoom (Z)</button>
              {playable.some(isWeb) &&
                <label>Viewport <select value={iframeWidth} onChange={(event) => setIframeWidth(Number(event.target.value))}>
                  <option value={0}>Fit</option>
                  <option value={1440}>1440px</option>
                  <option value={1080}>1080px</option>
                  <option value={728}>728px</option>
                  <option value={390}>Phone · 390px</option>
                </select></label>}
              {round.mode === "approve" && (
                <button type="button" className={onion ? "button primary" : "button secondary"}
                  onClick={() => setOnion((current) => !current)} aria-pressed={onion}>Overlay (V)</button>
              )}
              <span className="review-shortcuts"><kbd>J</kbd>/<kbd>K</kbd> queue · <kbd>1</kbd>–<kbd>4</kbd> options ·
                {" "}<kbd>Enter</kbd> submit · <kbd>R</kbd> reject · <kbd>M</kbd> more variants</span>
            </div>
            {pinning &&
              <p className="review-pin-hint" role="status">Click the stage to drop pin {pins.length + 1}. Comment on it in Feedback.</p>}
            {round.previous && round.mode === "approve" && activeArtifact ? (
              <div className="review-compare">
                <div className="review-compare-stage" ref={stageRef}
                  role="region" aria-label={`Before and after: ${activeArtifact.label || activeArtifact.id}`}
                  onClick={(event) => addPin(event)}>
                  <div className="review-compare-before">{image(round.previous)}</div>
                  <div className="review-compare-after" style={{ clipPath: onion ? undefined : `inset(0 ${100 - slider}% 0 0)`,
                    opacity: onion ? slider / 100 : 1 }}>
                    {image(activeArtifact)}
                  </div>
                  {pinsFor(activeArtifact).map((pin) => <span className="review-pin" key={pins.indexOf(pin)}
                    style={{ left: `${pin.x * 100}%`, top: `${pin.y * 100}%` }}>{pins.indexOf(pin) + 1}</span>)}
                </div>
                <label>Before / after <input type="range" value={slider} min={0} max={100}
                  onChange={(event) => setSlider(Number(event.target.value))} /></label>
              </div>
            ) : activeArtifact ? (
              <div className={["review-focus", gameZoom && "game-zoom", pinning && "pinning"].filter(Boolean).join(" ")}>
                <div className="review-focus-frame">
                  <span className="review-stage-tag" aria-hidden="true">
                    <kbd>{playable.indexOf(activeArtifact) + 1}</kbd>
                    <span className="review-artifact-label">{activeArtifact.label || activeArtifact.id}</span>
                    {round.mode === "pick" && choice === activeArtifact.id && <span className="review-artifact-mark">✓ Picked</span>}
                    {round.mode === "rank" && ranking.includes(activeArtifact.id) &&
                      <span className="review-artifact-mark ranked">#{ranking.indexOf(activeArtifact.id) + 1}</span>}
                  </span>
                  {/* The stage only drops pins; selection happens via the strip or 1–4 so
                      interacting with a live HTML candidate never picks it by accident. */}
                  <div className={["review-artifact-stage review-focus-stage", isWeb(activeArtifact) && "web",
                    isWeb(activeArtifact) && iframeWidth > 0 && iframeWidth <= 430 && "phone",
                    round.mode === "pick" && choice === activeArtifact.id && "chosen",
                    round.mode === "rank" && ranking.includes(activeArtifact.id) && "ranked"].filter(Boolean).join(" ")}
                    role="region"
                    aria-label={`Candidate ${playable.indexOf(activeArtifact) + 1} of ${playable.length}: ${activeArtifact.label || activeArtifact.id}`}
                    onClick={(event) => { if (pinMode || round.mode === "markup") addPin(event, activeArtifact); }}>
                    {image(activeArtifact)}
                    {pinsFor(activeArtifact).map((pin) => <span className="review-pin" key={pins.indexOf(pin)}
                      style={{ left: `${pin.x * 100}%`, top: `${pin.y * 100}%` }}>{pins.indexOf(pin) + 1}</span>)}
                  </div>
                </div>
              </div>
            ) : <div className="review-media-missing">This round has no candidate artifacts.</div>}
            {showStrip && playable.length > 0 && (
              <div className="review-candidate-strip" role="group" aria-label="Candidates">
                {playable.map((artifact, index) => {
                  const thumb = artifact.kind === "image" ? activeSrc(artifact)
                    : artifact.poster ? assets[artifact.poster] : "";
                  const pinCount = pinsFor(artifact).length;
                  const picked = round.mode === "pick" && choice === artifact.id;
                  const rank = round.mode === "rank" ? ranking.indexOf(artifact.id) : -1;
                  const status = picked ? "✓ Picked" : rank >= 0 ? `#${rank + 1}`
                    : artifact.kind.toUpperCase() + (isWeb(artifact) ? ` · ${iframeWidth ? `${iframeWidth}px` : "Fit"}` : "");
                  const focused = activeArtifact?.id === artifact.id;
                  return (
                    <button key={artifact.id} type="button"
                      className={["review-candidate-chip", picked && "chosen", rank >= 0 && "ranked", focused && "active"].filter(Boolean).join(" ")}
                      aria-current={focused ? "true" : undefined}
                      aria-pressed={round.mode === "pick" ? picked : round.mode === "rank" ? rank >= 0 : undefined}
                      aria-label={`${index + 1}: ${artifact.label || artifact.id}${picked ? ", picked" : ""}${rank >= 0 ? `, ranked ${rank + 1}` : ""}${pinCount ? `, ${pinCount} pin${pinCount === 1 ? "" : "s"}` : ""}`}
                      onClick={() => choose(artifact.id)}>
                      <span className="review-candidate-mini" aria-hidden="true">
                        {thumb ? <img src={thumb} alt="" /> : <span>{artifact.kind.toUpperCase()}</span>}
                      </span>
                      <span className="review-candidate-copy">
                        <strong><kbd>{index + 1}</kbd><span className="review-artifact-label">{artifact.label || artifact.id}</span></strong>
                        <small><span className={picked || rank >= 0 ? "review-artifact-mark" + (rank >= 0 ? " ranked" : "") : undefined}>{status}</span>
                          {pinCount > 0 && ` · ${pinCount} pin${pinCount === 1 ? "" : "s"}`}</small>
                      </span>
                    </button>
                  );
                })}
              </div>
            )}
              </div>
              <aside className="review-details" aria-label="Review context and feedback">
                <section className="review-details-card review-feedback-card" aria-label="Feedback">
                  <h3>Feedback <span>{pins.length} pin{pins.length === 1 ? "" : "s"}</span></h3>
                  {pins.length === 0 && <p className="review-muted">{pinning
                    ? "Click the stage to place pins on specific details."
                    : "Press P to pin specific details, then leave an overall note."}</p>}
                  {pins.length > 0 && <ol className="review-pin-list">{pins.map((pin, index) => (
                    <li key={index}>
                      <span className="review-pin-badge" aria-hidden="true">{index + 1}</span>
                      <label>
                        <span>{playable.find((a) => a.id === pin.artifact)?.label || pin.artifact} · {Math.round(pin.x * 100)}%, {Math.round(pin.y * 100)}%
                          {typeof pin.timestamp === "number" ? ` · ${pin.timestamp.toFixed(1)}s` : ""}
                          {pin.viewport_width ? ` · ${pin.viewport_width}px` : ""}</span>
                        <input value={pin.note || ""} placeholder="Pin comment" readOnly={resolved}
                          aria-label={`Comment for pin ${index + 1}`}
                          onChange={(event) => setPins((current) =>
                            current.map((candidate, i) => i === index ? { ...candidate, note: event.target.value } : candidate))} />
                      </label>
                      {!resolved &&
                        <button type="button" aria-label={`Remove pin ${index + 1}`}
                          onClick={() => setPins((current) => current.filter((_, i) => i !== index))}>×</button>}
                    </li>))}</ol>}
                  <label className="review-note">Overall note<textarea rows={2} value={note} onChange={(event) => setNote(event.target.value)}
                    readOnly={resolved} placeholder="What should the agent change or keep?" /></label>
                </section>
                <section className="review-details-card" aria-label="Context">
                  <h3>Context <span>References</span></h3>
                  {round.previous && <div className="review-reference">
                    <span className="review-reference-label">Previous</span>
                    <span>{round.previous.label}</span>
                  </div>}
                  {round.references.map((artifact) => (
                    <div className="review-reference" key={artifact.id}>
                      <span className="review-reference-label">Reference</span>
                      <span>{artifact.label}</span>
                    </div>
                  ))}
                  {!round.references.length && !round.previous && <p className="review-muted">No references supplied.</p>}
                </section>
              </aside>
            </div>
          </>
        )}
        </div>
        {reviewTask && round && (
          <footer className={resolved ? "review-decision resolved" : "review-decision"} aria-label="Review decision">
            <span className="review-decision-summary" aria-live="polite">
              {resolved ? <>{awaitingNextRound ? "This round was sent back · awaiting a revised review from the agent" : "Review resolved"} · <b>{round.result?.verdict || "resolved"}</b>
                {round.result?.choices.length ? <> · {round.result.choices.map(labelOf).join(", ")}</> : null}
                {" "}· stored verdict and pins are read-only</>
              : <>
                {round.mode === "pick" ? (choice ? <>Picked <b>{labelOf(choice)}</b></> : "No candidate picked yet")
                  : round.mode === "rank" ? (ranking.length
                    ? <>Ranked <b>{ranking.map(labelOf).join(" › ")}</b> · {ranking.length}/{playable.length} · top {round.top_k} ship</>
                    : <>0/{playable.length} ranked · top {round.top_k} ship</>)
                  : round.mode === "approve" ? <>Approving <b>{playable[0] ? labelOf(playable[0].id) : "candidate"}</b>
                    {round.previous ? " against previous" : ""}</>
                  : <>Markup · {playable.length} candidate{playable.length === 1 ? "" : "s"}</>}
                {" "}· {pins.length} pin{pins.length === 1 ? "" : "s"} · {note.trim() ? "note added" : "note optional"}
              </>}
            </span>
            {!resolved && <>
              <button type="button" className="button danger review-decision-reject" disabled={saving}
                onClick={() => void submit("rejected")}>Reject <kbd>R</kbd></button>
              <button type="button" className="button secondary review-decision-more" disabled={saving}
                onClick={() => void submit("more-variants")}>More variants <kbd>M</kbd></button>
              <button type="button" className="button primary review-decision-primary" disabled={saving || !primaryReady}
                title={primaryReady ? undefined : round.mode === "pick" ? "Pick a candidate first" : "Rank every candidate first"}
                onClick={() => void submit(primaryVerdict)}>
                {saving ? "Sending…" : <>{primaryLabel} <kbd>Enter</kbd></>}</button>
            </>}
          </footer>
        )}
      </section>
    </section>
  );
}
