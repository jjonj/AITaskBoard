import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { ProjectQueueWorkload, QueueWorkload, WorkloadPulse } from "./api";
import "./QueueIntelligence.css";

type Props = { data: QueueWorkload | null; error: boolean };
type SortMode = "queue" | "closed" | "open";
type WindowDays = 14 | 30;

const fmt = (value: number) => value.toLocaleString();
const signed = (value: number) => (value > 0 ? "+" : "") + fmt(value);
const trend = (current: number, previous: number) => {
  if (!previous) return current ? "New activity" : "No change";
  const pct = Math.round((current - previous) * 100 / previous);
  return (pct > 0 ? "+" : "") + pct + "% vs prior 7d";
};
const prettyDay = (day: string) => {
  const date = new Date(day + "T12:00:00");
  return Number.isNaN(date.getTime()) ? day : date.toLocaleDateString(undefined, { month: "short", day: "numeric" });
};

function FlowChart({ pulse, days }: { pulse: WorkloadPulse; days: WindowDays }) {
  const [hovered, setHovered] = useState<number | null>(null);
  const data = pulse.daily.slice(-days);
  if (!data.length) return <div className="queue-insight-empty">No dated task activity available yet.</div>;
  const width = 548;
  const height = 154;
  const xStart = 34;
  const xEnd = 538;
  const top = 10;
  const bottom = 128;
  const max = Math.max(1, ...data.map((item) => Math.max(item.closed, item.created)));
  const y = (v: number) => bottom - v / max * (bottom - top);
  const x = (i: number) => xStart + i / Math.max(1, data.length - 1) * (xEnd - xStart);
  const toPoints = (field: "closed" | "created") =>
    data.map((item, i) => x(i).toFixed(1) + "," + y(item[field]).toFixed(1)).join(" ");
  const active = hovered === null ? null : Math.max(0, Math.min(data.length - 1, hovered));
  const area = (field: "closed" | "created") =>
    "M" + x(0).toFixed(1) + " " + bottom + " L" +
      data.map((item, i) => x(i).toFixed(1) + " " + y(item[field]).toFixed(1)).join(" L") +
      " L" + x(data.length - 1).toFixed(1) + " " + bottom + " Z";
  return (
    <svg className="queue-insight-chart" viewBox={"0 0 " + width + " " + height} role="img"
      aria-label={days + "-day task flow chart: green completed, blue created"}>
      <defs>
        <linearGradient id="queue-closed-fill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#49d6ad" stopOpacity=".28"/>
          <stop offset="1" stopColor="#49d6ad" stopOpacity=".015"/>
        </linearGradient>
        <linearGradient id="queue-created-fill" x1="0" x2="0" y1="0" y2="1">
          <stop offset="0" stopColor="#81aaff" stopOpacity=".18"/>
          <stop offset="1" stopColor="#81aaff" stopOpacity="0"/>
        </linearGradient>
      </defs>
      {[0, 1, 2, 3].map((tick) => {
        const value = max * tick / 3;
        const yy = y(value);
        return <g key={tick}>
          <line x1={xStart} x2={xEnd} y1={yy} y2={yy} stroke="#293e4c" strokeDasharray="3 5"/>
          <text x={27} y={yy + 3} textAnchor="end" fill="#7792a2" fontSize="10">{Math.round(value)}</text>
        </g>;
      })}
      <path d={area("created")} fill="url(#queue-created-fill)"/>
      <path d={area("closed")} fill="url(#queue-closed-fill)"/>
      <polyline fill="none" stroke="#7ca9ec" strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" points={toPoints("created")}/>
      <polyline fill="none" stroke="#56d6b1" strokeWidth="2.7" strokeLinejoin="round" strokeLinecap="round" points={toPoints("closed")}/>
      {active !== null && <g pointerEvents="none">
        <line x1={x(active)} x2={x(active)} y1={top} y2={bottom} stroke="#adc4d0" strokeDasharray="3 3" opacity=".7" />
        <circle cx={x(active)} cy={y(data[active].closed)} r="4" stroke="#142431" strokeWidth="2" fill="#64e4b9"/>
        <circle cx={x(active)} cy={y(data[active].created)} r="4" stroke="#142431" strokeWidth="2" fill="#91b9fa"/>
        <rect x={x(active) > 400 ? x(active) - 125 : x(active) + 8} y="10" width="117" height="50" rx="6" fill="#263d4d" stroke="#4c697b"/>
        <text x={x(active) > 400 ? x(active) - 116 : x(active) + 17} y="24" fill="#dce9f1" fontSize="10" fontWeight="700">{prettyDay(data[active].day)}</text>
        <text x={x(active) > 400 ? x(active) - 116 : x(active) + 17} y="38" fill="#76edc3" fontSize="10">Closed: {fmt(data[active].closed)}</text>
        <text x={x(active) > 400 ? x(active) - 116 : x(active) + 17} y="51" fill="#a4c4ff" fontSize="10">Created: {fmt(data[active].created)}</text>
      </g>}
      <rect x={xStart} y={top} width={xEnd - xStart} height={bottom - top} fill="transparent"
        onPointerMove={(event) => {
          const svg = event.currentTarget.ownerSVGElement;
          if (!svg) return;
          const box = svg.getBoundingClientRect();
          const canvasX = (event.clientX - box.left) / box.width * width;
          setHovered(Math.max(0, Math.min(data.length - 1,
            Math.round((canvasX - xStart) / (xEnd - xStart) * (data.length - 1)))));
        }}
        onPointerLeave={() => setHovered(null)}
        onClick={(event) => {
          const box = event.currentTarget.ownerSVGElement?.getBoundingClientRect();
          if (!box) return;
          const canvasX = (event.clientX - box.left) / box.width * width;
          setHovered(Math.max(0, Math.min(data.length - 1,
            Math.round((canvasX - xStart) / (xEnd - xStart) * (data.length - 1)))));
        }}
      />
      <text x={xStart} y={149} fill="#97adbd" fontSize="11">{prettyDay(data[0].day)}</text>
      <text x={(xStart + xEnd) / 2} y={149} textAnchor="middle" fill="#97adbd" fontSize="11">{prettyDay(data[Math.floor((data.length - 1) / 2)].day)}</text>
      <text x={xEnd} y={149} textAnchor="end" fill="#97adbd" fontSize="11">{prettyDay(data[data.length - 1].day)}</text>
    </svg>
  );
}

