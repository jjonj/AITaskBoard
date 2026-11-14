import { useLayoutEffect, useRef, useState, type TextareaHTMLAttributes } from "react";
import { parseDescriptionTimeline, updateInitialDescription, type DescriptionUpdateKind } from "./descriptionTimeline";
import { MarkdownText } from "./MarkdownText";
import type { Task } from "./types";

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
  task?: Task;
  description: string;
  onChange: (description: string) => void;
  /** User-only Note to AI composer; omitted for epics. */
  noteToAi?: string;
  onNoteToAiChange?: (value: string) => void;
  legacyAiNotes?: string;
  /** Task creation uses the space normally occupied by the Note to AI composer. */
  descriptionMinRows?: number;
  descriptionMaxHeight?: number;
  /** An agent has read or worked on this task; use Note to AI instead of editing its original description. */
  readOnlyDescription?: boolean;
};

const updateLabel = (kind: DescriptionUpdateKind) => ({
  update: "Update",
  "user-note": "Note to AI",
  "legacy-ai": "Imported AI notes",
  blocked: "Blocked",
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
  task, description, onChange, noteToAi, onNoteToAiChange, legacyAiNotes,
  descriptionMinRows = 3, descriptionMaxHeight = 360, readOnlyDescription = false,
}: Props) {
  const { initial, updates } = parseDescriptionTimeline(description);
  const [showAll, setShowAll] = useState(false);
  const [previewDescription, setPreviewDescription] = useState(false);
  const hiddenCount = showAll ? 0 : Math.max(0, updates.length - VISIBLE_UPDATES);
  const visible = updates.slice(hiddenCount);

  return (
    <section className="description-timeline">
      <div className="description-timeline-heading">
        <strong title={readOnlyDescription ? "An agent has read or claimed this task. Use Note to AI for changes." : undefined}>Description</strong>
        {updates.length > 0 && (
          <span>{updates.length} update{updates.length === 1 ? "" : "s"}</span>
        )}
        {!readOnlyDescription && <button type="button" className="description-markdown-mode"
          onClick={() => setPreviewDescription((current) => !current)}
          aria-pressed={previewDescription}>
          {previewDescription ? "Edit Markdown" : "Preview Markdown"}
        </button>}
      </div>
      {previewDescription || readOnlyDescription
        ? <MarkdownText task={task} text={initial || "*No description yet.*"} className="markdown-field-preview description-markdown-preview" />
        : <AutoTextarea
            className="description-initial"
            minRows={descriptionMinRows}
            maxHeight={descriptionMaxHeight}
            aria-label="Task description"
            placeholder="Describe the task and the desired outcome…"
            value={initial}
            onChange={(event) => onChange(updateInitialDescription(description, event.currentTarget.value))}
          />}

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
              <MarkdownText task={task} text={entry.text} className="description-entry-text" />
            </li>
          ))}
          {legacyAiNotes?.trim() && (
            <li className="description-entry legacy-ai">
              <div className="description-entry-header">
                <strong>Existing AI notes</strong>
                <time>moved into history on save</time>
              </div>
              <MarkdownText task={task} text={legacyAiNotes} className="description-entry-text" />
            </li>
          )}
          {showAll && updates.length > VISIBLE_UPDATES && (
            <li className="description-history-more">
              <button type="button" onClick={() => setShowAll(false)}>Show fewer</button>
            </li>
          )}
        </ol>
      )}

      {onNoteToAiChange && (
        <div className="description-composer note">
          <div className="description-composer-tabs">
            <strong className="note">✎ Note to AI</strong>
            <span className="description-composer-hint">Clears For user and flags AI attention on save</span>
          </div>
          <AutoTextarea
            minRows={2}
            maxHeight={240}
            aria-label="Note to AI"
            placeholder="Instructions for the AI working on this task…"
            value={noteToAi ?? ""}
            onChange={(event) => onNoteToAiChange(event.currentTarget.value)}
          />
        </div>
      )}
    </section>
  );
}
