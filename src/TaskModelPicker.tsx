import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { taskModelFavorites } from "./api";
import { taskModelKey } from "./taskModels";
import { ProviderIcon } from "./TaskProviderIcons";
import { orderTaskModelFavorites } from "./modelFavoriteOrdering";
import type { TaskModelSelection } from "./types";
import "./taskModels.css";

type Favorite = { provider: string; model: string };
type Entry = Favorite & { key: string; active: boolean };
type Position = { top: number; left: number; width: number; maxHeight: number };

const EFFORTS = [
  { id: "low", label: "Low" },
  { id: "medium", label: "Medium" },
  { id: "high", label: "High" },
  { id: "xhigh", label: "Extra High" },
] as const;

function modelName(slug: string) {
  const segments = slug.split("/");
  const end = segments[segments.length - 1] ?? slug;
  if (end.toLowerCase().startsWith("gpt-")) {
    return end.replace(/^gpt-/i, "GPT-").replace(/-(sol|astra|terra|luna)$/i, (_, name: string) => `-${name[0].toUpperCase()}${name.slice(1).toLowerCase()}`);
  }
  if (end.toLowerCase().startsWith("claude-")) {
    return end.replace(/^claude-/i, "Claude ").replace(/-(\d+)-(\d+)$/, " $1.$2")
      .replace(/\b(opus|sonnet|fable|haiku)\b/gi, name => name[0].toUpperCase() + name.slice(1).toLowerCase());
  }
  return end.replace(/[-_]/g, " ").replace(/\b[a-z]/g, letter => letter.toUpperCase());
}

function providerLabel(provider: string) {
  if (provider === "claudeAgent" || provider.startsWith("claudeAgent_")) return "Claude";
  if (provider === "codex" || provider.startsWith("codex_")) return "Codex";
  if (provider === "opencode" || provider.startsWith("opencode_")) return "OpenCode";
  return provider;
}

