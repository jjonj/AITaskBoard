import { useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";

/** Read-only Markdown without raw HTML; task text stays editable as plain Markdown. */
export function MarkdownText({ text, className = "" }: { text: string; className?: string }) {
  return <div className={`markdown-content ${className}`}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} urlTransform={(url) =>
      /^wartribes-tour:\/\/[a-z][a-z0-9_]{0,63}\/?$/i.test(url) ? url :
      /^wartribes-tour:/i.test(url) ? "" : url
    } components={{
      a: ({ node: _node, ...props }) => /^wartribes-tour:\/\//i.test(props.href ?? "")
        ? <a {...props} href={props.href} onClick={(event) => {
            if (!("__TAURI_INTERNALS__" in window) || !props.href) return;
            event.preventDefault();
            void openUrl(props.href).catch(error => window.alert(String(error)));
          }} />
        : <a {...props} target="_blank" rel="noopener noreferrer" />,
    }}>{text}</ReactMarkdown>
  </div>;
}

/** Optional preview for textarea fields that should remain quick to edit. */
export function MarkdownPreviewToggle({ text }: { text: string }) {
  const [visible, setVisible] = useState(false);
  return <div className="markdown-preview-toggle">
    <button type="button" onClick={() => setVisible((value) => !value)}
      aria-expanded={visible} disabled={!text.trim()}>
      {visible ? "Hide preview" : "Preview Markdown"}
    </button>
    {visible && <MarkdownText text={text} className="markdown-field-preview" />}
  </div>;
}
