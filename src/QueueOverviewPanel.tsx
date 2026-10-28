import { useCallback, useEffect, useState } from "react";
import {
  taskQueueOverview, taskExecutionIntent, taskSetExecutionMode, taskSyncQueue, taskQueueReprioritize, taskQueueMove,
  type QueueOverview, type QueueOverviewItem,
} from "./api";
import "./QueueOverviewPanel.css";

type Props = {
  boardPath: string;
  onClose: () => void;
  onOpenTask: (taskId: string) => void;
  onChanged?: () => void;
};

const statusLabel = (item: QueueOverviewItem) => {
  const pickup = item.pickup as { status?: unknown; state?: unknown } | null;
  const reported = typeof pickup?.status === "string" ? pickup.status
    : typeof pickup?.state === "string" ? pickup.state : null;
  if (reported) return `${item.state} · ${reported}`;
  if (item.startupState === "reserved") return "Capacity reserved · launch not attempted";
  if (item.startupState === "starting") return "Launch submitted · reply not verified";
  if (item.startupState === "indeterminate") return "Launch outcome UNKNOWN · reconciliation required";
  if (item.state === "pending") return "Pending · pickup not verified";
  return item.state;
};

/** The native bridge reads OpenRig's exact board-scoped canonical queue.
 * No second job registry, launch poller, or inferred provider status. */
