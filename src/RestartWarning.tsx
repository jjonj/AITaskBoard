import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import "./RestartWarning.css";

type RestartRequest = { id: string; deadlineUnixMs: number };

/** A rebuild script publishes a short-lived request before stopping the app. */
export function RestartWarning() {
  const [request, setRequest] = useState<RestartRequest | null>(null);
  const [now, setNow] = useState(Date.now());
  const [postponing, setPostponing] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!isTauri()) return;
    let mounted = true;
    let pending = false;
    const poll = async () => {
      if (pending) return;
      pending = true;
      try {
        const next = await invoke<RestartRequest | null>("restart_warning_status");
        if (mounted) {
          setRequest(next);
          if (!next) { setPostponing(false); setError(""); }
        }
      } catch {
        // Keep TaskBoard usable if a dev build lacks the native restart command.
      } finally {
        pending = false;
      }
    };
    void poll();
    const timer = window.setInterval(() => {
      setNow(Date.now());
      void poll();
    }, 500);
    return () => { mounted = false; window.clearInterval(timer); };
  }, []);

  if (!request) return null;
  const seconds = Math.max(0, Math.ceil((request.deadlineUnixMs - now) / 1000));
  const postpone = async () => {
    setPostponing(true);
    setError("");
    try {
      await invoke("postpone_restart_warning", { id: request.id });
      setRequest(null);
    } catch (cause) {
      setError(String(cause));
      setPostponing(false);
    }
  };
  return (
    <div className="restart-warning" role="alert" aria-live="polite">
      <span className="restart-warning-icon" aria-hidden="true">↻</span>
      <div className="restart-warning-copy">
        <strong>TaskBoard restart in <span className="restart-warning-seconds">{seconds}s</span></strong>
        <span>Changes are ready to build. TaskBoard will close and reopen automatically.</span>
        {error && <span className="restart-warning-error">{error}</span>}
      </div>
      <button type="button" onClick={() => void postpone()} disabled={postponing || seconds === 0}
        title="Cancel this restart attempt; a later rebuild can retry.">
        {postponing ? "Postponing…" : "Postpone"}
      </button>
      <div className="restart-warning-progress" style={{
        transform: `scaleX(${Math.min(1, seconds / 30)})`,
      }} />
    </div>
  );
}
