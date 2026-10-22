import { useEffect, useLayoutEffect, useRef, useState, type TextareaHTMLAttributes } from "react";
import { parseDescriptionTimeline, updateInitialDescription } from "./descriptionTimeline";

type AutoTextareaProps = TextareaHTMLAttributes<HTMLTextAreaElement> & {
  minRows?: number;
  /** Upper bound in px; beyond it the field scrolls internally. */
  maxHeight?: number;
};

/** Textarea that grows with its content instead of reserving fixed rows. */
export function AutoTextarea({ minRows = 2, maxHeight = 420, value, style, ...props }: AutoTextareaProps) {
  const ref = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    const height = Math.min(el.scrollHeight + 2, maxHeight);
    el.style.height = `${height}px`;
    el.style.overflowY = el.scrollHeight + 2 > maxHeight ? "auto" : "hidden";
  }, [value, maxHeight]);
  return <textarea ref={ref} rows={minRows} value={value} style={{ resize: "none", ...style }} {...props} />;
}

type Props = {
  description: string;
  onChange: (description: string) => void;
  pendingUpdate: string;
  onPendingUpdateChange: (value: string) => void;
  /** Note to AI composer tab; omitted for epics (they keep separate notes). */
  noteToAi?: string;
  onNoteToAiChange?: (value: string) => void;
  legacyAiNotes?: string;
};

const updateLabel = (kind: "update" | "user-note" | "legacy-ai") => ({
  update: "Update",
  "user-note": "Note to AI",
  "legacy-ai": "Imported AI notes",
})[kind];

const VISIBLE_UPDATES = 2;

function relativeTime(iso: string) {
  const diff = Date.now() - Date.parse(iso);
  if (!Number.isFinite(diff)) return iso;
  const minutes = Math.round(diff / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 14) return `${days}d ago`;
  return new Date(iso).toLocaleDateString();
}

export function DescriptionTimeline({
  description, onChange, pendingUpdate, onPendingUpdateChange, noteToAi, onNoteToAiChange, legacyAiNotes,
}: Props) {
  const { initial, updates } = parseDescriptionTimeline(description);
  const [showAll, setShowAll] = useState(false);
  const hasNoteTab = !!onNoteToAiChange;
  const [tab, setTab] = useState<"update" | "note">(
    hasNoteTab && noteToAi?.trim() ? "note" : "update");
  const activeTab = hasNoteTab ? tab : "update";
  const composerRef = useRef<HTMLDivElement>(null);
  const focusComposer = useRef(false);
  useEffect(() => {
    if (!focusComposer.current) return;
    focusComposer.current = false;
    composerRef.current?.querySelector("textarea")?.focus();
  }, [tab]);
  const chooseTab = (next: "update" | "note") => { focusComposer.current = true; setTab(next); };
  const hiddenCount = showAll ? 0 : Math.max(0, updates.length - VISIBLE_UPDATES);
  const visible = updates.slice(hiddenCount);

  return (
    <section className="description-timeline">
      <div className="description-timeline-heading">
        <strong>Description</strong>
        {updates.length > 0 && (
          <span>{updates.length} update{updates.length === 1 ? "" : "s"}</span>
        )}
      </div>
      <AutoTextarea
        className="description-initial"
        minRows={3}
        maxHeight={360}
        aria-label="Task description"
        placeholder="Describe the task and the desired outcome…"
        value={initial}
        onChange={(event) => onChange(updateInitialDescription(description, event.currentTarget.value))}
      />

      {(updates.length > 0 || legacyAiNotes?.trim()) && (
        <ol className="description-history">
          {hiddenCount > 0 && (
            <li className="description-history-more">
              <button type="button" onClick={() => setShowAll(true)}>
                Show {hiddenCount} earlier update{hiddenCount === 1 ? "" : "s"}
              </button>
            </li>
          )}
          {visible.map((entry, index) => (
            <li key={hiddenCount + index} className={`description-entry ${entry.kind}`}>
              <div className="description-entry-header">
                <strong>{updateLabel(entry.kind)}</strong>
                <time dateTime={entry.at} title={new Date(entry.at).toLocaleString()}>
                  {relativeTime(entry.at)}
                </time>
              </div>
              <div className="description-entry-text">{entry.text}</div>
            </li>
          ))}
          {legacyAiNotes?.trim() && (
            <li className="description-entry legacy-ai">
              <div className="description-entry-header">
                <strong>Existing AI notes</strong>
                <time>moved into history on save</time>
              </div>
              <div className="description-entry-text">{legacyAiNotes}</div>
            </li>
          )}
          {showAll && updates.length > VISIBLE_UPDATES && (
            <li className="description-history-more">
              <button type="button" onClick={() => setShowAll(false)}>Show fewer</button>
            </li>
          )}
        </ol>
      )}

      <div ref={composerRef} className={`description-composer ${activeTab}`}>
        <div className="description-composer-tabs" role="tablist">
          <button type="button" role="tab" aria-selected={activeTab === "update"}
            className={activeTab === "update" ? "active" : ""} onClick={() => chooseTab("update")}>
            + Add update{pendingUpdate.trim() && activeTab !== "update" ? " •" : ""}
          </button>
          {hasNoteTab && (
            <button type="button" role="tab" aria-selected={activeTab === "note"}
              className={`note${activeTab === "note" ? " active" : ""}`} onClick={() => chooseTab("note")}>
              ✎ Note to AI{noteToAi?.trim() && activeTab !== "note" ? " •" : ""}
            </button>
          )}
          <span className="description-composer-hint">
            {activeTab === "note"
              ? "Clears For user and flags AI attention on save"
              : "Appended as a timestamped entry on save"}
          </span>
        </div>
        {activeTab === "update" ? (
          <AutoTextarea
            key="update"
            minRows={2}
            maxHeight={240}
            aria-label="Add description update"
            placeholder="Progress, decisions, observations or context…"
            value={pendingUpdate}
            onChange={(event) => onPendingUpdateChange(event.currentTarget.value)}
          />
        ) : (
          <AutoTextarea
            key="note"
            minRows={2}
            maxHeight={240}
            aria-label="Note to AI"
            placeholder="Instructions for the AI working on this task…"
            value={noteToAi ?? ""}
            onChange={(event) => onNoteToAiChange?.(event.currentTarget.value)}
          />
        )}
      </div>
    </section>
  );
}
