import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";

/** All banner errors pass through here; blanks dismiss without producing log entries. */
export function useLoggedError(boardPath: string): [string, (message: string) => void] {
  const [error, updateError] = useState("");
  const record = (message: string, source = "ui") => {
    if (!message.trim()) return;
    const taskId = message.match(/\b[A-Z][A-Z0-9]*-\d{3,}\b/)?.[0] ?? null;
    const category = /T3 start|T3 launch|T3 spawn|open.task/i.test(message) ? "t3_launch"
      : /queue/i.test(message) ? "queue" : /refresh|sync/i.test(message) ? "sync" : "ui";
    void invoke("record_ui_error", {
      entry: { timestamp: new Date().toISOString(), source, category, taskId, boardPath, message },
    }).catch((reason) => console.error("TaskBoard persistent error log failed", reason, message));
  };
  const setError = (message: string) => {
    updateError(message);
    record(message);
  };
  useEffect(() => {
    const onError = (event: ErrorEvent) => record(`${event.message} (${event.filename}:${event.lineno})`, "window_error");
    const onRejection = (event: PromiseRejectionEvent) => record(String(event.reason), "unhandled_rejection");
    window.addEventListener("error", onError);
    window.addEventListener("unhandledrejection", onRejection);
    return () => {
      window.removeEventListener("error", onError);
      window.removeEventListener("unhandledrejection", onRejection);
    };
  }, [boardPath]);
  return [error, setError];
}
