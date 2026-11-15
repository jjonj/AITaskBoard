import { useEffect, useRef, useState, type ClipboardEvent, type DragEvent, type MouseEvent, type ReactNode, type CSSProperties, type PointerEvent as ReactPointerEvent, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { createPortal } from "react-dom";
import "./ReviewImageFullscreen.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { ReviewArtifact, ReviewPin, ReviewRound } from "./reviewApi";
import type { Task } from "./types";
import { InboxResponseFooter, reviewOutcomes } from "./InboxResponseFooter";
import { InboxRichText, openFileInFomni } from "./InboxRichText";
import { InboxQueueSidebar } from "./InboxQueueSidebar";
import { DashboardEvidenceItem } from "./DashboardEvidenceItem";
import { useLinkCopyMenu } from "./LinkCopyMenu";

type Item = { path: string; project: string; task: Task };
type Props = {
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
  onIframeWidth: (value: number) => void;
  note: string;
  setNote: (note: string) => void;
  saving: boolean;
  outcome: string;
  onOutcome: (value: string) => void;
  onSkip: () => void;
  onSnooze: (milliseconds: number) => void;
  resolved: boolean;
  awaitingNextRound: boolean;
  error: string;
  primaryVerdict: string;
  primaryReady: boolean;
  submit: (verdict: string) => Promise<void>;
  onNoteOnly: () => Promise<void>;
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
    review, round, history, items, selectedKey, onSelectTask, onBack, onExitInbox, hasPreviousItem, queueThumbnails, queueModes, queueCounts, onOpenTask, onOpenSession, sessionOpening, onTogglePriority, priorityBusy, allBoards, showResolved,
    onAllBoards, onShowResolved, playable, activeIndex, onFocusArtifact, onChooseArtifact,
    choice, ranking, pins, setPins, pinning, onTogglePin, addPin, renderArtifact, remoteReviewUrl,
    posterOf, iframeWidth, onIframeWidth, note, setNote, saving,
    outcome, onOutcome, onSkip, onSnooze,
    resolved, awaitingNextRound, error, primaryVerdict, primaryReady, submit, onNoteOnly,
    onBrowseFiles, imagePreviews, imageCount, onPaste, onDrop, onMoveToDraft, onArchive, onReconsider,
  } = props;
  const [feedbackExpanded, setFeedbackExpanded] = useState(true);
  const [focus, setFocus] = useState(false);
  const [compare, setCompare] = useState<"single" | "side" | "overlay">("single");
  const [opacity, setOpacity] = useState(50);
  const [zoom, setZoom] = useState(1);
  const [openError, setOpenError] = useState("");
  const { showCopyMenu, copyMenu, copyMenuOpen, dismissCopyMenu } = useLinkCopyMenu(setOpenError);
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
      return Number.isFinite(remembered) && remembered >= 145 ? Math.min(remembered, 650) : defaultUpperHeight;
    } catch { return defaultUpperHeight; }
  });
  const bodyRef = useRef<HTMLDivElement>(null);
  const dragOrigin = useRef<{ pointer: number; y: number; height: number } | null>(null);
  const maxUpperHeight = () => Math.max(145, (bodyRef.current?.clientHeight ?? window.innerHeight) - 200);
  const clampUpperHeight = (height: number) =>
    Math.max(145, Math.min(Math.round(height), maxUpperHeight()));
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
  const primaryIncomplete = outcome === primaryVerdict && !primaryReady;

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
  }, [focus, imageFullscreen, dashboardMode, gallerySelectedIndex, galleryCount, onBack, onFocusArtifact, activeIndex, playable, active?.kind, moreOpen, compare, pinning, onTogglePin, copyMenuOpen]);

  const originalAddress = (artifact: ReviewArtifact) => {
    if (artifact.kind === "url") return artifact.path;
    const source = artifact.kind === "html" && !/\.html?$/i.test(artifact.path)
      ? artifact.path.replace(/[\\/]$/, "") + "/index.html" : artifact.path;
    return /^(?:[a-z]:[\\/]|\/|\\\\)/i.test(source)
      ? source : review.path.replace(/[\\/]$/, "") + "/" + source;
  };

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
      onClick={(event) => { if (pinning) addPin(event, active); else if (active.kind === "image") openImageFullscreen(); }}>
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
      className={"review-visual-inspector" + (focus ? " focus" : "") +
      (feedbackExpanded ? "" : " feedback-hidden") + (dashboardMode ? " dashboard-review" : "")}>
      <header className="review-visual-header">
        <button type="button" className="button secondary" onClick={navigateBack}
          title="Go back one navigation step (Esc)">
          {dashboardMode && gallerySelectedIndex !== null ? "← All captures"
            : focus ? "← Exit focus" : compare !== "single" ? "← Single view"
            : pinning ? "← Exit pin mode" : hasPreviousItem ? "← Previous item" : "← Board"}</button>
        <div className="review-visual-title"><small>{review.project} · {review.task.id} · Round {round.number}</small>
          <strong title={review.task.title}>{review.task.title}</strong></div>
        <span className={`inbox-task-column-chip column-${review.task.column}`}>{review.task.column}</span>
        <button type="button" className={"inbox-priority-button" +
          (review.task.attention.includes("priority") ? " is-priority" : "")}
          aria-pressed={review.task.attention.includes("priority")}
          aria-label={review.task.attention.includes("priority")
            ? "Remove Priority from " + review.task.id : "Mark " + review.task.id + " as Priority"}
          title={review.task.attention.includes("priority")
            ? "Priority enabled · click to remove" : "Mark Priority so agents handle this task sooner"}
          disabled={priorityBusy} onClick={onTogglePriority}>
          <span aria-hidden="true">!</span> {review.task.attention.includes("priority") ? "Priority" : "Prioritize"}
        </button>
        <span className="review-visual-mode">{round.mode.toUpperCase()}</span>
        <div className="review-visual-header-actions">
          <button type="button" className="button secondary" onClick={onExitInbox}
            title="Exit the feedback Inbox and return to the board" aria-label="Close Inbox">× Close Inbox</button>
          {onOpenTask && <button type="button" className="button secondary" onClick={onOpenTask} title="Inspect the full task and return to Inbox on closing">Full task ↗</button>}
          {(review.task.claimed_by || review.task.live?.with_me) && (
            <button type="button" className="button secondary"
              aria-label={`Open session for ${review.task.id}`}
              disabled={sessionOpening} onClick={onOpenSession}>
              {sessionOpening ? "Opening…" : "Open Session"}
            </button>
          )}
          <div className="inbox-more-wrap">
            <button type="button" className="button secondary" aria-label="More Inbox actions"
              aria-expanded={moreOpen} onClick={() => setMoreOpen(open => !open)}>⋯</button>
            {moreOpen && <div className="inbox-more-menu">
              <button disabled={saving} type="button" onClick={() => { setMoreOpen(false); void onMoveToDraft(); }}>Move to Draft</button>
              <button disabled={saving} type="button" onClick={() => { setMoreOpen(false); void onArchive(); }}>Archive</button>
            </div>}
          </div>
          <button type="button" className={"button " + (feedbackExpanded ? "primary" : "secondary")}
            onClick={() => setFeedbackExpanded((value) => !value)} aria-pressed={feedbackExpanded}>
            ☷ Feedback</button>
          <button type="button" className={"button " + (focus ? "primary" : "secondary")}
            onClick={() => setFocus((value) => !value)} aria-pressed={focus}
            title="Focus artwork (F)">{focus ? "Exit focus" : "⛶ Focus"} <kbd>F</kbd></button>
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

        <main className="review-visual-center">
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
                {active.kind === "video" && <button type="button" className="button secondary"
                  onClick={openImageFullscreen} title="Open a full-window video player; Esc to return">
                  ⛶ Full-window video</button>}
                {active.kind === "video" && remoteReviewUrl && <button type="button" className="button secondary"
                  title="Open authenticated OmniHub phone review gallery"
                  onClick={() => void openUrl(remoteReviewUrl)}>Phone link ↗</button>}
                <button type="button" className="button secondary"
                  onClick={() => void openOriginal(active)}
                  onContextMenu={(event) => showCopyMenu(event, originalAddress(active))}
                  title="Open the original in its default application · right-click to copy link">
                  ↗ Open original</button>
              </div>
              {compareMode === "single" ? imageStage :
                <div className={"review-visual-comparison " + compareMode}>
                  {compareMode === "side" ? <>
                    <div><span>Previous</span>{round.previous && renderArtifact(round.previous)}</div>
                    <div><span>Proposed</span>{renderArtifact(active)}</div>
                  </> : <>
                    <div className="review-visual-comparison-old">{round.previous && renderArtifact(round.previous)}</div>
                    <div className="review-visual-comparison-new" style={{ opacity: opacity / 100 }}>
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
            busy={saving}
            disabled={resolved}
            submitDisabled={primaryIncomplete}
            summary={resolved ? (awaitingNextRound ? "Awaiting another round" : "Review resolved")
              : round.mode === "pick" ? (choice ? "Selected " + labelOf(choice) : "Select an option")
              : round.mode === "rank" ? ranking.length + "/" + playable.length + " ranked"
              : pinCount > 0 ? pinCount + " annotations" : "Inspect artwork and choose an outcome"}
          />}
        </main>
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
                onContextMenu={(event) => showCopyMenu(event, originalAddress(round.previous!))}>Previous: {round.previous.label} ↗</button>}
              {round.references.map((reference) => <button className="inbox-resource-link" type="button"
                key={reference.id} onClick={() => void openOriginal(reference)}
                onContextMenu={(event) => showCopyMenu(event, originalAddress(reference))}>{reference.label} ↗</button>)}
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
            title={active.kind === "image" ? "Click to return to the review" : undefined}
            onClick={active.kind === "image" ? closeImageFullscreen : undefined}>
            {renderArtifact(active)}
          </div>
        </div>, document.body)}
      {copyMenu}
    </section>
  );
}
