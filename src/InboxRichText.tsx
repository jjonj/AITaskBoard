import { Children, type ReactNode } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { invoke } from "@tauri-apps/api/core";
import { openClaimedT3Session } from "./api";
import type { Task } from "./types";
import { useLinkCopyMenu } from "./LinkCopyMenu";

const UUID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const SESSION_LINK = new RegExp(`^(?:t3(?:-browser)?:|session:)(?:\\/\\/(?:session\\/)?)?(${UUID})\\/?$`, "i");
const BARE_RESOURCE = new RegExp(
  `(?:[A-Za-z]:[\\\\/]|\\\\\\\\|\\.\\.[\\\\/])[^\\r\\n<>"|` + "`" + `]+?\\.(?:html?|pdf|png|jpe?g|webp|gif|svg|md|txt|log|json|csv|docx?|xlsx?|pptx?|mp4|webm)(?:[?#][^\\s]*)?|` +
  `(?:t3(?:-browser)?:|session:)(?:\\/\\/(?:session\\/)?)?${UUID}`, "gi"
);

function decodeLink(link: string) {
  try { return decodeURIComponent(link); } catch { return link; }
}

/** Resolve a task-relative attachment against the task Markdown, not the UI URL. */
export function localInboxPath(link: string, taskPath: string): string {
  let value = decodeLink(link.trim());
  // A URL fragment/query is navigation metadata, not part of a local filename.
  value = value.replace(/([?#]).*$/, "");
  if (/^file:\/\//i.test(value)) {
    const uri = new URL(value);
    value = uri.host && uri.host !== "localhost"
      ? "//" + uri.host + decodeLink(uri.pathname)
      : decodeLink(uri.pathname).replace(/^\/(?=[a-z]:\/)/i, "");
  }
  value = value.replace(/\\/g, "/");
  if (!/^(?:[a-z]:\/|\/)/i.test(value)) {
    const base = taskPath.replace(/\\/g, "/");
    value = base.slice(0, base.lastIndexOf("/") + 1) + value;
  }
  const prefix = value.startsWith("//") ? "//" : value.startsWith("/") ? "/" : "";
  const parts: string[] = [];
  for (const segment of value.slice(prefix.length).split("/")) {
    if (!segment || segment === ".") continue;
    if (segment === "..") { if (parts.length && parts[parts.length - 1] !== "..") parts.pop(); }
    else parts.push(segment);
  }
  return prefix + parts.join("/");
}

/** Open local Inbox files in Fomni rather than the system default viewer. */
export async function openFileInFomni(file: string) {
  if (!("__TAURI_INTERNALS__" in window)) throw new Error("Local file links require desktop TaskBoard.");
  await invoke("open_file_in_fomni", { file });
}

function sessionId(link: string) {
  return SESSION_LINK.exec(link)?.[1] ?? null;
}

function isResourceLink(link: string) {
  return /^wartribes-tour:\/\/[a-z][a-z0-9_]{0,63}\/?$/i.test(link) ||
    /^https?:\/\//i.test(link) || /^file:\/\//i.test(link) ||
    /^(?:[a-z]:[\\/]|\\\\|\.{1,2}[\\/]|\/)/i.test(link) ||
    !!sessionId(link) || (!/^[a-z][a-z\d+.-]*:/i.test(link) && !link.startsWith("#"));
}

/** Only user-initiated navigation; never launch or message a new T3 worker. */
async function openResource(link: string, task: Task) {
  const value = decodeLink(link);
  const session = sessionId(value);
  if (session) {
    if (!("__TAURI_INTERNALS__" in window)) throw new Error("T3 session links require desktop TaskBoard.");
    await openClaimedT3Session(session);
  } else if (/^(?:https?:\/\/|wartribes-tour:\/\/)/i.test(value)) {
    if ("__TAURI_INTERNALS__" in window) await openUrl(value);
    else window.open(value, "_blank", "noopener,noreferrer");
  } else {
    if (!("__TAURI_INTERNALS__" in window)) throw new Error("Local file links require desktop TaskBoard.");
    await openFileInFomni(localInboxPath(value, task.path));
  }
}

/** Clipboard gets the actual destination, not an attachment-relative Markdown fragment. */
function copyableAddress(link: string, task: Task): string {
  const value = decodeLink(link);
  return /^(?:https?:\/\/|wartribes-tour:\/\/|t3(?:-browser)?:|session:)/i.test(value)
    ? value : localInboxPath(value, task.path);
}

type Props = {
  task: Task;
  text: string;
  className?: string;
  onError: (message: string) => void;
};

/** Linked Markdown and detected loose file/session references in human Inbox text. */
export function InboxRichText({ task, text, className = "", onError }: Props) {
  const { showCopyMenu, copyMenu } = useLinkCopyMenu(onError);
  const launch = (href: string) => {
    void openResource(href, task).catch((error) =>
      onError(`Could not open "${href}": ${String(error)}`));
  };
  const anchor = (href: string, label: ReactNode, key?: string) =>
    <a key={key} href={href} className="inbox-resource-link"
      title={decodeLink(href) + "\nRight-click to copy link"}
      onClick={(event) => { event.preventDefault(); launch(href); }}
      onContextMenu={(event) => showCopyMenu(event, copyableAddress(href, task))}>{label}</a>;
  // Convert bare local paths into anchors within their original sentence, never
  // detached chips. Explicit Markdown links are handled by the <a> renderer.
  const inlineText = (children: ReactNode) => Children.map(children, (child) => {
    if (typeof child !== "string") return child;
    const pieces: ReactNode[] = [];
    let offset = 0;
    for (const match of child.matchAll(BARE_RESOURCE)) {
      const start = match.index;
      const value = match[0].replace(/[.,;!]+$/, "");
      if (start > offset) pieces.push(child.slice(offset, start));
      if (value && isResourceLink(value)) pieces.push(anchor(value, value, "path-" + start));
      else pieces.push(value);
      offset = start + match[0].length;
      if (value.length < match[0].length) pieces.push(match[0].slice(value.length));
    }
    if (!pieces.length) return child;
    if (offset < child.length) pieces.push(child.slice(offset));
    return pieces;
  });
  return <div className={`markdown-content inbox-rich-text ${className}`}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} urlTransform={(url) => url} components={{
      a: ({ href, children }) => href && isResourceLink(decodeLink(href))
        ? anchor(href, children)
        : <span>{children}</span>,
      img: ({ src, alt }) => src && isResourceLink(decodeLink(src))
        ? anchor(src, alt || src.split(/[\\/]/).pop() || "Image")
        : <span>{alt || "Image"}</span>,
      p: ({ children }) => <p>{inlineText(children)}</p>,
      li: ({ children }) => <li>{inlineText(children)}</li>,
    }}>{text}</ReactMarkdown>
    {copyMenu}
  </div>;
}
