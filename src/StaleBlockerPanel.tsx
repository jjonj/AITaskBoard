import type { StaleBlocker } from "./api";

type Props = {
  items: StaleBlocker[];
  onClose: () => void;
  onOpenTask: (id: string) => void;
  onAcknowledge: (item: StaleBlocker) => void;
  busy: boolean;
};

/** Read-only review candidates. No agent is spawned and nothing is unblocked. */
export function StaleBlockerPanel({ items, onClose, onOpenTask, onAcknowledge, busy }: Props) {
  return <div className="stale-blocker-overlay" role="presentation" onMouseDown={onClose}>
    <aside className="stale-blocker-panel" role="dialog" aria-modal="true"
      aria-label="Stale technical blocker review" onMouseDown={event => event.stopPropagation()}>
      <header><div><h2>Technical blocker rechecks</h2>
        <p>Low priority, after seven idle days. Dependency waits are excluded. No agents are started.</p>
      </div><button type="button" className="button secondary" onClick={onClose}>Close</button></header>
      {!items.length ? <p className="stale-blocker-empty">No eligible stale technical blockers.</p> :
        <div className="stale-blocker-list">{items.map(item => <section key={item.id}>
          <div className="stale-blocker-heading"><strong>{item.id}</strong> {item.title}</div>
          <div className="stale-blocker-subtitle">Last task activity: {item.ageDays} days ago</div>
          <p>{item.reason}</p>
          <div className="stale-blocker-actions">
            <button type="button" className="button secondary" onClick={() => onOpenTask(item.id)}>Inspect task</button>
            <button type="button" className="button secondary" disabled={busy}
              title="Hide this reminder until the actual blocker, instructions or prerequisites change"
              onClick={() => onAcknowledge(item)}>Reviewed · hide until changed</button>
          </div>
        </section>)}</div>}
    </aside>
  </div>;
}
