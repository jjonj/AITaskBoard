import { useState } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useLinkCopyMenu } from "./LinkCopyMenu";
import { InboxRichText } from "./InboxRichText";
import type { Task } from "./types";

/** Read-only Markdown without raw HTML; task text stays editable as plain Markdown. */
export function MarkdownText({ text, className = "", task }: { text: string; className?: string; task?: Task }) {
  const { showCopyMenu, copyMenu } = useLinkCopyMenu();
  const menuFor = (event: React.MouseEvent<HTMLElement>, address: string) =>
    showCopyMenu(event, address, /^(?:[a-z]:[\\/]|\\\\)/i.test(address)
      ? { fomniFile: address } : undefined);
  if (task) return <InboxRichText task={task} text={text} className={className}
    onError={(message) => window.alert(message)} />;
  return <div className={`markdown-content ${className}`}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} urlTransform={(url) =>
      /^wartribes-tour:\/\/[a-z][a-z0-9_]{0,63}\/?$/i.test(url) ? url :
      /^wartribes-tour:/i.test(url) ? "" : defaultUrlTransform(url)
    } components={{
      a: ({ node: _node, ...props }) => /^wartribes-tour:\/\//i.test(props.href ?? "")
        ? <a {...props} href={props.href}
          onContextMenu={(event) => menuFor(event, props.href ?? "")}
          onClick={(event) => {
            if (!("__TAURI_INTERNALS__" in window) || !props.href) return;
            event.preventDefault();
            void openUrl(props.href).catch(error => window.alert(String(error)));
          }} />
        : <a {...props} target="_blank" rel="noopener noreferrer"
            onContextMenu={(event) => menuFor(event, props.href ?? "")} />,
      img: ({ node: _node, ...props }) => <img {...props}
        onContextMenu={(event) => menuFor(event, props.src ?? "")} />,
    }}>{text}</ReactMarkdown>
    {copyMenu}
  </div>;
}

/** Optional preview for textarea fields that should remain quick to edit. */
export function MarkdownPreviewToggle({ text, task }: { text: string; task?: Task }) {
  const [visible, setVisible] = useState(false);
  return <div className="markdown-preview-toggle">
    <button type="button" onClick={() => setVisible((value) => !value)}
      aria-expanded={visible} disabled={!text.trim()}>
      {visible ? "Hide preview" : "Preview Markdown"}
    </button>
    {visible && <MarkdownText text={text} task={task} className="markdown-field-preview" />}
  </div>;
}
