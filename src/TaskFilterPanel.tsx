import type { ReactNode } from "react";
import { TASK_ICON_OPTIONS, TaskIconGlyph } from "./TaskIcons";
import type { TaskAttention, TaskIcon, TaskKind } from "./types";

export type TaskReadinessFilter = "ready" | "waiting" | "blocked" | "for_user" | "epic";

export type TaskFilterSelection = {
  icons: TaskIcon[];
  noIcon: boolean;
  columns: string[];
  readiness: TaskReadinessFilter[];
  kinds: TaskKind[];
  attention: TaskAttention[];
};

type FilterColumn = { id: string; name: string };

type Props = {
  filters: TaskFilterSelection;
  columns: FilterColumn[];
  matchingCount: number;
  onChange: (filters: TaskFilterSelection) => void;
  onClear: () => void;
  onClose: () => void;
};

const readinessOptions: { value: TaskReadinessFilter; label: string }[] = [
  { value: "ready", label: "Ready" },
  { value: "waiting", label: "Waiting on dependencies" },
  { value: "blocked", label: "Blocked" },
  { value: "for_user", label: "For user" },
  { value: "epic", label: "Epic" },
];

const kindOptions: { value: TaskKind; label: string }[] = [
  { value: "task", label: "Task" },
  { value: "epic", label: "Epic" },
  { value: "for_user", label: "For user" },
  { value: "recurring", label: "Recurring" },
  { value: "release", label: "Release" },
];

const attentionOptions: { value: TaskAttention; label: string }[] = [
  { value: "question", label: "Question" },
  { value: "user-note", label: "User note" },
  { value: "blocked", label: "Blocked flag" },
  { value: "priority", label: "Priority" },
];

export function TaskFilterPanel({ filters, columns, matchingCount, onChange, onClear, onClose }: Props) {
  const toggle = <K extends Exclude<keyof TaskFilterSelection, "noIcon">>(
    key: K,
    value: TaskFilterSelection[K][number],
  ) => {
    const selected = filters[key] as string[];
    const next = selected.includes(String(value))
      ? selected.filter(item => item !== value)
      : [...selected, value];
    onChange({ ...filters, [key]: next } as TaskFilterSelection);
  };

  const option = (key: Exclude<keyof TaskFilterSelection, "noIcon">, value: string, label: string, icon?: TaskIcon) => {
    const selected = filters[key] as string[];
    return (
      <label className="task-filter-option" key={value}>
        <input type="checkbox" checked={selected.includes(value)}
          onChange={() => toggle(key, value as never)} />
        {icon ? <TaskIconGlyph icon={icon} size={17} /> : null}
        <span>{label}</span>
      </label>
    );
  };

  const group = (title: string, children: ReactNode, className = "") => (
    <section className={`task-filter-group ${className}`}>
      <h3>{title}</h3>
      <div className="task-filter-options">{children}</div>
    </section>
  );

  const activeCount = filters.icons.length + Number(filters.noIcon) + filters.columns.length +
    filters.readiness.length + filters.kinds.length + filters.attention.length;

  return (
    <section className="task-filter-panel" role="dialog" aria-label="Filter tasks">
      <header className="task-filter-header">
        <div>
          <strong>Filter tasks</strong>
          <span>{activeCount ? `${activeCount} selected` : "No filters active"}</span>
        </div>
        <div className="task-filter-header-actions">
          <button type="button" className="task-filter-clear" onClick={onClear} disabled={!activeCount}>
            Clear all
          </button>
          <button type="button" className="task-filter-close" onClick={onClose} aria-label="Close filters">×</button>
        </div>
      </header>

      {group("Task icon", <>
        <label className="task-filter-option">
          <input type="checkbox" checked={filters.noIcon}
            onChange={event => onChange({ ...filters, noIcon: event.currentTarget.checked })} />
          <span className="task-filter-empty-icon">—</span>
          <span>No icon</span>
        </label>
        {TASK_ICON_OPTIONS.map(item => option("icons", item.value, item.label, item.value))}
      </>, "task-filter-icons")}

      {group("Column", columns.map(column => option("columns", column.id, column.name)))}
      {group("Readiness", readinessOptions.map(item => option("readiness", item.value, item.label)), "task-filter-grid")}
      {group("Task type", kindOptions.map(item => option("kinds", item.value, item.label)), "task-filter-grid")}
      {group("Attention", attentionOptions.map(item => option("attention", item.value, item.label)), "task-filter-grid")}

      <footer className="task-filter-footer">
        Showing <strong>{matchingCount}</strong> matching cards
      </footer>
    </section>
  );
}
