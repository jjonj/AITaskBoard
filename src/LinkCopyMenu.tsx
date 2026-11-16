import { useEffect, useState, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { highlightInFomni } from "./api";
import "./LinkCopyMenu.css";

type LinkMenu = { address: string; x: number; y: number; fomniFile?: string; fomniLabel?: string };

/** Copy a user-visible resource address, including desktop-only custom URL schemes. */
async function copyAddress(address: string): Promise<void> {
  if (navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(address);
      return;
    } catch {
      // WebView clipboard permissions vary. Fall back to an ordinary selection copy.
    }
  }
  const input = document.createElement("textarea");
  input.value = address;
  input.style.position = "fixed";
  input.style.opacity = "0";
  document.body.appendChild(input);
  try {
    input.select();
    if (!document.execCommand("copy")) throw new Error("Clipboard access was denied");
  } finally {
    input.remove();
  }
}

/** Native-style right-click Copy link for Markdown, Inbox and artifact links. */
export function useLinkCopyMenu(onError?: (message: string) => void) {
  const [menu, setMenu] = useState<LinkMenu | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!menu) return;
    const dismiss = (event: PointerEvent) => {
      if (!(event.target instanceof Element) || !event.target.closest(".link-copy-menu")) setMenu(null);
    };
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        event.stopImmediatePropagation(); // Close only this topmost menu, not its parent review.
        setMenu(null);
      }
    };
    const close = () => setMenu(null);
    window.addEventListener("pointerdown", dismiss, true);
    window.addEventListener("keydown", keydown, true);
    window.addEventListener("scroll", close, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("pointerdown", dismiss, true);
      window.removeEventListener("keydown", keydown, true);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("blur", close);
    };
  }, [menu]);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const showCopyMenuAt = (x: number, y: number, address: string,
    options?: { fomniFile?: string; fomniLabel?: string }) => {
    if (!address) return;
    setCopied(false);
    setMenu({
      address,
      fomniFile: options?.fomniFile,
      fomniLabel: options?.fomniLabel,
      x: Math.max(8, Math.min(x, window.innerWidth - 210)),
      y: Math.max(8, Math.min(y, window.innerHeight - (options?.fomniFile ? 92 : 48))),
    });
  };
  const showCopyMenu = (event: MouseEvent<HTMLElement>, address: string,
    options?: { fomniFile?: string; fomniLabel?: string }) => {
    event.preventDefault();
    event.stopPropagation();
    showCopyMenuAt(event.clientX, event.clientY, address, options);
  };

  const copy = async () => {
    if (!menu) return;
    const address = menu.address;
    setMenu(null);
    try {
      await copyAddress(address);
      setCopied(true);
    } catch (error) {
      onError?.(`Could not copy link: ${String(error)}`);
    }
  };

  const revealInFomni = async () => {
    const file = menu?.fomniFile;
    if (!file) return;
    setMenu(null);
    try {
      await highlightInFomni(file);
    } catch (error) {
      onError?.(`Could not reveal file in Fomni: ${String(error)}`);
    }
  };

  const copyMenu = typeof document === "undefined" ? null : createPortal(<>
    {menu && <div className="link-copy-menu" role="menu"
      style={{ left: menu.x, top: menu.y }}>
      {menu.fomniFile && <button type="button" role="menuitem"
        onClick={() => void revealInFomni()}>{menu.fomniLabel || "Open in Fomni"}</button>}
      <button type="button" role="menuitem" onClick={() => void copy()}>Copy link address</button>
    </div>}
    {copied && <div className="link-copy-notice" role="status">Link copied</div>}
  </>, document.body);

  return { showCopyMenu, showCopyMenuAt, copyMenu, copyMenuOpen: menu !== null, dismissCopyMenu: () => setMenu(null) };
}
