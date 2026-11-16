import { useEffect, useMemo, useState } from "react";
import { readTaskImage } from "./api";
import { useLinkCopyMenu } from "./LinkCopyMenu";

export type PendingClipboardImage = {
  dataUrl: string;
  mime: string;
  name: string;
};

type Props = {
  description: string;
  boardPath: string;
  pending: PendingClipboardImage[];
  onRemovePending: (index: number) => void;
};

export const imageAttachmentPaths = (description: string): string[] =>
  [...new Set(
    [...description.matchAll(/!\[[^\]]*\]\((?:\.\.\/)?(Attachments\/[A-Za-z0-9-]+\/[a-f0-9]{64}\.(?:png|jpg|webp|gif))\)/gi)]
      .map((match) => match[1].replace(/^\.\.\//, "")),
  )];

export function TaskImageGallery({ description, boardPath, pending, onRemovePending }: Props) {
  const paths = useMemo(() => imageAttachmentPaths(description), [description]);
  const [savedImages, setSavedImages] = useState<Record<string, string>>({});
  const { showCopyMenu, copyMenu } = useLinkCopyMenu((message) => window.alert(message));
  const pathKey = paths.join("\n");

  useEffect(() => {
    let cancelled = false;
    setSavedImages({});
    void Promise.all(paths.map(async (path) => {
      try {
        return [path, await readTaskImage(path, boardPath)] as const;
      } catch {
        return [path, ""] as const;
      }
    })).then((pairs) => {
      if (!cancelled) setSavedImages(Object.fromEntries(pairs));
    });
    return () => { cancelled = true; };
  }, [boardPath, pathKey]);

  if (!paths.length && !pending.length) return null;

  return (
    <section className="task-image-gallery" aria-label="Task images">
      <strong>Images</strong>
      <div className="task-image-gallery-grid">
        {paths.map((path) => (
          <div className="task-image-thumb" key={path}>
            {savedImages[path] ? (
              <a href={savedImages[path]} target="_blank" rel="noreferrer" title="Open full image · right-click to reveal in Fomni"
                onContextMenu={(event) => {
                  const file = boardPath.replace(/[\\/]$/, "") + "/" + path;
                  showCopyMenu(event, file, { fomniFile: file });
                }}>
                <img src={savedImages[path]} alt="Task attachment" />
              </a>
            ) : (
              <span className="task-image-loading">Loading image…</span>
            )}
            <small>Attached</small>
          </div>
        ))}
        {pending.map((image, index) => (
          <div className="task-image-thumb pending" key={index}>
            <img src={image.dataUrl} alt={image.name} />
            <button type="button" onClick={() => onRemovePending(index)} aria-label={`Remove pending image ${index + 1}`}>×</button>
            <small>Unsaved</small>
          </div>
        ))}
      </div>
      {copyMenu}
    </section>
  );
}
