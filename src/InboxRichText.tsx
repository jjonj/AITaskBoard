import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
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
    await openPath(localInboxPath(value, task.path));
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
  const looseLinks = [...new Set(text.match(BARE_RESOURCE) ?? [])]
    .filter(link => !text.includes(`]("${link}"`) && !text.includes(`](${link}`));
  return <div className={`markdown-content inbox-rich-text ${className}`}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} urlTransform={(url) => url} components={{
      a: ({ href, children }) => href && isResourceLink(decodeLink(href))
        ? <button className="inbox-resource-link" type="button" title={decodeLink(href) + "\nRight-click to copy link"}
            onClick={() => launch(href)}
            onContextMenu={(event) => showCopyMenu(event, copyableAddress(href, task))}>{children} ↗</button>
        : <span>{children}</span>,
      img: ({ src, alt }) => src && isResourceLink(decodeLink(src))
        ? <button className="inbox-resource-link" type="button" title={decodeLink(src) + "\nRight-click to copy link"}
            onClick={() => launch(src)}
            onContextMenu={(event) => showCopyMenu(event, copyableAddress(src, task))}>▧ {alt || src.split(/[\\/]/).pop() || "Image"} ↗</button>
        : <span>{alt || "Image"}</span>,
    }}>{text}</ReactMarkdown>
    {looseLinks.length > 0 && <div className="inbox-resource-list" aria-label="Referenced resources">
      {looseLinks.map(link => <button className="inbox-resource-chip" type="button" key={link}
        title={link + "\nRight-click to copy link"} onClick={() => launch(link)}
        onContextMenu={(event) => showCopyMenu(event, copyableAddress(link, task))}>
        {sessionId(link) ? "◉ T3 session" : /\.html?(?:[?#]|$)/i.test(link) ? "▣ Presentation" : "▧ File"} · {link.replace(/\\/g, "/").split("/").pop()} ↗
      </button>)}
    </div>}
    {copyMenu}
  </div>;
}
