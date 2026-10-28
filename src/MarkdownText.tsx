import { useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";

/** Read-only Markdown without raw HTML; task text stays editable as plain Markdown. */
export function MarkdownText({ text, className = "" }: { text: string; className?: string }) {
  return <div className={`markdown-content ${className}`}>
    <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
      a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noopener noreferrer" />,
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
