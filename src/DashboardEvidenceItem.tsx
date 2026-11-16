import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { openFileInFomni } from "./InboxRichText";
import { useLinkCopyMenu } from "./LinkCopyMenu";
import "./DashboardVideo.css";

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
  mediaStreamUrl?: string;
  posterDataUrl?: string;
  durationSeconds?: number;
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

function DashboardMedia({ item, mediaReady }: { item: DashboardItem; mediaReady: boolean }) {
  const [error, setError] = useState("");
  if (isVideo(item)) {
    if (!mediaReady) return <div className="dashboard-evidence-message" role="status">
      Preparing Dashboard video streaming…
    </div>;
    if (!item.mediaStreamUrl) return <div className="dashboard-evidence-message" role="alert">
      Video location is unavailable; open the original.
    </div>;
    return <div className="dashboard-video-wrapper">
      <video src={item.mediaStreamUrl} poster={item.posterDataUrl} controls playsInline preload="metadata"
        onError={() => setError("Video unavailable or unsupported; try the H.264 MP4 original.")}
        onLoadedMetadata={() => setError("")} />
      {error && <p role="alert">{error}</p>}
    </div>;
  }
  if (!item.mediaDataUrl) return <div className="dashboard-evidence-message" role="alert">
    {item.mediaUnavailable || "Image preview unavailable."}
  </div>;
  return <img src={item.mediaDataUrl} alt={item.title + (item.caption ? " — " + item.caption : "")} />;
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
  const { showCopyMenu, copyMenu } = useLinkCopyMenu(setError);
  const [loading, setLoading] = useState(false);
  const [mediaReady, setMediaReady] = useState(false);
  const [streamError, setStreamError] = useState("");
  const railRef = useRef<HTMLDivElement>(null);
  const closeLightboxRef = useRef<HTMLButtonElement>(null);
  const lightboxRef = useRef<HTMLDivElement>(null);

  // Restore keyboard focus to the thumbnail when the fullscreen dialog closes.
  useEffect(() => {
    if (selectedIndex === null) return;
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    closeLightboxRef.current?.focus();
    return () => { if (previous?.isConnected) previous.focus(); };
  }, [selectedIndex !== null]);

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
    if (!items.some(isVideo)) return;
    let cancelled = false;
    setMediaReady(false);
    setStreamError("");
    void invoke<string>("ensure_review_media_server")
      .then(() => { if (!cancelled) setMediaReady(true); })
      .catch(reason => { if (!cancelled) setStreamError("Video streaming unavailable: " + String(reason)); });
    return () => { cancelled = true; };
  }, [items]);
  useEffect(() => {
    onGalleryCountChange?.(items.length);
  }, [items.length, onGalleryCountChange]);

  const showMediaMenu = (event: React.MouseEvent<HTMLElement>, item: DashboardItem) => {
    if (!item.mediaPath) return;
    showCopyMenu(event, item.mediaPath, { fomniFile: item.mediaPath });
  };
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
  const lightboxKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab") return;
    const buttons = [...(lightboxRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? [])];
    if (!buttons.length) return;
    const active = document.activeElement;
    if (event.shiftKey && active === buttons[0]) {
      event.preventDefault();
      buttons[buttons.length - 1].focus();
    } else if (!event.shiftKey && active === buttons[buttons.length - 1]) {
      event.preventDefault();
      buttons[0].focus();
    }
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

    <div className="dashboard-evidence-rail-header">
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
    </div>

    <div ref={railRef} className="dashboard-evidence-gallery"
      tabIndex={0} role="region" aria-label="Scrollable Wartribes Dashboard evidence"
      onScroll={syncRailPosition} onKeyDown={railKeyDown}>
      {items.map((item, index) => <article className="dashboard-evidence-card" key={item.id}
        onContextMenu={(event) => showMediaMenu(event, item)}>
        {isVideo(item) ? <div className="dashboard-evidence-card-media">
          <DashboardMedia item={item} mediaReady={mediaReady} />
        </div> : <button type="button" className="dashboard-evidence-card-media"
          aria-label={"Enlarge " + item.title}
          onClick={() => onSelectedIndexChange(index)}>
          <DashboardMedia item={item} mediaReady={mediaReady} />
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
    </div>
    {chosen && createPortal(
      <div className="dashboard-lightbox-backdrop"
        onPointerDown={(event) => {
          if (event.target === event.currentTarget) onSelectedIndexChange(null);
        }}>
        <div ref={lightboxRef} role="dialog" aria-modal="true" aria-label={"Enlarged Dashboard capture: " + chosen.title}
          className="dashboard-lightbox" onKeyDown={lightboxKeyDown}>
          <header className="dashboard-lightbox-header">
            <div className="dashboard-lightbox-heading">
              <small>{chosen.task} · Capture {selectedIndex! + 1} of {items.length}</small>
              <strong title={chosen.title}>{chosen.title}</strong>
            </div>
            <div className="dashboard-lightbox-actions">
              <button type="button" className="button secondary" aria-label="Previous image"
                disabled={selectedIndex === 0}
                onClick={() => onSelectedIndexChange(Math.max(0, selectedIndex! - 1))}>‹ Previous</button>
              <button type="button" className="button secondary" aria-label="Next image"
                disabled={selectedIndex === items.length - 1}
                onClick={() => onSelectedIndexChange(Math.min(items.length - 1, selectedIndex! + 1))}>Next ›</button>
              <button ref={closeLightboxRef} type="button" className="button primary"
                title="Return to evidence gallery (Esc)" onClick={() => onSelectedIndexChange(null)}>✕ Close</button>
            </div>
          </header>
          <div className="dashboard-lightbox-media" onContextMenu={(event) => showMediaMenu(event, chosen)}>
            <DashboardMedia item={chosen} mediaReady={mediaReady} />
          </div>
          <footer className="dashboard-lightbox-caption">
            <div>
              {chosen.caption && <p>{chosen.caption}</p>}
              {chosen.capturedAt && <small>Captured {displayDate(chosen.capturedAt)}</small>}
            </div>
            {chosen.mediaPath && <button type="button" className="button secondary"
              onClick={() => openSource(chosen)}>Open original in Fomni ↗</button>}
          </footer>
        </div>
      </div>,
      document.body,
    )}
    {(error || streamError) && <div className="error-banner" role="alert">{error || streamError}</div>}
    {copyMenu}
  </section>;
}
