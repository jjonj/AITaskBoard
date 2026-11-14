import { useEffect, useState } from "react";

const INBOX_RETURN_WINDOW_MS = 5 * 60_000;

// Keep each project's Inbox open across a brief tab switch, but not indefinitely.
export function useRememberedInbox(key: string) {
  const [open, setOpen] = useState(() => {
    try { return Number(window.sessionStorage.getItem(key)) > Date.now(); }
    catch { return false; }
  });

  useEffect(() => {
    if (!open) {
      try { window.sessionStorage.removeItem(key); } catch { /* Storage may be unavailable. */ }
      return;
    }
    const remember = () => {
      try { window.sessionStorage.setItem(key, String(Date.now() + INBOX_RETURN_WINDOW_MS)); }
      catch { /* The Inbox still works without persistence. */ }
    };
    remember();
    return remember; // A tab switch starts the return window when this board unmounts.
  }, [key, open]);

  return [open, setOpen] as const;
}
