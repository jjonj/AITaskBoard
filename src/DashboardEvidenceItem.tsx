import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openFileInFomni } from "./InboxRichText";

type DashboardItem = {
  id: string;
  task: string;
  title: string;
  caption?: string;
  kind?: string;
  mediaType?: string;
  sourceReport?: string;
  capturedAt?: string;
  publishedAt?: string;
  mediaDataUrl?: string;
  mediaPath?: string;
  mediaUnavailable?: string;
};

function selectionFromUrl(address: string) {
  const url = new URL(address);
  const listed = url.searchParams.get("items");
  const ids = listed ? listed.split(",").map(value => value.trim()) :
    url.searchParams.get("item") ? [url.searchParams.get("item")!] : [];
  return { ids, task: url.searchParams.get("task") };
}

function displayDate(raw?: string) {
  if (!raw) return "";
  const parsed = new Date(raw);
  return Number.isFinite(parsed.getTime()) ? parsed.toLocaleString() : raw;
}

function isVideo(item: DashboardItem) {
  return item.mediaType === "video" || /\.(?:mp4|webm)$/i.test(item.mediaPath || "");
}

function DashboardMedia({ item }: { item: DashboardItem }) {
  if (!item.mediaDataUrl) return <div className="dashboard-evidence-message">
    {item.mediaUnavailable || "No inline media available."}
  </div>;
  return isVideo(item) ? <video src={item.mediaDataUrl} controls preload="metadata" /> :
    <img src={item.mediaDataUrl} alt={item.title + (item.caption ? " — " + item.caption : "")} />;
}

/** Native gallery of exact published Wartribes Dashboard captures. Generic
 * web/HTML reviews continue through the separate ordinary iframe renderer. */
export function DashboardEvidenceItem({ address }: { address: string }) {
  const [items, setItems] = useState<DashboardItem[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    setItems([]);
    setSelected(null);
    setError("");
    let selection: ReturnType<typeof selectionFromUrl>;
    try {
      selection = selectionFromUrl(address);
      if (!selection.ids.length && !selection.task)
        throw new Error("Supply an exact capture (?item=), a curated gallery (?items=id1,id2), or a task gallery (?task=WT-0000).");
    } catch (reason) {
      setError(String(reason));
      return;
    }
    let cancelled = false;
    setLoading(true);
    void invoke<DashboardItem[]>("dashboard_review_items", selection)
      .then(result => { if (!cancelled) setItems(result); })
      .catch(reason => { if (!cancelled) setError(String(reason)); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [address]);

  const openSource = (item: DashboardItem) => {
    if (!item.mediaPath) return;
    void openFileInFomni(item.mediaPath).catch(reason => setError(String(reason)));
  };

  if (loading) return <div className="dashboard-evidence-message">Loading published Dashboard captures…</div>;
  if (!items.length) return <section className="dashboard-evidence-message" role="alert">
    <strong>Dashboard gallery unavailable</strong><p>{error || "This task has no published captures."}</p>
  </section>;

  const chosen = selected === null ? null : items[selected] ?? null;
  return <section className="dashboard-evidence-item"
    aria-label={"Wartribes Dashboard review gallery for " + items[0].task}>
    <header className="dashboard-evidence-head">
      <div className="dashboard-evidence-heading">
        <small>{items[0].task} · Published Dashboard evidence</small>
        <h2>{chosen ? chosen.title : items.length + (items.length === 1 ? " capture" : " captures")}</h2>
        {chosen && chosen.caption && <p>{chosen.caption}</p>}
        {!chosen && <p>Original published captures and captions. Select any image to inspect it larger.</p>}
      </div>
      <div className="dashboard-evidence-meta">
        <span>{chosen ? (selected! + 1) + " of " + items.length : "Gallery · " + items.length + " items"}</span>
        {chosen?.capturedAt && <span>Captured <strong>{displayDate(chosen.capturedAt)}</strong></span>}
        <div className="dashboard-evidence-actions">
          {chosen && <>
            <button type="button" className="button secondary" onClick={() => setSelected(null)}>All captures</button>
            <button type="button" className="button secondary"
              aria-label="Previous capture" disabled={selected === 0}
              onClick={() => setSelected(Math.max(0, selected! - 1))}>←</button>
            <button type="button" className="button secondary"
              aria-label="Next capture" disabled={selected === items.length - 1}
              onClick={() => setSelected(Math.min(items.length - 1, selected! + 1))}>→</button>
            {chosen.mediaPath && <button type="button" className="button secondary"
              onClick={() => openSource(chosen)}>Open in Fomni ↗</button>}
          </>}
        </div>
      </div>
    </header>

    {chosen ? <div className="dashboard-evidence-media dashboard-evidence-selected">
      <DashboardMedia item={chosen} />
    </div> : <div className="dashboard-evidence-gallery" aria-label="Dashboard captures">
      {items.map((item, index) => <article className="dashboard-evidence-card" key={item.id}>
        {isVideo(item) ? <div className="dashboard-evidence-card-media">
          <DashboardMedia item={item} />
        </div> : <button type="button" className="dashboard-evidence-card-media"
          aria-label={"Enlarge " + item.title}
          onClick={() => setSelected(index)}>
          <DashboardMedia item={item} />
        </button>}
        <div className="dashboard-evidence-card-info">
          <span className="dashboard-evidence-date">Captured {displayDate(item.capturedAt)}</span>
          <h3>{item.title}</h3>
          {item.caption && <p>{item.caption}</p>}
          <div className="dashboard-evidence-card-actions">
            <button type="button" className="button secondary" onClick={() => setSelected(index)}>Inspect larger</button>
            {item.mediaPath && <button type="button" className="button secondary"
              onClick={() => openSource(item)}>Open original ↗</button>}
          </div>
        </div>
      </article>)}
    </div>}
    {error && <div className="error-banner" role="alert">{error}</div>}
  </section>;
}
