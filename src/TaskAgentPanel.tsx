import { useCallback, useEffect, useRef, useState } from "react";
import { taskAgentAction, taskAgentStatus, type TaskAgentStatus } from "./api";
import type { Task } from "./types";
import "./TaskAgentPanel.css";

const needsAttention = new Set(["waiting", "pending", "snoozed"]);

/** Existing task-bound T3 session only; actions never start a new agent. */
export function TaskAgentPanel({ path, task }: { path: string; task: Task }) {
  const [open, setOpen] = useState(false);
  const [state, setState] = useState<TaskAgentStatus | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const [interval, setIntervalValue] = useState("10");
  const [duration, setDuration] = useState("");
  const [reboot, setReboot] = useState("60");
  const [runs, setRuns] = useState("5");
  const [delay, setDelay] = useState("30");
  const [prompt, setPrompt] = useState("");
  const [depth, setDepth] = useState<"light" | "medium" | "heavy">("medium");
  const [request, setRequest] = useState("");
  const container = useRef<HTMLDivElement>(null);
  const pendingKey = "task-agent:prepfile:" + path + ":" + task.id;
  const [pending, setPending] = useState(() => localStorage.getItem(pendingKey) ?? "");

  const refresh = useCallback(async () => {
    try {
      const next = await taskAgentStatus(path, task.id);
      setState(next);
      setError("");
    } catch (cause) {
      setState(null);
      setError(String(cause));
    }
  }, [path, task.id]);

  // Opening the menu is the only reason to fetch session data; closing stops polling.
  useEffect(() => {
    if (!open) return;
    void refresh();
    const timer = window.setInterval(() => void refresh(), 15_000);
    return () => window.clearInterval(timer);
  }, [open, refresh]);

  useEffect(() => {
    if (!open) return;
    const onPointerDown = (event: PointerEvent) => {
      if (!container.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopPropagation();
        setOpen(false);
        container.current?.querySelector<HTMLButtonElement>(".task-agent-trigger")?.focus();
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown, true);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown, true);
    };
  }, [open]);

  const invoke = async (command: string, options: Record<string, unknown> = {}) => {
    if (busy) return;
    setBusy(true);
    setNotice("");
    setError("");
    try {
      await taskAgentAction(path, task.id, command, options);
      setNotice(command === "caffy_pause" ? "Paused" :
        command === "caffy_resume" ? "Resumed" :
        command === "caffy_wake" ? "Wake requested" :
        command === "caffy_stop" ? "Scheduler stopped" : "Updated");
      await refresh();
    } catch (cause) {
      setError("Not confirmed. Check T3 before repeating: " + String(cause));
    } finally {
      setBusy(false);
    }
  };
  const integer = (value: string, min: number, max: number) => {
    const n = Number(value);
    return /^\d+$/.test(value) && n >= min && n <= max ? n : null;
  };
  const field = (name: string) => state?.caffy?.[name] == null ? "" : String(state.caffy[name]);
  const setMode = (mode: "idle" | "periodic" | "startup") => {
    const minutes = duration.trim() ? integer(duration, 1, 10080) : null;
    const step = integer(interval, 1, 1440);
    if ((duration.trim() && minutes == null) || (mode === "periodic" && step == null)) {
      setError("Duration must be 1-10080 min (or blank); interval 1-1440 min.");
      return;
    }
    void invoke("caffy_set", { mode, ...(minutes ? { durationMinutes: minutes } : {}),
      ...(mode === "periodic" ? { intervalMinutes: step } : {}), startup: mode === "startup" });
  };
  const attentions = state?.attentions?.filter(a => needsAttention.has(a.status)) ?? [];
  const paused = field("paused") === "true";

  return <div className="task-agent-menu" ref={container}>
    <button type="button" className="button secondary task-agent-trigger"
      aria-expanded={open} aria-haspopup="dialog" title="Controls for this task's linked T3 session"
      onClick={() => {
        if (!open) { setState(null); setError(""); setNotice(""); }
        setOpen(value => !value);
      }}>
      Agent <span aria-hidden="true">{open ? "▴" : "▾"}</span>
    </button>
    {open && <div className="task-agent-popover" role="dialog" aria-label="Linked T3 agent controls">
      <div className="task-agent-heading">
        <div className="task-agent-identity">
          <strong>Linked agent</strong>
          {state && <span className="task-agent-status" title={state.statusObservedAt ?? ""}>
            {state.activity ?? state.status ?? "Unknown"}
          </span>}
        </div>
        <button type="button" className="task-agent-refresh" aria-label="Refresh agent status"
          title="Refresh" disabled={busy} onClick={() => void refresh()}>↻</button>
      </div>
      {!state && <p className={error ? "task-agent-warning" : "task-agent-muted"}>
        {error ? "Agent unavailable. " + error : "Checking linked session…"}
      </p>}
      {state && <>
        {state.title && <div className="task-agent-title" title={state.title}>{state.title}</div>}
        <div className="task-agent-actions">
          <button type="button" disabled={busy || !state.caffy}
            onClick={() => void invoke(paused ? "caffy_resume" : "caffy_pause")}>
            {paused ? "Resume" : "Pause"}
          </button>
          <button type="button" disabled={busy} onClick={() => void invoke("caffy_wake")}>Wake now</button>
          <button type="button" disabled={busy || !state.caffy} onClick={() => {
            if (window.confirm("Stop auto-resume for this agent?")) void invoke("caffy_stop");
          }}>Stop</button>
        </div>
        <div className="task-agent-muted">
          {state.caffy ? "Auto-resume: " + (field("mode") || "configured") +
            (paused ? " (paused)" : "") : "Auto-resume is not configured"}
        </div>
        {!!attentions.length && <section className="task-agent-attention">
          <strong>Needs attention ({attentions.length})</strong>
          {attentions.map(a => <div className="task-agent-attention-item" key={a.id}>
            <p>{a.message}</p>
            <div className="task-agent-actions">
              <button type="button" disabled={busy} onClick={() => void invoke("attention_open", { requestId: a.id })}>Acknowledge</button>
              <button type="button" disabled={busy} onClick={() => void invoke("attention_snooze", { requestId: a.id, minutes: 15 })}>Snooze 15m</button>
              <button type="button" disabled={busy} onClick={() => void invoke("attention_dismiss", { requestId: a.id })}>Dismiss</button>
            </div>
          </div>)}
        </section>}
        <details className="task-agent-details">
          <summary>Scheduling options</summary>
          <div className="task-agent-fields">
            <label>Duration (min)<input inputMode="numeric" value={duration} placeholder="Unlimited"
              onChange={e => setDuration(e.target.value)} /></label>
            <label>Interval (min)<input inputMode="numeric" value={interval}
              onChange={e => setIntervalValue(e.target.value)} /></label>
          </div>
          <div className="task-agent-actions">
            <button type="button" disabled={busy} onClick={() => setMode("idle")}>On idle</button>
            <button type="button" disabled={busy} onClick={() => setMode("periodic")}>Periodic</button>
            <button type="button" disabled={busy} onClick={() => setMode("startup")}>On startup</button>
          </div>
          <div className="task-agent-fields task-agent-schedule">
            <label>Start in (min)<input inputMode="numeric" value={delay}
              onChange={e => setDelay(e.target.value)} /></label>
            <label>Prompt (optional)<input value={prompt} onChange={e => setPrompt(e.target.value.slice(0, 8000))} /></label>
          </div>
          <div className="task-agent-actions">
            <button type="button" disabled={busy} onClick={() => {
              const n = integer(delay, 1, 129600);
              if (!n) { setError("Delay must be 1-129600 min."); return; }
              void invoke("caffy_begin_in", { beginAt: Date.now() + n * 60000, ...(prompt ? { prompt } : {}) });
            }}>Schedule start</button>
            <button type="button" disabled={busy} onClick={() => void invoke("caffy_cancel_begin")}>Cancel start</button>
          </div>
          <details className="task-agent-subdetails">
            <summary>Automatic reboot</summary>
            <div className="task-agent-fields">
              <label>After (min)<input inputMode="numeric" value={reboot} onChange={e => setReboot(e.target.value)} /></label>
              <label>Or after runs<input inputMode="numeric" value={runs} onChange={e => setRuns(e.target.value)} /></label>
            </div>
            <div className="task-agent-actions">
              <button type="button" disabled={busy} onClick={() => {
                const n = integer(reboot, 1, 525600);
                if (!n) { setError("Reboot minutes invalid."); return; }
                void invoke("caffy_auto_reboot", { rebootMinutes: n });
              }}>By time</button>
              <button type="button" disabled={busy} onClick={() => {
                const n = integer(runs, 1, 10000);
                if (!n) { setError("Reboot runs invalid."); return; }
                void invoke("caffy_auto_reboot", { rebootEveryRuns: n });
              }}>By runs</button>
              <button type="button" disabled={busy} onClick={() => void invoke("caffy_auto_reboot")}>Disable</button>
            </div>
          </details>
          <div className="task-agent-actions">
            <button type="button" disabled={busy || !state.caffy}
              onClick={() => void invoke("caffy_startup", { startup: field("startup") !== "true" })}>
              {field("startup") === "true" ? "Disable startup resume" : "Enable startup resume"}
            </button>
          </div>
        </details>
        <details className="task-agent-details">
          <summary>Prepfile</summary>
          <textarea value={request} aria-label="Prepfile research focus" placeholder="Research focus"
            onChange={e => setRequest(e.target.value.slice(0, 6000))} />
          <div className="task-agent-actions">
            <label>Depth<select value={depth} onChange={e => setDepth(e.target.value as typeof depth)}>
              <option value="light">Light</option><option value="medium">Medium</option>
              <option value="heavy">Heavy</option>
            </select></label>
            <button type="button" disabled={busy || !request.trim() || !!pending} onClick={() => {
              const id = crypto.randomUUID();
              localStorage.setItem(pendingKey, id);
              setPending(id);
              void invoke("prepfile", { requestId: id, request: request.trim(), depth });
            }}>Request Prepfile</button>
          </div>
          {pending && <div className="task-agent-warning">Existing request retained until verified.
            <div className="task-agent-actions">
              <button type="button" disabled={busy || !request.trim()} onClick={() =>
                void invoke("prepfile", { requestId: pending, request: request.trim(), depth })}>Retry same request</button>
              <button type="button" disabled={busy} onClick={() => {
                localStorage.removeItem(pendingKey); setPending("");
              }}>Clear after verification</button>
            </div>
          </div>}
        </details>
      </>}
      {error && state && <p role="alert" className="task-agent-warning">{error}</p>}
      {notice && <p role="status" className="task-agent-notice">{notice}</p>}
    </div>}
  </div>;
}
