import { useEffect, useLayoutEffect, useRef, useState, type ClipboardEvent, type DragEvent, type MouseEvent, type ReactNode, type RefObject, type CSSProperties, type PointerEvent as ReactPointerEvent, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import "./ReviewImageFullscreen.css";
import "./ReviewInstrumentHeader.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ReviewArtifact, ReviewPin, ReviewRound } from "./reviewApi";
import type { Task } from "./types";
import { InboxResponseFooter, reviewOutcomes } from "./InboxResponseFooter";
import { InboxRichText, openFileInFomni } from "./InboxRichText";
import { highlightInFomni } from "./api";
import { InboxQueueSidebar } from "./InboxQueueSidebar";
import { DashboardEvidenceItem } from "./DashboardEvidenceItem";
import { useLinkCopyMenu } from "./LinkCopyMenu";

type Item = { path: string; project: string; task: Task };

// Direct grid children are required for the Dashboard's full-width media stage.
// WebView2 does not reliably lay out nested grid items through display:contents.
function ReviewCenter({ dashboardMode, focus, children }: { dashboardMode: boolean; focus: boolean; children: ReactNode }) {
  return dashboardMode && !focus ? <>{children}</> : <main className="review-visual-center">{children}</main>;
}

function ReviewHeaderIcon({ name }: { name: "previous" | "close" | "focus" | "feedback" | "more" }) {
  const paths = {
    previous: <><path d="m14.5 5.5-6.5 6.5 6.5 6.5" /><path d="M8.5 12H21" /></>,
    close: <path d="M6 6l12 12M18 6 6 18" />,
    focus: <path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5" />,
    feedback: <path d="M4 5h16M4 12h10M4 19h14M18 12h2" />,
    more: <><circle cx="5" cy="12" r="1.1" fill="currentColor" /><circle cx="12" cy="12" r="1.1" fill="currentColor" /><circle cx="19" cy="12" r="1.1" fill="currentColor" /></>,
  };
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor"
    strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round">{paths[name]}</svg>;
}

const reviewModeDetails: Record<ReviewRound["mode"], { label: string; explanation: string }> = {
  approve: { label: "Approval review", explanation: "Approval review: you are being asked whether to approve or reject this work. Nothing is approved yet." },
  pick: { label: "Pick review", explanation: "Pick review: choose one of the proposed options; no selection has been accepted yet." },
  rank: { label: "Ranking review", explanation: "Ranking review: order all proposed options before submitting a decision." },
  markup: { label: "Markup review", explanation: "Markup review: annotate the proposed work before submitting feedback." },
};
type Props = {
  suspended?: boolean;
  review: Item;
  round: ReviewRound;
  history: ReviewRound[];
  items: Item[];
  selectedKey: string;
  onSelectTask: (key: string) => void;
  onBack: () => void;
  onExitInbox: () => void;
  hasPreviousItem: boolean;
  queueThumbnails: Record<string, string>;
  queueModes: Record<string, string>;
  queueCounts: Record<string, number>;
  onOpenTask?: () => void;
  onOpenSession: () => void;
  sessionOpening: boolean;
  onTogglePriority: () => void;
  priorityBusy: boolean;
  allBoards: boolean;
  showResolved: boolean;
  onAllBoards: (value: boolean) => void;
  onShowResolved: (value: boolean) => void;
  playable: ReviewArtifact[];
  activeIndex: number;
  onFocusArtifact: (index: number) => void;
  onChooseArtifact: (id: string) => void;
  onClearRanking: () => void;
  choice: string;
  ranking: string[];
  pins: ReviewPin[];
  setPins: (pins: ReviewPin[]) => void;
  pinning: boolean;
  onTogglePin: () => void;
  addPin: (event: MouseEvent<HTMLDivElement>, artifact?: ReviewArtifact) => void;
  renderArtifact: (artifact: ReviewArtifact) => ReactNode;
  remoteReviewUrl?: string;
  posterOf: (artifact: ReviewArtifact) => string;
  iframeWidth: number;
  htmlFrameRef: RefObject<HTMLIFrameElement | null>;
  onIframeWidth: (value: number) => void;
  note: string;
  setNote: (note: string) => void;
  saving: boolean;
  outcome: string;
  onOutcome: (value: string) => void;
  onSkip: () => void;
  onSnooze: (milliseconds: number) => void;
  resolved: boolean;
  closedWithoutDecision: boolean;
  awaitingNextRound: boolean;
  error: string;
  primaryVerdict: string;
  primaryReady: boolean;
  submit: (verdict: string) => Promise<void>;
  onNoteOnly: () => Promise<void>;
  onCloseTask: () => void;
  noteFeedback: string;
  onBrowseFiles: () => void;
  imagePreviews: ReactNode;
  imageCount: number;
  onPaste: (event: ClipboardEvent<HTMLElement>) => void;
  onDrop: (event: DragEvent<HTMLElement>) => void;
  onMoveToDraft: () => Promise<void>;
  onArchive: () => Promise<void>;
  onReconsider: () => Promise<void>;
};

const isWeb = (artifact: ReviewArtifact) => artifact.kind === "html" || artifact.kind === "url";
const isDashboard = (artifact: ReviewArtifact | undefined, boardPath: string) =>
  artifact?.kind === "url" && /[\\/]Wartribes[\\/]Tasks[\\/]?$/i.test(boardPath) &&
  /\/wartribes-development-dashboard(?:[/?#]|$)/i.test(artifact.path);

/** First-class task review workspace. This is independent of the artifact count,
 * mode (pick/approve/rank/markup), and project: a single focused media stage,
 * filmstrip, optional comparison and a collapsible feedback rail.
 */
export function VisualReviewInspector(props: Props) {
  const {
    suspended = false, review, round, history, items, selectedKey, onSelectTask, onBack, onExitInbox, queueThumbnails, queueModes, queueCounts, onOpenTask, onOpenSession, sessionOpening, onTogglePriority, priorityBusy, allBoards, showResolved,
    onAllBoards, onShowResolved, playable, activeIndex, onFocusArtifact, onChooseArtifact, onClearRanking,
    choice, ranking, pins, setPins, pinning, onTogglePin, addPin, renderArtifact, remoteReviewUrl,
    posterOf, iframeWidth, htmlFrameRef, onIframeWidth, note, setNote, saving,
    outcome, onOutcome, onSkip, onSnooze,
    resolved, closedWithoutDecision, awaitingNextRound, error, primaryVerdict, primaryReady, submit, onNoteOnly, onCloseTask, noteFeedback,
    onBrowseFiles, imagePreviews, imageCount, onPaste, onDrop, onMoveToDraft, onArchive, onReconsider,
  } = props;
  const [feedbackExpanded, setFeedbackExpanded] = useState(true);
  const [focus, setFocus] = useState(false);
  const [compare, setCompare] = useState<"single" | "side" | "overlay">("single");
  const [opacity, setOpacity] = useState(50);
  const [zoom, setZoom] = useState(1);
  const [openError, setOpenError] = useState("");
  const { showCopyMenu, showCopyMenuAt, copyMenu, copyMenuOpen, dismissCopyMenu } = useLinkCopyMenu(setOpenError);
  const [moreOpen, setMoreOpen] = useState(false);
  const [galleryCount, setGalleryCount] = useState(0);
  // The enlarged Dashboard image is a nested viewer, not a separate Inbox item.
  // Keep its selection here so the Inbox-level Escape handler can close it first.
  const [gallerySelectedIndex, setGallerySelectedIndex] = useState<number | null>(null);
  const [imageFullscreen, setImageFullscreen] = useState(false);
  const fullscreenOwned = useRef(false);
  const fullscreenCloseRef = useRef<HTMLButtonElement>(null);
  const videoResumeTime = useRef(0);
  const stripRef = useRef<HTMLDivElement>(null);
  const active = playable[Math.min(activeIndex, playable.length - 1)];
  const dashboardMode = isDashboard(active, review.path);
  const defaultUpperHeight = 176;
  const [upperHeight, setUpperHeight] = useState(() => {
    try {
      const remembered = Number(localStorage.getItem("taskboard.dashboardReview.controlsHeight"));
      return Number.isFinite(remembered) && remembered >= 145 ? remembered : defaultUpperHeight;
    } catch { return defaultUpperHeight; }
  });
  const bodyRef = useRef<HTMLDivElement>(null);

  const dragOrigin = useRef<{ pointer: number; y: number; height: number } | null>(null);
  // Keep the evidence gallery visible, but allow the controls to use most of
  // the window when the user drags the divider downward. clientHeight is
  // already in CSS pixels, even at high DPI; dividing by devicePixelRatio
  // again made the divider effectively immovable on scaled displays.
  const maxUpperHeight = () => {
    const height = bodyRef.current?.clientHeight ?? window.innerHeight;
    return Math.max(145, Math.floor(height - Math.max(170, height * 0.2)));
  };
  const clampUpperHeight = (height: number) =>
    Math.max(145, Math.min(Math.round(height), maxUpperHeight()));
  useLayoutEffect(() => {
    if (!dashboardMode || focus || !bodyRef.current) return;
    const body = bodyRef.current;
    const fitControls = () => setUpperHeight((current) => {
      const fitted = Math.min(current, maxUpperHeight());
      if (fitted !== current) {
        try { localStorage.setItem("taskboard.dashboardReview.controlsHeight", String(fitted)); }
        catch { /* storage optional */ }
      }
      return fitted;
    });
    fitControls();
    const observer = new ResizeObserver(fitControls);
    observer.observe(body);
    return () => observer.disconnect();
  }, [dashboardMode, focus]);
  const persistUpperHeight = (height: number) => {
    const bounded = clampUpperHeight(height);
    setUpperHeight(bounded);
    try { localStorage.setItem("taskboard.dashboardReview.controlsHeight", String(bounded)); } catch { /* storage optional */ }
  };
  const resizePointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    dragOrigin.current = { pointer: event.pointerId, y: event.clientY, height: upperHeight };
  };
  const resizePointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    const origin = dragOrigin.current;
    if (origin?.pointer !== event.pointerId) return;
    setUpperHeight(clampUpperHeight(origin.height + event.clientY - origin.y));
  };
  const resizePointerEnd = (event: ReactPointerEvent<HTMLDivElement>) => {
    const origin = dragOrigin.current;
    if (origin?.pointer !== event.pointerId) return;
    const finalHeight = clampUpperHeight(origin.height + event.clientY - origin.y);
    dragOrigin.current = null;
    if (event.currentTarget.hasPointerCapture(event.pointerId))
      event.currentTarget.releasePointerCapture(event.pointerId);
    persistUpperHeight(finalHeight);
  };
  const resizeKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const change = event.key === "ArrowUp" ? -24 : event.key === "ArrowDown" ? 24 : null;
    if (change === null && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    event.stopPropagation();
    persistUpperHeight(event.key === "Home" ? 145 :
      event.key === "End" ? maxUpperHeight() : upperHeight + change!);
  };
  const canCompare = active?.kind === "image" && round.previous?.kind === "image";
  const compareMode = canCompare ? compare : "single";
  const pinCount = active ? pins.filter((pin) => pin.artifact === active.id).length : 0;
  const labelOf = (id: string) => playable.find((value) => value.id === id)?.label || id;
  const outcomeOptions = reviewOutcomes(round.mode);
  const modeDetails = reviewModeDetails[round.mode];
  const previousIndex = items.findIndex((item) => `${item.path}|${item.task.id}` === selectedKey) - 1;
  const previousItem = previousIndex >= 0 ? items[previousIndex] : undefined;
  const goPreviousItem = () => {
    if (previousItem) onSelectTask(`${previousItem.path}|${previousItem.task.id}`);
  };
  // The primary action must not look actionable before a pick/top-K ranking,
  // regardless of which verdict the user last hovered or selected.
  const primaryIncomplete = !primaryReady;

  const closeImageFullscreen = () => {
    if (active?.kind === "video") {
      const full = document.querySelector<HTMLVideoElement>(".review-image-fullscreen video");
      if (full) { videoResumeTime.current = full.currentTime; full.pause(); }
      const stage = document.querySelector<HTMLVideoElement>(".review-visual-stage video");
      if (stage && Number.isFinite(videoResumeTime.current)) stage.currentTime = videoResumeTime.current;
    }
    setImageFullscreen(false);
    if (fullscreenOwned.current && document.fullscreenElement === document.documentElement)
      void document.exitFullscreen().catch(() => undefined);
    fullscreenOwned.current = false;
  };
  const openImageFullscreen = () => {
    if (!active || !["image", "video"].includes(active.kind) || pinning) return;
    if (active.kind === "video") {
      const stage = document.querySelector<HTMLVideoElement>(".review-visual-stage video");
      if (stage) { videoResumeTime.current = stage.currentTime; stage.pause(); }
    }
    setImageFullscreen(true);
    // Try true screen fullscreen from the click gesture. The portal still fills
    // the TaskBoard window when WebView2 does not grant the Fullscreen API.
    if (!document.fullscreenElement && document.documentElement.requestFullscreen)
      void document.documentElement.requestFullscreen()
        .then(() => { fullscreenOwned.current = true; })
        .catch(() => undefined);
  };
  useEffect(() => {
    if (!imageFullscreen) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (active?.kind === "video") {
      const full = document.querySelector<HTMLVideoElement>(".review-image-fullscreen video");
      if (full) {
        const restore = () => { if (Number.isFinite(videoResumeTime.current)) full.currentTime = videoResumeTime.current; };
        if (full.readyState >= 1) restore();
        else full.addEventListener("loadedmetadata", restore, { once: true });
      }
    }
    fullscreenCloseRef.current?.focus();
    const onFullscreenChange = () => {
      if (fullscreenOwned.current && document.fullscreenElement !== document.documentElement) {
        fullscreenOwned.current = false;
        setImageFullscreen(false);
      }
    };
    document.addEventListener("fullscreenchange", onFullscreenChange);
    return () => {
      document.removeEventListener("fullscreenchange", onFullscreenChange);
      if (previous?.isConnected) previous.focus();
    };
  }, [imageFullscreen]);
  useEffect(() => () => {
    if (fullscreenOwned.current && document.fullscreenElement === document.documentElement)
      void document.exitFullscreen().catch(() => undefined);
  }, []);
  useEffect(() => {
    closeImageFullscreen();
  }, [selectedKey]);

  useEffect(() => {
    setCompare("single");
    setZoom(1);
    setOpenError("");
    setGallerySelectedIndex(null);
  }, [selectedKey, active?.id, active?.path]);
  useEffect(() => {
    stripRef.current?.querySelector('[aria-current="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active?.id]);

  // Escape and the visible Back control unwind one layer at a time. The
  // separate Exit Inbox control is always available when a review is open.
  const navigateBack = () => {
    if (copyMenuOpen) dismissCopyMenu();
    else if (moreOpen) setMoreOpen(false);
    else if (dashboardMode && gallerySelectedIndex !== null) setGallerySelectedIndex(null);
    else if (focus) setFocus(false);
    else if (compare !== "single") setCompare("single");
    else if (pinning) onTogglePin();
    else onBack();
  };

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (suspended) return;
      if (imageFullscreen) {
        if (event.key === "Escape") {
          event.preventDefault();
          event.stopImmediatePropagation();
          closeImageFullscreen();
        } else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
          // Let focused native video controls seek; arrows elsewhere navigate media.
          if (active?.kind === "video" && event.target instanceof HTMLMediaElement) return;
          const step = event.key === "ArrowRight" ? 1 : -1;
          const images = playable.flatMap((artifact, index) => ["image", "video"].includes(artifact.kind) ? [index] : []);
          const next = (step > 0 ? images.find(index => index > activeIndex)
            : images.reverse().find(index => index < activeIndex)) ?? -1;
          if (next >= 0) {
            event.preventDefault();
            event.stopImmediatePropagation();
            onFocusArtifact(next);
          }
        }
        return;
      }
      // Header shortcuts are real actions, not merely labels in hover tooltips.
      // Modifier combinations are distinct from the unmodified media/decision keys.
      if (!event.ctrlKey && !event.metaKey && event.altKey && !event.repeat) {
        const key = event.key.toLowerCase();
        if (key === "x") { event.preventDefault(); event.stopImmediatePropagation(); onExitInbox(); return; }
        if (key === "d" && !saving) {
          event.preventDefault(); event.stopImmediatePropagation();
          setMoreOpen(false); void onMoveToDraft(); return;
        }
        if (key === "a" && !saving) {
          event.preventDefault(); event.stopImmediatePropagation();
          setMoreOpen(false); void onArchive(); return;
        }
      }
      if (event.ctrlKey || event.altKey || event.metaKey) return;
      if (active?.kind === "video" && event.target instanceof HTMLMediaElement) return;
      const target = event.target;
      if (target instanceof HTMLElement &&
        (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopImmediatePropagation();
        target.blur(); // First Escape leaves the field without discarding its draft.
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        navigateBack();
      } else if (dashboardMode && gallerySelectedIndex !== null &&
        (event.key === "ArrowLeft" || event.key === "ArrowRight")) {
        event.preventDefault();
        event.stopImmediatePropagation();
        const direction = event.key === "ArrowLeft" ? -1 : 1;
        setGallerySelectedIndex((current) => current === null ? null :
          Math.min(Math.max(0, galleryCount - 1), Math.max(0, current + direction)));
      } else if (!event.repeat && event.key.toLowerCase() === "k" && previousItem) {
        event.preventDefault();
        event.stopImmediatePropagation();
        goPreviousItem();
      } else if (!event.repeat && event.key.toLowerCase() === "b") {
        event.preventDefault(); event.stopImmediatePropagation();
        setFeedbackExpanded((value) => !value);
      } else if (!event.repeat && event.key.toLowerCase() === "g") {
        event.preventDefault(); event.stopImmediatePropagation();
        setMoreOpen((value) => !value);
      } else if (!event.repeat && event.key.toLowerCase() === "t" && onOpenTask && !saving) {
        event.preventDefault(); event.stopImmediatePropagation(); setMoreOpen(false); onOpenTask();
      } else if (!event.repeat && event.key.toLowerCase() === "s" &&
        (review.task.claimed_by || review.task.live?.with_me) && !sessionOpening && !saving) {
        event.preventDefault(); event.stopImmediatePropagation(); setMoreOpen(false); onOpenSession();
      } else if (!event.repeat && event.key.toLowerCase() === "u" && !priorityBusy) {
        event.preventDefault(); event.stopImmediatePropagation(); setMoreOpen(false); onTogglePriority();
      } else if (event.key.toLowerCase() === "f" && gallerySelectedIndex === null) {
        event.preventDefault();
        event.stopImmediatePropagation();
        setFocus((state) => !state);
      } else if (event.key === "ArrowRight" && activeIndex + 1 < playable.length) {
        event.preventDefault();
        event.stopImmediatePropagation();
        onFocusArtifact(activeIndex + 1);
      } else if (event.key === "ArrowLeft" && activeIndex > 0) {
        event.preventDefault();
        event.stopImmediatePropagation();
        onFocusArtifact(activeIndex - 1);
      }
    };
    // Capture before the shared Inbox Escape handler so Focus only exits Focus.
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [suspended, focus, imageFullscreen, dashboardMode, gallerySelectedIndex, galleryCount, onBack, onFocusArtifact, activeIndex, playable, active?.kind, moreOpen, compare, pinning, onTogglePin, copyMenuOpen, previousItem, onSelectTask, onExitInbox, onMoveToDraft, onArchive, saving, onOpenTask, onOpenSession, sessionOpening, review.task.claimed_by, review.task.live?.with_me, priorityBusy, onTogglePriority]);

  const originalAddress = (artifact: ReviewArtifact) => {
    if (artifact.kind === "url") return artifact.path;
    const source = artifact.kind === "html" && !/\.html?$/i.test(artifact.path)
      ? artifact.path.replace(/[\\/]$/, "") + "/index.html" : artifact.path;
    return /^(?:[a-z]:[\\/]|\/|\\\\)/i.test(source)
      ? source : review.path.replace(/[\\/]$/, "") + "/" + source;
  };

  // A website has no local file to reveal; if it has a synced poster,
  // Fomni selects that preview instead. Native HTML/images/videos select
  // their actual review files, including local-only media.
  const fomniFile = (artifact?: ReviewArtifact): string | undefined => {
    if (!artifact) return undefined;
    if (artifact.kind === "url") {
      const poster = artifact.poster;
      return poster ? originalAddress({ ...artifact, kind: "image", path: poster }) : undefined;
    }
    return originalAddress(artifact);
  };
  const showArtifactMenu = (event: MouseEvent<HTMLElement>, artifact: ReviewArtifact) => {
    const file = fomniFile(artifact);
    showCopyMenu(event, originalAddress(artifact),
      file ? { fomniFile: file,
        fomniLabel: artifact.kind === "url" ? "Open poster in Fomni" : "Open in Fomni" } : undefined);
  };

  // Opaque-origin HTML previews forward right-click coordinates via the review
  // bridge. The source check prevents unrelated embedded websites from invoking
  // Fomni or replacing the current review's menu.
  useEffect(() => {
    const onMessage = (event: MessageEvent) => {
      const frame = htmlFrameRef.current;
      if (active?.kind !== "html" || !frame ||
          event.source !== frame.contentWindow ||
          event.data?.type !== "taskboard-review-contextmenu") return;
      const x = Number(event.data.x), y = Number(event.data.y);
      const rect = frame.getBoundingClientRect();
      if (!Number.isFinite(x) || !Number.isFinite(y) ||
          x < 0 || y < 0 || x > rect.width || y > rect.height) return;
      const file = fomniFile(active);
      showCopyMenuAt(rect.left + x, rect.top + y, originalAddress(active),
        file ? { fomniFile: file } : undefined);
    };
    window.addEventListener("message", onMessage);
    return () => window.removeEventListener("message", onMessage);
  }, [active, htmlFrameRef, review.path, showCopyMenuAt]);

  const openOriginal = async (artifact?: ReviewArtifact) => {
    if (!artifact) return;
    setOpenError("");
    try {
      if (artifact.kind === "url") await openUrl(artifact.path);
      else if (artifact.storage === "local" || artifact.storage === "synced") {
        await openFileInFomni(originalAddress(artifact));
      } else throw new Error("No local file is available for this artifact.");
    } catch (reason) {
      setOpenError("Unable to open original: " + String(reason));
    }
  };
  const thumbnail = (item: ReviewArtifact) => {
    const poster = posterOf(item);
    return poster ? <img src={poster} alt="" /> :
      <span className="review-visual-no-thumb">{item.kind.toUpperCase()}</span>;
  };

  const imageStage = active && (
    <div className={"review-visual-stage" + (pinning ? " pinning" : "") +
      (active.kind === "image" && !pinning ? " enlargeable" : "") +
      (isWeb(active) ? " web" : "") + (zoom > 1 ? " zoomed" : "")}
      role={active.kind === "image" && !pinning ? "button" : "region"}
      tabIndex={active.kind === "image" && !pinning ? 0 : undefined}
      aria-label={"Artifact " + (activeIndex + 1) + " of " + playable.length + ": " + active.label +
        (active.kind === "image" && !pinning ? ". Enlarge fullscreen" : "")}
      title={active.kind === "image" && !pinning ? "Click to view fullscreen (Esc to exit)" : undefined}
      onKeyDown={(event) => {
        if (active.kind === "image" && !pinning && (event.key === "Enter" || event.key === " ")) {
          event.preventDefault();
          openImageFullscreen();
        }
      }}
      onClick={(event) => { if (pinning) addPin(event, active); else if (active.kind === "image") openImageFullscreen(); }}
      onContextMenu={(event) => showArtifactMenu(event, active)}>
      <div className={"review-visual-media" + (zoom > 1 ? " enlarged" : "")}
        style={zoom > 1 && active.kind === "image" ? { width: zoom * 100 + "%" } : undefined}>
        {renderArtifact(active)}
        {pins.filter((pin) => pin.artifact === active.id).map((pin, i) =>
          <span key={i} className="review-pin"
            style={{ left: pin.x * 100 + "%", top: pin.y * 100 + "%" }}>
            {pins.indexOf(pin) + 1}
          </span>)}
      </div>
    </div>
  );

  return (
    <section onPaste={onPaste} onDragOver={event => event.preventDefault()} onDrop={onDrop}
      className={"review-visual-inspector instrument-review" + (focus ? " focus" : "") +
      (feedbackExpanded ? "" : " feedback-hidden") + (dashboardMode ? " dashboard-review" : "")}>
      <header className="review-visual-header review-instrument-header">
        <div className="review-instrument-navigation" role="group" aria-label="Inbox navigation">
          <button type="button" className="review-instrument-icon" onClick={goPreviousItem}
            aria-disabled={!previousItem} aria-label="Previous inbox item"
            title={previousItem ? "Previous inbox item (K)" : "No previous inbox item (K)"}>
            <ReviewHeaderIcon name="previous" />
          </button>
          <button type="button" className="review-instrument-icon exit" onClick={onExitInbox}
            aria-label="Close Inbox" title="Close Inbox and return to the board (Alt+X) · Task remains unchanged">
            <ReviewHeaderIcon name="close" />
          </button>
        </div>
        <span className="review-instrument-id"
          title={`${review.project} · ${review.task.id} · Round ${round.number}`}>{review.task.id}</span>
        <div className="review-instrument-title">
          <strong title={review.task.title}>{review.task.title}</strong>
        </div>
        <span className={`review-instrument-state inbox-task-column-chip column-${review.task.column}`}
          title={`TaskBoard workflow column: ${review.task.column}. This is not a review decision.`}>
          {review.task.column.replace(/-/g, " ")}
        </span>
        <div className="review-instrument-actions" role="group" aria-label="Review controls">
          <button type="button" className={"review-instrument-action" + (focus ? " active" : "")}
            onClick={() => setFocus((value) => !value)} aria-pressed={focus}
            aria-label={focus ? "Exit focus mode" : "Focus artwork"}
            title={(focus ? "Exit focus mode" : "Focus artwork") + " (F)"}>
            <ReviewHeaderIcon name="focus" />
          </button>
          <button type="button" className={"review-instrument-action" + (feedbackExpanded ? " active" : "")}
            onClick={() => setFeedbackExpanded((value) => !value)} aria-pressed={feedbackExpanded}
            aria-label={feedbackExpanded ? "Hide feedback" : "Show feedback"}
            title={(feedbackExpanded ? "Hide feedback" : "Show feedback") + " (B)"}>
            <ReviewHeaderIcon name="feedback" />
          </button>
          <div className="inbox-more-wrap review-instrument-more">
            <button type="button" className="review-instrument-action" aria-label="More review actions"
              aria-expanded={moreOpen} aria-haspopup="menu" title="More review actions (G)"
              onClick={() => setMoreOpen((value) => !value)}><ReviewHeaderIcon name="more" /></button>
            {moreOpen && <div className="inbox-more-menu review-instrument-menu" role="menu">
              <div className="review-instrument-menu-caption">{modeDetails.explanation}</div>
              <button role="menuitem" type="button" disabled={!onOpenTask || saving}
                title={onOpenTask ? "Open full task (T)" : "Full task unavailable in this project"}
                onClick={() => { setMoreOpen(false); onOpenTask?.(); }}>
                Full task <kbd>T</kbd>
              </button>
              <button role="menuitem" type="button"
                disabled={sessionOpening || saving || !(review.task.claimed_by || review.task.live?.with_me)}
                title={(review.task.claimed_by || review.task.live?.with_me)
                  ? "Open active task session (S)" : "No active task session"}
                onClick={() => { setMoreOpen(false); onOpenSession(); }}>
                {sessionOpening ? "Opening session…" : "Open session"} <kbd>S</kbd>
              </button>
              <button role="menuitem" type="button" disabled={priorityBusy}
                title={(review.task.attention.includes("priority") ? "Remove" : "Set") + " task priority (U)"}
                onClick={() => { setMoreOpen(false); onTogglePriority(); }}>
                {review.task.attention.includes("priority") ? "★ Remove priority" : "☆ Prioritize"} <kbd>U</kbd>
              </button>
              <div className="review-instrument-menu-divider" />
              <button role="menuitem" type="button" disabled={saving} title="Move task to Draft (Alt+D)"
                onClick={() => { setMoreOpen(false); void onMoveToDraft(); }}>Move to Draft <kbd>Alt+D</kbd></button>
              <button role="menuitem" type="button" disabled={saving} title="Archive task (Alt+A)"
                onClick={() => { setMoreOpen(false); void onArchive(); }}>Archive <kbd>Alt+A</kbd></button>
            </div>}
          </div>
        </div>
      </header>

      <div ref={bodyRef} className={"review-visual-body" + (dashboardMode ? " dashboard-review" : "")}
        style={dashboardMode ? { "--dashboard-controls-height": upperHeight + "px" } as CSSProperties : undefined}>
        {!focus && <InboxQueueSidebar
          items={items}
          selectedKey={selectedKey}
          onSelect={onSelectTask}
          allBoards={allBoards}
          showResolved={showResolved}
          onAllBoards={onAllBoards}
          onShowResolved={onShowResolved}
          thumbnails={queueThumbnails}
          modes={queueModes}
          counts={queueCounts}
        />}

        <ReviewCenter dashboardMode={dashboardMode} focus={focus}>
          <div className="review-visual-toolbar">
            {dashboardMode ? <div className="review-visual-web-summary">
              <div className="dashboard-gallery-heading">
                <strong>Published Dashboard evidence gallery</strong>
                <span aria-live="polite" className="dashboard-gallery-count">
                  {galleryCount ? `${galleryCount} capture${galleryCount === 1 ? "" : "s"}` : "Loading…"}
                </span>
              </div>
              <p>Original Wartribes captures · browse horizontally, then select one for closer inspection.</p>
            </div> : <>
              <span className="review-visual-counter">{activeIndex + 1} / {playable.length}</span>
              <strong title={active?.label}>{active?.label || "Review media"}</strong>
              <span className="review-visual-toolbar-spacer" />
              <button type="button" className="button secondary" disabled={activeIndex === 0}
                title="Previous item (←)" onClick={() => onFocusArtifact(activeIndex - 1)}>←</button>
              <button type="button" className="button secondary"
                disabled={activeIndex >= playable.length - 1}
                title="Next item (→)" onClick={() => onFocusArtifact(activeIndex + 1)}>→</button>
              <button type="button" className={"button " + (compareMode === "single" ? "primary" : "secondary")}
                onClick={() => setCompare("single")} aria-pressed={compareMode === "single"}>Single</button>
              <button type="button" className={"button " + (compareMode === "side" ? "primary" : "secondary")}
                disabled={!canCompare} title={canCompare ? "Side-by-side image comparison" : "Comparison requires two images"}
                onClick={() => setCompare("side")} aria-pressed={compareMode === "side"}>Compare</button>
              <button type="button" className={"button " + (compareMode === "overlay" ? "primary" : "secondary")}
                disabled={!canCompare} title={canCompare ? "Image overlay with opacity slider" : "Comparison requires two images"}
                onClick={() => setCompare("overlay")} aria-pressed={compareMode === "overlay"}>Overlay</button>
            </>}
          </div>
          {error && <div className="error-banner" role="alert">{error}</div>}
          {openError && <div className="error-banner" role="alert">{openError}</div>}
          {round.criteria.length > 0 && (
            <details className="review-visual-criteria">
              <summary>Quality bar · {round.criteria.length} criteria</summary>
              <ul>{round.criteria.map((criterion, i) => <li key={i}>{criterion}</li>)}</ul>
            </details>
          )}
          <div id={dashboardMode ? "dashboard-review-evidence" : undefined} className={"review-visual-viewer" + (dashboardMode ? " dashboard-capture-viewer" : "")}>
            {dashboardMode ? <DashboardEvidenceItem address={active!.path} selectedIndex={gallerySelectedIndex}
              onSelectedIndexChange={setGallerySelectedIndex} onGalleryCountChange={setGalleryCount} /> : active && <>
              <div className="review-visual-viewer-actions">
                <button type="button" className={"button " + (pinning ? "primary" : "secondary")}
                  aria-pressed={pinning} disabled={resolved}
                  onClick={onTogglePin} title="Place a pin on an image or video (P)">⌖ Pin <kbd>P</kbd></button>
                <button type="button" className="button secondary"
                  disabled={active.kind !== "image" || compareMode !== "single"}
                  onClick={() => setZoom((current) => current === 1 ? 1.25 : current === 1.25 ? 1.5 : 1)}
                  title="Fit / 125% / 150%">{zoom === 1 ? "⌕ Fit" : Math.round(zoom * 100) + "%"}</button>
                {isWeb(active) && <select aria-label="Prototype viewport width" value={iframeWidth}
                  onChange={(event) => onIframeWidth(Number(event.target.value))}>
                  <option value={0}>Viewport · Fit</option>
                  <option value={1440}>1440px</option>
                  <option value={1080}>1080px</option>
                  <option value={728}>728px</option>
                  <option value={390}>390px</option>
                </select>}
                {isWeb(active) && fomniFile(active) && <button type="button" className="button secondary"
                  title={active.kind === "url" ? "Reveal the website poster in Fomni" : "Reveal the HTML file in Fomni"}
                  onClick={() => {
                    const file = fomniFile(active);
                    if (file) void highlightInFomni(file).catch(reason =>
                      setOpenError("Unable to reveal media in Fomni: " + String(reason)));
                  }}>▤ Fomni folder</button>}
                {active.kind === "video" && <button type="button" className="button secondary"
                  onClick={openImageFullscreen} title="Open a full-window video player; Esc to return">
                  ⛶ Full-window video</button>}
                {active.kind === "video" && remoteReviewUrl && <button type="button" className="button secondary"
                  title="Open authenticated OmniHub phone review gallery"
                  onClick={() => void openUrl(remoteReviewUrl)}>Phone link ↗</button>}
                <button type="button" className="button secondary"
                  onClick={() => void openOriginal(active)}
                  onContextMenu={(event) => showArtifactMenu(event, active)}
                  title="Open original · right-click to reveal file in Fomni">
                  ↗ Open original</button>
              </div>
              {compareMode === "single" ? imageStage :
                <div className={"review-visual-comparison " + compareMode}>
                  {compareMode === "side" ? <>
                    <div onContextMenu={(event) => round.previous && showArtifactMenu(event, round.previous)}>
                      <span>Previous</span>{round.previous && renderArtifact(round.previous)}</div>
                    <div onContextMenu={(event) => showArtifactMenu(event, active)}>
                      <span>Proposed</span>{renderArtifact(active)}</div>
                  </> : <>
                    <div className="review-visual-comparison-old"
                      onContextMenu={(event) => round.previous && showArtifactMenu(event, round.previous)}>
                      {round.previous && renderArtifact(round.previous)}</div>
                    <div className="review-visual-comparison-new" style={{ opacity: opacity / 100 }}
                      onContextMenu={(event) => showArtifactMenu(event, active)}>
                      {renderArtifact(active)}
                    </div>
                    <label className="review-visual-opacity">
                      Old <input type="range" min={0} max={100} value={opacity} aria-label="New image opacity"
                        onChange={(event) => setOpacity(Number(event.target.value))} /> New
                    </label>
                  </>}
                </div>
              }
            </>}
          </div>
          {!focus && !resolved && (round.mode === "pick" || round.mode === "rank") &&
            <div className="review-selection-guide" role="status">
              {round.mode === "pick"
                ? choice ? `Selected: ${labelOf(choice)}. Click another thumbnail to change it.`
                  : "Pick an option: click its thumbnail below, then click Pick selected option."
                : `Rank your top ${round.top_k}: click thumbnails in preference order ` +
                  `(${Math.min(ranking.length, round.top_k)}/${round.top_k} required). ` +
                  "You can rank additional options, but don't have to. Click a ranked thumbnail to remove it."}
              {round.mode === "rank" && ranking.length > 0 &&
                <button type="button" className="button secondary" disabled={saving}
                  onClick={onClearRanking}>
                  Clear ranking
                </button>}
            </div>}
          {!focus && <nav className="review-visual-filmstrip" ref={stripRef} aria-label="Review artifacts">
            {playable.map((artifact, index) => {
              const selected = index === activeIndex;
              const picked = round.mode === "pick" && choice === artifact.id;
              const rank = round.mode === "rank" ? ranking.indexOf(artifact.id) : -1;
              return <button key={artifact.id} type="button"
                className={"review-visual-thumb" + (selected ? " active" : "") +
                  (picked ? " picked" : "")}
                aria-current={selected ? "true" : undefined}
                aria-pressed={round.mode === "pick" ? picked :
                  round.mode === "rank" ? rank >= 0 : undefined}
                onClick={() => onChooseArtifact(artifact.id)}
                onContextMenu={(event) => showArtifactMenu(event, artifact)}
                title={(picked ? "Picked · " : rank >= 0 ? "Rank " + (rank + 1) + " · " : "") + artifact.label}>
                {thumbnail(artifact)}
                <span><em>{index + 1}. {artifact.label || artifact.id}</em>
                  {picked && <b>✓ Picked</b>}
                  {rank >= 0 && <b>#{rank + 1}</b>}
                  <small>{artifact.kind}</small>
                </span>
              </button>;
            })}
          </nav>}
          {!focus && closedWithoutDecision && <div className="review-reconsider" role="status">
            Closed without a review decision. The task was moved to Done; no option was approved or rejected.
          </div>}
          {!focus && awaitingNextRound && <div className="review-reconsider">
            <span>This round was {round.result?.verdict}. The decision is saved, but you can open a new round to change it without erasing history. Missing or moved previews do not prevent your decision.</span>
            <button type="button" className="button secondary" disabled={saving}
              onClick={() => void onReconsider()}>Change decision</button>
          </div>}
          {!focus && playable.some((artifact) => artifact.pruned) && !awaitingNextRound &&
            <div className="review-reconsider">Preview files were removed or moved. You can still submit an explicit outcome for this review.</div>}
          {!focus && <InboxResponseFooter
            mode="visual"
            outcome={outcome}
            outcomes={outcomeOptions}
            onOutcome={onOutcome}
            onSkip={onSkip}
            onSnooze={onSnooze}
            onSubmit={(verdict) => void submit(verdict ?? primaryVerdict)}
            primaryLabel={round.mode === "approve" && review.task.column === "done" ? "Done" : undefined}
            notePresent={!!note.trim() || imageCount > 0}
            onNoteOnly={() => void onNoteOnly()}
            onCloseTask={onCloseTask}
            busy={saving}
            disabled={resolved}
            submitDisabled={primaryIncomplete}
            summary={closedWithoutDecision ? "Closed without decision · Done"
              : resolved ? (awaitingNextRound ? "Awaiting another round" : "Review resolved")
              : round.mode === "pick" ? (choice ? "Selected " + labelOf(choice) : "Select an option")
              : round.mode === "rank" ? ranking.length + " ranked · " + round.top_k + " required"
              : pinCount > 0 ? pinCount + " annotations" : "Inspect artwork and choose an outcome"}
          />}
        </ReviewCenter>
        {dashboardMode && !focus && <div className="dashboard-review-resizer"
          role="separator" tabIndex={0} aria-controls="dashboard-review-evidence" aria-label="Resize TaskBoard controls and evidence viewer"
          aria-orientation="horizontal" aria-valuemin={145} aria-valuemax={maxUpperHeight()}
          aria-valuenow={Math.round(upperHeight)}
          title="Drag to resize TaskBoard controls / evidence · double-click to reset · Up/Down arrow keys"
          onPointerDown={resizePointerDown}
          onPointerMove={resizePointerMove}
          onPointerUp={resizePointerEnd}
          onPointerCancel={resizePointerEnd}
          onDoubleClick={() => persistUpperHeight(defaultUpperHeight)}
          onKeyDown={resizeKeyDown}><span aria-hidden="true" /></div>}

        {!focus && feedbackExpanded && <aside className="review-visual-feedback" aria-label="Review feedback">
          <header><strong>YOUR RESPONSE</strong>
            <button type="button" className="button secondary" onClick={() => setFeedbackExpanded(false)}
              aria-label="Hide feedback">×</button></header>
          <div className="review-visual-feedback-content">
            <InboxRichText task={review.task} text={round.judge} className="review-visual-judge" onError={setOpenError} />

            <label className="review-note">Overall note
              <textarea rows={6} value={note} disabled={resolved}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey && !event.ctrlKey && !event.metaKey && note.trim()) {
                    event.preventDefault();
                    void onNoteOnly();
                  }
                }}
                onChange={(event) => setNote(event.target.value)}
                placeholder="Tell the agent what to keep or change." />
            </label>
            {noteFeedback && <p className="review-note-saved" role="status">{noteFeedback}</p>}
            {!!review.task.user_notes.trim() && <details className="review-visual-references" open={!!noteFeedback}>
              <summary>Saved task notes</summary>
              <InboxRichText task={review.task} text={review.task.user_notes} onError={setOpenError} />
            </details>}
            <button type="button" className="button secondary inbox-attach-button"
              disabled={resolved} onClick={onBrowseFiles}>+ Images / local files…</button>
            {imagePreviews}
            <h3>Annotations · {pins.length}</h3>
            <p className="review-muted">{pinning ? "Click the artwork to place a pin." :
              "Choose Pin to annotate an image or a video frame."}</p>
            {pins.length > 0 && <ol className="review-pin-list">
              {pins.map((pin, index) => <li key={index}>
                <span className="review-pin-badge">{index + 1}</span>
                <label>
                  <span>{labelOf(pin.artifact || "")} · {Math.round(pin.x * 100)}%,
                    {Math.round(pin.y * 100)}%{pin.timestamp != null ? " · " + pin.timestamp.toFixed(1) + "s" : ""}</span>
                  <input value={pin.note || ""} readOnly={resolved} placeholder="Pin comment"
                    onChange={(event) => setPins(pins.map((existing, i) => i === index ?
                      { ...existing, note: event.target.value } : existing))}/>
                </label>
                {!resolved && <button type="button" aria-label={"Remove pin " + (index + 1)}
                  onClick={() => setPins(pins.filter((_, i) => i !== index))}>×</button>}
              </li>)}
            </ol>}
            {history.length > 0 && <details className="review-visual-references">
              <summary>Prior rounds · {history.length}</summary>
              {history.map(past => <p key={past.number}>Round {past.number} · {past.mode} · {past.result?.verdict ?? "No verdict"} — {past.judge}</p>)}
            </details>}
            <details className="review-visual-references">
              <summary>References · {round.references.length + (round.previous ? 1 : 0)}</summary>
              {round.previous && <button className="inbox-resource-link" type="button"
                onClick={() => void openOriginal(round.previous)}
                onContextMenu={(event) => showArtifactMenu(event, round.previous!)}>Previous: {round.previous.label} ↗</button>}
              {round.references.map((reference) => <button className="inbox-resource-link" type="button"
                key={reference.id} onClick={() => void openOriginal(reference)}
                onContextMenu={(event) => showArtifactMenu(event, reference)}>{reference.label} ↗</button>)}
              {!round.previous && round.references.length === 0 && <p>No references supplied.</p>}
            </details>
          </div>
        </aside>}
      </div>
      {imageFullscreen && (active?.kind === "image" || active?.kind === "video") && createPortal(
        <div className="review-image-fullscreen" role="dialog" aria-modal="true"
          aria-label={"Fullscreen review media: " + (active.label || active.id)}
          onKeyDown={(event) => {
            if (active.kind === "image" && event.key === "Tab") {
              event.preventDefault();
              fullscreenCloseRef.current?.focus();
            }
          }}>
          <header className="review-image-fullscreen-header">
            <strong title={active.label}>{active.label || active.id}</strong>
            <span>{activeIndex + 1} / {playable.length} · Esc to close</span>
            <button ref={fullscreenCloseRef} type="button" className="button primary"
              onClick={closeImageFullscreen}>× Exit fullscreen</button>
          </header>
          <div className="review-image-fullscreen-media"
            onContextMenu={(event) => showArtifactMenu(event, active)}
            title={active.kind === "image" ? "Click to return to the review" : undefined}
            onClick={active.kind === "image" ? closeImageFullscreen : undefined}>
            {renderArtifact(active)}
          </div>
        </div>, document.body)}
      {copyMenu}
    </section>
  );
}
