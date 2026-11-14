import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openFileInFomni } from "./InboxRichText";

type DashboardItem = {
  id: string;
  task: string;
  title: string;
  caption?: string;
  kind?: string;
  mediaType?: string;
  source?: string;
  sourceReport?: string;
  capturedAt?: string;
  eventAt?: string;
  publishedAt?: string;
  mediaDataUrl?: string;
  mediaPath?: string;
  mediaUnavailable?: string;
};

function captureId(address: string): string | null {
  try {
    const url = new URL(address);
    return url.searchParams.get("item");
  } catch {
    return null;
  }
}

function displayDate(raw?: string) {
  if (!raw) return "";
  const parsed = new Date(raw);
  return Number.isFinite(parsed.getTime()) ? parsed.toLocaleString() : raw;
}

/** Single Dashboard capture, rendered within TaskBoard instead of embedding
 * the entire unrelated Dashboard navigation/workspace in an iframe. */
export function DashboardEvidenceItem({ address }: { address: string }) {
  const id = useMemo(() => captureId(address), [address]);
  const [item, setItem] = useState<DashboardItem | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setItem(null);
    setError("");
    if (!id) {
      setError("This review must link one published Dashboard capture using ?item=<capture ID>.");
      return;
    }
    let cancelled = false;
    setLoading(true);
    void invoke<DashboardItem>("dashboard_review_item", { id })
      .then((result) => { if (!cancelled) setItem(result); })
      .catch((reason) => { if (!cancelled) setError(String(reason)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [id]);

  const mediaPath = item?.mediaPath || "";
  const openSource = () => {
    if (!mediaPath) return;
    void openFileInFomni(mediaPath).catch((reason) => setError(String(reason)));
  };
  const isVideo = item?.mediaType === "video" ||
    /\.(?:mp4|webm)$/i.test(item?.source || "");

  if (error && !item) return <section className="dashboard-evidence-message" role="alert">
    <strong>Dashboard capture unavailable</strong><p>{error}</p>
  </section>;
  if (loading || !item) return <div className="dashboard-evidence-message">Loading published Dashboard capture…</div>;
  return <section className="dashboard-evidence-item" aria-label={"Dashboard evidence " + item.title}>
    <header className="dashboard-evidence-head">
      <div className="dashboard-evidence-heading">
        <small>{item.task} · Dashboard capture · {item.kind || item.mediaType || "Evidence"}</small>
        <h2>{item.title}</h2>
        {item.caption && <p>{item.caption}</p>}
      </div>
      <div className="dashboard-evidence-meta">
        {item.capturedAt && <span>Captured <strong>{displayDate(item.capturedAt)}</strong></span>}
        {item.publishedAt && <span>Published <strong>{displayDate(item.publishedAt)}</strong></span>}
        {mediaPath && <button type="button" className="button secondary" onClick={openSource}>Open published media in Fomni ↗</button>}
      </div>
    </header>
    <div className="dashboard-evidence-media">
      {item.mediaDataUrl ? (
        isVideo ? <video src={item.mediaDataUrl} controls preload="metadata" /> :
          <img src={item.mediaDataUrl} alt={item.title + (item.caption ? " — " + item.caption : "")} />
      ) : <div className="dashboard-evidence-message">{item.mediaUnavailable || "No inline media available."}</div>}
    </div>
    {error && <div className="error-banner" role="alert">{error}</div>}
    {item.sourceReport && <footer className="dashboard-evidence-origin">
      Source report: <code>{item.sourceReport}</code>
    </footer>}
  </section>;
}
