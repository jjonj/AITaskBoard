import { useEffect, useState } from "react";
import { editRelease, enableReleases, listReleases, newRelease, outcomeRelease, prepareRelease, reconcileReleases, verifyReleaseMember } from "./api";
import type { ReleaseSummary, ReleaseRecord } from "./releaseTypes";
import "./ReleaseCenter.css";

export function ReleaseCenter({ boardPath, enabled, onClose }: { boardPath: string; enabled: boolean; onClose: () => void }) {
  const [summary, setSummary] = useState<ReleaseSummary | null>(null);
  const [collectionEnabled, setCollectionEnabled] = useState(enabled);
  const [selection, setSelection] = useState("");
  const [error, setError] = useState("");
  const [working, setWorking] = useState(false);
  const [taskId, setTaskId] = useState("");
  const [destination, setDestination] = useState("");
  const [repo, setRepo] = useState("");
  const [commit, setCommit] = useState("");
  const [integratedRef, setIntegratedRef] = useState("");
  const [evidence, setEvidence] = useState("");
  const [selectedMember, setSelectedMember] = useState("");
  const [outcome, setOutcome] = useState<"build_failed" | "deployed" | "verified" | "verification_failed">("deployed");

  async function reload() {
    const next = await listReleases(boardPath);
    setSummary(next);
    setSelection(current => next.releases.some(r => r.id === current) ? current : next.releases[next.releases.length - 1]?.id ?? "");
  }
  useEffect(() => { void reload().catch(e => setError(String(e))); }, [boardPath]);
  async function act(fn: () => Promise<unknown>) {
    setError(""); setWorking(true);
    try { await fn(); await reload(); } catch (err) { setError(String(err)); } finally { setWorking(false); }
  }
  const record: ReleaseRecord | undefined = summary?.releases.find(r => r.id === selection);
  const latest = record?.attempts[record.attempts.length - 1];
  const validOutcome = latest?.status === "deployed" && outcome !== "verified" && outcome !== "verification_failed"
    ? "verified" : latest?.status === "prepared" && outcome !== "build_failed" && outcome !== "deployed" ? "deployed" : outcome;
  return (
    <div className="release-overlay" role="presentation" onMouseDown={event => { if (event.target === event.currentTarget) onClose(); }}>
      <section className="release-panel" role="dialog" aria-modal="true" aria-label="Releases">
        <header className="release-header">
          <div><strong>Releases</strong><p>Rolling source integration and deployment evidence. Task completion never triggers a deployment or app restart.</p></div>
          <button className="button secondary" onClick={onClose}>Close</button>
        </header>
        {error && <p className="release-error" role="alert">{error}</p>}
        <div className="release-toolbar">
          <button className="button secondary" disabled={working} onClick={() => void act(async () => { await enableReleases(!collectionEnabled, boardPath); setCollectionEnabled(!collectionEnabled); })}>{collectionEnabled ? "Disable collection" : "Enable collection"}</button>
          <button className="button secondary" disabled={working || !collectionEnabled} title="Explicit operator control: start a new collecting release without stopping active agents" onClick={() => void act(() => newRelease(boardPath))}>New release</button>
          <button className="button secondary" disabled={working} onClick={() => void act(reload)}>Refresh from Markdown</button>
          <button className="button secondary" disabled={working || !collectionEnabled} title="Explicit opt-in; include historical Done tasks" onClick={() => {
            if (window.confirm("Enroll eligible historic Done tasks that were completed before Releases were enabled? This does not reopen or deploy anything.")) {
              void act(() => reconcileReleases(true, boardPath));
            }
          }}>Import historical Done</button>
          <span>{collectionEnabled ? "Enabled" : "Disabled by default"}</span>
        </div>
        <div className="release-metrics">
          <div><strong>{summary?.integration_needed ?? 0}</strong><small>Integration needed</small></div>
          <div><strong>{summary?.ready ?? 0}</strong><small>Ready</small></div>
          <div><strong>{summary?.deployed ?? 0}</strong><small>Deployed</small></div>
          <div><strong>{summary?.verified ?? 0}</strong><small>Verified</small></div>
          <div><strong>{summary?.verification_failed ?? 0}</strong><small>Failed verification</small></div>
        </div>
        <div className="release-layout">
          <nav className="release-nav" aria-label="Release history">
            {(summary?.releases ?? []).map(r => <button type="button" key={r.id} className={r.id === selection ? "selected" : ""} onClick={() => { setSelection(r.id); setSelectedMember(""); }}>
              <strong>{r.id}</strong> <small>{r.status} · {r.members.length} members · {r.attempts.length} attempts</small>
            </button>)}
            {!summary?.releases.length && <p>No releases yet. The first eligible Done task automatically creates one after enablement.</p>}
          </nav>
          <article className="release-detail">
            {!record ? <p>Choose a release to see its live membership and immutable deployment attempts.</p> : <>
              <h2>{record.id} <small>{record.status}</small></h2>
              <div className="release-inline">
                <input placeholder="Completed task ID" value={taskId} onChange={e => setTaskId(e.target.value)} aria-label="Task ID" />
                <button disabled={working || !taskId.trim()} onClick={() => void act(() => editRelease("include", record.id, taskId, undefined, undefined, boardPath))}>Include Done</button>
              </div>
              <h3>Members ({record.members.length})</h3>
              <div className="release-members">
                {record.members.map(m => <div className="release-member" key={m.task_id}>
                  <div className="release-inline"><strong>{m.task_id}</strong><span className={"release-stage "+m.stage}>{m.stage.replace(/_/g, " ")}</span><span>attempts: {m.attempt_ids.join(", ") || "none"}</span>
                    <button disabled={working} title="Manual excluded membership is remembered during reconciliation" onClick={() => void act(() => editRelease("exclude", record.id, m.task_id, undefined, undefined, boardPath))}>Exclude</button>
                    <button onClick={() => setSelectedMember(m.task_id)}>Source / move</button>
                  </div>
                  {m.sources.map(s => <small key={s.repository+s.commit} className="release-source">{s.repository} @ {s.commit} → {s.integrated_ref ?? "not yet integrated"}</small>)}
                  {m.evidence.map((e,i) => <small key={i} className="release-source">{e}</small>)}
                  {selectedMember === m.task_id && <div className="release-editor">
                    <label>Repository <input value={repo} onChange={e => setRepo(e.target.value)} placeholder="T3 Code" /></label>
                    <label>Commit <input value={commit} onChange={e => setCommit(e.target.value)} placeholder="exact git SHA" /></label>
                    <label>Integrated source revision <input value={integratedRef} onChange={e => setIntegratedRef(e.target.value)} placeholder="blank = detached/unintegrated" /></label>
                    <button disabled={working || !repo || !commit} onClick={() => void act(() => editRelease("source", record.id, m.task_id, {repository:repo,commit,integrated_ref:integratedRef || null}, evidence || undefined, boardPath))}>Record source</button>
                    <label>Destination <input value={destination} onChange={e => setDestination(e.target.value)} placeholder="REL-0002" /></label>
                    <button disabled={working || !destination} onClick={() => void act(() => editRelease("move", record.id, m.task_id, undefined, destination, boardPath))}>Move future work</button>
                  </div>}
                </div>)}
              </div>
              {record.excluded.length > 0 && <p className="release-muted">Excluded: {record.excluded.join(", ")}</p>}
              <h3>Deployment attempts ({record.attempts.length})</h3>
              <label className="release-evidence">Evidence / operator note <textarea rows={2} value={evidence} onChange={e => setEvidence(e.target.value)} placeholder="Build output, change reference, deployment/verification evidence…" /></label>
              <div className="release-inline">
                <button disabled={working || record.members.every(m => m.stage !== "ready")} title="Captures an immutable snapshot of ready, integrated members; never builds or deploys." onClick={() => void act(() => prepareRelease(record.id, evidence, boardPath))}>Prepare immutable attempt</button>
                {latest && (latest.status === "prepared" || latest.status === "unknown" || latest.status === "deployed") && <>
                  <select aria-label="Outcome" value={validOutcome} onChange={e => setOutcome(e.target.value as typeof outcome)}>
                    {(latest.status === "deployed" ? ["verified","verification_failed"] : ["build_failed","unknown","deployed"]).map(v => <option key={v} value={v}>{v}</option>)}
                  </select>
                  <button disabled={working || !evidence.trim() || (latest.status === "deployed" &&
                    (!selectedMember || !latest.members.some(m => m.task_id === selectedMember)))}
                    title={latest.status === "deployed" ? "Select an exact deployed member above and provide specific verification evidence" : "Record actual build/restart outcome with objective evidence"}
                    onClick={() => void act(() => latest.status === "deployed"
                      ? verifyReleaseMember(record.id, latest.number, selectedMember, validOutcome === "verified", evidence, boardPath)
                      : outcomeRelease(record.id, latest.number, validOutcome, evidence, boardPath))}>
                    {latest.status === "deployed" ? "Verify selected member" : "Record outcome"}</button>
                </>}
              </div>
              {record.attempts.slice().reverse().map(a => <div className="release-attempt" key={a.number}>
                <strong>Attempt {a.number} · {a.status}</strong><small>{a.created}{a.run_id ? " · "+a.run_id : ""}</small>
                <p>Immutable members: {a.members.map(m => m.task_id).join(", ")}</p>
                {a.manifest.map((s,i) => <small key={i} className="release-source">{s.repository} · commit {s.commit} · integrated {s.integrated_ref ?? "UNVERIFIED"}</small>)}
                {a.evidence.map((e,i) => <p key={i} className="release-muted">{e}</p>)}
                {a.repair_task_id && <p>Linked repair: {a.repair_task_id}</p>}
              </div>)}
            </>}
          </article>
        </div>
      </section>
    </div>
  );
}
