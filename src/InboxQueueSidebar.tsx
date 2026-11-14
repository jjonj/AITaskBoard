import type { Task } from "./types";

export type InboxQueueItem = { path: string; project: string; task: Task };
const keyOf = (item: InboxQueueItem) => item.path + "|" + item.task.id;

type Props = {
  items: InboxQueueItem[];
  selectedKey: string;
  onSelect: (key: string) => void;
  allBoards: boolean;
  showResolved: boolean;
  onAllBoards: (value: boolean) => void;
  onShowResolved: (value: boolean) => void;
  thumbnails: Record<string, string>;
  modes: Record<string, string>;
  counts: Record<string, number>;
};

/** One visual/interaction contract for the compact and expanded Inbox sidebar. */
export function InboxQueueSidebar({
  items, selectedKey, onSelect, allBoards, showResolved, onAllBoards,
  onShowResolved, thumbnails, modes, counts,
}: Props) {
  return <aside className="review-queue">
    <header><strong>Feedback inbox <span>{items.length}</span></strong></header>
    <div className="review-queue-filters">
      <label className="review-all-boards">
        <input type="checkbox" checked={allBoards} onChange={(event) => onAllBoards(event.target.checked)} />
        All boards
      </label>
      <label className="review-all-boards">
        <input type="checkbox" checked={showResolved} onChange={(event) => onShowResolved(event.target.checked)} />
        Include past decisions
      </label>
    </div>
    <nav className="review-queue-list" aria-label="Unified human feedback queue">
      {items.map((item) => {
        const key = keyOf(item);
        const mode = modes[key] || (item.task.review_status ? "review" : "user");
        return <button key={key} type="button" title={item.task.title}
          aria-current={key === selectedKey ? "true" : undefined}
          className={["review-queue-item", thumbnails[key] && "has-preview",
            key === selectedKey && "selected",
            item.task.review_status !== "open" && !!item.task.review_status && "resolved",
            !item.task.review_status && "review-user-item",
          ].filter(Boolean).join(" ")}
          onClick={() => onSelect(key)}>
          {thumbnails[key] && <span className="review-queue-media" aria-hidden="true">
            <img className="review-queue-preview" src={thumbnails[key]} alt="" />
          </span>}
          <span className="review-queue-copy">
            <small className="inbox-queue-breadcrumb">{item.project} · {item.task.id}
              {item.task.attention.includes("priority") &&
                <span className="inbox-priority-mark" title="Priority">! Priority</span>}
            </small>
            <strong>{item.task.title}</strong>
            <span className="review-queue-meta">
              <em className={"review-queue-mode review-queue-mode-" + mode}>
                {modes[key] || (item.task.review_status ? "review" : "response")}
              </em>
              {item.task.review_status === "awaiting_revision" ? "Rejected · awaiting AI revision" :
                item.task.review_status === "resolved" ? "Resolved" :
                item.task.review_round ? "Round " + item.task.review_round : "Awaiting response"}
              {(counts[key] ?? 0) > 1 && " · " + counts[key] + " candidates"}
            </span>
          </span>
        </button>;
      })}
    </nav>
    {!items.length && <p className="review-empty">No pending feedback.</p>}
  </aside>;
}
