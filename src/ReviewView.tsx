import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type DragEvent, type MouseEvent } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { archiveTask, listProjects, loadBoard, openTaskSession, readLocalImage, saveTaskImage, taskSyncQueue, updateTask, type ProjectTab } from "./api";
import { reviewAsset, reviewClose, reviewReconsider, reviewResolve, reviewShow,
  type ReviewArtifact, type ReviewManifest, type ReviewPin } from "./reviewApi";
import type { Task } from "./types";
import { isUserQueueEligible } from "./userQueueEligibility";
import { VisualReviewInspector } from "./VisualReviewInspector";
import { InboxQueueSidebar, type InboxQueueItem } from "./InboxQueueSidebar";
import { InboxResponseFooter, reviewOutcomes } from "./InboxResponseFooter";
import { InboxRichText } from "./InboxRichText";
import { parseDescriptionTimeline } from "./descriptionTimeline";

type QueueItem = InboxQueueItem;
// Only the most recent text-Inbox action is reversible. Its saved revision
// guards against undo overwriting progress made by another agent afterwards.
type AnswerUndo = { path: string; key: string; before: Task; savedRevision: string };
type InboxDraft = {
  note: string; pins: ReviewPin[]; choice: string; ranking: string[];
  userNote: string; userAnswer: string; userCriteria: string; keepForUser: boolean;
  images: Array<{ dataUrl: string; mime: string; name: string }>;
};
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

/** Review HTML is displayed with an opaque iframe origin; relative image URLs
 * therefore need to be resolved via the existing bounded review_asset command.
 * The bridge also asks the desktop host to open links that sandbox navigation
 * cannot launch (notably wartribes-tour://). */
const HTML_REVIEW_BRIDGE = `
(() => {
  const loaded = new Map(), pending = new Set();
  const relative = ref => ref && !/^(?:[a-z][a-z0-9+.-]*:|\\/|#)/i.test(ref);
  function resolveImages() {
    for (const img of document.querySelectorAll('img[src]')) {
      const ref = img.getAttribute('src');
      if (!relative(ref)) continue;
      if (loaded.has(ref)) { img.src = loaded.get(ref); continue; }
      if (!pending.has(ref)) {
        pending.add(ref);
        parent.postMessage({ type: 'taskboard-review-asset-request', ref }, '*');
      }
    }
  }
  new MutationObserver(resolveImages).observe(document.documentElement,
    { childList: true, subtree: true, attributes: true, attributeFilter: ['src'] });
  addEventListener('DOMContentLoaded', resolveImages);
  addEventListener('message', event => {
    if (event.source !== parent || event.data?.type !== 'taskboard-review-asset-response') return;
    const { ref, dataUrl } = event.data;
    if (typeof ref !== 'string' || typeof dataUrl !== 'string' ||
        !dataUrl.startsWith('data:image/')) return;
    loaded.set(ref, dataUrl);
    pending.delete(ref);
    resolveImages();
  });
  // This iframe has an opaque sandbox origin: its native context menu never
  // bubbles to TaskBoard. Forward the pointer location to the parent so users
  // can reveal the original HTML file in Fomni without intercepting left-clicks.
  addEventListener('contextmenu', event => {
    event.preventDefault();
    parent.postMessage({ type: 'taskboard-review-contextmenu', x: event.clientX, y: event.clientY }, '*');
  }, true);
  addEventListener('click', event => {
    const anchor = event.target instanceof Element && event.target.closest('a[href]');
    const href = anchor?.getAttribute('href') || '';
    if (!/^(?:https?:\\/\\/|wartribes-tour:\\/\\/)/i.test(href)) return;
    event.preventDefault();
    event.stopPropagation();
    parent.postMessage({ type: 'taskboard-review-open-url', url: href }, '*');
  }, true);
})();
`;