function ProjectRow({ project, maxQueue }: { project: ProjectQueueWorkload; maxQueue: number }) {
  const portion = Math.max(0, Math.min(100, project.count / Math.max(1, maxQueue) * 100));
  const ready = project.count ? project.readyNow / project.count * 100 : 0;
  const change = project.created7 - project.closed7;
  return <div className="queue-insight-project-row">
    <div className="queue-insight-project-top">
      <span className="queue-insight-project-name" title={project.name}>{project.name}</span>
      <span className="queue-insight-project-numbers">
        <strong>{fmt(project.count)}</strong> queued
      </span>
    </div>
    <div className="queue-insight-project-track" title={fmt(project.readyNow) + " dispatchable · " + fmt(project.dependencyWaiting) + " waiting"}>
      <div className="queue-insight-project-total" style={{ width: portion + "%" }}>
        <span style={{ width: ready + "%" }} />
      </div>
    </div>
    <div className="queue-insight-project-bottom">
      <span><i className="queue-insight-dot ready" />{fmt(project.readyNow)} ready</span>
      <span><i className="queue-insight-dot waiting" />{fmt(project.dependencyWaiting)} waiting</span>
      <span className="queue-insight-project-spacer" />
      {project.oldestQueueTaskDays !== null && project.oldestQueueTaskDays >= 14 &&
        <span title="Age of the oldest queued task since its creation (not time spent queued)">{fmt(project.oldestQueueTaskDays)}d oldest</span>}
      <span title="All task types: ordinary and recurring, excluding epics and review items">
        <b>{fmt(project.closed7)}</b> closed / 7d
      </span>
      <span className={change > 0 ? "queue-insight-growth" : change < 0 ? "queue-insight-shrink" : "queue-insight-neutral"}
        title={fmt(project.created7) + " created minus " + fmt(project.closed7) + " closed over 7d"}>
        {signed(change)} net
      </span>
    </div>
  </div>;
}

