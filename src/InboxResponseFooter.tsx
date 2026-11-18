import { useState } from "react";
import type { ReviewRound } from "./reviewApi";

export type InboxOutcome = { value: string; label: string };
export function reviewOutcomes(mode: ReviewRound["mode"]): InboxOutcome[] {
  const primary: Record<ReviewRound["mode"], InboxOutcome> = {
    approve: { value: "approved", label: "Approve" },
    pick: { value: "picked", label: "Pick selected option" },
    rank: { value: "ranked", label: "Submit ranking" },
    markup: { value: "marked", label: "Send annotated feedback" },
  };
  return [primary[mode], { value: "rejected", label: "Request changes" },
    ...(mode === "approve" || mode === "pick" ?
      [{ value: "more-variants", label: "More variants" }] : [])];
}
export const SNOOZE_DURATIONS = [
  { label: "1 hour", ms: 60 * 60 * 1000 },
  { label: "24 hours", ms: 24 * 60 * 60 * 1000 },
  { label: "1 week", ms: 7 * 24 * 60 * 60 * 1000 },
] as const;

type Props = {
  mode: "reply" | "visual";
  outcome: string;
  outcomes?: InboxOutcome[];
  onOutcome: (value: string) => void;
  onSkip: () => void;
  onSnooze: (milliseconds: number) => void;
  onSubmit: (verdict?: string) => void;
  onComplete?: () => void;
  completeLabel?: string;
  onReopen?: () => void;
  onUndoLast?: () => void;
  onNoteOnly?: () => void;
  onCloseTask?: () => void;
  notePresent?: boolean;
  primaryLabel?: string;
  busy: boolean;
  disabled?: boolean;
  submitDisabled?: boolean;
  summary?: string;
};

export function InboxResponseFooter({
  mode, outcomes = [], onOutcome, onSkip, onSnooze, onSubmit,
  onComplete, completeLabel = "Done", onReopen, onUndoLast, onNoteOnly, onCloseTask, notePresent = false, primaryLabel,
  busy, disabled = false, submitDisabled = false, summary,
}: Props) {
  const [snooze, setSnooze] = useState<number>(24 * 60 * 60 * 1000);
  return <footer className="inbox-response-footer" aria-label="Inbox response actions">
    <div className="inbox-response-left">
      {onUndoLast && <button className="button secondary inbox-undo-answer" type="button"
        disabled={busy} onClick={onUndoLast} title="Return to the last answered question">↶ Undo last answer</button>}
      <button className="button secondary" type="button" disabled={busy} onClick={onSkip}>Skip</button>
      <div className="inbox-response-snooze">
        <button className="button secondary" type="button" disabled={busy}
          onClick={() => onSnooze(snooze)} title="Hide this item for the chosen duration">Snooze</button>
        <select aria-label="Snooze duration" value={snooze} disabled={busy}
          onChange={(event) => setSnooze(Number(event.target.value))}>
          {SNOOZE_DURATIONS.map((span) =>
            <option key={span.ms} value={span.ms}>{span.label}</option>)}
        </select>
      </div>
    </div>
    {summary && <span className="inbox-response-summary">{summary}</span>}
    <div className="inbox-response-right">
      {mode === "visual" ? <>
        {notePresent && onNoteOnly && <button className="button secondary" type="button"
          disabled={busy || disabled} onClick={onNoteOnly}
          title="Save your note on the task; the review remains open">Save note only ↵</button>}
        {onCloseTask && <button className="button secondary" type="button" disabled={busy || disabled}
          onClick={onCloseTask} title="Move this task to Done without approving, rejecting, picking or ranking any review option">
          Close task · no decision
        </button>}
        {outcomes.map((option, index) => <button key={option.value}
          className={index === 0 ? "button primary" : "button secondary"} type="button"
          disabled={busy || disabled || (index === 0 && submitDisabled)}
          onClick={() => { onOutcome(option.value); onSubmit(option.value); }}
          title={index === 0 && submitDisabled ? "Choose an option in the filmstrip, or rank every option first" : undefined}>
          {busy ? "Saving…" : index === 0 && primaryLabel ? primaryLabel : option.label}
          {index === 0 && notePresent ? " + note" : ""}
        </button>)}
      </> : <>
        {!onReopen && <button className="button primary" type="button"
          disabled={busy || disabled || submitDisabled || !notePresent} onClick={() => onSubmit()}
          title={submitDisabled ? "Choose at least one answer in the questionnaire" : !notePresent ? "Choose an option or write an answer" : "Send your answer and return this task to AI Ready"}>
          Submit ↵
        </button>}
        {onReopen && <button className="button secondary" type="button"
          disabled={busy || disabled} onClick={onReopen}>Needs changes / Reopen</button>}
        {onComplete && <button className={onReopen ? "button primary" : "button secondary"} type="button"
          disabled={busy || disabled} onClick={onComplete} title={onReopen
            ? "Accept this completed implementation"
            : "Close this task without requesting any changes"}>{completeLabel}</button>}
      </>}
    </div>
  </footer>;
}
