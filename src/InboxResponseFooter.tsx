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
  return [
    primary[mode],
    { value: "rejected", label: "Request changes" },
    ...(mode === "approve" || mode === "pick"
      ? [{ value: "more-variants", label: "More variants" }] : []),
  ];
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
  onSubmit: () => void;
  onComplete?: () => void;
  busy: boolean;
  disabled?: boolean;
  submitDisabled?: boolean;
  summary?: string;
};

export function InboxResponseFooter({
  mode, outcome, outcomes = [], onOutcome, onSkip, onSnooze, onSubmit, onComplete,
  busy, disabled = false, submitDisabled = false, summary,
}: Props) {
  const [snooze, setSnooze] = useState<number>(24 * 60 * 60 * 1000);
  const canSend = !busy && !disabled && !submitDisabled &&
    (mode === "reply" || outcomes.some((option) => option.value === outcome));
  return (
    <footer className="inbox-response-footer" aria-label="Inbox response actions">
      <div className="inbox-response-left">
        <button className="button secondary" type="button" disabled={busy} onClick={onSkip}>
          Skip
        </button>
        <div className="inbox-response-snooze">
          <button className="button secondary" type="button" disabled={busy}
            onClick={() => onSnooze(snooze)} title="Hide this item for the chosen duration">
            Snooze
          </button>
          <select aria-label="Snooze duration" value={snooze} disabled={busy}
            onChange={(event) => setSnooze(Number(event.target.value))}>
            {SNOOZE_DURATIONS.map((duration) =>
              <option key={duration.ms} value={duration.ms}>{duration.label}</option>)}
          </select>
        </div>
      </div>
      {summary && <span className="inbox-response-summary">{summary}</span>}
      <div className="inbox-response-right">
        <label className="inbox-response-outcome">Outcome
          {mode === "visual" ?
            <select aria-label="Response outcome" value={outcome} disabled={busy || disabled}
              onChange={(event) => onOutcome(event.target.value)}>
              <option value="">Select outcome…</option>
              {outcomes.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
            </select>
            : <span>Respond</span>}
        </label>
        {mode === "reply" && onComplete && (
          <button className="button secondary" type="button"
            disabled={busy || disabled} onClick={onComplete}
            title="Accept and close this request; for a human-owned task, also move it to Done">
            ✅ Done
          </button>
        )}
        <button className="button primary inbox-response-submit" type="button"
          disabled={!canSend} onClick={onSubmit} title={mode === "visual" && !outcome ? "Choose an outcome before submitting" : undefined}>
          {busy ? "Saving…" : "Submit & Next →"}
        </button>
      </div>
    </footer>
  );
}