export function QueueIntelligence({ data, error }: Props) {
  const [open, setOpen] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [days, setDays] = useState<WindowDays>(14);
  const [sort, setSort] = useState<SortMode>("queue");
  const [bounds, setBounds] = useState({ left: 12, top: 46 });
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const closeTimer = useRef<number | undefined>(undefined);
  const clearClose = () => { window.clearTimeout(closeTimer.current); closeTimer.current = undefined; };
  const closeSoon = () => {
    clearClose();
    if (!pinned) closeTimer.current = window.setTimeout(() => setOpen(false), 220);
  };
  useEffect(() => () => window.clearTimeout(closeTimer.current), []);

  useEffect(() => {
    if (!open) return;
    const position = () => {
      const rect = trigger.current?.getBoundingClientRect();
      if (!rect) return;
      const panelWidth = Math.min(634, window.innerWidth - 20);
      setBounds({
        left: Math.max(10, Math.min(rect.left, window.innerWidth - panelWidth - 10)),
        top: rect.bottom + 8,
      });
    };
    const dismiss = (event: PointerEvent) => {
      if (!pinned) return;
      const target = event.target as Node;
      if (!panel.current?.contains(target) && !trigger.current?.contains(target)) {
        setPinned(false);
        setOpen(false);
      }
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") { setPinned(false); setOpen(false); }
    };
    position();
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", escape);
    };
  }, [open, pinned]);

  const pulse = data?.pulse;
  const velocity = pulse ? pulse.closed7 - pulse.created7 : 0;
  const weekPace = pulse?.closed7 ?? 0;
  const weeksToClear = pulse && pulse.closed7 > 0 ? pulse.openTasks / pulse.closed7 : null;
  const fourWeeksOpen = pulse ? Math.max(0, pulse.openTasks - velocity * 4) : 0;
  const projects = data?.projects.slice().sort((a, b) => {
    const sortKey = sort === "closed" ? "closed7" : sort === "open" ? "openTasks" : "count";
    return b[sortKey] - a[sortKey] || a.name.localeCompare(b.name);
  }) ?? [];
  const maxQueue = Math.max(1, ...(data?.projects.map((p) => p.count) ?? [0]));
  const queuePressure = data?.count ? Math.round(data.dependencyWaiting / data.count * 100) : 0;
  const critical = data?.projects.filter((project) => project.count && !project.readyNow).length ?? 0;
  const best = [...(data?.projects ?? [])].sort((a, b) => b.closed7 - a.closed7)[0];

  return <>
    <button ref={trigger} type="button" className={"global-queue-workload" + (error || data?.unavailableBoards.length ? " incomplete" : "") + (open ? " active" : "")}
      aria-label={data ? fmt(data.count) + " queued tasks. Open Keep Busy intelligence." : "Queue intelligence loading"}
      aria-haspopup="dialog" aria-expanded={open} aria-controls="queue-intelligence-panel"
      onMouseEnter={() => { clearClose(); setOpen(true); }}
      onMouseLeave={closeSoon}
      onFocus={() => { clearClose(); setOpen(true); }}
      onClick={() => { clearClose(); setOpen(true); setPinned((value) => !value); }}>
      <span className="global-queue-workload-led" aria-hidden="true" /> Queue
      <strong>{error ? "?" : data ? (data.unavailableBoards.length ? "≥" : "") + fmt(data.count) : "…"}</strong>
      <span className="global-queue-workload-caret" aria-hidden="true">▾</span>
    </button>
    {open && createPortal(
      <div ref={panel} id="queue-intelligence-panel" className="queue-insight-panel" role="dialog"
        aria-label="Keep Busy workload intelligence"
        style={{ left: bounds.left, top: bounds.top, maxHeight: "calc(100vh - " + (bounds.top + 12) + "px)" }}
        onMouseEnter={clearClose} onMouseLeave={closeSoon}>
        <div className="queue-insight-header">
          <div>
            <div className="queue-insight-eyebrow"><span className="queue-insight-live" /> T3 KEEP BUSY · LIVE INTELLIGENCE</div>
            <h2>Workload overview</h2>
            <p>All registered TaskBoards · refreshes every 30 seconds</p>
          </div>
          <button type="button" className={"queue-insight-pin" + (pinned ? " pinned" : "")} title={pinned ? "Unpin panel" : "Pin panel open"} aria-pressed={pinned}
            onClick={() => { clearClose(); setPinned(!pinned); }}>{pinned ? "Pinned" : "Pin"}</button>
        </div>
        {!data ? <div className="queue-insight-empty">{error ? "Workload scan unavailable." : "Scanning all TaskBoards…"}</div>
          : <>
            {error && <div className="queue-insight-warning">The latest refresh failed. Showing previously loaded metrics, which may be stale.</div>}
            {!!data.unavailableBoards.length && <div className="queue-insight-warning">
              Partial data · unavailable: {data.unavailableBoards.join(", ")}. Queue and flow totals are lower bounds.
            </div>}
            <div className="queue-insight-hero">
              <div className="queue-insight-total">
                <span>EXPECTED QUEUED WORK</span>
                <div>{data.unavailableBoards.length ? "≥" : ""}{fmt(data.count)}</div>
                <small>Resolvable queued work, across projects</small>
              </div>
              <div className="queue-insight-split">
                <div><b className="queue-insight-green">{fmt(data.readyNow)}</b><span>Ready for pickup</span></div>
                <div><b className="queue-insight-gold">{fmt(data.dependencyWaiting)}</b><span>Waiting on dependencies</span></div>
              </div>
            </div>
            <div className="queue-insight-metrics">
              <div><span>CLOSED · LAST 7D</span><strong>{fmt(pulse?.closed7 ?? 0)}</strong>
                <small>{trend(pulse?.closed7 ?? 0, pulse?.closedPrev7 ?? 0)}</small></div>
              <div><span>CREATED · LAST 7D</span><strong>{fmt(pulse?.created7 ?? 0)}</strong>
                <small>{trend(pulse?.created7 ?? 0, pulse?.createdPrev7 ?? 0)}</small></div>
              <div><span>NET OPEN FLOW · 7D</span><strong className={velocity > 0 ? "queue-insight-green" : velocity < 0 ? "queue-insight-gold" : ""}>
                {signed(-velocity)}</strong><small>{velocity > 0 ? "Shrinking backlog" : velocity < 0 ? "Growing backlog" : "Balanced throughput"}</small></div>
              <div><span>CLOSED · ALL TIME</span><strong>{fmt(pulse?.closedAllTime ?? 0)}</strong>
                <small>{fmt(pulse?.openTasks ?? 0)} still open</small></div>
            </div>
            <div className="queue-insight-section">
              <div className="queue-insight-section-heading">
                <div><h3>Work completed vs incoming</h3><p>Daily task records, not T3 session events</p></div>
                <div className="queue-insight-segment" aria-label="Chart time range">
                  <button type="button" className={days === 14 ? "selected" : ""} aria-pressed={days === 14} onClick={() => setDays(14)}>14d</button>
                  <button type="button" className={days === 30 ? "selected" : ""} aria-pressed={days === 30} onClick={() => setDays(30)}>30d</button>
                </div>
              </div>
              {pulse && <FlowChart pulse={pulse} days={days} />}
              <div className="queue-insight-legend">
                <span><i className="queue-insight-dot ready"/> Closed ({fmt(days === 14 ? pulse?.daily.slice(-14).reduce((a, b) => a + b.closed, 0) ?? 0 : pulse?.closed30 ?? 0)})</span>
                <span><i className="queue-insight-dot incoming"/> Created ({fmt(days === 14 ? pulse?.daily.slice(-14).reduce((a, b) => a + b.created, 0) ?? 0 : pulse?.created30 ?? 0)})</span>
                <span className="queue-insight-legend-right">Calendar days · local time</span>
              </div>
            </div>
            <div className="queue-insight-insights">
              <div className="queue-insight-insight">
                <span className="queue-insight-insight-icon">↗</span>
                <div><b>4-week trajectory</b><p>If recent creation and completion rates persist, <strong>{fmt(fourWeeksOpen)}</strong> open tasks vs {fmt(pulse?.openTasks ?? 0)} now.</p></div>
              </div>
              <div className="queue-insight-insight">
                <span className="queue-insight-insight-icon">◷</span>
                <div><b>Runway · zero incoming work</b><p>{weeksToClear !== null ? "About " + (weeksToClear >= 10 ? Math.round(weeksToClear) : weeksToClear.toFixed(1)) + " weeks to close all open tasks" : "Not enough recent closures to estimate"} at {fmt(weekPace)} closed per week.</p></div>
              </div>
              <div className="queue-insight-insight">
                <span className="queue-insight-insight-icon">≋</span>
                <div><b>Pipeline concentration</b><p>{queuePressure}% of dispatchable pipeline awaits prerequisites · {critical} projects have queued work but nothing immediately ready.{best?.closed7 ? " Top 7d closer: " + best.name + " (" + fmt(best.closed7) + ")." : ""}</p></div>
              </div>
            </div>
            <div className="queue-insight-section queue-insight-project-section">
              <div className="queue-insight-section-heading">
                <div><h3>Projects <span className="queue-insight-count">{projects.length}</span></h3>
                  <p>Ready vs dependent work · throughput and net task flow</p></div>
                <div className="queue-insight-segment" aria-label="Sort projects">
                  <button type="button" className={sort === "queue" ? "selected" : ""} aria-pressed={sort === "queue"} onClick={() => setSort("queue")}>Queue</button>
                  <button type="button" className={sort === "closed" ? "selected" : ""} aria-pressed={sort === "closed"} onClick={() => setSort("closed")}>Closed</button>
                  <button type="button" className={sort === "open" ? "selected" : ""} aria-pressed={sort === "open"} onClick={() => setSort("open")}>Open</button>
                </div>
              </div>
              <div className="queue-insight-projects">
                {projects.map((project, index) => <ProjectRow key={project.name + index} project={project} maxQueue={maxQueue}/>)}
                {!projects.length && <div className="queue-insight-empty">No registered boards could be scanned.</div>}
              </div>
            </div>
            <div className="queue-insight-footnote">
              <span>METHOD</span> Queue: executable chains only, excluding in-flight, human-held and technically blocked work.
              Flow: ordinary and recurring task records across all loaded boards; not limited to T3.
              Historical dates may be missing from legacy records. Trajectory is a pace scenario, not a promise or ETA.
            </div>
          </>}
      </div>, document.body
    )}
  </>;
}