function htmlIndexPath(path: string) {
  const normalized = path.replace(/\\/g, "/");
  return /\.html?$/i.test(normalized)
    ? normalized : normalized.replace(/\/$/, "") + "/index.html";
}
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
  // An Inbox opened empty is intentionally allowed to show its empty state.
  // Once it has shown feedback, close when the queue is exhausted.
  const hasShownFeedback = useRef(false);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  // Skips last for this Inbox pass; manual selection can still revisit the item.
  const skippedThisPass = useRef<Set<string>>(new Set());
  const pendingResponses = useRef(new Set<string>());
  const locallyHidden = useRef(new Map<string, number>());
  const failedDrafts = useRef(new Map<string, InboxDraft>());
  const [pendingResponseCount, setPendingResponseCount] = useState(0);
  const [loadedManifest, setLoadedManifest] = useState<{ key: string; value: ReviewManifest } | null>(null);
  const manifest = loadedManifest?.key === selectedKey ? loadedManifest.value : null;
  const [assets, setAssets] = useState<Record<string, string>>({});
  const [htmlDocuments, setHtmlDocuments] = useState<Record<string, string>>({});
  const [queueThumbnails, setQueueThumbnails] = useState<Record<string, string>>({});
  const [queueModes, setQueueModes] = useState<Record<string, string>>({});
  const [queueCounts, setQueueCounts] = useState<Record<string, number>>({});
  const [iframeWidth, setIframeWidth] = useState(0);
  const [showPoster, setShowPoster] = useState(false);
  const scrollOffset = useRef({ x: 0, y: 0 });
  const embedFrameRef = useRef<HTMLIFrameElement>(null);
  const htmlFrameRef = useRef<HTMLIFrameElement>(null);
  const htmlAssetFolderRef = useRef("");
  const htmlAssetBoardRef = useRef("");
  const embedTimeRef = useRef<number | null>(null);
  const [embedPlayback, setEmbedPlayback] = useState<{ duration: number; playing: boolean; t: number } | null>(null);
  const [choice, setChoice] = useState("");
  const [ranking, setRanking] = useState<string[]>([]);
  const [pins, setPins] = useState<ReviewPin[]>([]);
  const [note, setNote] = useState("");
  const [noteFeedback, setNoteFeedback] = useState("");
  const [pinMode, setPinMode] = useState(false);
  const [gameZoom, setGameZoom] = useState(false);
  const [onion, setOnion] = useState(false);
  const [slider, setSlider] = useState(50);
  const [mediaIndex, setMediaIndex] = useState(0);
  const [saving, setSaving] = useState(false);
  const [openingTaskSession, setOpeningTaskSession] = useState(false);
  const [error, setError] = useState("");
  const [backgroundError, setBackgroundError] = useState("");
  const [userNote, setUserNote] = useState("");
  const [userCriteria, setUserCriteria] = useState("");
  const [keepForUser, setKeepForUser] = useState(false);
  const [userAnswer, setUserAnswer] = useState("");
  const [lastAnswered, setLastAnswered] = useState<AnswerUndo | null>(null);
  const [snoozedLocally, setSnoozedLocally] = useState<Record<string, number>>(loadReviewSnoozes);
  const [responseOutcome, setResponseOutcome] = useState("");
  const [moreOpen, setMoreOpen] = useState(false);
  const [inboxImages, setInboxImages] = useState<Array<{dataUrl:string;mime:string;name:string}>>([]);
  const videoRef = useRef<HTMLVideoElement>(null);
  const [mediaServer, setMediaServer] = useState("");
  const [mediaError, setMediaError] = useState("");
  const [videoError, setVideoError] = useState("");
  const stageRef = useRef<HTMLDivElement>(null);
  const reviewTask = items.find((item) => keyOf(item) === selectedKey);
  const toggleInboxPriority = async (item: QueueItem) => {
    if (saving) return;
    setSaving(true);
    setError("");
    try {
      // Only change the Priority flag on the latest revision. Do not resolve
      // the question, discard typed feedback, or change the task's column.
      const snapshot = await loadBoard(item.path);
      const current = snapshot.tasks.find(task => task.id === item.task.id);
      if (!current) throw new Error("Task no longer exists.");
      const prioritized = current.attention.includes("priority");
      const saved = await updateTask({
        ...current,
        attention: prioritized
          ? current.attention.filter(flag => flag !== "priority")
          : [...current.attention, "priority"],
      }, item.path);
      setItems(previous => previous.map(entry =>
        keyOf(entry) === keyOf(item) ? { ...entry, task: saved } : entry));
      onUserTaskUpdated?.();
      await refresh();
    } catch (cause) {
      setError("Unable to update Priority: " + String(cause));
    } finally {
      setSaving(false);
    }
  };
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
      // Keep the actionable queue unchanged. With history enabled, also show
      // answered text decisions (including Done/Needs changes verifications)
      // and completed visual reviews, even after their tasks were archived.
      const pending: QueueItem[] = snapshots.flatMap(({ project: board, snapshot }) =>
        snapshot.tasks.flatMap((task) => {
          const actionable = !task.archived && (task.review_status === "open" ||
            (task.id === initialTaskId && board.path === project.path) ||
            (isUserQueueEligible(task, snapshot, {}) && (allBoards || board.id !== project.id ||
              userTasks.some((candidate) => candidate.id === task.id))));
          const historical = !actionable && (
            task.review_status === "resolved" || task.review_status === "awaiting_revision" ||
            !!task.question?.answer?.trim());
          return actionable || (showResolved && historical)
            ? [{ project: board.name, path: board.path, task, historical }]
            : [];
        }))
        .filter((item) => !pendingResponses.current.has(keyOf(item)) &&
          (item.historical || (
            !(Number(locallyHidden.current.get(keyOf(item))) > Date.now()) &&
            !(Number(snoozedLocally[keyOf(item)]) > Date.now()))));
      pending.sort((a, b) => Number(!!a.historical) - Number(!!b.historical) ||
        b.task.created.localeCompare(a.task.created) || a.task.id.localeCompare(b.task.id));
      if (pending.length) hasShownFeedback.current = true;
      else if (!showResolved && hasShownFeedback.current && pendingResponses.current.size === 0) {
        closeRef.current();
        return;
      }
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
    setNoteFeedback("");
    setLoadedManifest(null);
    setAssets({});
    setHtmlDocuments({});
    setShowPoster(false);
    setChoice("");
    setRanking([]);
    setPins([]);
    const draft = failedDrafts.current.get(selectedKey);
    setNote(draft?.note ?? "");
    setMediaIndex(0);
    setPinMode(false);
    setUserNote(draft?.userNote ?? "");
    setUserCriteria(draft?.userCriteria ?? reviewTask?.task.acceptance_criteria.join("\n") ?? "");
    setKeepForUser(draft?.keepForUser ?? (reviewTask?.task.kind === "for_user" ||
      !!reviewTask?.task.attention.includes("needs-user")));
    setUserAnswer(draft?.userAnswer ?? "");
    setInboxImages(draft?.images ?? []);
    setResponseOutcome("");
    if (!reviewTask || !reviewTask.task.review_status) return;
    let cancelled = false;
    void reviewShow(reviewTask.task.id, reviewTask.path).then((value) => {
      if (cancelled) return;
      setLoadedManifest({ key: selectedKey, value });
      const verdict = value.rounds[value.rounds.length - 1]?.result;
      const draft = failedDrafts.current.get(selectedKey);
      setPins(draft?.pins ?? verdict?.pins ?? []);
      setNote(draft?.note ?? verdict?.note ?? "");
      setChoice(draft?.choice ?? verdict?.choices[0] ?? "");
      setRanking(draft?.ranking ?? verdict?.ranking ?? []);
    }).catch((cause) => { if (!cancelled) setError(String(cause)); });
    return () => { cancelled = true; };
  }, [selectedKey]);

  // Captures can be published while an unanswered Inbox review is already
  // open (e.g. the verified boar gait video after its still-image round).
  // Refresh only the manifest, preserving the user's draft answer and pins.
  useEffect(() => {
    if (!reviewTask?.task.review_status || !selectedKey) return;
    let cancelled = false;
    let pending = false;
    const reload = () => {
      if (pending || cancelled) return;
      pending = true;
      void reviewShow(reviewTask.task.id, reviewTask.path).then((value) => {
        if (cancelled) return;
        setLoadedManifest((current) => {
          if (!current || current.key !== selectedKey ||
              current.value.revision === value.revision) return current;
          return { key: selectedKey, value };
        });
      }).catch(() => { /* A transient write does not erase the active review. */ })
        .finally(() => { pending = false; });
    };
    const interval = window.setInterval(reload, 8000);
    return () => { cancelled = true; window.clearInterval(interval); };
  }, [selectedKey, reviewTask?.task.id, reviewTask?.path, reviewTask?.task.review_status]);

  useEffect(() => {
    if (!round || !reviewTask) return;
    let cancelled = false;
    const list = [...round.artifacts, ...round.references, ...(round.previous ? [round.previous] : [])];
    const paths = [...new Set(list.flatMap((artifact) => [artifact.path, artifact.poster].filter(Boolean) as string[]))];
    const videos = new Set(list.filter(artifact => artifact.kind === "video").map(a => a.path));
    void Promise.all(paths.map(async (path) => {
      // Videos must never travel through IPC/base64: the browser obtains byte
      // ranges on demand from the bounded TaskBoard media server.
      if (videos.has(path)) return [path, ""] as const;
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
      const index = htmlIndexPath(artifact.path);
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
      bridge.textContent = HTML_REVIEW_BRIDGE + '\naddEventListener("scroll",function(){parent.postMessage({type:"taskboard-review-scroll",x:scrollX,y:scrollY},"*")},true);';
      doc.head.prepend(bridge);
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
      if (event.source === htmlFrameRef.current?.contentWindow) {
        if (event.data?.type === "taskboard-review-open-url") {
          const url = event.data.url;
          if (typeof url === "string" && /^(?:https?:\/\/|wartribes-tour:\/\/)/i.test(url)) {
            void ("__TAURI_INTERNALS__" in window ? openUrl(url)
              : Promise.resolve(window.open(url, "_blank", "noopener,noreferrer")))
              .catch((cause) => setError("Could not open review link: " + String(cause)));
          }
          return;
        }
        if (event.data?.type === "taskboard-review-asset-request") {
          const ref = event.data.ref;
          const base = htmlAssetFolderRef.current;
          const file = typeof ref === "string" ? ref.split(/[?#]/)[0] : "";
          // The native review_asset command remains the authority for allowed paths.
          if (!base || !file || file.length > 512 || /^(?:[a-z][a-z0-9+.-]*:|\/)/i.test(file) ||
              !/\.(?:png|jpe?g|webp|gif|svg|ico)$/i.test(file)) return;
          const frame = htmlFrameRef.current.contentWindow;
          const board = htmlAssetBoardRef.current;
          void reviewAsset(base + "/" + file, board).then((dataUrl) => {
            if (htmlFrameRef.current?.contentWindow === frame && htmlAssetFolderRef.current === base)
              frame?.postMessage({ type: "taskboard-review-asset-response", ref, dataUrl }, "*");
          }).catch(() => { /* A missing media file stays visibly broken in the preview. */ });
          return;
        }
      }
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
  const activeVideo = playable.some(artifact => artifact.kind === "video");
  useEffect(() => {
    if (!activeVideo) return;
    let cancelled = false;
    setMediaError("");
    void import("@tauri-apps/api/core").then(({ invoke }) =>
      invoke<string>("ensure_review_media_server"))
      .then(base => { if (!cancelled) setMediaServer(base); })
      .catch(cause => { if (!cancelled) setMediaError("Media streaming unavailable: " + String(cause)); });
    return () => { cancelled = true; };
  }, [selectedKey, activeVideo]);
  const boardName = reviewTask?.path.match(/[\\/]([^\\/]+)[\\/]Tasks[\\/]?$/i)?.[1] || "";
  const remoteReviewUrl = boardName && reviewTask
    ? "https://hub.rancorous.online/gateway/tools/taskboard-review-media/review/" +
      encodeURIComponent(boardName) + "/" + encodeURIComponent(reviewTask.task.id) : "";
  const videoSrc = (artifact: ReviewArtifact) =>
    mediaServer && boardName && reviewTask && round
      ? mediaServer + "/media/" + [boardName, reviewTask.task.id, String(round.number), artifact.id]
          .map(encodeURIComponent).join("/")
      : "";
  const activeArtifact = playable[mediaIndex] ?? playable[0];
  const htmlFile = activeArtifact?.kind === "html" ? htmlIndexPath(activeArtifact.path) : "";
  htmlAssetFolderRef.current = htmlFile.slice(0, htmlFile.lastIndexOf("/"));
  htmlAssetBoardRef.current = reviewTask?.path ?? "";
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
  const advance = (excludingKey?: string, skipCurrent = false, backgroundSave = false) => {
    if (skipCurrent && selectedKey) skippedThisPass.current.add(selectedKey);
    const index = items.findIndex((item) => keyOf(item) === selectedKey);
    const ordered = index < 0 ? items : [...items.slice(index + 1), ...items.slice(0, index)];
    const next = ordered.find((item) => keyOf(item) !== excludingKey &&
      !pendingResponses.current.has(keyOf(item)) &&
      !(Number(locallyHidden.current.get(keyOf(item))) > Date.now()) &&
      !skippedThisPass.current.has(keyOf(item)) &&
      !(Number(snoozedLocally[keyOf(item)]) > Date.now()));
    if (next) setSelectedKey(keyOf(next));
    else if (backgroundSave || showResolved) setSelectedKey("");
    else onClose();
  };

  const hideLocally = (item: QueueItem, milliseconds: number, persist = false) => {
    const until = Date.now() + milliseconds;
    const key = keyOf(item);
    locallyHidden.current.set(key, until);
    setSnoozedLocally((current) => {
      const updated = { ...current, [key]: until };
      if (persist) {
        try { window.localStorage.setItem(REVIEW_SNOOZES_KEY, JSON.stringify(updated)); }
        catch { /* An in-memory snooze still works if browser storage is unavailable. */ }
      }
      return updated;
    });
  };

  const captureDraft = (): InboxDraft => ({
    note, pins: [...pins], choice, ranking: [...ranking],
    userNote, userAnswer, userCriteria, keepForUser, images: [...inboxImages],
  });

  // Leave the current item immediately; the revision-checked native write runs
  // independently. Hide only this item during its write, never the entire Inbox.
  const submitInBackground = (item: QueueItem, work: () => Promise<void>) => {
    const key = keyOf(item);
    if (saving || pendingResponses.current.has(key)) return;
    pendingResponses.current.add(key);
    failedDrafts.current.set(key, captureDraft());
    setPendingResponseCount(pendingResponses.current.size);
    setItems(previous => previous.filter(entry => keyOf(entry) !== key));
    setBackgroundError("");
    setError("");
    advance(key, false, true);
    void work().then(() => {
      failedDrafts.current.delete(key);
      hideLocally(item, 60 * 60 * 1000);
    }).catch((cause) => {
      const message = `Could not confirm ${item.task.id}: ${String(cause)}. Item returned to Inbox; check before retrying.`;
      setBackgroundError(message);
      onUserTaskUpdated?.(message);
      setSelectedKey((current) => current || key);
    }).finally(() => {
      pendingResponses.current.delete(key);
      setPendingResponseCount(pendingResponses.current.size);
      void refresh();
    });
  };

  const submitUser = (markDone = false, reopen = false) => {
    if (!reviewTask || reviewTask.historical || isVisualReview || saving) return;
    const item = reviewTask;
    const old = item.task;
    const aiQuestion = old.kind === "task" && old.attention.includes("question") &&
      !!old.question && !old.question.answer;
    const isApproval = aiQuestion && (old.question?.request_type === "verification" || old.column === "done");
    // Done on a decision means keep the current implementation, not a change request.
    const answer = reopen ? "Needs changes" : aiQuestion && markDone
      ? (isApproval ? "Done" : "Keep current implementation")
      : userAnswer.trim() ? userAnswer : userNote;
    if (aiQuestion && !answer.trim()) {
      setError("Choose an option or enter an answer before submitting.");
      return;
    }
    const isWaitingAnswer = aiQuestion && !isApproval && !markDone;
    const needsChanges = reopen || (isApproval && answer.toLowerCase() === "needs changes");
    const attention = old.attention.filter(flag => flag !== "question" && flag !== "needs-user" && flag !== "blocked");
    if ((isWaitingAnswer || needsChanges) && !attention.includes("user-note")) attention.push("user-note");
    const newNotes = userNote;
    const criteria = userCriteria;
    const retainForUser = keepForUser;
    submitInBackground(item, async () => {
      let doneColumn: string | undefined;
      if (markDone && !isWaitingAnswer) {
        const board = await loadBoard(item.path);
        doneColumn = board.config.columns.find(col => col.id === "done" && col.terminal)?.id
          ?? board.config.columns.find(col => col.terminal)?.id;
        if (!doneColumn) throw new Error("This board has no Done column.");
      }
      const imageLinks = await attachInboxImages(old.id, item.path);
      const updated: Task = {
        ...old,
        ...(isWaitingAnswer || needsChanges ? { column: "ai-ready", claimed_by: null, blocked_reason: null } :
          doneColumn ? { column: doneColumn, claimed_by: null } : {}),
        user_notes: [old.user_notes, newNotes, ...imageLinks].filter(Boolean).join("\n\n"),
        acceptance_criteria: criteria.split(/\r?\n/).filter(Boolean),
        kind: needsChanges ? "task" : aiQuestion ? old.kind : markDone ? "task" : retainForUser
          ? (old.kind === "epic" ? "epic" : "for_user") : "task",
        attention,
        question: old.question ? { ...old.question, answer: answer || null } : null,
      };
      const saved = await updateTask(updated, item.path);
      let notice: string | undefined;
      if (saved.completed && saved.category === "Implementation Review" && saved.related_to.length === 1) {
        try {
          const board = await loadBoard(item.path);
          const parent = board.tasks.find((task) => task.id === saved.related_to[0]);
          const entry = parent?.notes.split("\n").find((line) => line.includes(`[review:${saved.id}]`));
          if (parent && entry) notice = `Review ${saved.id} — ${parent.id}: ` +
            (entry.includes("approved_parent_done") ? "approved, original moved to Done"
              : entry.includes("needs_changes") ? "needs changes, original returned to work"
              : entry.includes("partial_or_blocked") ? "partial approval or other work remains"
              : "review closed; original disposition unresolved");
        } catch { /* The answer is saved even if its optional parent notice cannot be loaded. */ }
      }
      setLastAnswered({ path: item.path, key: keyOf(item), before: old, savedRevision: saved.revision });
      onUserTaskUpdated?.(notice);
    });
  };
  const undoLastAnswer = async () => {
    if (!lastAnswered || saving) return;
    setSaving(true);
    setError("");
    try {
      const board = await loadBoard(lastAnswered.path);
      const current = board.tasks.find(task => task.id === lastAnswered.before.id);
      if (!current || current.revision !== lastAnswered.savedRevision)
        throw new Error("This task has changed since your response. Undo was stopped to protect the agent's newer work.");
      await updateTask({ ...lastAnswered.before, revision: current.revision }, lastAnswered.path);
      const key = lastAnswered.key;
      locallyHidden.current.delete(key);
      setSnoozedLocally(current => {
        const next = { ...current };
        delete next[key];
        return next;
      });
      skippedThisPass.current.delete(key);
      setLastAnswered(null);
      onUserTaskUpdated?.();
      await refresh();
      setSelectedKey(key);
    } catch (cause) {
      setError(`Could not undo your last answer: ${String(cause)}`);
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
      setLoadedManifest({ key: selectedKey, value: reopened });
    } catch (cause) {
      setError(`Unable to reconsider review: ${String(cause)}`);
      await refresh();
    } finally { setSaving(false); }
  };

  const submit = async (verdict: string) => {
    // A rejected/more-variants round stays in an open conversation, but its
    // verdict is immutable. Only a NEW requested round can accept another send.
    if (!reviewTask || !manifest || !round || saving || manifest.status !== "open" || round.result) return;
    if (!reviewOutcomes(round.mode).some((option) => option.value === verdict)) {
      setError("Select a valid response outcome before submitting.");
      return;
    }
    if (verdict === "picked" && !choice ||
        verdict === "ranked" && ranking.length !== playable.length) {
      setError("Choose the required variant or finish ranking before submitting.");
      return;
    }
    const item = reviewTask;
    const chosen = verdict === "picked" ? [choice] : verdict === "approved" ? [playable[0]?.id ?? "A"] : [];
    const rankingSnapshot = [...ranking];
    const pinSnapshot = pins.map(pin => ({ ...pin }));
    const message = note;
    const taskRevision = item.task.revision;
    const manifestRevision = manifest.revision;
    submitInBackground(item, async () => {
      await reviewResolve({
        task_id: item.task.id, verdict,
        choices: chosen,
        ranking: verdict === "ranked" ? rankingSnapshot : [],
        pins: pinSnapshot,
        note: [message, ...await attachInboxImages(item.task.id, item.path)].filter(Boolean).join("\n\n"),
        expected_task_revision: taskRevision,
        expected_manifest_revision: manifestRevision,
      }, item.path);
      onUserTaskUpdated?.();
    });
  };
  const closeReviewTask = () => {
    if (!reviewTask || !manifest || !round || saving || manifest.status !== "open" || round.result) return;
    const item = reviewTask;
    const taskRevision = item.task.revision;
    const manifestRevision = manifest.revision;
    const message = note.trim();
    submitInBackground(item, async () => {
      const attachments = await attachInboxImages(item.task.id, item.path);
      await reviewClose({
        task_id: item.task.id,
        note: [message, ...attachments].filter(Boolean).join("\n\n"),
        expected_task_revision: taskRevision,
        expected_manifest_revision: manifestRevision,
      }, item.path);
      await taskSyncQueue(item.task.id, item.path).catch(() => null);
      onUserTaskUpdated?.();
    });
  };
  const submitNoteOnly = async () => {
    if (!reviewTask || !isVisualReview || !(note.trim() || inboxImages.length) || saving || resolved) return;
    setSaving(true);
    setError("");
    setNoteFeedback("");
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
      setNoteFeedback("Note saved to the task. The review is still open; select an outcome or Close task.");
      onUserTaskUpdated?.();
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
  const closedWithoutDecision = manifest?.status === "closed";
  const resolved = manifest?.status !== "open" || !!round?.result;
  // Markup rounds still open in browse mode so scrolling/dragging an embedded
  // preview cannot accidentally create pins. Pin (P) explicitly enters annotation mode.
  const pinning = pinMode && !resolved;
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
    if (!pinning || !artifact) return;
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
        // The inspector owns its nested stack. Text Inbox closes its menu first,
        // then returns to the board, matching the visual Inbox exit behavior.
        if (isVisualReview) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        if (moreOpen) setMoreOpen(false);
        else onClose();
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

  // Only explicit Inbox selections create a navigation step. When the history
  // is exhausted, Back/Escape exits to the board; it must never cycle siblings.
  const navigationHistory = useRef<string[]>([]);
  const selectInboxItem = (key: string) => {
    if (selectedKey && selectedKey !== key) navigationHistory.current.push(selectedKey);
    setSelectedKey(key);
  };
  const returnToPrevious = () => {
    while (navigationHistory.current.length) {
      const previous = navigationHistory.current.pop()!;
      if (items.some(item => keyOf(item) === previous)) {
        setSelectedKey(previous);
        return;
      }
    }
    onClose();
  };
  const hasPreviousItem = navigationHistory.current.some(key => items.some(item => keyOf(item) === key));

  const pinsFor = (artifact: ReviewArtifact) => pins.filter((pin) => pin.artifact === artifact.id);
  const image = (artifact: ReviewArtifact, className = "") => {
    const src = activeSrc(artifact);
    if (artifact.kind === "video") {
      const stream = videoSrc(artifact);
      if (!stream) return <div className="review-media-missing" role="alert">
        {mediaError || "Preparing video streaming…"}
      </div>;
      return <div className="review-video-playback">
        <video key={artifact.path} ref={videoRef} controls playsInline preload="metadata"
          src={stream} poster={artifact.poster ? assets[artifact.poster] : undefined}
          className={className} onError={() => setVideoError(
            "This video cannot be loaded or decoded. Check the source clip, MP4/H.264 compatibility and media server."
          )} onLoadedMetadata={() => setVideoError("")} />
        {videoError && <p className="review-media-missing" role="alert">{videoError}</p>}
      </div>;
    }
    if (artifact.kind === "html" || artifact.kind === "url") {
      const posterSrc = artifact.poster ? assets[artifact.poster] : "";
      const html = htmlDocuments[artifact.path];
      const iframe = artifact.kind === "html"
        ? (html ? <iframe ref={htmlFrameRef} title={artifact.label} srcDoc={html}
            style={{ width: iframeWidth || "100%", maxWidth: "none" }}
            sandbox="allow-scripts allow-forms" referrerPolicy="no-referrer" />
          : null)
        : (showPoster ? null : <iframe ref={isAnimationLabEmbed(artifact) ? embedFrameRef : undefined}
            title={artifact.label} src={reviewEmbedUrl(artifact.path)}
            sandbox={/^https?:\/\/(?:localhost|127\.0\.0\.1):(?:3333|8768)\//i.test(artifact.path)
              ? "allow-scripts allow-forms allow-same-origin" : "allow-scripts allow-forms"}
            referrerPolicy="no-referrer"
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
          {pinning && <div className="review-web-pin-surface" aria-label="Place review pin" />}
        </div>
      );
    }
    return src ? <img src={src} alt={artifact.label} className={className} />
      : <div className="review-media-missing" role="alert">Media unavailable{artifact.storage === "local" ? " (stored on the PC)" : ""}</div>;
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
      onSelectTask={selectInboxItem}
      onBack={returnToPrevious}
      onExitInbox={onClose}
      hasPreviousItem={hasPreviousItem}
      queueThumbnails={queueThumbnails}
      queueModes={queueModes}
      queueCounts={queueCounts}
      onOpenTask={reviewTask.path === project.path && onOpenUserTask ? () => onOpenUserTask(reviewTask.task) : undefined}
      onOpenSession={() => void openReviewTaskSession(reviewTask)}
      sessionOpening={openingTaskSession}
      onTogglePriority={() => void toggleInboxPriority(reviewTask)}
      priorityBusy={saving}
      allBoards={allBoards}
      showResolved={showResolved}
      onAllBoards={setAllBoards}
      onShowResolved={setShowResolved}
      playable={playable}
      activeIndex={Math.min(mediaIndex, Math.max(0, playable.length - 1))}
      onFocusArtifact={setMediaIndex}
      onChooseArtifact={choose}
      onClearRanking={() => setRanking([])}
      choice={choice}
      ranking={ranking}
      pins={pins}
      setPins={setPins}
      pinning={pinning}
      onTogglePin={() => setPinMode((current) => !current)}
      addPin={addPin}
      renderArtifact={image}
      remoteReviewUrl={remoteReviewUrl}
      posterOf={(artifact) => artifact.kind === "image"
        ? activeSrc(artifact) || "" : artifact.poster ? assets[artifact.poster] || "" : ""}
      iframeWidth={iframeWidth}
      htmlFrameRef={htmlFrameRef}
      onIframeWidth={setIframeWidth}
      note={note}
      setNote={(value) => { setNote(value); setNoteFeedback(""); }}
      noteFeedback={noteFeedback}
      saving={saving}
      outcome={responseOutcome}
      onOutcome={setResponseOutcome}
      onSkip={() => advance(undefined, true)}
      onSnooze={snoozeSelected}
      resolved={resolved}
      closedWithoutDecision={closedWithoutDecision}
      awaitingNextRound={awaitingNextRound}
      error={backgroundError || error}
      primaryVerdict={primaryVerdict}
      primaryReady={primaryReady}
      submit={submit}
      onNoteOnly={submitNoteOnly}
      onCloseTask={closeReviewTask}
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
      <InboxQueueSidebar
        items={items}
        selectedKey={selectedKey}
        onSelect={selectInboxItem}
        allBoards={allBoards}
        showResolved={showResolved}
        onAllBoards={setAllBoards}
        onShowResolved={setShowResolved}
        thumbnails={queueThumbnails}
        modes={queueModes}
        counts={queueCounts}
      />
      <section className="review-main">
        <div className="review-scroll">
        {(backgroundError || error) && <div className="error-banner" role="alert">{backgroundError || error}</div>}
        {!reviewTask ? (
          <div className="review-placeholder">
            <p>{pendingResponseCount ? "No more items to review — saving your response in the background." : "All caught up."}</p>
            {lastAnswered && <button type="button" className="button secondary"
              disabled={saving} onClick={() => void undoLastAnswer()}>
              ↶ Undo last answer · {lastAnswered.before.id}
            </button>}
          </div>
        ) : !isVisualReview ? (
          <div className="review-human-stage">
            <header className="review-heading">
              <div>
                <small className="inbox-task-heading-kicker">{reviewTask.project} · {reviewTask.task.id} · TASK</small>
                <h2 className="inbox-task-heading-title" title={reviewTask.task.title}>{reviewTask.task.title}</h2>
              </div>
              <div className="flex items-center gap-2">
                {!reviewTask.historical && <button type="button" className={"inbox-priority-button" +
                  (reviewTask.task.attention.includes("priority") ? " is-priority" : "")}
                  aria-pressed={reviewTask.task.attention.includes("priority")}
                  aria-label={reviewTask.task.attention.includes("priority")
                    ? "Remove Priority from " + reviewTask.task.id
                    : "Mark " + reviewTask.task.id + " as Priority"}
                  title={reviewTask.task.attention.includes("priority")
                    ? "Priority enabled · click to remove" : "Mark Priority so agents handle this task sooner"}
                  disabled={saving} onClick={() => void toggleInboxPriority(reviewTask)}>
                  <span aria-hidden="true">!</span> {reviewTask.task.attention.includes("priority") ? "Priority" : "Prioritize"}
                </button>}
                {(reviewTask.task.claimed_by || reviewTask.task.live?.with_me) && (
                  <button type="button" className="button secondary"
                    aria-label={`Open session for ${reviewTask.task.id}`}
                    disabled={openingTaskSession}
                    onClick={() => void openReviewTaskSession(reviewTask)}>
                    {openingTaskSession ? "Opening…" : "Open Session"}
                  </button>
                )}
                <span className={`inbox-task-column-chip column-${reviewTask.task.column}`}>{reviewTask.task.column === "blocked" ? "Waiting" : reviewTask.task.column}</span>
                {!reviewTask.historical && <div className="inbox-more-wrap">
                  <button className="button secondary" type="button" aria-label="More Inbox actions"
                    aria-expanded={moreOpen} onClick={() => setMoreOpen(value => !value)}>⋯</button>
                  {moreOpen && <div className="inbox-more-menu">
                    <button type="button" disabled={saving} onClick={() => void moveSelectedToDraft()}>Move to Draft</button>
                    <button type="button" disabled={saving} onClick={() => void archiveSelected()}>Archive</button>
                  </div>}
                </div>}
              </div>
            </header>
            <div className="review-workspace review-human-workspace">
              <div className="review-stage-column">
                <div className="review-user-fields">
                  {reviewTask.historical ? <div className="review-user-question">
                    <strong>Past decision · {reviewTask.task.question?.request_type === "verification" ? "Verification" : "Response"}</strong>
                    {reviewTask.task.question?.prompt && <InboxRichText task={reviewTask.task}
                      text={reviewTask.task.question.prompt} onError={setError} className="inbox-question-markdown" />}
                    <p><strong>Answer: </strong>{reviewTask.task.question?.answer}</p>
                  </div> : <>
                  {reviewTask.task.question?.prompt && (
                    <div className="review-user-question">
                      <InboxRichText task={reviewTask.task} text={reviewTask.task.question.prompt}
                        onError={setError} className="inbox-question-markdown" />
                      {!!reviewTask.task.question.options?.length &&
                        // For final verification, Done and Needs changes in
                        // the question are the same actions as the footer.
                        // Never make the user select one and then click it again.
                        !(reviewTask.task.question.options.length === 2 &&
                          (reviewTask.task.question.request_type === "verification" ||
                            reviewTask.task.column === "done") &&
                          reviewTask.task.question.options.some(option =>
                            /^(done|approve|approved|accept|accepted)$/i.test(option.trim())) &&
                          reviewTask.task.question.options.some(option =>
                            /^(needs changes|request changes|reopen|reject|rejected)$/i.test(option.trim()))) &&
                        <div className="review-user-choice"
                        role="group" aria-label="Choose your answer">
                        {reviewTask.task.question.options.map((value) =>
                          <button type="button" key={value}
                            disabled={saving} aria-pressed={userAnswer === value}
                            className={userAnswer === value ? "button primary" : "button secondary"}
                            onClick={() => { setUserAnswer(value); setKeepForUser(false); }}>
                            <span className="review-choice-marker" aria-hidden="true">{userAnswer === value ? "✓" : ""}</span>
                            <span>{value}</span>
                          </button>)}
                      </div>}
                    </div>
                  )}
                  <label>{reviewTask.task.question?.options.length ? "Add a note (optional)" : "Your answer"}
                    <textarea onKeyDown={(event) => {
                      if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey) {
                        event.preventDefault();
                        void submitUser();
                      }
                    }} value={userNote} onChange={(event) => {
                      setUserNote(event.currentTarget.value);
                      setKeepForUser(false);
                    }}
                      rows={4} placeholder={reviewTask.task.question?.options.length
                        ? "Optional explanation or extra instructions…" : "Write your answer…"} />
                  </label>
                  <button type="button" className="button secondary inbox-attach-button"
                    onClick={() => void browseInboxFiles()}>+ Images / local files…</button>
                  {inboxImages.length > 0 && inboxImagePreview}

                  {(reviewTask.task.kind === "for_user" || reviewTask.task.attention.includes("needs-user")) &&
                    <label className="review-user-checkbox">
                      <input type="checkbox" checked={keepForUser}
                        onChange={(event) => setKeepForUser(event.target.checked)} />
                      Keep marked For user
                    </label>}
                  </>}
                </div>
              </div>
              <div className="review-feedback-rail review-human-details">
                <strong>{reviewTask.historical ? "Decision history" : "After you respond"}</strong>
                <p>{reviewTask.historical ? "This response was already submitted. You can inspect the original task without changing its verdict." :
                  reviewTask.task.question?.request_type === "verification" || reviewTask.task.column === "done"
                    ? "Done accepts the finished work. Needs changes sends it back for implementation."
                    : "Submit sends your answer to AI Ready. Done · Keep current closes the task without changes."}</p>
                <details className="review-task-context" open>
                  <summary>Task context & links</summary>
                  <InboxRichText task={reviewTask.task}
                    text={parseDescriptionTimeline(reviewTask.task.description).initial || "No additional context."}
                    onError={setError} />
                  {parseDescriptionTimeline(reviewTask.task.description).updates.slice(-3).map((entry, index) =>
                    <div key={index} className="inbox-context-update">
                      <strong>{entry.kind === "user-note" ? "Note to AI" : "Task update"}</strong>
                      <InboxRichText task={reviewTask.task} text={entry.text} onError={setError} />
                    </div>)}
                  {!!reviewTask.task.user_notes.trim() && <>
                    <strong>User notes</strong>
                    <InboxRichText task={reviewTask.task} text={reviewTask.task.user_notes} onError={setError} />
                  </>}
                  {!!reviewTask.task.ai_notes.trim() && <>
                    <strong>AI notes</strong>
                    <InboxRichText task={reviewTask.task} text={reviewTask.task.ai_notes} onError={setError} />
                  </>}
                </details>
                <details className="review-task-context">
                  <summary>Requirements {!reviewTask.historical && userCriteria.trim() && "· editable"}</summary>
                  <textarea aria-label="Acceptance criteria" value={userCriteria} readOnly={reviewTask.historical} onChange={event => {
                    setUserCriteria(event.currentTarget.value);
                    setKeepForUser(false);
                  }} rows={5} placeholder="One criterion per line" />
                </details>
                <div className="review-human-task-links">
                  <small>{reviewTask.task.category || "Uncategorized"}</small>
                  {reviewTask.path === project.path && (
                    <button className="button secondary" type="button"
                      onClick={() => onOpenUserTask?.(reviewTask.task)}>Open full task ↗</button>
                  )}
                </div>
              </div>
            </div>
            {reviewTask.historical ?
              <footer className="inbox-response-footer"><span className="inbox-response-summary">Past decision · Read-only</span></footer> :
            <InboxResponseFooter
              mode="reply"
              outcome="respond"
              onOutcome={() => {}}
              onSkip={() => advance(undefined, true)}
              onSnooze={snoozeSelected}
              onSubmit={() => void submitUser()}
              onComplete={() => void submitUser(true)}
              completeLabel={reviewTask.task.attention.includes("question") &&
                reviewTask.task.column !== "done" && reviewTask.task.question?.request_type !== "verification"
                ? "Done · Keep current" : "Done"}
              onReopen={reviewTask.task.question?.request_type === "verification" || reviewTask.task.column === "done" ? () => void submitUser(false, true) : undefined}
              onUndoLast={lastAnswered ? () => void undoLastAnswer() : undefined}
              notePresent={!!userNote.trim() || !!userAnswer.trim() || inboxImages.length > 0}
              busy={saving}
              summary={reviewTask.task.kind === "for_user"
                ? "Submit instructions or finish the human task"
                : reviewTask.task.question?.request_type === "verification" || reviewTask.task.column === "done"
                  ? "Done accepts; Needs changes reopens"
                  : "Submit → AI Ready · Done keeps current implementation"}
            />}
          </div>
        ) : !round ? (
          <div className="review-placeholder">Loading feedback details…</div>
        ) : (
          <>
            <header className="review-heading">
              <div><small>{reviewTask.project} · {reviewTask.task.id} · Round {round.number}</small>
                <h2>{reviewTask.task.title}</h2><p>{round.judge}</p></div>
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
                    onClick={(event) => { if (pinning) addPin(event, activeArtifact); }}>
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
