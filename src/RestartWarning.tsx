import { invoke, isTauri } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import "./RestartWarning.css";

type RestartRequest = { id: string; deadlineUnixMs: number; finalize: boolean };

export type RestartSaveDetail = { save?: Promise<void> };

/** A rebuild script publishes a short-lived request before stopping the app. */
export function RestartWarning() {
  const [request, setRequest] = useState<RestartRequest | null>(null);
  const [now, setNow] = useState(Date.now());
  const [postponing, setPostponing] = useState(false);
  const [error, setError] = useState("");
  const finalizingId = useRef<string | null>(null);

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
          if (!next) {
            if (finalizingId.current) window.dispatchEvent(new Event("taskboard:restart-cancelled"));
            setPostponing(false); setError(""); finalizingId.current = null;
          }
          if (next?.finalize && finalizingId.current !== next.id) {
            finalizingId.current = next.id;
            void (async () => {
              try {
                if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
                const detail: RestartSaveDetail = {};
                window.dispatchEvent(new CustomEvent("taskboard:save-before-restart", { detail }));
                if (!detail.save) throw new Error("Task editor is not ready. Restart will be postponed.");
                await detail.save;
                await invoke("confirm_restart_ready", { id: next.id });
              } catch (cause) {
                setError(`Restart cannot proceed: ${String(cause)}. Your work stays open.`);
              }
            })();
          }
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
        <strong>{request.finalize ? "Saving edits before restart…" : <>TaskBoard restart in <span className="restart-warning-seconds">{seconds}s</span></>}</strong>
        <span>{request.finalize ? "The rebuild will only proceed after all open task editors are saved." : "Changes are ready to build. TaskBoard will close and reopen automatically."}</span>
        {error && <span className="restart-warning-error">{error}</span>}
      </div>
      <button type="button" onClick={() => void postpone()} disabled={postponing}
        title="Cancel this restart attempt; a later rebuild can retry.">
        {postponing ? "Postponing…" : "Postpone"}
      </button>
      <div className="restart-warning-progress" style={{
        transform: `scaleX(${Math.min(1, seconds / 30)})`,
      }} />
    </div>
  );
}