export function QueueOverviewPanel({ boardPath, onClose, onOpenTask, onChanged }: Props) {
  const [snapshot, setSnapshot] = useState<QueueOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [actionWarning, setActionWarning] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const result = await taskQueueOverview(boardPath);
      setSnapshot(result);
      setError(null);
    } catch (cause) {
      setSnapshot(null);
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [boardPath]);

  useEffect(() => {
    let active = true;
    const poll = () => {
      void taskQueueOverview(boardPath).then(result => {
        if (!active) return;
        setSnapshot(result);
        setError(null);
      }).catch(cause => {
        if (!active) return;
        setSnapshot(null);
        setError(cause instanceof Error ? cause.message : String(cause));
      });
    };
    poll();
    const timer = window.setInterval(poll, 15000);
    return () => { active = false; window.clearInterval(timer); };
  }, [boardPath]);

  const reprioritize = async (item: QueueOverviewItem, priority: "routine" | "urgent" | "critical") => {
    if (!item.taskId || busyId || item.priority === priority) return;
    setActionWarning(null);
    setBusyId(item.qitemId);
    try {
      const result = await taskQueueReprioritize(item.taskId, item.qitemId, priority, boardPath);
      if (result.status !== "registered")
        setActionWarning(`${item.taskId}: ${result.status} — ${result.reason ?? "inspect the exact OpenRig receipt"}`);
      await refresh();
    } catch (cause) {
      setActionWarning(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusyId(null);
    }
  };

  const movePending = async (item: QueueOverviewItem, direction: "up" | "down") => {
    if (!item.taskId || busyId) return;
    setBusyId(item.qitemId);
    setActionWarning(null);
    try {
      const result = await taskQueueMove(item.taskId, item.qitemId, direction, boardPath);
      if (result.status !== "registered")
        setActionWarning(`${item.taskId}: ${result.status} — ${result.reason ?? "verify OpenRig order receipt"}`);
      await refresh();
    } catch (cause) {
      setActionWarning(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusyId(null);
    }
  };

  const cancel = async (item: QueueOverviewItem) => {
    if (!item.taskId || busyId) return;
    setActionWarning(null);
    setBusyId(item.qitemId);
    try {
      const intent = await taskExecutionIntent(item.taskId, boardPath);
      if (intent.mode !== "queue" || intent.queue_receipt !== item.qitemId) {
        throw new Error("TaskBoard Queue receipt no longer matches this OpenRig item. No cancellation sent.");
      }
      // This first saves a human Backlog intent with an optimistic revision.
      // Native sync then checks the original queue row and performs a guarded
      // pending-only cancel. Timeout/UNKNOWN is not replayed automatically.
      await taskSetExecutionMode(item.taskId, intent.revision, "backlog", boardPath);
      const result = await taskSyncQueue(item.taskId, boardPath);
      if (result.status !== "canceled" && result.status !== "terminal") {
        setActionWarning(`${item.taskId}: ${result.status} — ${result.reason ?? "exact queue reconciliation required"}`);
      }
      onChanged?.();
      await refresh();
    } catch (cause) {
      setActionWarning(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <aside className="omni-queue-drawer" aria-label="OpenRig queued work" role="dialog" aria-modal="false">
      <div className="omni-queue-heading">
        <div>
          <strong>Queue</strong>
          <span>{snapshot ? snapshot.hasMore ? `${snapshot.limit}+ active`
            : `${snapshot.activeCount ?? "?"} active` : "Status unavailable"}</span>
        </div>
        <button type="button" className="button secondary" onClick={() => void refresh()}>Refresh</button>
        <button type="button" className="omni-queue-close" onClick={onClose} aria-label="Close queue overview">×</button>
      </div>
      {error && <p className="omni-queue-warning" role="status">OpenRig unavailable: {error}. No capacity or job state inferred.</p>}
      {actionWarning && <p className="omni-queue-warning" role="status">{actionWarning}</p>}
      {snapshot?.hasMore && <p className="omni-queue-warning">Queue list truncated. Counts are not exact.</p>}
      {snapshot && !snapshot.items.length && <p className="omni-queue-empty">No active queued jobs on this board.</p>}
      <div className="omni-queue-items">
        {snapshot?.items.map(item => (
          <article className="omni-queue-item" key={item.qitemId}>
            <div className="omni-queue-line">
              <strong>{item.taskId ?? "Unverified task reference"}</strong>
              <span>{item.priority}</span>
            </div>
            <p>{item.summary ?? "No title available from OpenRig"}</p>
            <small>{statusLabel(item)}</small>
            <div className="omni-queue-actions">
              <button type="button" disabled={!item.taskId}
                onClick={() => { if (item.taskId) onOpenTask(item.taskId); }}>Open task</button>
              {item.state === "pending" && !["starting","indeterminate"].includes(item.startupState ?? "") && (
                <>
                  <label className="omni-queue-priority-label">
                    Priority
                    <select aria-label={`Priority for ${item.taskId ?? item.qitemId}`}
                      value={item.priority} disabled={!!busyId || !item.taskId}
                      onChange={event => void reprioritize(item, event.currentTarget.value as "routine" | "urgent" | "critical")}>
                      <option value="routine">Routine</option>
                      <option value="urgent">Urgent</option>
                      <option value="critical">Critical</option>
                    </select>
                  </label>
                  <button type="button" disabled={!!busyId || !item.taskId ||
                    !snapshot?.items.some(other => other.priority === item.priority &&
                      other.qitemId !== item.qitemId && other.state === "pending" &&
                      snapshot.items.indexOf(other) < snapshot.items.indexOf(item))}
                    title="Move ahead within the same priority group" aria-label={`Move ${item.taskId} earlier`}
                    onClick={() => void movePending(item, "up")}>↑</button>
                  <button type="button" disabled={!!busyId || !item.taskId ||
                    !snapshot?.items.some(other => other.priority === item.priority &&
                      other.qitemId !== item.qitemId && other.state === "pending" &&
                      snapshot.items.indexOf(other) > snapshot.items.indexOf(item))}
                    title="Move later within the same priority group" aria-label={`Move ${item.taskId} later`}
                    onClick={() => void movePending(item, "down")}>↓</button>
                  <button type="button" disabled={!!busyId || !item.taskId}
                    onClick={() => void cancel(item)}>{busyId === item.qitemId ? "Updating…" : "Cancel queue"}</button>
                </>
              )}
              {item.t3SessionId
                ? <span>Linked T3: {item.t3SessionId}</span>
                : <span title="OW-0017 worker pickup/binding not yet available">T3 link unverified</span>}
            </div>
          </article>
        ))}
      </div>
    </aside>
  );
}
