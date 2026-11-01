import { useEffect, useState } from "react";
import { taskReadAgentPolicy, taskSaveAgentPolicy, type AgentPolicy, type AgentPolicyResponse } from "./api";

type Props = { boardPath: string; onClose: () => void; };

/** Per-board human policy: never selects or spawns a replacement agent. */
export function AgentRecoverySettings({boardPath,onClose}:Props) {
  const [current,setCurrent] = useState<AgentPolicyResponse | null>(null);
  const [draft,setDraft] = useState<AgentPolicy | null>(null);
  const [busy,setBusy] = useState(false);
  const [error,setError] = useState("");
  useEffect(() => {
    let active=true;
    void taskReadAgentPolicy(boardPath).then(policy => {
      if(active){setCurrent(policy);setDraft(policy.values);}
    }).catch(err=>{if(active)setError(String(err));});
    return ()=>{active=false;};
  },[boardPath]);
  const setNumber = (key: keyof AgentPolicy, value: string) =>
    setDraft(current => current ? {...current,[key]: Number(value)} : current);
  const number = (key:keyof AgentPolicy,label:string,min:number,max:number,description:string) =>
    <label className="agent-recovery-field"><span>{label}</span>
      <input type="number" min={min} max={max} step="1"
        value={draft?.[key] as number ?? ""} disabled={busy || !draft}
        onChange={event=>setNumber(key,event.currentTarget.value)} />
      <small>{description}</small>
    </label>;
  const save = async () => {
    if(!current || !draft || busy)return;
    setBusy(true);setError("");
    try {
      await taskSaveAgentPolicy(boardPath,draft,current.revision);
      onClose();
    } catch(err){setError(String(err));}
    finally {setBusy(false);}
  };
  return <div className="agent-recovery-overlay" role="presentation" onMouseDown={onClose}>
    <section role="dialog" aria-modal="true" aria-label="Agent recovery settings"
      className="agent-recovery-dialog" onMouseDown={event=>event.stopPropagation()}>
      <h2>Agent recovery</h2>
      <p>Task identity and existing owner claims are preserved while T3 is unavailable or uncertain.
        A new worker is never spawned automatically.</p>
      {draft && <div className="agent-recovery-fields">
        {number("resumeWindowHours","Agent Resume Window (hours)",1,168,
          "Idle agents are contacted only if their last verified work falls within this window.")}
        <label className="agent-recovery-check">
          <input type="checkbox" checked={draft.wakeRecentIdle} disabled={busy}
            onChange={event=>setDraft({...draft,wakeRecentIdle:event.currentTarget.checked})}/>
          <span>Resume a recent idle T3 agent on human feedback</span>
        </label>
        {number("t3OrphanGraceHours","Confirmed-gone T3 grace (hours)",1,168,
          "Only starts after an authoritative permanent-gone receipt; errors and unloaded sessions do not start it.")}
        {number("externalLeaseHours","External agent inactivity lease (hours)",1,2160,
          "Non-T3 agents can renew their lease by updating/claiming their task.")}
        {number("technicalRecheckDays","Technical blocker recheck age (days)",1,365,
          "Low-priority reminders, never worker spawning or dependency-only rechecks.")}
      </div>}
      {error && <p className="agent-recovery-error" role="alert">{error}</p>}
      <footer>
        <button type="button" className="button secondary" onClick={onClose}>Cancel</button>
        <button type="button" className="button primary" disabled={!current || !draft || busy}
          onClick={()=>void save()}>{busy?"Saving...":"Save recovery settings"}</button>
      </footer>
    </section>
  </div>;
}
