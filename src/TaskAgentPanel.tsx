import { useCallback, useEffect, useState } from "react";
import { taskAgentAction, taskAgentStatus, type TaskAgentStatus } from "./api";
import type { Task } from "./types";

/** Controls only the T3 session already bound to this exact task. */
export function TaskAgentPanel({ path, task }: { path: string; task: Task }) {
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
  const [depth, setDepth] = useState<"light"|"medium"|"heavy">("medium");
  const [request, setRequest] = useState("");
  const pendingKey = "task-agent:prepfile:" + path + ":" + task.id;
  const [pending, setPending] = useState(() => localStorage.getItem(pendingKey) ?? "");
  const refresh = useCallback(async () => {
    try { setState(await taskAgentStatus(path, task.id)); setError(""); }
    catch (cause) { setState(null); setError(String(cause)); }
  }, [path, task.id]);
  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), 15_000);
    return () => window.clearInterval(timer);
  }, [refresh]);
  const invoke = async (command: string, options: Record<string,unknown> = {}) => {
    if (busy) return;
    setBusy(true); setNotice(""); setError("");
    try {
      const receipt = await taskAgentAction(path, task.id, command, options);
      setNotice("T3 " + command + " confirmed (" + String(receipt.status ?? receipt.action ?? "ok") + ").");
      await refresh();
    } catch (cause) {
      setError("T3 outcome unconfirmed: " + String(cause) + ". Inspect T3 before repeating.");
    } finally { setBusy(false); }
  };
  const integer = (value: string, min: number, max: number) => {
    const n = Number(value);
    return /^\d+$/.test(value) && n >= min && n <= max ? n : null;
  };
  const field = (name: string) => state?.caffy?.[name] == null ? "" : String(state.caffy[name]);
  const setMode = (mode: "idle"|"periodic"|"startup") => {
    const minutes = duration.trim() ? integer(duration,1,10080) : null;
    const step = integer(interval,1,1440);
    if ((duration.trim() && minutes == null) || (mode === "periodic" && step == null)) {
      setError("Duration must be 1–10080 min (or blank); periodic interval 1–1440 min."); return;
    }
    void invoke("caffy_set", {mode, ...(minutes ? {durationMinutes:minutes}:{}),
      ...(mode === "periodic" ? {intervalMinutes:step}:{}), startup:mode === "startup"});
  };
  return <section aria-label="Linked T3 agent controls" style={{padding:12, margin:"10px 0",
    border:"1px solid #555",borderRadius:10}}>
    <strong>Linked T3 agent</strong>{" "}
    <button type="button" disabled={busy} onClick={() => void refresh()}>Refresh</button>
    {!state ? <p>{error || "Checking linked agent…"} No new session will be started.</p> : <>
      <p>{state.title ?? state.sessionId} · {state.activity ?? state.status ?? "unknown"} · observed {state.statusObservedAt ?? "unverified"}</p>
      <button type="button" disabled={busy} onClick={() => void invoke("open")}>Open / focus this agent</button>
      <p><b>Caffy:</b> {state.caffy ? (field("mode") || "active") +
        (field("paused") === "true" ? " · paused" : "") +
        (field("nextDueAt") ? " · next " + field("nextDueAt") : "") : "No active scheduler"}</p>
      <div style={{display:"flex",gap:6,flexWrap:"wrap",alignItems:"center"}}>
        <label>Duration min (blank = unlimited) <input style={{width:65}} value={duration} onChange={e=>setDuration(e.target.value)}/></label>
        <label>Interval min <input style={{width:65}} value={interval} onChange={e=>setIntervalValue(e.target.value)}/></label>
        <button type="button" disabled={busy} onClick={() => setMode("idle")}>Keep running</button>
        <button type="button" disabled={busy} onClick={() => setMode("periodic")}>Periodic</button>
        <button type="button" disabled={busy} onClick={() => setMode("startup")}>On startup</button>
      </div>
      <div style={{display:"flex",gap:6,flexWrap:"wrap",marginTop:8}}>
        <button type="button" disabled={busy || !state.caffy} onClick={() => void invoke(field("paused") === "true" ? "caffy_resume" : "caffy_pause")}>{field("paused") === "true" ? "Resume" : "Pause"}</button>
        <button type="button" disabled={busy} onClick={() => void invoke("caffy_wake")}>Wake now</button>
        <button type="button" disabled={busy || !state.caffy} onClick={() => void invoke("caffy_stop")}>Stop</button>
        <button type="button" disabled={busy || !state.caffy} onClick={() => void invoke("caffy_startup",{startup:field("startup") !== "true"})}>Toggle startup resume</button>
      </div>
      <div style={{display:"flex",gap:6,flexWrap:"wrap",alignItems:"center",marginTop:8}}>
        <label>Begin in min <input style={{width:65}} value={delay} onChange={e=>setDelay(e.target.value)}/></label>
        <label>Prompt <input style={{width:170}} value={prompt} onChange={e=>setPrompt(e.target.value.slice(0,8000))}/></label>
        <button type="button" disabled={busy} onClick={() => {
          const n=integer(delay,1,129600);
          if(!n) {setError("Begin delay must be 1–129600 min");return;}
          void invoke("caffy_begin_in",{beginAt:Date.now()+n*60000,...(prompt?{prompt}:{})});
        }}>Schedule</button>
        <button type="button" disabled={busy} onClick={() => void invoke("caffy_cancel_begin")}>Cancel scheduled</button>
      </div>
      <div style={{display:"flex",gap:6,flexWrap:"wrap",alignItems:"center",marginTop:8}}>
        <label>Reboot after min <input style={{width:65}} value={reboot} onChange={e=>setReboot(e.target.value)}/></label>
        <button type="button" disabled={busy} onClick={()=>{
          const n=integer(reboot,1,525600);
          if(!n){setError("Reboot minutes invalid");return;}
          void invoke("caffy_auto_reboot",{rebootMinutes:n});
        }}>Set time</button>
        <label>or runs <input style={{width:50}} value={runs} onChange={e=>setRuns(e.target.value)}/></label>
        <button type="button" disabled={busy} onClick={()=>{
          const n=integer(runs,1,10000);if(!n){setError("Reboot runs invalid");return;}
          void invoke("caffy_auto_reboot",{rebootEveryRuns:n});
        }}>Set runs</button>
        <button type="button" disabled={busy} onClick={() => void invoke("caffy_auto_reboot")}>Reboot off</button>
      </div>
      <details style={{marginTop:10}}><summary>Get My Attention · {state.attentions?.filter(a => ["waiting","pending","snoozed"].includes(a.status)).length ?? 0}</summary>
        {state.gmaUnavailable && <p>{state.gmaUnavailable}</p>}
        {state.attentions?.filter(a => ["waiting","pending","snoozed"].includes(a.status)).map(a =>
          <div key={a.id} style={{padding:8,margin:"6px 0",border:"1px solid #666",borderRadius:6}}>
            <p>{a.message} · {a.status}</p>
            <button type="button" disabled={busy} onClick={() => void invoke("attention_open",{requestId:a.id})}>Acknowledge</button>{" "}
            <button type="button" disabled={busy} onClick={() => void invoke("open")}>Open agent</button>{" "}
            <button type="button" disabled={busy} onClick={() => void invoke("attention_snooze",{requestId:a.id,minutes:15})}>Snooze 15m</button>{" "}
            <button type="button" disabled={busy} onClick={() => void invoke("attention_dismiss",{requestId:a.id})}>Dismiss</button>
          </div>
        )}
        {!state.gmaUnavailable && !state.attentions?.some(a => ["waiting","pending","snoozed"].includes(a.status)) &&
          <p>No pending GMA requests for this task-linked session.</p>}
      </details>
      <details style={{marginTop:10}}><summary>New Prepfile for {task.id}</summary>
        <p>Uses this existing agent and task. Does not spawn or duplicate work.</p>
        <textarea style={{width:"100%",minHeight:60}} value={request}
          onChange={e=>setRequest(e.target.value.slice(0,6000))} placeholder="Research focus"/>
        <label>Depth <select value={depth} onChange={e=>setDepth(e.target.value as typeof depth)}>
          <option value="light">Light</option><option value="medium">Medium</option><option value="heavy">Heavy</option>
        </select></label>
        <button type="button" disabled={busy || !request.trim() || !!pending} onClick={()=>{
          const id=crypto.randomUUID();localStorage.setItem(pendingKey,id);setPending(id);
          void invoke("prepfile",{requestId:id,request:request.trim(),depth});
        }}>Request Prepfile</button>
        {pending && <p>Original request: {pending}. An uncertain receipt must be reconciled before retrying.{" "}
          <button type="button" disabled={busy || !request.trim()} onClick={()=>
            void invoke("prepfile",{requestId:pending,request:request.trim(),depth})}>Retry same ID</button>{" "}
          <button type="button" disabled={busy} onClick={()=>{
            localStorage.removeItem(pendingKey);setPending("");
          }}>New request (after verification)</button></p>}
      </details>
    </>}
    {error && state && <p role="alert" style={{color:"#e39c70"}}>{error}</p>}
    {notice && <p role="status">{notice}</p>}
  </section>;
}
