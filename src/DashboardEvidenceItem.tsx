import { useEffect, useRef, useState, type KeyboardEvent } from "react";
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

type Props = {
  address: string;
  selectedIndex: number | null;
  onSelectedIndexChange: (index: number | null) => void;
  onGalleryCountChange?: (count: number) => void;
};

/** Original published Wartribes media in a non-shrinking, scroll-snap gallery.
 * Other boards and ordinary HTML/URL reviews use the generic web renderer. */
export function DashboardEvidenceItem({ address, selectedIndex, onSelectedIndexChange, onGalleryCountChange }: Props) {
  const [items, setItems] = useState<DashboardItem[]>([]);
  const [railActive, setRailActive] = useState(0);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const railRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setItems([]);
    setRailActive(0);
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

  useEffect(() => {
    onGalleryCountChange?.(items.length);
  }, [items.length, onGalleryCountChange]);

  const openSource = (item: DashboardItem) => {
    if (!item.mediaPath) return;
    void openFileInFomni(item.mediaPath).catch(reason => setError(String(reason)));
  };

  const scrollToCard = (index: number) => {
    const rail = railRef.current;
    const cards = rail?.querySelectorAll<HTMLElement>(".dashboard-evidence-card");
    if (!rail || !cards?.length) return;
    const bounded = Math.max(0, Math.min(index, cards.length - 1));
    const firstLeft = cards[0].offsetLeft;
    rail.scrollTo({ left: cards[bounded].offsetLeft - firstLeft, behavior: "smooth" });
    setRailActive(bounded);
  };
  const syncRailPosition = () => {
    const rail = railRef.current;
    const cards = rail?.querySelectorAll<HTMLElement>(".dashboard-evidence-card");
    if (!rail || !cards?.length) return;
    const firstLeft = cards[0].offsetLeft;
    let nearest = 0;
    let nearestDistance = Number.POSITIVE_INFINITY;
    cards.forEach((card, index) => {
      const distance = Math.abs(card.offsetLeft - firstLeft - rail.scrollLeft);
      if (distance < nearestDistance) { nearest = index; nearestDistance = distance; }
    });
    setRailActive(nearest);
  };
  const railKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.target !== event.currentTarget) return;
    if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
      event.preventDefault();
      event.stopPropagation();
      scrollToCard(railActive + (event.key === "ArrowLeft" ? -1 : 1));
    }
  };

  if (loading) return <div className="dashboard-evidence-message">Loading published Dashboard captures…</div>;
  if (!items.length) return <section className="dashboard-evidence-message" role="alert">
    <strong>Dashboard gallery unavailable</strong><p>{error || "This task has no published captures."}</p>
  </section>;

  const chosen = selectedIndex === null ? null : items[selectedIndex] ?? null;
  return <section className="dashboard-evidence-item"
    aria-label={"Wartribes Dashboard review gallery for " + items[0].task}>

    {chosen ? <header className="dashboard-evidence-head dashboard-evidence-detail-head">
      <div className="dashboard-evidence-heading">
        <small>{chosen.task} · Published capture {selectedIndex! + 1} of {items.length}</small>
        <h2>{chosen.title}</h2>
        {chosen.caption && <p>{chosen.caption}</p>}
      </div>
      <div className="dashboard-evidence-meta">
        {chosen.capturedAt && <span>Captured <strong>{displayDate(chosen.capturedAt)}</strong></span>}
        <div className="dashboard-evidence-actions">
          <button type="button" className="button secondary"
            onClick={() => onSelectedIndexChange(null)}>← All captures</button>
          <button type="button" className="button secondary"
            aria-label="Previous capture" disabled={selectedIndex === 0}
            onClick={() => onSelectedIndexChange(Math.max(0, selectedIndex! - 1))}>‹</button>
          <button type="button" className="button secondary"
            aria-label="Next capture" disabled={selectedIndex === items.length - 1}
            onClick={() => onSelectedIndexChange(Math.min(items.length - 1, selectedIndex! + 1))}>›</button>
          {chosen.mediaPath && <button type="button" className="button secondary"
            onClick={() => openSource(chosen)}>Open in Fomni ↗</button>}
        </div>
      </div>
    </header> : <div className="dashboard-evidence-rail-header">
      <span><strong>{items[0].task}</strong> · Original captures · Scroll sideways to compare</span>
      <div className="dashboard-evidence-rail-actions">
        <span aria-live="polite">{railActive + 1} / {items.length}</span>
        <button type="button" className="button secondary"
          aria-label="Scroll to previous capture" disabled={railActive === 0}
          onClick={() => scrollToCard(railActive - 1)}>‹</button>
        <button type="button" className="button secondary"
          aria-label="Scroll to next capture" disabled={railActive >= items.length - 1}
          onClick={() => scrollToCard(railActive + 1)}>›</button>
      </div>
    </div>}

    {chosen ? <div className="dashboard-evidence-media dashboard-evidence-selected">
      <DashboardMedia item={chosen} />
    </div> : <div ref={railRef} className="dashboard-evidence-gallery"
      tabIndex={0} role="region" aria-label="Scrollable Wartribes Dashboard evidence"
      onScroll={syncRailPosition} onKeyDown={railKeyDown}>
      {items.map((item, index) => <article className="dashboard-evidence-card" key={item.id}>
        {isVideo(item) ? <div className="dashboard-evidence-card-media">
          <DashboardMedia item={item} />
        </div> : <button type="button" className="dashboard-evidence-card-media"
          aria-label={"Enlarge " + item.title}
          onClick={() => onSelectedIndexChange(index)}>
          <DashboardMedia item={item} />
        </button>}
        <div className="dashboard-evidence-card-info">
          <div className="dashboard-evidence-card-heading">
            <span className="dashboard-evidence-index">{String(index + 1).padStart(2, "0")}</span>
            <span className="dashboard-evidence-date">{displayDate(item.capturedAt)}</span>
          </div>
          <h3 title={item.title}>{item.title}</h3>
          {item.caption && <p title={item.caption}>{item.caption}</p>}
          <div className="dashboard-evidence-card-actions">
            <button type="button" className="button secondary"
              onClick={() => onSelectedIndexChange(index)}>Inspect larger</button>
            {item.mediaPath && <button type="button" className="button secondary"
              onClick={() => openSource(item)}>Open original ↗</button>}
          </div>
        </div>
      </article>)}
    </div>}
    {error && <div className="error-banner" role="alert">{error}</div>}
  </section>;
}
