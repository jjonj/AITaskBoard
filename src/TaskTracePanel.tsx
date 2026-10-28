import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./TaskTracePanel.css";

type TraceRow = {
  queueItemId: string;
  state: string;
  createdAt: string;
  claimedAt: string | null;
  destinationSession: string;
  lastDeliveryResult: string | null;
  seat?: { resolution: string; id: string | null };
  t3?: { available: boolean; primary: {
    managedSessionId: string; status: string; conversationId: string | null;
    browserSessionId: string | null;
  } | null };
  queueResult?: { resolution: string | null; closureReason: string | null;
    taskBoardCompletionVerified: boolean };
  recordedWakes?: { entries: Array<{ deliveryState: string; pickupVerified: boolean }> };
};
type Trace = {
  schema: "openrig-taskboard-trace/v1";
  linked: boolean;
  available: boolean;
  observedAt: string;
  hasMore: boolean;
  items: TraceRow[];
};

function deliveryStatus(row: TraceRow) {
  if (row.queueResult?.resolution) return `Queue resolved: ${row.queueResult.resolution}`;
  if (row.lastDeliveryResult && /unknown|error|failed|timeout/i.test(row.lastDeliveryResult))
    return `Transport uncertain: ${row.lastDeliveryResult}`;
  if (row.state === "pending" && row.t3?.primary?.status !== "active")
    return row.t3?.primary ? "Waiting for verified agent pickup" : "Waiting for worker session";
  if (row.state === "claimed") return "Queue claimed; TaskBoard completion not proven";
  return `Queue: ${row.state} (execution not inferred)`;
}

export function TaskTracePanel({ path, taskId }: { path: string; taskId: string }) {
  const [open, setOpen] = useState(false);
  const [trace, setTrace] = useState<Trace | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setTrace(null); setError(null); setOpen(false);
  }, [path, taskId]);
  useEffect(() => {
    if (!open) return;
    let alive = true;
    const refresh = () => {
      void invoke<Trace>("task_openrig_trace", { path, id: taskId })
        .then(result => {
          if (!alive) return;
          setTrace(result); setError(null);
        }).catch(cause => {
          if (!alive) return;
          setTrace(null); setError(cause instanceof Error ? cause.message : String(cause));
        });
    };
    refresh();
    const timer = window.setInterval(refresh, 20000);
    return () => { alive = false; window.clearInterval(timer); };
  }, [open, path, taskId]);
  return <section className="task-trace-panel">
    <button type="button" className="task-trace-toggle" aria-expanded={open}
      onClick={() => setOpen(value => !value)}>
      {open ? "▾" : "▸"} OpenRig execution trace
    </button>
    {open && <div className="task-trace-content">
      {error && <p className="task-trace-warning">Trace unavailable: {error}. No result inferred.</p>}
      {!error && !trace && <p>Checking canonical OpenRig trace…</p>}
      {trace && !trace.linked && <p>No explicitly linked OpenRig queue history for this exact task.
        No agent pickup or result inferred from the title or claimant.</p>}
      {trace?.hasMore && <p className="task-trace-warning">History is truncated; older entries remain in OpenRig.</p>}
      {trace?.items.map(row => <article key={row.queueItemId} className="task-trace-item">
        <div className="task-trace-line"><strong>{deliveryStatus(row)}</strong>
          <small>{row.state}</small></div>
        <div>OpenRig item: <code>{row.queueItemId}</code></div>
        <div>Destination: <code>{row.destinationSession}</code></div>
        {row.seat?.id && <div>Exact OpenRig owner: <code>{row.seat.id}</code></div>}
        {row.seat && row.seat.resolution !== "exact" &&
          <div className="task-trace-warning">Owner {row.seat.resolution}; no guessed seat.</div>}
        {row.t3?.primary && <>
          <div>Managed T3: <code>{row.t3.primary.managedSessionId}</code> ({row.t3.primary.status})</div>
          {row.t3.primary.browserSessionId && <div>Browser session:
            <code>{row.t3.primary.browserSessionId}</code></div>}
          {row.t3.primary.conversationId && <div>Conversation:
            <code>{row.t3.primary.conversationId}</code></div>}
        </>}
        {row.queueResult?.closureReason && <div>Queue closure: {row.queueResult.closureReason}</div>}
        <small>Recorded: {row.createdAt}. Queue closure does not prove task completion.</small>
      </article>)}
    </div>}
  </section>;
}
