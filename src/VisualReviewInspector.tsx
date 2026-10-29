import { useEffect, useRef, useState, type MouseEvent, type ReactNode } from "react";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import type { ReviewArtifact, ReviewPin, ReviewRound } from "./reviewApi";
import type { Task } from "./types";
import { InboxResponseFooter, reviewOutcomes } from "./InboxResponseFooter";

type Item = { path: string; project: string; task: Task };
type Props = {
  review: Item;
  round: ReviewRound;
  items: Item[];
  selectedKey: string;
  onSelectTask: (key: string) => void;
  onClose: () => void;
  onOpenSession: () => void;
  sessionOpening: boolean;
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
  onReconsider: () => Promise<void>;
};

const keyOf = (item: Item) => item.path + "|" + item.task.id;
const isWeb = (artifact: ReviewArtifact) => artifact.kind === "html" || artifact.kind === "url";

/** First-class task review workspace. This is independent of the artifact count,
 * mode (pick/approve/rank/markup), and project: a single focused media stage,
 * filmstrip, optional comparison and a collapsible feedback rail.
 */
export function VisualReviewInspector(props: Props) {
  const {
    review, round, items, selectedKey, onSelectTask, onClose, onOpenSession, sessionOpening, allBoards, showResolved,
    onAllBoards, onShowResolved, playable, activeIndex, onFocusArtifact, onChooseArtifact,
    choice, ranking, pins, setPins, pinning, onTogglePin, addPin, renderArtifact,
    posterOf, iframeWidth, onIframeWidth, note, setNote, saving,
    outcome, onOutcome, onSkip, onSnooze,
    resolved, awaitingNextRound, error, primaryVerdict, primaryReady, submit, onReconsider,
  } = props;
  const [queueExpanded, setQueueExpanded] = useState(false);
  const [feedbackExpanded, setFeedbackExpanded] = useState(true);
  const [focus, setFocus] = useState(false);
  const [compare, setCompare] = useState<"single" | "side" | "overlay">("single");
  const [opacity, setOpacity] = useState(50);
  const [zoom, setZoom] = useState(1);
  const [openError, setOpenError] = useState("");
  const stripRef = useRef<HTMLDivElement>(null);
  const active = playable[Math.min(activeIndex, playable.length - 1)];
  const canCompare = active?.kind === "image" && round.previous?.kind === "image";
  const compareMode = canCompare ? compare : "single";
  const pinCount = active ? pins.filter((pin) => pin.artifact === active.id).length : 0;
  const labelOf = (id: string) => playable.find((value) => value.id === id)?.label || id;
  const outcomeOptions = reviewOutcomes(round.mode);
  const primaryIncomplete = outcome === primaryVerdict && !primaryReady;

  useEffect(() => {
    setCompare("single");
    setZoom(1);
    setOpenError("");
  }, [selectedKey, active?.id]);
  useEffect(() => {
    stripRef.current?.querySelector('[aria-current="true"]')?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [active?.id]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey || event.altKey || event.metaKey) return;
      const target = event.target;
      if (target instanceof HTMLElement &&
        (target.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(target.tagName))) {
        if (event.key !== "Escape") return;
        (target as HTMLElement).blur();
      }
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation();
        if (focus) setFocus(false);
        else onClose();
      } else if (event.key.toLowerCase() === "f") {
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
  }, [focus, onClose, onFocusArtifact, activeIndex, playable.length]);

  const openOriginal = async () => {
    if (!active) return;
    setOpenError("");
    try {
      if (active.kind === "url") await openUrl(active.path);
      else if (active.storage === "local" || active.storage === "synced") {
        const source = active.kind === "html" && !/\.html?$/i.test(active.path)
          ? active.path.replace(/[\\/]$/, "") + "/index.html" : active.path;
        await openPath(source);
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
      (isWeb(active) ? " web" : "") + (zoom > 1 ? " zoomed" : "")}
      role="region" aria-label={"Artifact " + (activeIndex + 1) + " of " + playable.length + ": " + active.label}
      onClick={(event) => { if (pinning) addPin(event, active); }}>
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
    <section className={"review-visual-inspector" + (focus ? " focus" : "") +
      (feedbackExpanded ? "" : " feedback-hidden") + (queueExpanded ? " queue-expanded" : "")}>
      <header className="review-visual-header">
        <button type="button" className="button secondary" onClick={onClose}
          title="Back to Inbox/board (Esc)">← Inbox <span className="review-visual-count">{items.length}</span></button>
        <div className="review-visual-title"><small>{review.project} · {review.task.id} · Round {round.number}</small>
          <strong title={review.task.title}>{review.task.title}</strong></div>
        <span className="review-visual-mode">{round.mode.toUpperCase()}</span>
        <div className="review-visual-header-actions">
          {(review.task.claimed_by || review.task.live?.with_me) && (
            <button type="button" className="button secondary"
              aria-label={`Open session for ${review.task.id}`}
              disabled={sessionOpening} onClick={onOpenSession}>
              {sessionOpening ? "Opening…" : "Open Session"}
            </button>
          )}
          <button type="button" className={"button " + (feedbackExpanded ? "primary" : "secondary")}
            onClick={() => setFeedbackExpanded((value) => !value)} aria-pressed={feedbackExpanded}>
            ☷ Feedback</button>
          <button type="button" className={"button " + (focus ? "primary" : "secondary")}
            onClick={() => setFocus((value) => !value)} aria-pressed={focus}
            title="Focus artwork (F)">{focus ? "Exit focus" : "⛶ Focus"} <kbd>F</kbd></button>
        </div>
      </header>

      <div className="review-visual-body">
        {!focus && (
          <aside className="review-visual-queue" aria-label="Feedback queue">
            <button type="button" className="button secondary review-visual-queue-toggle"
              onClick={() => setQueueExpanded((value) => !value)}
              aria-expanded={queueExpanded} title="Expand/collapse task queue">
              {queueExpanded ? "← Collapse" : "☷"}
            </button>
            {queueExpanded ? <>
              <h3>Feedback inbox <span>{items.length}</span></h3>
              <div className="review-visual-filters">
                <label><input type="checkbox" checked={allBoards}
                  onChange={(event) => onAllBoards(event.target.checked)} /> All boards</label>
                <label><input type="checkbox" checked={showResolved}
                  onChange={(event) => onShowResolved(event.target.checked)} /> Resolved</label>
              </div>
              <div className="review-visual-queue-cards">
                {items.map((item) => <button type="button" key={keyOf(item)}
                  className={"review-visual-queue-card" + (keyOf(item) === selectedKey ? " selected" : "")}
                  onClick={() => onSelectTask(keyOf(item))}>
                  <small>{item.project} · {item.task.id}</small>
                  <strong>{item.task.title}</strong>
                  <span>{item.task.review_status === "open" ? "Visual review" : "Response"}</span>
                </button>)}
              </div>
            </> : <span className="review-visual-queue-vertical">QUEUE · {items.length}</span>}
          </aside>
        )}

        <main className="review-visual-center">
          <div className="review-visual-toolbar">
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
          </div>
          {error && <div className="error-banner" role="alert">{error}</div>}
          {openError && <div className="error-banner" role="alert">{openError}</div>}
          {round.criteria.length > 0 && (
            <details className="review-visual-criteria">
              <summary>Quality bar · {round.criteria.length} criteria</summary>
              <ul>{round.criteria.map((criterion, i) => <li key={i}>{criterion}</li>)}</ul>
            </details>
          )}
          <div className="review-visual-viewer">
            {active && <>
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
                <button type="button" className="button secondary"
                  onClick={() => void openOriginal()} title="Open the original in its default application">
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
            onSubmit={() => void submit(outcome)}
            busy={saving}
            disabled={resolved}
            submitDisabled={primaryIncomplete}
            summary={resolved ? (awaitingNextRound ? "Awaiting another round" : "Review resolved")
              : round.mode === "pick" ? (choice ? "Selected " + labelOf(choice) : "Select an option")
              : round.mode === "rank" ? ranking.length + "/" + playable.length + " ranked"
              : pinCount > 0 ? pinCount + " annotations" : "Inspect artwork and choose an outcome"}
          />}
        </main>

        {!focus && feedbackExpanded && <aside className="review-visual-feedback" aria-label="Review feedback">
          <header><strong>YOUR RESPONSE</strong>
            <button type="button" className="button secondary" onClick={() => setFeedbackExpanded(false)}
              aria-label="Hide feedback">×</button></header>
          <div className="review-visual-feedback-content">
            <p className="review-visual-judge">{round.judge}</p>
            <label className="review-visual-response-choice">Your response
              <select aria-label="Review decision" value={outcome}
                disabled={saving || resolved} onChange={(event) => onOutcome(event.target.value)}>
                <option value="">Select outcome…</option>
                {outcomeOptions.map((option) => <option key={option.value}
                  value={option.value}>{option.label}</option>)}
              </select>
            </label>
            <label className="review-note">Overall note
              <textarea rows={6} value={note} disabled={resolved}
                onChange={(event) => setNote(event.target.value)}
                placeholder="Tell the agent what to keep or change." />
            </label>
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
            <details className="review-visual-references">
              <summary>References · {round.references.length + (round.previous ? 1 : 0)}</summary>
              {round.previous && <p>Previous: {round.previous.label}</p>}
              {round.references.map((reference) => <p key={reference.id}>{reference.label}</p>)}
              {!round.previous && round.references.length === 0 && <p>No references supplied.</p>}
            </details>
          </div>
        </aside>}
      </div>
    </section>
  );
}
