import { useLayoutEffect, useState, type RefObject } from "react";
import type { Task, Column } from "./types";
import { crossColumnCurve, interveningColumns } from "./dependencyRoutes";

type Rect = { x: number; y: number; width: number; height: number };
type Edge = { id: string; sourceId: string; targetId: string; path: string; sx: number; sy: number };
type ColumnGeometry = {
  id: string;
  rect: DOMRect;
  clip: Rect;
  cards: Map<string, DOMRect>;
};
type CrossEdge = Edge & { occluders: Rect[] };
type Layout = {
  width: number;
  height: number;
  boardClip: Rect;
  columns: Array<{ id: string; clip: Rect; edges: Edge[] }>;
  crossEdges: CrossEdge[];
};
const emptyLayout: Layout = {
  width: 1, height: 1, boardClip: { x: 0, y: 0, width: 0, height: 0 },
  columns: [], crossEdges: [],
};

const intersection = (rects: Array<Rect | DOMRect>): Rect => {
  const left = Math.max(...rects.map((r) => r.x));
  const top = Math.max(...rects.map((r) => r.y));
  const right = Math.min(...rects.map((r) => r.x + r.width));
  const bottom = Math.min(...rects.map((r) => r.y + r.height));
  return { x: left, y: top, width: Math.max(0, right - left), height: Math.max(0, bottom - top) };
};

const centerY = (rect: DOMRect) => rect.top + rect.height / 2;
const isVisibleCenter = (rect: DOMRect, clip: Rect) => {
  const y = centerY(rect);
  return y >= clip.y && y <= clip.y + clip.height &&
    rect.right > clip.x && rect.left < clip.x + clip.width;
};
const fmt = (value: number) => value.toFixed(1);

/** Fixed overlay that follows card scrolling without changing board geometry.
 * Cross-column paths are hidden only where they would cover unrelated cards.
 */