/** Floating T3-style two-pane picker; portaled to avoid the editor drawer's overflow clipping. */
export function TaskModelPicker({
  value, onChange, caption = "Model",
}: {
  value: TaskModelSelection | null;
  onChange: (next: TaskModelSelection | null) => void;
  caption?: string;
}) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [favorites, setFavorites] = useState<Favorite[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const [position, setPosition] = useState<Position>({ top: 8, left: 8, width: 524, maxHeight: 386 });
  const triggerRef = useRef<HTMLButtonElement>(null);
  const panelRef = useRef<HTMLElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    let mounted = true;
    const refresh = () => void taskModelFavorites().then(rows => {
      if (mounted) {
        setFavorites(rows);
        setError(null);
      }
    }).catch(reason => {
      if (mounted) setError(String(reason));
    });
    refresh();
    if (!open) return () => { mounted = false; };
    const timer = window.setInterval(refresh, 4000);
    return () => {
      mounted = false;
      window.clearInterval(timer);
    };
  }, [open]);

  const selectedKey = value ? taskModelKey(value.provider, value.model) : "chatgpt";
  const options = useMemo(() => {
    const rows = new Map<string, Entry>();
    for (const favorite of favorites) {
      const key = taskModelKey(favorite.provider, favorite.model);
      rows.set(key, { ...favorite, key, active: true });
    }
    if (value) {
      const key = taskModelKey(value.provider, value.model);
      if (!rows.has(key)) rows.set(key, { provider: value.provider, model: value.model, key, active: false });
    }
    return orderTaskModelFavorites([...rows.values()]);
  }, [favorites, value]);

  const filtered = useMemo(() => {
    const query = search.trim().toLowerCase();
    const rows: Array<{ key: string; provider: string; model: string; active: boolean }> = [
      { key: "chatgpt", provider: "chatgpt", model: "ChatGPT", active: true }, ...options,
    ];
    if (!query) return rows;
    return rows.filter(row => `${modelName(row.model)} ${row.provider} ${providerLabel(row.provider)}`
      .toLowerCase().includes(query));
  }, [search, options]);

  const reposition = useCallback(() => {
    const rect = triggerRef.current?.getBoundingClientRect();
    if (!rect) return;
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const width = Math.min(524, Math.max(240, viewportWidth - 16));
    const maxHeight = Math.min(386, Math.max(160, viewportHeight - 16));
    const left = Math.max(8, Math.min(rect.right - width, viewportWidth - width - 8));
    const below = rect.bottom + 6;
    const above = rect.top - maxHeight - 6;
    const top = below + maxHeight < viewportHeight - 8 ? below
      : above >= 8 ? above : Math.max(8, Math.min(below, viewportHeight - maxHeight - 8));
    setPosition({ top, left, width, maxHeight });
  }, []);

  const close = useCallback((restoreFocus = false) => {
    setOpen(false);
    if (restoreFocus) triggerRef.current?.focus();
  }, []);

  useEffect(() => {
    if (!open) return;
    reposition();
    const frame = window.requestAnimationFrame(() => inputRef.current?.focus());
    const outside = (event: PointerEvent) => {
      if (panelRef.current?.contains(event.target as Node) || triggerRef.current?.contains(event.target as Node)) return;
      close();
    };
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      event.preventDefault();
      close(true);
    };
    window.addEventListener("resize", reposition);
    document.addEventListener("scroll", reposition, true);
    document.addEventListener("pointerdown", outside, true);
    document.addEventListener("keydown", onKey, true);
    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", reposition);
      document.removeEventListener("scroll", reposition, true);
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", onKey, true);
    };
  }, [open, reposition, close]);

  const choose = (key: string) => {
    if (key === "chatgpt") {
      onChange(null);
      close(true);
      return;
    }
    const entry = options.find(row => row.key === key);
    if (!entry) return;
    if (selectedKey === key) return;
    onChange({ provider: entry.provider, model: entry.model, effort: "medium" });
    inputRef.current?.focus();
  };

  const onSearchKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      setActiveIndex(previous => Math.max(0, Math.min(filtered.length - 1,
        previous + (event.key === "ArrowDown" ? 1 : -1))));
    } else if (event.key === "Enter" && filtered.length) {
      event.preventDefault();
      choose(filtered[Math.min(activeIndex, filtered.length - 1)].key);
    }
  };

  const display = value ? modelName(value.model) : "ChatGPT";
  return <div className="task-model-picker">
    <span className="task-model-field-label">{caption}</span>
    <button ref={triggerRef} type="button" className="task-model-trigger" aria-label={`${caption}: ${display}`}
      aria-haspopup="dialog" aria-expanded={open} onClick={() => {
        if (open) { close(true); return; }
        setSearch("");
        setActiveIndex(Math.max(0, filtered.findIndex(row => row.key === selectedKey)));
        setOpen(true);
      }}>
      <ProviderIcon provider={value?.provider ?? "chatgpt"} />
      <span className="task-model-trigger-value" title={display}>{display}</span>
      {value && <span className="task-model-trigger-effort">{value.effort === "xhigh" ? "Extra High" :
        value.effort[0].toUpperCase() + value.effort.slice(1)}</span>}
      <span className="task-model-chevron" aria-hidden="true">⌄</span>
    </button>
    {open && createPortal(
      <section ref={panelRef} role="dialog" aria-label={caption} className="task-model-popover"
        style={{ top: position.top, left: position.left, width: position.width, maxHeight: position.maxHeight }}
        onKeyDown={event => {
          const digit = /^[1-9]$/.test(event.key) ? Number(event.key) : 0;
          if (!digit || (!event.altKey && event.target === inputRef.current)) return;
          const row = filtered[digit - 1];
          if (!row) return;
          event.preventDefault();
          event.stopPropagation();
          choose(row.key);
        }}>
        <div className="task-model-list-pane">
          <div className="task-model-search">
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor"
              strokeWidth="1.8" strokeLinecap="round" aria-hidden="true">
              <circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" />
            </svg>
            <input ref={inputRef} type="search" placeholder="Search models…" aria-label="Search models"
              value={search} onChange={event => { setSearch(event.currentTarget.value); setActiveIndex(0); }}
              onKeyDown={onSearchKeyDown} />
          </div>
          <div className="task-model-options" role="group" aria-label="Available task models">
            {filtered.map((row, index) => (
              <button key={row.key} type="button" className={`task-model-option${row.key === selectedKey ? " selected" : ""}${index === activeIndex ? " keyboard-active" : ""}`}
                aria-pressed={row.key === selectedKey}
                aria-keyshortcuts={index < 9 ? `Alt+${index + 1}` : undefined}
                title={`${row.model} · ${providerLabel(row.provider)}`}
                onClick={() => choose(row.key)}>
                <ProviderIcon provider={row.provider} />
                <span className="task-model-option-text">
                  <span>{modelName(row.model)}</span>
                  {!row.active && <small>No longer favorited</small>}
                </span>
                <span className="task-model-row-index" aria-hidden="true">
                  {index < 9 ? index + 1 : ""}
                </span>
                <span className={`task-model-row-check${row.key === selectedKey ? " visible" : ""}`}
                  aria-hidden="true">✓</span>
              </button>
            ))}
            {!filtered.length && <p className="task-model-empty">No models match “{search}”.</p>}
          </div>
          {error && <small className="task-model-fetch-error" title={error}>
            T3 favorites unavailable; showing last known models.
          </small>}
        </div>
        <div className="task-model-effort-pane">
          <header>
            <span>Reasoning</span>
            <small title={display}>{display}</small>
          </header>
          {value ? <div className="task-model-effort-options" role="group" aria-label="Reasoning effort">
            {EFFORTS.map(effort => <button type="button" key={effort.id}
              aria-pressed={value.effort === effort.id}
              className={`task-model-effort-option${value.effort === effort.id ? " selected" : ""}`}
              onClick={() => { onChange({ ...value, effort: effort.id }); close(true); }}>
              <span>{effort.label}</span>
              {effort.id === "medium" && <small>Default</small>}
              {value.effort === effort.id && <span className="task-model-selected-check" aria-hidden="true">✓</span>}
            </button>)}
          </div> : <p className="task-model-chatgpt-message">
            ChatGPT tasks open in browser sessions. Reasoning effort is managed inside ChatGPT.
          </p>}
          <footer>Model favorites from T3 Code</footer>
        </div>
      </section>, document.body)}
  </div>;
}