export function DependencyArrows({
  boardRef, visibleTasks, columns, layoutKey, focusedTaskId,
}: {
  boardRef: RefObject<HTMLElement | null>;
  visibleTasks: Task[];
  columns: Column[];
  layoutKey: string;
  focusedTaskId?: string | null;
}) {
  const [layout, setLayout] = useState<Layout>(emptyLayout);

  useLayoutEffect(() => {
    const board = boardRef.current;
    if (!board) return;
    let frame = 0;
    let disposed = false;

    const measure = () => {
      frame = 0;
      if (disposed) return;
      const width = window.innerWidth;
      const height = window.innerHeight;
      const viewport: Rect = { x: 0, y: 0, width, height };
      const boardClip = intersection([board.getBoundingClientRect(), viewport]);
      const byId = new Map(visibleTasks.map((task) => [task.id, task]));
      const terminalColumns = new Set(columns.filter((column) => column.terminal).map((column) => column.id));
      const geometry = new Map<string, ColumnGeometry>();

      const columnNodes = Array.from(board.querySelectorAll<HTMLElement>(".column"));
      for (const column of columns) {
        if (column.hidden) continue;
        const el = columnNodes.find((node) => node.dataset.columnId === column.id);
        const list = el?.querySelector<HTMLElement>(".cards");
        if (!el || !list) continue;
        const rect = el.getBoundingClientRect();
        const clip = intersection([boardClip, rect, list.getBoundingClientRect()]);
        if (clip.width < 8 || clip.height < 8) continue;
        const cards = new Map<string, DOMRect>();
        for (const node of list.querySelectorAll<HTMLElement>("[data-task-id]")) {
          if (node.dataset.taskId) cards.set(node.dataset.taskId, node.getBoundingClientRect());
        }
        geometry.set(column.id, { id: column.id, rect, clip, cards });
      }

      const sameColumn = new Map<string, Edge[]>();
      const crossEdges: CrossEdge[] = [];

      for (const dependent of visibleTasks) {
        const destination = geometry.get(dependent.column);
        const dependentRect = destination?.cards.get(dependent.id);
        if (!destination || !dependentRect || !isVisibleCenter(dependentRect, destination.clip)) continue;

        for (const prerequisiteId of dependent.depends_on || []) {
          const prerequisite = byId.get(prerequisiteId);
          // Completed prerequisites no longer block anything; draw no red
          // arrow from Done even if the column is visible in the viewport.
          if (!prerequisite || prerequisite.archived || prerequisite.completed ||
              terminalColumns.has(prerequisite.column)) continue;
          const source = geometry.get(prerequisite.column);
          const prerequisiteRect = source?.cards.get(prerequisiteId);
          if (!source || !prerequisiteRect || !isVisibleCenter(prerequisiteRect, source.clip)) continue;

          const id = `${prerequisiteId}->${dependent.id}`;
          const sy = centerY(prerequisiteRect);
          const ty = centerY(dependentRect);

          if (source.id === destination.id) {
            // Use the existing compact elbow, overlaid on the right edge of
            // the card. No padding / gutter is added to the card list.
            const sx = prerequisiteRect.right - 16;
            const tx = dependentRect.right - 17;
            const railX = Math.max(Math.min(source.rect.right - 13, source.clip.x + source.clip.width - 13), sx + 5, tx + 5);
            const bend = Math.min(9, Math.max(3, Math.abs(ty - sy) / 3));
            const sign = ty >= sy ? 1 : -1;
            const path = [
              `M ${fmt(sx)} ${fmt(sy)}`,
              `L ${fmt(railX - bend)} ${fmt(sy)}`,
              `Q ${fmt(railX)} ${fmt(sy)} ${fmt(railX)} ${fmt(sy + bend * sign)}`,
              `L ${fmt(railX)} ${fmt(ty - bend * sign)}`,
              `Q ${fmt(railX)} ${fmt(ty)} ${fmt(railX - bend)} ${fmt(ty)}`,
              `L ${fmt(tx)} ${fmt(ty)}`,
            ].join(" ");
            if (!sameColumn.has(source.id)) sameColumn.set(source.id, []);
            sameColumn.get(source.id)!.push({ id, sourceId: prerequisiteId, targetId: dependent.id, path, sx, sy });
            continue;
          }

          // Cross-column edges start on the card edge facing the destination,
          // flow across the board, and end at the dependent's facing edge.
          const rightward = source.rect.left < destination.rect.left;
          const sx = rightward ? prerequisiteRect.right - 5 : prerequisiteRect.left + 5;
          const tx = rightward ? dependentRect.left + 7 : dependentRect.right - 7;
          const path = crossColumnCurve(sx, sy, tx, ty, rightward ? 1 : -1);

          // Mask actual card faces, not whole intervening columns: the old
          // column-wide mask made long dependencies appear disconnected.
          const between = interveningColumns(source, destination, [...geometry.values()]);
          const occluders = [...geometry.values()]
            .filter((geo) => between.some((bounds) =>
              geo.rect.x + geo.rect.width / 2 > bounds.x &&
              geo.rect.x + geo.rect.width / 2 < bounds.x + bounds.width))
            .flatMap((geo) => [...geo.cards.values()].map((card) => {
              const clip = intersection([geo.clip, card]);
              return { x: clip.x - 3, y: clip.y - 3, width: clip.width + 6, height: clip.height + 6 };
            }).filter((rect) => rect.width > 6 && rect.height > 6));
          crossEdges.push({ id, sourceId: prerequisiteId, targetId: dependent.id, path, sx, sy, occluders });
        }
      }

      setLayout({
        width, height, boardClip,
        columns: [...geometry.values()].map((geo) => ({
          id: geo.id, clip: geo.clip, edges: sameColumn.get(geo.id) || [],
        })),
        crossEdges,
      });
    };

    const schedule = () => {
      if (!frame && !disposed) frame = requestAnimationFrame(measure);
    };
    const resize = new ResizeObserver(schedule);
    resize.observe(board);
    for (const el of board.querySelectorAll<HTMLElement>(".cards, .column")) resize.observe(el);
    board.addEventListener("scroll", schedule, true);
    window.addEventListener("resize", schedule);
    schedule();
    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      resize.disconnect();
      board.removeEventListener("scroll", schedule, true);
      window.removeEventListener("resize", schedule);
    };
  }, [boardRef, visibleTasks, columns, layoutKey]);

  if (!layout.crossEdges.length && !layout.columns.some((col) => col.edges.length)) return null;

  const drawEdge = (edge: Edge) => (
    <g key={edge.id} className={focusedTaskId
      ? edge.sourceId === focusedTaskId || edge.targetId === focusedTaskId ? "dependency-link-focus" : "dependency-link-muted"
      : undefined}>
      <path className="dependency-line-halo" d={edge.path} />
      <path className="dependency-line" d={edge.path} markerEnd="url(#tb-dependency-arrow)" />
      <circle className="dependency-line-origin" cx={edge.sx} cy={edge.sy} r="3" />
    </g>
  );

  return (
    <svg className="dependency-arrows-overlay" viewBox={`0 0 ${layout.width} ${layout.height}`}
      width={layout.width} height={layout.height} aria-hidden="true">
      <defs>
        <marker id="tb-dependency-arrow" markerWidth="7" markerHeight="7"
          refX="6" refY="3.5" orient="auto" markerUnits="userSpaceOnUse">
          <path d="M 0 0 L 7 3.5 L 0 7 z" fill="#ff5968" />
        </marker>
        <clipPath id="tb-dependency-board-clip">
          <rect {...layout.boardClip} />
        </clipPath>
        {layout.columns.map((column, index) =>
          <clipPath key={column.id} id={`tb-dep-clip-${index}`}>
            <rect {...column.clip} />
          </clipPath>
        )}
        {layout.crossEdges.map((edge, index) =>
          <mask key={edge.id} id={`tb-dependency-under-${index}`}
            x="0" y="0" width={layout.width} height={layout.height}
            maskUnits="userSpaceOnUse" maskContentUnits="userSpaceOnUse">
            <rect x="0" y="0" width={layout.width} height={layout.height} fill="white" />
            {edge.occluders.map((rect, i) => <rect key={i} {...rect} fill="black" />)}
          </mask>
        )}
      </defs>
      <g clipPath="url(#tb-dependency-board-clip)">
        {layout.crossEdges.map((edge, index) =>
          <g key={edge.id} mask={`url(#tb-dependency-under-${index})`}>
            {drawEdge(edge)}
          </g>
        )}
        {layout.columns.map((column, index) =>
          <g key={column.id} clipPath={`url(#tb-dep-clip-${index})`}>
            {column.edges.map(drawEdge)}
          </g>
        )}
      </g>
    </svg>
  );
}
